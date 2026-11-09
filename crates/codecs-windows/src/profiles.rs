//! Cached functional validation at the requested output size/FPS/bitrate.
//! Runs on the existing codec/publisher worker, never the UI thread. No capture.
use crate::{Encoded, ffmpeg};
use anyhow::{Context, Result, ensure};
use p2pshare_media::{
    Dimensions,
    config::{StreamConfig, ThreadCount, VideoFormat},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};
const MAX_ENTRIES: usize = 64;
// Bump when the functional validation contract changes.
const VALIDATION_REVISION: u32 = 1;
const FAILURE_RETRY_SECS: u64 = 300;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Environment {
    revision: u32,
    app: String,
    runtime: String,
    drivers: String,
    cpu: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Adapter {
    index: u32,
    vendor: u32,
    device: u32,
    subsystem: u32,
    revision: u32,
    driver: i64,
    name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Key {
    backend: String,
    codec: VideoFormat,
    width: u32,
    height: u32,
    fps: u32,
    bitrate_kbps: u32,
    threads: ThreadCount,
    gpu_input: bool,
    adapter: Option<Adapter>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    key: Key,
    ok: bool,
    error: Option<String>,
    checked: u64,
}
#[derive(Serialize, Deserialize)]
struct Store {
    environment: Environment,
    entries: Vec<Entry>,
}
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
// Prevent concurrent startup probes from overcommitting weak GPUs or duplicating work.
static VALIDATING: Mutex<()> = Mutex::new(());
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn file() -> Result<PathBuf> {
    Ok(p2pshare_media::portable::data_dir()?.join("validated-stream-profiles.json"))
}
fn environment() -> Result<Environment> {
    Ok(Environment {
        revision: VALIDATION_REVISION,
        app: env!("CARGO_PKG_VERSION").into(),
        runtime: ffmpeg::runtime_identity()?,
        drivers: ffmpeg::driver_fingerprint(),
        cpu: ffmpeg::cpu_description(),
    })
}
fn key(
    name: &str,
    codec: VideoFormat,
    size: Dimensions,
    settings: StreamConfig,
    index: u32,
    gpu_input: bool,
) -> Result<Key> {
    settings.validate()?;
    ensure!(
        !size.is_empty()
            && size.width <= 4096
            && size.height <= 4096
            && u64::from(size.width) * u64::from(size.height) <= 3840 * 2160,
        "Invalid validation dimensions"
    );
    ensure!(
        !(gpu_input && name.ends_with("qsv")),
        "QSV direct D3D11 input is not implemented"
    );
    let adapter = if crate::adapter_policy::software_matches(name, codec) {
        None
    } else {
        let g = ffmpeg::gpu(index).context("Encoder adapter disappeared")?;
        ensure!(
            crate::adapter_policy::encoder_matches(name, codec, g.vendor),
            "Wrong encoder/vendor binding"
        );
        Some(Adapter {
            index,
            vendor: g.vendor,
            device: g.device,
            subsystem: g.sub_system,
            revision: g.revision,
            driver: g.driver,
            name: String::from_utf16_lossy(
                &g.name[..g.name.iter().position(|c| *c == 0).unwrap_or(g.name.len())],
            ),
        })
    };
    Ok(Key {
        backend: name.into(),
        codec,
        width: size.width,
        height: size.height,
        fps: settings.fps,
        bitrate_kbps: settings.bitrate_kbps,
        threads: settings.software_encode_threads,
        gpu_input,
        adapter,
    })
}
fn valid_key(k: &Key) -> bool {
    let dimensions = k.width >= 2
        && k.height >= 2
        && k.width <= 4096
        && k.height <= 4096
        && u64::from(k.width) * u64::from(k.height) <= 3840 * 2160
        && k.width.is_multiple_of(2)
        && k.height.is_multiple_of(2);
    dimensions
        && (1..=120).contains(&k.fps)
        && (100..=50000).contains(&k.bitrate_kbps)
        && match &k.adapter {
            Some(a) => {
                crate::adapter_policy::encoder_matches(&k.backend, k.codec, a.vendor)
                    && a.name.len() <= 512
            }
            None => !k.gpu_input && crate::adapter_policy::software_matches(&k.backend, k.codec),
        }
}
fn read(path: &Path, env: &Environment) -> Store {
    let loaded = (|| -> Result<Store> {
        ensure!(
            std::fs::metadata(path)?.len() <= 128 * 1024,
            "Profile cache too large"
        );
        let value: Store = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            value.environment == *env
                && value.entries.len() <= MAX_ENTRIES
                && value
                    .entries
                    .iter()
                    .all(|e| valid_key(&e.key) && e.error.as_ref().is_none_or(|s| s.len() <= 4096)),
            "Profile cache context changed or invalid"
        );
        Ok(value)
    })();
    loaded.unwrap_or_else(|_| Store {
        environment: env.clone(),
        entries: Vec::new(),
    })
}
fn state() -> Result<&'static Mutex<Store>> {
    if let Some(store) = STORE.get() {
        return Ok(store);
    }
    let env = environment()?;
    let value = read(&file()?, &env);
    let _ = STORE.set(Mutex::new(value));
    Ok(STORE.get().unwrap())
}
fn save(store: &Store) {
    let result = (|| -> Result<()> {
        let path = file()?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temp, serde_json::to_vec_pretty(store)?)?;
        if let Err(e) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(e.into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        tracing::warn!(%error,"profile_cache_write_failed_using_memory_results");
    }
}
fn lookup(store: &Store, k: &Key, time: u64) -> Option<Result<()>> {
    let e = store.entries.iter().find(|e| e.key == *k)?;
    if e.ok {
        return Some(Ok(()));
    }
    if time >= e.checked && time - e.checked < FAILURE_RETRY_SECS {
        return Some(Err(anyhow::anyhow!(
            "Cached profile failure: {}",
            e.error.as_deref().unwrap_or("unknown")
        )));
    }
    None
}
fn transient(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    [
        "memory",
        "busy",
        "temporar",
        "timeout",
        "session limit",
        "device removed",
        "device reset",
    ]
    .iter()
    .any(|s| e.contains(s))
}
fn remember(store: &mut Store, k: Key, result: &Result<()>, time: u64) {
    store.entries.retain(|e| e.key != k);
    let error = result
        .as_ref()
        .err()
        .map(|e| format!("{e:#}").chars().take(512).collect::<String>());
    if error.as_ref().is_some_and(|e| transient(e)) {
        return;
    }
    if store.entries.len() >= MAX_ENTRIES {
        store.entries.remove(0);
    }
    store.entries.push(Entry {
        key: k,
        ok: result.is_ok(),
        error,
        checked: time,
    });
}
fn validate(k: Key, probe: impl FnOnce() -> Result<()>) -> Result<()> {
    let _serial = VALIDATING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(result) = lookup(
        &state()?.lock().unwrap_or_else(|e| e.into_inner()),
        &k,
        now(),
    ) {
        tracing::info!(backend=%k.backend,width=k.width,height=k.height,fps=k.fps,gpu_input=k.gpu_input,supported=result.is_ok(),"stream_profile_cache_hit");
        return result;
    }
    let started = Instant::now();
    let result = probe();
    tracing::info!(backend=%k.backend,width=k.width,height=k.height,fps=k.fps,bitrate_kbps=k.bitrate_kbps,gpu_input=k.gpu_input,supported=result.is_ok(),elapsed_ms=started.elapsed().as_millis() as u64,error=?result.as_ref().err().map(|e|format!("{e:#}")),"stream_profile_validation");
    let mut store = state()?.lock().unwrap_or_else(|e| e.into_inner());
    remember(&mut store, k, &result, now());
    save(&store);
    result
}
pub(crate) fn invalidate() {
    if let Some(s) = STORE.get() {
        s.lock().unwrap_or_else(|e| e.into_inner()).entries.clear();
    }
    if let Ok(p) = file() {
        let _ = std::fs::remove_file(p);
    }
}
/// Keep only one decoder at a time to bound memory even at 4K. Restart it twice,
/// each time requiring the encoder's IDR+headers, then decode dependent pictures.
fn exercise(
    size: Dimensions,
    codec: VideoFormat,
    fps: u32,
    mut encode: impl FnMut(u64, bool) -> Result<Vec<Encoded>>,
) -> Result<()> {
    let mut decoder = None;
    let mut count = 0;
    let mut encode_time = std::time::Duration::ZERO;
    for n in 0..12 {
        let restart = n == 0 || n == 4 || n == 8;
        if restart {
            drop(decoder.take());
            decoder = Some(ffmpeg::Decoder::new(codec, size, 1, false, 0)?);
        }
        let start = Instant::now();
        let frames = encode(n * 1_000_000 / u64::from(fps), restart)?;
        encode_time += start.elapsed();
        ensure!(
            frames.len() == 1,
            "Encoder delayed/dropped functional test frame {n}"
        );
        for packet in frames {
            ensure!(
                !restart || packet.keyframe,
                "Encoder did not honor requested IDR at frame {n}"
            );
            let frame = decoder
                .as_mut()
                .unwrap()
                .decode(&packet.bytes)?
                .context("Fresh/dependent decoder output missing")?;
            ensure!(
                frame.size == size,
                "Profile decoder output dimensions differ"
            );
            count += 1;
        }
    }
    ensure!(count == 12, "Profile frame count changed");
    tracing::info!(
        frames = count,
        encode_ms = encode_time.as_secs_f64() * 1000.0,
        target_fps = fps,
        "stream_profile_encode_timing"
    );
    Ok(())
}
pub(crate) fn cpu(
    name: &str,
    codec: VideoFormat,
    size: Dimensions,
    settings: StreamConfig,
    index: u32,
) -> Result<()> {
    let k = key(name, codec, size, settings, index, false)?;
    validate(k, || {
        let pixels = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        let mut encoder = ffmpeg::Encoder::new(name, codec, size, settings, None, index)?;
        exercise(size, codec, settings.fps, |pts, idr| {
            encoder.submit(Some(&pixels), None, pts, idr)
        })
    })
}
pub(crate) fn gpu(
    name: &str,
    codec: VideoFormat,
    source: Dimensions,
    settings: StreamConfig,
    index: u32,
    graphics: &p2pshare_platform_windows::GraphicsDevice,
) -> Result<()> {
    let size = crate::GpuEncoder::output_size(source, settings);
    let k = key(name, codec, size, settings, index, true)?;
    validate(k, || {
        let pixels = [31, 63, 127, 255].repeat((size.width * size.height) as usize);
        let mut texture = None;
        unsafe {
            graphics.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size.width,
                    Height: size.height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0 as u32,
                    ..Default::default()
                },
                Some(&D3D11_SUBRESOURCE_DATA {
                    pSysMem: pixels.as_ptr().cast(),
                    SysMemPitch: size.width * 4,
                    SysMemSlicePitch: 0,
                }),
                Some(&mut texture),
            )?;
        }
        let mut encoder =
            crate::GpuEncoder::new_backend(graphics.clone(), size, settings, name, codec, index)?;
        exercise(size, codec, settings.fps, |pts, idr| {
            encoder.encode(texture.as_ref().unwrap(), pts, idr)
        })
    })
}
pub(crate) fn failed(
    name: &str,
    codec: VideoFormat,
    size: Dimensions,
    settings: StreamConfig,
    index: u32,
    gpu_input: bool,
    error: &anyhow::Error,
) {
    if let Ok(k) = key(name, codec, size, settings, index, gpu_input)
        && let Ok(state) = state()
    {
        let mut store = state.lock().unwrap_or_else(|e| e.into_inner());
        let result = Err(anyhow::anyhow!(format!("{error:#}")));
        remember(&mut store, k, &result, now());
        save(&store);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn env() -> Environment {
        Environment {
            revision: VALIDATION_REVISION,
            app: "test".into(),
            runtime: "runtime-1".into(),
            drivers: "driver-1".into(),
            cpu: "cpu-1".into(),
        }
    }
    fn software_key() -> Key {
        Key {
            backend: "libx264".into(),
            codec: VideoFormat::H264,
            width: 640,
            height: 360,
            fps: 60,
            bitrate_kbps: 6000,
            threads: ThreadCount::Count(2),
            gpu_input: false,
            adapter: None,
        }
    }
    #[test]
    fn cached_success_is_setting_specific_and_failure_retries_are_bounded() {
        let k = software_key();
        let mut store = Store {
            environment: env(),
            entries: vec![],
        };
        remember(&mut store, k.clone(), &Ok(()), 10);
        assert!(lookup(&store, &k, 100000).unwrap().is_ok());
        for n in 0..5 {
            let mut other = k.clone();
            match n {
                0 => other.width = 1280,
                1 => other.fps = 120,
                2 => other.bitrate_kbps = 10000,
                3 => other.threads = ThreadCount::Auto,
                _ => {
                    other.codec = VideoFormat::Hevc;
                    other.backend = "libx265".into();
                }
            }
            assert!(lookup(&store, &other, 20).is_none());
        }
        remember(
            &mut store,
            k.clone(),
            &Err(anyhow::anyhow!("IDR missing")),
            20,
        );
        assert!(lookup(&store, &k, 21).unwrap().is_err());
        assert!(lookup(&store, &k, 20 + FAILURE_RETRY_SECS).is_none());
        remember(
            &mut store,
            k.clone(),
            &Err(anyhow::anyhow!("Device busy")),
            30,
        );
        assert!(lookup(&store, &k, 31).is_none());
    }
    #[test]
    fn disk_cache_reuses_success_but_rejects_changed_runtime_and_poisoned_adapters() -> Result<()> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../.build/profile-cache-test-{}.json",
            std::process::id()
        ));
        let mut store = Store {
            environment: env(),
            entries: vec![],
        };
        let k = software_key();
        remember(&mut store, k.clone(), &Ok(()), 10);
        std::fs::write(&path, serde_json::to_vec(&store)?)?;
        let stamp = std::fs::metadata(&path)?.modified()?;
        assert!(lookup(&read(&path, &env()), &k, 11).unwrap().is_ok());
        assert_eq!(std::fs::metadata(&path)?.modified()?, stamp);
        for field in 0..4 {
            let mut changed = env();
            match field {
                0 => changed.runtime = "runtime-2".into(),
                1 => changed.drivers = "driver-2".into(),
                2 => changed.cpu = "cpu-2".into(),
                _ => changed.revision += 1,
            }
            assert!(read(&path, &changed).entries.is_empty());
        }
        let mut poisoned = k;
        poisoned.backend = "h264_amf".into();
        poisoned.adapter = Some(Adapter {
            index: 0,
            vendor: 0x10de,
            device: 1,
            subsystem: 0,
            revision: 0,
            driver: 1,
            name: "NVIDIA".into(),
        });
        assert!(!valid_key(&poisoned));
        std::fs::write(&path, b"broken cache")?;
        assert!(read(&path, &env()).entries.is_empty());
        Ok(())
    }
    #[test]
    #[ignore = "Requires NVIDIA GPU; synthetic headless profile validation"]
    fn actual_gpu_profiles_validate_and_are_reused_without_retesting() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        ffmpeg::initialize()?;
        let index = (0..16)
            .find(|&i| ffmpeg::gpu(i).is_some_and(|g| g.vendor == 0x10de))
            .context("No NVIDIA GPU")?;
        let graphics =
            p2pshare_platform_windows::GraphicsDevice::from_device(ffmpeg::device(index)?)?;
        let settings = StreamConfig {
            width: 1920,
            height: 1080,
            ..Default::default()
        };
        invalidate();
        for (name, codec) in [
            ("h264_nvenc", VideoFormat::H264),
            ("hevc_nvenc", VideoFormat::Hevc),
        ] {
            gpu(
                name,
                codec,
                Dimensions::new(1920, 1080),
                settings,
                index,
                &graphics,
            )?;
            cpu(name, codec, Dimensions::new(1920, 1080), settings, index)?;
            let modified = std::fs::metadata(file()?)?.modified()?;
            gpu(
                name,
                codec,
                Dimensions::new(1920, 1080),
                settings,
                index,
                &graphics,
            )?;
            assert_eq!(
                std::fs::metadata(file()?)?.modified()?,
                modified,
                "Cache hit rewrote successful profile"
            );
        }
        let stamp = std::fs::metadata(file()?)?.modified()?;
        let production =
            crate::GpuEncoder::new(graphics.clone(), Dimensions::new(1920, 1080), settings)?;
        assert_eq!(
            std::fs::metadata(file()?)?.modified()?,
            stamp,
            "Production constructor did not reuse the profile cache"
        );
        drop(production);
        Ok(())
    }

    #[test]
    fn real_software_profiles_cover_join_resume_and_cache_hits() -> Result<()> {
        let _com = crate::native::CodecApartment::new();
        let settings = StreamConfig {
            software_encode_threads: ThreadCount::Count(1),
            width: 640,
            height: 360,
            ..Default::default()
        };
        for (name, codec) in [
            ("libx264", VideoFormat::H264),
            ("libx265", VideoFormat::Hevc),
        ] {
            cpu(name, codec, Dimensions::new(320, 180), settings, 0)?;
            cpu(name, codec, Dimensions::new(320, 180), settings, 0)?;
        }
        Ok(())
    }
}
