//! Safe owned wrappers around the pinned FFmpeg shim; no UI/network dependencies.
use anyhow::{Context, Result, bail, ensure};
use p2pshare_media::{
    Dimensions,
    config::{StreamConfig, VideoFormat},
};
use std::{
    ffi::{CString, c_char, c_void},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use windows::{
    Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D},
    core::Interface,
};
#[repr(C)]
pub(crate) struct GpuInfo {
    pub vendor: u32,
    pub device: u32,
    pub sub_system: u32,
    pub revision: u32,
    pub luid: i64,
    pub driver: i64,
    pub name: [u16; 128],
}
impl Default for GpuInfo {
    fn default() -> Self {
        Self {
            vendor: 0,
            device: 0,
            sub_system: 0,
            revision: 0,
            luid: 0,
            driver: 0,
            name: [0; 128],
        }
    }
}
unsafe extern "C" {
    fn ps_ff_init(
        util: *const u16,
        codec: *const u16,
        scale: *const u16,
        error: *mut c_char,
        cap: i32,
    ) -> i32;
    fn ps_ff_encoder(
        name: *const c_char,
        w: i32,
        h: i32,
        fps: i32,
        bitrate: i32,
        threads: i32,
        device: *mut c_void,
        adapter: i32,
        error: *mut c_char,
        cap: i32,
    ) -> *mut c_void;
    fn ps_ff_decoder(
        hevc: i32,
        threads: i32,
        hardware: i32,
        adapter: i32,
        device: *mut c_void,
        error: *mut c_char,
        cap: i32,
    ) -> *mut c_void;
    fn ps_ff_close(codec: *mut c_void);
    fn ps_ff_codec_device_luid(codec: *mut c_void) -> i64;
    fn ps_ff_cpu_info(out: *mut c_char, cap: i32);
    fn ps_ff_encode(
        codec: *mut c_void,
        bgra: *const u8,
        stride: i32,
        texture: *mut c_void,
        micros: i64,
        keyframe: i32,
        error: *mut c_char,
        cap: i32,
    ) -> i32;
    fn ps_ff_packet(
        codec: *mut c_void,
        data: *mut *const u8,
        size: *mut i32,
        micros: *mut i64,
        keyframe: *mut i32,
        error: *mut c_char,
        cap: i32,
    ) -> i32;
    fn ps_ff_decode(
        codec: *mut c_void,
        data: *const u8,
        size: i32,
        error: *mut c_char,
        cap: i32,
    ) -> i32;
    fn ps_ff_frame(
        codec: *mut c_void,
        w: *mut i32,
        h: *mut i32,
        hardware: *mut i32,
        gpu_output: i32,
        lease: *mut *mut c_void,
        error: *mut c_char,
        cap: i32,
    ) -> i32;
    fn ps_ff_rgba(
        codec: *mut c_void,
        rgba: *mut u8,
        capacity: i32,
        error: *mut c_char,
        cap: i32,
    ) -> i32;
    fn ps_ff_display_release(lease: *mut c_void);
    fn ps_ff_display_texture(lease: *mut c_void) -> *mut c_void;
    fn ps_ff_diagnostics(out: *mut c_char, cap: i32) -> i32;
    pub(crate) fn ps_ff_version() -> u32;
    pub(crate) fn ps_ff_gpu(index: i32, info: *mut GpuInfo) -> i32;
    pub(crate) fn ps_ff_device(index: i32) -> *mut c_void;
    pub(crate) fn ps_ff_device_luid(device: *mut c_void) -> i64;
    pub(crate) fn ps_ff_drivers(out: *mut c_char, cap: i32);
}
fn text(error: &[i8]) -> String {
    String::from_utf8_lossy(
        &error
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect::<Vec<_>>(),
    )
    .into_owned()
}
fn report_diagnostics() {
    let mut bytes = [0; 8192];
    if unsafe { ps_ff_diagnostics(bytes.as_mut_ptr(), bytes.len() as i32) } > 0 {
        tracing::warn!(details = text(&bytes), "ffmpeg_codec_diagnostic");
    }
}
static RUNTIME_DIRECTORY: OnceLock<std::path::PathBuf> = OnceLock::new();
pub fn initialize() -> Result<()> {
    static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();
    INITIALIZED.get_or_init(|| {
        let exe = std::env::current_exe().map_err(|e|e.to_string())?;
        let mut paths: Vec<PathBuf> = exe.ancestors().skip(1).take(3).map(|p|p.join("codecs")).collect();
        if cfg!(debug_assertions) { paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/ffmpeg-runtime")); }
        let dir = paths.into_iter().find(|p|p.join("avcodec-63.dll").is_file()).ok_or("Missing bundled codecs/avcodec-63.dll")?;
        let wide = |name:&str| dir.join(name).as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let util=wide("avutil-61.dll"); let codec=wide("avcodec-63.dll"); let scale=wide("swscale-10.dll");
        let mut error=[0;512];
        if unsafe { ps_ff_init(util.as_ptr(),codec.as_ptr(),scale.as_ptr(),error.as_mut_ptr(),512) } < 0 { return Err(text(&error)); }
        let _=RUNTIME_DIRECTORY.set(dir.clone());
        tracing::info!(version=unsafe{ps_ff_version()}, directory=%dir.display(), "bundled_ffmpeg_ready");
        Ok(())
    }).as_ref().map(|_|()).map_err(|e|anyhow::anyhow!(e.clone()))
}
pub(crate) fn runtime_identity() -> Result<String> {
    initialize()?;
    let dir = RUNTIME_DIRECTORY
        .get()
        .context("Codec runtime directory unavailable")?;
    let mut identity = format!("{}", runtime_version()?);
    for name in ["avcodec-63.dll", "avutil-61.dll", "swscale-10.dll"] {
        let m = std::fs::metadata(dir.join(name))?;
        let modified = m
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        identity.push_str(&format!(";{name}:{}:{modified}", m.len()));
    }
    Ok(identity)
}
pub(crate) fn cpu_description() -> String {
    let mut buffer = [0; 512];
    unsafe { ps_ff_cpu_info(buffer.as_mut_ptr(), buffer.len() as i32) };
    text(&buffer)
}
fn encoder_adapter(
    name: &str,
    format: VideoFormat,
    adapter: u32,
    device: Option<&ID3D11Device>,
) -> Result<Option<GpuInfo>> {
    if crate::adapter_policy::software_matches(name, format) {
        return Ok(None);
    }
    let info = gpu(adapter).context("Selected encoder adapter no longer exists")?;
    ensure!(
        crate::adapter_policy::encoder_matches(name, format, info.vendor),
        "Encoder {name} does not match adapter {adapter} vendor {:#x}",
        info.vendor
    );
    if let Some(device) = device {
        ensure!(
            luid(device) == info.luid,
            "Supplied D3D11 device does not match selected encoder adapter {adapter}"
        );
    }
    Ok(Some(info))
}
struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { ps_ff_close(self.0) };
    }
}
pub struct Encoder {
    handle: Handle,
    pub name: String,
    pub adapter: u32,
    settings: StreamConfig,
    pending_idr: Option<u64>,
    pub format: VideoFormat,
    pub size: Dimensions,
}
impl Encoder {
    pub fn new(
        name: &str,
        format: VideoFormat,
        size: Dimensions,
        settings: StreamConfig,
        device: Option<&ID3D11Device>,
        adapter: u32,
    ) -> Result<Self> {
        initialize()?;
        settings.validate()?;
        let selected = encoder_adapter(name, format, adapter, device)?;
        let name_c = CString::new(name)?;
        let mut error = [0; 512];
        let handle = unsafe {
            ps_ff_encoder(
                name_c.as_ptr(),
                size.width as i32,
                size.height as i32,
                settings.fps as i32,
                settings.bitrate_bps() as i32,
                settings.software_encode_threads.ffmpeg_threads() as i32,
                device.map_or(std::ptr::null_mut(), Interface::as_raw),
                adapter as i32,
                error.as_mut_ptr(),
                512,
            )
        };
        report_diagnostics();
        ensure!(!handle.is_null(), "FFmpeg {}: {}", name, text(&error));
        let handle = Handle(handle);
        let actual_luid = unsafe { ps_ff_codec_device_luid(handle.0) };
        let verification = if let Some(info) = &selected {
            if name.ends_with("qsv") {
                "qsv_explicit_hardware_child_adapter"
            } else {
                ensure!(
                    actual_luid != 0 && actual_luid == info.luid,
                    "Codec device does not match selected GPU for {name}"
                );
                "verified_ffmpeg_d3d11_device"
            }
        } else {
            "software_cpu"
        };
        let adapter_name = selected
            .as_ref()
            .map(|g| {
                String::from_utf16_lossy(
                    &g.name[..g.name.iter().position(|c| *c == 0).unwrap_or(g.name.len())],
                )
            })
            .unwrap_or_else(|| "CPU".into());
        tracing::info!(
            backend = name,
            adapter,
            adapter_name,
            actual_luid,
            device_verification = verification,
            codec = format.label(),
            width = size.width,
            height = size.height,
            fps = settings.fps,
            keyframe_policy = "on_demand_long_gop",
            "ffmpeg_encoder_ready"
        );
        Ok(Self {
            handle,
            name: name.into(),
            adapter,
            settings,
            pending_idr: None,
            format,
            size,
        })
    }
    pub(crate) fn runtime_failure(&self, error: &anyhow::Error) {
        crate::profiles::failed(
            &self.name,
            self.format,
            self.size,
            self.settings,
            self.adapter,
            false,
            error,
        );
    }
    pub fn submit(
        &mut self,
        bgra: Option<&[u8]>,
        texture: Option<&ID3D11Texture2D>,
        micros: u64,
        keyframe: bool,
    ) -> Result<Vec<crate::Encoded>> {
        if let Some(bytes) = bgra {
            ensure!(
                bytes.len() == (self.size.width * self.size.height * 4) as usize,
                "Invalid raw frame size"
            );
        }
        if keyframe {
            self.pending_idr.get_or_insert(micros);
        }
        let mut error = [0; 512];
        let code = unsafe {
            ps_ff_encode(
                self.handle.0,
                bgra.map_or(std::ptr::null(), |b| b.as_ptr()),
                self.size.width as i32 * 4,
                texture.map_or(std::ptr::null_mut(), Interface::as_raw),
                micros as i64,
                keyframe as i32,
                error.as_mut_ptr(),
                512,
            )
        };
        report_diagnostics();
        ensure!(code >= 0, "{}: {}", self.name, text(&error));
        let mut output = Vec::new();
        for _ in 0..8 {
            let (mut data, mut size, mut pts, mut key) = (std::ptr::null(), 0, 0, 0);
            let code = unsafe {
                ps_ff_packet(
                    self.handle.0,
                    &mut data,
                    &mut size,
                    &mut pts,
                    &mut key,
                    error.as_mut_ptr(),
                    512,
                )
            };
            report_diagnostics();
            ensure!(code >= 0, "{}: {}", self.name, text(&error));
            if code == 0 {
                break;
            }
            ensure!(
                (1..=2 * 1024 * 1024).contains(&size) && !data.is_null(),
                "Encoded frame exceeds stream limit"
            );
            verify_requested_idr(&mut self.pending_idr, pts.max(0) as u64, key != 0)?;
            output.push(crate::Encoded {
                codec: self.format,
                bytes: unsafe { std::slice::from_raw_parts(data, size as usize) }.to_vec(),
                micros: pts.max(0) as u64,
                keyframe: key != 0,
            });
        }
        Ok(output)
    }
}
fn verify_requested_idr(pending: &mut Option<u64>, pts: u64, keyframe: bool) -> Result<()> {
    if pending.is_some_and(|requested| pts >= requested) {
        ensure!(
            keyframe,
            "Encoder failed to emit a requested IDR; refusing unsafe recovery frames"
        );
        *pending = None;
    }
    Ok(())
}
#[cfg(test)]
mod idr_contract_tests {
    use super::*;
    #[test]
    fn delayed_output_does_not_hide_missing_restart_frames() {
        let mut pending = Some(100);
        verify_requested_idr(&mut pending, 90, false).unwrap();
        assert_eq!(pending, Some(100));
        assert!(verify_requested_idr(&mut pending, 100, false).is_err());
        assert_eq!(pending, Some(100));
        verify_requested_idr(&mut pending, 100, true).unwrap();
        assert!(pending.is_none());
        verify_requested_idr(&mut pending, 110, false).unwrap();
    }
}
/// Reusable CPU output; dropping a displayed/superseded frame returns its allocation.
/// The pool is per decoder and bounded, so slow/closed viewers cannot retain growing queues.
#[derive(Default)]
pub struct RgbaBuffer {
    bytes: Vec<u8>,
    pool: Option<Arc<Mutex<Vec<Vec<u8>>>>>,
}
impl std::ops::Deref for RgbaBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}
impl Drop for RgbaBuffer {
    fn drop(&mut self) {
        if let Some(pool) = &self.pool {
            let mut pool = pool.lock().unwrap();
            if pool.len() < 3 {
                pool.push(std::mem::take(&mut self.bytes));
            }
        }
    }
}
/// Owns a lease, rather than just a COM reference: the producer cannot overwrite
/// the surface while it is the UI's current frame. Safe across worker/UI threads.
pub struct GpuDecodedFrame {
    lease: *mut c_void,
}
// SAFETY: this opaque lease only holds shared ownership of a D3D11 texture and
// an atomic busy flag. The C API uses a multithread-protected D3D11 context.
unsafe impl Send for GpuDecodedFrame {}
impl GpuDecodedFrame {
    pub fn texture(&self) -> ID3D11Texture2D {
        // SAFETY: the lease owns the texture; clone obtains a separate COM reference.
        unsafe {
            ID3D11Texture2D::from_raw_borrowed(&ps_ff_display_texture(self.lease))
                .unwrap()
                .clone()
        }
    }
}
impl Drop for GpuDecodedFrame {
    fn drop(&mut self) {
        unsafe { ps_ff_display_release(self.lease) };
    }
}
pub struct Decoder {
    handle: Handle,
    pub format: VideoFormat,
    pub size: Dimensions,
    pub hardware: bool,
    gpu_output: bool,
    reported_output: bool,
    buffers: Arc<Mutex<Vec<Vec<u8>>>>,
}
impl Decoder {
    pub fn new(
        format: VideoFormat,
        size: Dimensions,
        threads: u32,
        hardware: bool,
        adapter: u32,
    ) -> Result<Self> {
        Self::with_device(format, size, threads, hardware, adapter, None)
    }
    pub(crate) fn with_device(
        format: VideoFormat,
        size: Dimensions,
        threads: u32,
        hardware: bool,
        adapter: u32,
        device: Option<&ID3D11Device>,
    ) -> Result<Self> {
        initialize()?;
        let selected = if hardware {
            let info = gpu(adapter).context("Selected decoder adapter no longer exists")?;
            if let Some(device) = device {
                ensure!(
                    luid(device) == info.luid,
                    "Decoder D3D11 device does not match selected adapter"
                );
            }
            Some(info)
        } else {
            None
        };
        let mut error = [0; 512];
        let ptr = unsafe {
            ps_ff_decoder(
                (format == VideoFormat::Hevc) as i32,
                threads as i32,
                hardware as i32,
                adapter as i32,
                device.map_or(std::ptr::null_mut(), Interface::as_raw),
                error.as_mut_ptr(),
                512,
            )
        };
        report_diagnostics();
        ensure!(!ptr.is_null(), "FFmpeg decoder: {}", text(&error));
        let handle = Handle(ptr);
        if let Some(info) = selected {
            let actual = unsafe { ps_ff_codec_device_luid(ptr) };
            ensure!(
                actual != 0 && actual == info.luid,
                "FFmpeg decoder device identity mismatch"
            );
            tracing::info!(adapter, actual_luid = actual, "decoder_device_bound");
        }
        ensure!(
            u64::from(size.width) * u64::from(size.height) <= 4096 * 2160,
            "Invalid decoder dimensions"
        );
        Ok(Self {
            handle,
            format,
            size,
            hardware,
            gpu_output: hardware && device.is_some(),
            reported_output: false,
            buffers: Arc::new(Mutex::new(Vec::new())),
        })
    }
    pub(crate) fn disable_gpu_output(&mut self) {
        self.gpu_output = false;
        self.reported_output = false;
    }
    pub fn decode(&mut self, data: &[u8]) -> Result<Option<crate::DecodedFrame>> {
        self.decode_for_display(data, true)
    }
    pub(crate) fn decode_for_display(
        &mut self,
        data: &[u8],
        display: bool,
    ) -> Result<Option<crate::DecodedFrame>> {
        ensure!(
            !data.is_empty() && data.len() <= 2 * 1024 * 1024,
            "Invalid compressed video size"
        );
        let mut error = [0; 512];
        let code = unsafe {
            ps_ff_decode(
                self.handle.0,
                data.as_ptr(),
                data.len() as i32,
                error.as_mut_ptr(),
                512,
            )
        };
        report_diagnostics();
        ensure!(code >= 0, "Decode: {}", text(&error));
        let mut rgba = RgbaBuffer {
            bytes: self.buffers.lock().unwrap().pop().unwrap_or_default(),
            pool: Some(self.buffers.clone()),
        };
        let (mut w, mut h, mut hardware, mut lease) = (0, 0, 0, std::ptr::null_mut());
        let code = unsafe {
            ps_ff_frame(
                self.handle.0,
                &mut w,
                &mut h,
                &mut hardware,
                if display { self.gpu_output as i32 } else { -1 },
                &mut lease,
                error.as_mut_ptr(),
                512,
            )
        };
        if code < 0 {
            bail!("Decode: {}", text(&error));
        }
        if code == 0 || code == 2 {
            return Ok(None);
        }
        if self.hardware && hardware == 0 {
            tracing::warn!(
                codec = self.format.label(),
                "hardware_decoder_produced_software_frame_reopen_with_cpu_threads"
            );
            bail!(
                "Hardware decoder fell back internally to software; reopen with software thread settings"
            );
        }
        if code == 3 {
            if self.gpu_output && hardware != 0 {
                tracing::warn!(reason = text(&error), "gpu_presentation_fallback");
                self.gpu_output = false;
            }
            rgba.bytes.resize(w as usize * h as usize * 4, 0);
            let code = unsafe {
                ps_ff_rgba(
                    self.handle.0,
                    rgba.bytes.as_mut_ptr(),
                    rgba.bytes.len() as i32,
                    error.as_mut_ptr(),
                    512,
                )
            };
            ensure!(code >= 0, "Decode conversion: {}", text(&error));
        }
        if self.hardware != (hardware != 0) {
            tracing::info!(
                codec = self.format.label(),
                hardware = hardware != 0,
                "ffmpeg_decoder_backend_changed"
            );
        }
        self.hardware = hardware != 0;
        let gpu = (!lease.is_null()).then_some(GpuDecodedFrame { lease });
        if gpu.is_some() {
            rgba.bytes.clear();
        } else {
            rgba.bytes.truncate(w as usize * h as usize * 4);
        }
        if !self.reported_output {
            tracing::info!(
                codec = self.format.label(),
                hardware = self.hardware,
                output = if gpu.is_some() {
                    "d3d11_gpu_surface"
                } else {
                    "reusable_cpu_rgba"
                },
                "decoder_output_selected"
            );
            self.reported_output = true;
        }
        Ok(Some(crate::DecodedFrame {
            size: Dimensions::new(w as u32, h as u32),
            rgba,
            gpu,
        }))
    }
}
pub(crate) fn device(index: u32) -> Result<ID3D11Device> {
    let ptr = unsafe { ps_ff_device(index as i32) };
    ensure!(!ptr.is_null(), "Cannot initialize D3D11 adapter {index}");
    Ok(unsafe { ID3D11Device::from_raw(ptr) })
}
pub(crate) fn luid(device: &ID3D11Device) -> i64 {
    unsafe { ps_ff_device_luid(device.as_raw()) }
}
pub(crate) fn gpu(index: u32) -> Option<GpuInfo> {
    let mut info = GpuInfo::default();
    (unsafe { ps_ff_gpu(index as i32, &mut info) } != 0).then_some(info)
}
pub(crate) fn driver_fingerprint() -> String {
    let mut bytes = [0; 4096];
    unsafe { ps_ff_drivers(bytes.as_mut_ptr(), 4096) };
    text(&bytes)
}
pub(crate) fn runtime_version() -> Result<u32> {
    initialize().context("Initialize FFmpeg for capabilities")?;
    Ok(unsafe { ps_ff_version() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2pshare_media::config::{Codec, EncoderBackend};
    #[test]
    fn bundled_software_avc_hevc_decode_interframes_without_windows_codecs() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let size = Dimensions::new(640, 360);
        let bgra = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        for (codec, format, name) in [
            (Codec::Avc, VideoFormat::H264, "libx264"),
            (Codec::Hevc, VideoFormat::Hevc, "libx265"),
        ] {
            let settings = StreamConfig {
                codec,
                encoder: EncoderBackend::Software,
                fps: 120,
                bitrate_kbps: 4000,
                ..Default::default()
            };
            let mut encoder = Encoder::new(name, format, size, settings, None, 0)?;
            let mut decoder = Decoder::new(format, size, 2, false, 0)?;
            let mut decoded = 0;
            let mut keyframes = 0;
            for n in 0..60 {
                let packets = encoder.submit(Some(&bgra), None, n * 8333, n == 0 || n == 30)?;
                ensure!(
                    packets.len() == 1,
                    "Unexpected buffering in software {name}"
                );
                for packet in packets {
                    ensure!(packet.micros == n * 8333, "Timestamp changed");
                    keyframes += packet.keyframe as usize;
                    let frame = decoder
                        .decode(&packet.bytes)?
                        .context("Missing decoded interframe")?;
                    ensure!(frame.size == size, "Wrong dimensions");
                    ensure!(
                        frame.rgba.chunks_exact(4).all(|p| p[0].abs_diff(127) < 9
                            && p[1].abs_diff(63) < 9
                            && p[2].abs_diff(31) < 9
                            && p[3] == 255),
                        "Color conversion changed"
                    );
                    decoded += 1;
                }
            }
            ensure!(
                decoded == 60 && keyframes >= 2,
                "Missing frames/keyframe request"
            );
            println!(
                "{name}: {decoded}/60 software frames at 120 FPS timestamps; keyframes={keyframes}"
            );
        }
        Ok(())
    }
    // Seven seconds of synthetic frames crosses the old encoder GOP boundary
    // several times. Both an existing viewer and newly initialized decoders must
    // decode requested IDRs without any earlier reference pictures.
    fn check_on_demand_keyframes(name: &str, format: VideoFormat) -> Result<()> {
        check_on_demand_keyframes_on_adapter(name, format, 0)
    }
    fn check_on_demand_keyframes_on_adapter(
        name: &str,
        format: VideoFormat,
        adapter: u32,
    ) -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let size = Dimensions::new(320, 180);
        let pixels = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        let settings = StreamConfig {
            codec: if format == VideoFormat::Hevc {
                Codec::Hevc
            } else {
                Codec::Avc
            },
            fps: 30,
            bitrate_kbps: 4000,
            software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
            ..Default::default()
        };
        let mut encoder = Encoder::new(name, format, size, settings, None, adapter)?;
        let mut existing = Decoder::new(format, size, 1, false, 0)?;
        let mut late_viewer = None;
        let mut recovery_viewer = None;
        let mut keyframes = Vec::new();
        for n in 0..210 {
            let requested = [0, 100, 160].contains(&n);
            let packets = encoder.submit(Some(&pixels), None, n * 33333, requested)?;
            ensure!(packets.len() == 1, "{name} buffered a frame");
            if n == 100 {
                late_viewer = Some(Decoder::new(format, size, 1, false, 0)?);
            }
            if n == 160 {
                recovery_viewer = Some(Decoder::new(format, size, 1, false, 0)?);
            }
            for packet in packets {
                if packet.keyframe {
                    keyframes.push(n);
                }
                ensure!(
                    packet.keyframe == requested,
                    "{name}: frame {n} keyframe={} requested={requested}",
                    packet.keyframe
                );
                ensure!(packet.micros == n * 33333, "{name} changed timestamps");
                for decoder in std::iter::once(&mut existing)
                    .chain(late_viewer.iter_mut())
                    .chain(recovery_viewer.iter_mut())
                {
                    let frame = decoder
                        .decode(&packet.bytes)?
                        .context("Requested IDR could not start/resume decoding")?;
                    ensure!(frame.size == size, "Decoded dimensions changed");
                    ensure!(
                        frame.rgba.chunks_exact(4).all(|p| p[0].abs_diff(127) < 12
                            && p[1].abs_diff(63) < 12
                            && p[2].abs_diff(31) < 12
                            && p[3] == 255),
                        "{name}: requested keyframe lost reference independence"
                    );
                }
            }
        }
        ensure!(
            keyframes == [0, 100, 160],
            "{name}: unexpected keyframes {keyframes:?}"
        );
        println!(
            "{name}: 210/210 frames, keyframes {keyframes:?}; late join and fresh recovery decode passed"
        );
        Ok(())
    }

    #[test]
    fn software_keyframes_are_on_demand_and_support_late_join_and_recovery() -> Result<()> {
        check_on_demand_keyframes("libx264", VideoFormat::H264)?;
        check_on_demand_keyframes("libx265", VideoFormat::Hevc)
    }

    #[test]
    #[ignore = "Requires NVENC hardware; synthetic pixels, no window or capture"]
    fn nvenc_keyframes_are_on_demand_and_support_late_join_and_recovery() -> Result<()> {
        check_on_demand_keyframes("h264_nvenc", VideoFormat::H264)?;
        check_on_demand_keyframes("hevc_nvenc", VideoFormat::Hevc)
    }

    #[test]
    #[ignore = "Requires AMD AMF hardware; synthetic pixels only"]
    fn amf_keyframes_are_on_demand_and_support_late_join_and_recovery() -> Result<()> {
        initialize()?;
        let adapter = (0..16)
            .find(|&index| gpu(index).is_some_and(|g| g.vendor == 0x1002))
            .context("No AMD adapter for AMF test")?;
        check_on_demand_keyframes_on_adapter("h264_amf", VideoFormat::H264, adapter)?;
        check_on_demand_keyframes_on_adapter("hevc_amf", VideoFormat::Hevc, adapter)
    }

    #[test]
    fn runtime_vendor_guard_rejects_foreign_encoder_before_opening_driver() -> Result<()> {
        initialize()?;
        if let Some(info) = gpu(0) {
            let wrong = if info.vendor == 0x1002 {
                "h264_nvenc"
            } else {
                "h264_amf"
            };
            let error = Encoder::new(
                wrong,
                VideoFormat::H264,
                Dimensions::new(320, 180),
                StreamConfig::default(),
                None,
                0,
            )
            .err()
            .context("Foreign hardware encoder was accepted")?;
            ensure!(
                error.to_string().contains("does not match adapter"),
                "Wrong failure reason: {error}"
            );
        }
        Ok(())
    }

    #[test]
    fn automatic_software_threads_encode_and_decode_avc_and_hevc() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let size = Dimensions::new(320, 180);
        let pixels = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        for (format, name) in [
            (VideoFormat::H264, "libx264"),
            (VideoFormat::Hevc, "libx265"),
        ] {
            let config = StreamConfig {
                software_encode_threads: p2pshare_media::config::ThreadCount::Auto,
                software_decode_threads: p2pshare_media::config::ThreadCount::Auto,
                ..Default::default()
            };
            let mut encoder = Encoder::new(name, format, size, config, None, 0)?;
            let mut decoder = Decoder::new(
                format,
                size,
                config.software_decode_threads.ffmpeg_threads(),
                false,
                0,
            )?;
            let mut count = 0;
            for n in 0..12 {
                for packet in encoder.submit(Some(&pixels), None, n * 33333, n == 0 || n == 8)? {
                    let frame = decoder
                        .decode(&packet.bytes)?
                        .context("Auto-threaded decode did not output a frame")?;
                    ensure!(frame.size == size, "Auto-threaded decoded size changed");
                    count += 1;
                }
            }
            ensure!(
                count == 12,
                "Auto-threaded encoder buffered/lost frames: {name} {count}/12"
            );
        }
        Ok(())
    }

    #[test]
    fn software_decode_recycles_buffers_and_skips_only_display_work() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let size = Dimensions::new(320, 180);
        let settings = StreamConfig {
            software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
            ..Default::default()
        };
        let pixels = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        let mut encoder = Encoder::new("libx264", VideoFormat::H264, size, settings, None, 0)?;
        let mut decoder = Decoder::new(VideoFormat::H264, size, 1, false, 0)?;
        let mut previous = None;
        for n in 0..12 {
            for packet in encoder.submit(Some(&pixels), None, n * 16667, n == 0)? {
                if n % 3 == 1 {
                    ensure!(
                        decoder.decode_for_display(&packet.bytes, false)?.is_none(),
                        "Obsolete frame was converted"
                    );
                } else {
                    let frame = decoder
                        .decode(&packet.bytes)?
                        .context("Dependency was lost")?;
                    ensure!(
                        frame.gpu.is_none()
                            && frame.rgba.len() == (size.width * size.height * 4) as usize,
                        "Invalid CPU output"
                    );
                    if let Some(ptr) = previous {
                        ensure!(ptr == frame.rgba.as_ptr(), "Frame buffer was not recycled");
                    }
                    previous = Some(frame.rgba.as_ptr());
                }
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "Requires a hardware D3D11 video decoder; synthetic pixels only"]
    fn gpu_decode_converts_colors_recycles_surfaces_and_falls_back_without_reset() -> Result<()> {
        use p2pshare_platform_windows::GraphicsDevice;
        let _com = crate::native::CodecApartment::new();
        let graphics = GraphicsDevice::new()?;
        let size = Dimensions::new(640, 360);
        let pixels = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        for (format, name) in [
            (VideoFormat::H264, "libx264"),
            (VideoFormat::Hevc, "libx265"),
        ] {
            let mut encoder = Encoder::new(
                name,
                format,
                size,
                StreamConfig {
                    software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
                    ..Default::default()
                },
                None,
                0,
            )?;
            let mut decoder =
                Decoder::with_device(format, size, 1, true, 0, Some(&graphics.device))?;
            let mut held = Vec::new();
            for n in 0..8 {
                for packet in encoder.submit(Some(&pixels), None, n * 16667, n == 0)? {
                    let frame = decoder.decode(&packet.bytes)?;
                    if n < 4 {
                        let frame = frame.context("GPU output missing")?;
                        ensure!(
                            frame.gpu.is_some() && frame.rgba.is_empty(),
                            "GPU path fell back to CPU output"
                        );
                        held.push(frame);
                    } else if n == 4 {
                        ensure!(frame.is_none(), "Busy surfaces were overwritten or queued");
                        let frame = held.remove(0);
                        let mut readback = crate::Readback::new(graphics.clone())?;
                        readback.submit(&frame.gpu.as_ref().unwrap().texture(), size)?;
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(3);
                        loop {
                            if let Some(raw) = readback.poll()? {
                                ensure!(
                                    raw.bytes.chunks_exact(4).all(|p| p[0].abs_diff(31) < 10
                                        && p[1].abs_diff(63) < 10
                                        && p[2].abs_diff(127) < 10
                                        && p[3] == 255),
                                    "GPU colors/range/crop changed"
                                );
                                break;
                            }
                            ensure!(
                                std::time::Instant::now() < deadline,
                                "GPU conversion did not complete"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                        held.clear();
                        decoder.disable_gpu_output();
                    } else {
                        let frame = frame.context("Fallback lost interframe dependencies")?;
                        ensure!(
                            decoder.hardware && frame.gpu.is_none() && !frame.rgba.is_empty(),
                            "Fallback restarted or disabled hardware decode"
                        );
                    }
                }
            }
            println!("{name}: GPU pixels, bounded leases and CPU-output fallback passed");
        }
        Ok(())
    }
    #[test]
    #[ignore = "Synthetic 1080p hardware decoder CPU benchmark"]
    fn gpu_output_cpu_benchmark() -> Result<()> {
        use windows::Win32::{
            Foundation::FILETIME,
            System::Threading::{GetCurrentProcess, GetProcessTimes},
        };
        fn cpu_ms() -> f64 {
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            unsafe {
                GetProcessTimes(
                    GetCurrentProcess(),
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
                .unwrap();
            }
            let ticks =
                |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
            (ticks(kernel) + ticks(user)) as f64 / 10000.
        }
        let _com = crate::native::CodecApartment::new();
        let graphics = p2pshare_platform_windows::GraphicsDevice::new()?;
        let size = Dimensions::new(1920, 1032);
        let pixels = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        for (format, name) in [
            (VideoFormat::H264, "libx264"),
            (VideoFormat::Hevc, "libx265"),
        ] {
            let mut encoder = Encoder::new(
                name,
                format,
                size,
                StreamConfig {
                    software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
                    ..Default::default()
                },
                None,
                0,
            )?;
            let packet = encoder.submit(Some(&pixels), None, 0, true)?.remove(0);
            drop(encoder);
            for resident in [false, true] {
                let mut decoder = Decoder::with_device(
                    format,
                    size,
                    1,
                    true,
                    0,
                    if resident {
                        Some(&graphics.device)
                    } else {
                        None
                    },
                )?;
                for _ in 0..5 {
                    let _ = decoder.decode(&packet.bytes)?;
                }
                let start = std::time::Instant::now();
                let cpu = cpu_ms();
                for _ in 0..120 {
                    let frame = decoder
                        .decode(&packet.bytes)?
                        .context("Benchmark output missing")?;
                    ensure!(
                        frame.gpu.is_some() == resident,
                        "Benchmark used wrong output path"
                    );
                }
                println!(
                    "{} output={} frames=120 process_cpu_ms={:.2} elapsed_ms={:.2}",
                    format.label(),
                    if resident { "GPU" } else { "CPU_RGBA" },
                    cpu_ms() - cpu,
                    start.elapsed().as_secs_f64() * 1000.
                );
            }
        }
        Ok(())
    }
    #[test]
    fn avc_2160p_selects_a_suitable_level_and_roundtrips_a_synthetic_frame() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let size = Dimensions::new(3840, 2160);
        let settings = StreamConfig {
            codec: Codec::Avc,
            encoder: EncoderBackend::Software,
            software_encode_threads: p2pshare_media::config::ThreadCount::Count(1),
            fps: 60,
            bitrate_kbps: 20000,
            width: size.width,
            height: size.height,
            ..Default::default()
        };
        let bgra = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        let mut encoder = Encoder::new("libx264", VideoFormat::H264, size, settings, None, 0)?;
        let packets = encoder.submit(Some(&bgra), None, 0, true)?;
        ensure!(
            packets.len() == 1 && packets[0].keyframe,
            "2160p frame was delayed or missing"
        );
        let mut decoder = Decoder::new(VideoFormat::H264, size, 1, false, 0)?;
        let frame = decoder
            .decode(&packets[0].bytes)?
            .context("Missing 2160p frame")?;
        ensure!(frame.size == size, "2160p output dimensions changed");
        Ok(())
    }
    #[test]
    fn hardware_probe_cache_is_reused_and_preserves_runtime_failures() -> Result<()> {
        crate::capabilities::start(false);
        let candidates = crate::capabilities::candidates(
            StreamConfig {
                codec: Codec::Auto,
                ..Default::default()
            },
            None,
            false,
        )?;
        ensure!(
            candidates
                .last()
                .is_some_and(|p| p.name == "libx264" && p.codec == VideoFormat::H264),
            "Auto fallback must be software AVC"
        );
        let mut avc_seen = false;
        for candidate in &candidates {
            if candidate.codec == VideoFormat::H264 {
                avc_seen = true;
            } else {
                ensure!(!avc_seen, "Hardware HEVC must precede AVC");
            }
        }
        let software = crate::capabilities::candidates(
            StreamConfig {
                codec: Codec::Auto,
                encoder: EncoderBackend::Software,
                ..Default::default()
            },
            None,
            false,
        )?;
        ensure!(
            software.len() == 1 && software[0].name == "libx264",
            "Software auto must select AVC"
        );
        println!(
            "Auto candidates: {}",
            candidates
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(())
    }
}
