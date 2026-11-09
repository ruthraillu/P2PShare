use anyhow::Result;
use p2pshare_media::{
    Dimensions, VideoQuality,
    config::{StreamConfig, VideoFormat},
    diagnostics::{self, FrameMeter, LatencyMeter},
};
use p2pshare_platform_windows::GraphicsDevice;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Instant,
};
use windows::{
    Win32::Graphics::{
        Direct3D::Fxc::*,
        Direct3D::*,
        Direct3D11::*,
        Dxgi::{Common::*, DXGI_ERROR_WAS_STILL_DRAWING},
    },
    core::s,
};
#[derive(Clone)]
pub struct RawFrame {
    pub size: Dimensions,
    pub bytes: Vec<u8>,
    pub queued_at: Instant,
    pub micros: u64,
}
pub struct DecodedFrame {
    pub size: Dimensions,
    pub rgba: crate::ffmpeg::RgbaBuffer,
    pub gpu: Option<crate::ffmpeg::GpuDecodedFrame>,
}
pub struct Encoded {
    pub codec: VideoFormat,
    pub bytes: Vec<u8>,
    pub micros: u64,
    pub keyframe: bool,
}
enum Work {
    Decode(String, Arc<[u8]>, VideoFormat, Dimensions),
    Retain(Vec<String>),
    CpuOutput(String),
    Wake,
    Reset,
}
#[derive(Default)]
struct Shared {
    raw: Option<RawFrame>,
    encoded: VecDeque<Encoded>,
    decoded: HashMap<String, DecodedFrame>,
    error: Option<String>,
    backend: String,
    settings: StreamConfig,
    raw_coalesced: u64,
    encoded_drops: u64,
    encoder_skips: u64,
    keyframe_requests: HashSet<String>,
}
pub struct CodecWorker {
    work: mpsc::SyncSender<Work>,
    shared: Arc<Mutex<Shared>>,
    force: Arc<AtomicBool>,
}
impl CodecWorker {
    pub fn new() -> Result<Self> {
        Self::with_display_device(None)
    }
    pub fn with_display_device(device: Option<ID3D11Device>) -> Result<Self> {
        crate::ffmpeg::initialize()?;
        crate::capabilities::start(std::env::args().any(|a| a == "--rescan-hardware"));
        let (work, rx) = mpsc::sync_channel(32);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let force = Arc::new(AtomicBool::new(true));
        let s = shared.clone();
        let f = force.clone();
        std::thread::Builder::new().name("p2pshare-video-codecs".into()).spawn(move|| {
            let _apartment=CodecApartment::new();
            let mut decoders:HashMap<String,crate::ffmpeg::Decoder>=HashMap::new();
            let mut cpu_output=HashSet::new();
            let mut encoder:Option<crate::ffmpeg::Encoder>=None;
            let mut last_raw:Option<RawFrame>=None;let mut dimensions=Dimensions::default();
            let mut settings=StreamConfig::default();let mut failed_encoders=HashSet::new();
            let mut last_time=0;
            let mut meter=FrameMeter::new("encoded");let mut latency=LatencyMeter::new("encode");
            let mut queue_latency=LatencyMeter::new("capture_to_encoder_queue");
            let mut batch=VecDeque::with_capacity(32);
            while let Ok(first)=rx.recv() {
                batch.push_back(first);
                batch.extend(rx.try_iter().take(31));
                while let Some(work)=batch.pop_front() {
                let display=match &work {Work::Decode(peer,..)=>!batch.iter().any(|w|matches!(w,Work::Decode(p,..) if p==peer)),_=>true};
                let result=(||->Result<()> {
                    match work {
                        Work::Decode(peer,data,format,size)=> {
                            let threads=s.lock().unwrap().settings.software_decode_threads.ffmpeg_threads();
                            if decoders.get(&peer).is_none_or(|d|d.format!=format || d.size!=size) {
                                let mut decoder=crate::capabilities::decoder(format,size,threads,device.as_ref())?;
                                if cpu_output.contains(&peer) {decoder.disable_gpu_output();}
                                decoders.insert(peer.clone(),decoder);
                            }
                            match decoders.get_mut(&peer).unwrap().decode_for_display(&data,display) {
                                Ok(Some(frame))=>{s.lock().unwrap().decoded.insert(peer,frame);},
                                Ok(None)=>{},
                                Err(error)=>{
                                    tracing::warn!(%error,peer,codec=format.label(),"decoder_failed_using_software");
                                    decoders.insert(peer.clone(),crate::ffmpeg::Decoder::new(format,size,threads,false,0)?);
                                    if let Ok(Some(frame))=decoders.get_mut(&peer).unwrap().decode_for_display(&data,display) {s.lock().unwrap().decoded.insert(peer.clone(),frame);}
                                    s.lock().unwrap().keyframe_requests.insert(peer);
                                }
                            }
                            return Ok(());
                        }
                        Work::CpuOutput(peer)=>{cpu_output.insert(peer.clone()); if let Some(d)=decoders.get_mut(&peer) {d.disable_gpu_output();} return Ok(());},
                        Work::Retain(peers)=>{cpu_output.retain(|p|peers.contains(p));decoders.retain(|p,_|peers.contains(p));s.lock().unwrap().decoded.retain(|p,_|peers.contains(p));return Ok(());},
                        Work::Reset=>{encoder=None;last_raw=None;dimensions=Dimensions::default();failed_encoders.clear();f.store(true,Ordering::Relaxed);},
                        Work::Wake=>{},
                    }
                    let requested=s.lock().unwrap().settings;
                    if requested!=settings {settings=requested;meter.set_fps(settings.fps);encoder=None;dimensions=Dimensions::default();last_raw=None;failed_encoders.clear();s.lock().unwrap().encoded.clear();f.store(true,Ordering::Relaxed);}
                    let raw=s.lock().unwrap().raw.take().or_else(||if f.load(Ordering::Relaxed){last_raw.clone()}else{None});
                    let Some(raw)=raw else {return Ok(());};
                    if raw.size!=dimensions || encoder.is_none() {
                        if raw.size!=dimensions {failed_encoders.clear();}
                        dimensions=raw.size;s.lock().unwrap().encoded.clear();
                        let enc=crate::capabilities::open_cpu_encoder(settings,dimensions,&mut failed_encoders)?;
                        s.lock().unwrap().backend=format!("FFmpeg {}",enc.name);encoder=Some(enc);
                        f.store(true,Ordering::Relaxed);
                    }
                    let started=Instant::now();queue_latency.record(raw.queued_at.elapsed().as_secs_f64()*1000.);
                    // Requests are set for encoder creation/reset, join and recovery.
                    // Do not periodically reset the reference picture while it is healthy.
                    let keyframe=f.swap(false,Ordering::Relaxed);
                    let micros=raw.micros.max(last_time+100);last_time=micros;
                    let frames=match encoder.as_mut().unwrap().submit(Some(&raw.bytes),None,micros,keyframe) {
                        Ok(frames)=>frames,
                        Err(error)=>{encoder.as_ref().unwrap().runtime_failure(&error);failed_encoders.insert((encoder.as_ref().unwrap().name.clone(),encoder.as_ref().unwrap().adapter));encoder=None;f.store(true,Ordering::Relaxed);if settings.encoder==p2pshare_media::config::EncoderBackend::Auto {tracing::warn!(%error,"encoder_runtime_failure_trying_next_backend");last_raw=Some(raw);return Ok(());}return Err(error);}
                    };
                    latency.record(started.elapsed().as_secs_f64()*1000.);
                    if frames.is_empty(){s.lock().unwrap().encoder_skips+=1;}
                    for frame in frames {
                        meter.frame(frame.bytes.len());
                        if diagnostics::enabled()&&settings.log_per_frame {tracing::info!(pts_us=frame.micros,bytes=frame.bytes.len(),encode_ms=started.elapsed().as_secs_f64()*1000.,"encoder_frame");}
                        let mut shared=s.lock().unwrap();
                        if shared.encoded.len()>=8 || shared.encoded.iter().map(|e|e.bytes.len()).sum::<usize>()+frame.bytes.len()>8*1024*1024 {
                            shared.encoded_drops+=shared.encoded.len() as u64;shared.encoded.clear();f.store(true,Ordering::Relaxed);
                            if !frame.keyframe {shared.encoded_drops+=1;continue;}
                        }
                        shared.encoded.push_back(frame);
                    }
                    meter.report();last_raw=Some(raw);Ok(())
                })();
                if let Err(error)=result {tracing::error!(%error,"codec_worker_error");s.lock().unwrap().error=Some(format!("Video codec: {error:#}"));}
                }
            }
        })?;
        Ok(Self {
            work,
            shared,
            force,
        })
    }
    pub fn set_quality(&self, quality: VideoQuality) {
        self.set_config(quality.into());
    }
    pub fn set_config(&self, settings: StreamConfig) {
        self.shared.lock().unwrap().settings = settings;
        let _ = self.work.try_send(Work::Wake);
    }
    pub fn encode(&self, raw: RawFrame) {
        let mut s = self.shared.lock().unwrap();
        if s.raw.is_some() {
            s.raw_coalesced += 1;
        }
        s.raw = Some(raw);
        drop(s);
        let _ = self.work.try_send(Work::Wake);
    }
    pub fn decode(&self, peer: String, data: Arc<[u8]>) -> bool {
        self.decode_codec(peer, data, VideoFormat::H264, Dimensions::default())
    }
    pub fn decode_codec(
        &self,
        peer: String,
        data: Arc<[u8]>,
        codec: VideoFormat,
        size: Dimensions,
    ) -> bool {
        self.work
            .try_send(Work::Decode(peer, data, codec, size))
            .is_ok()
    }
    pub fn use_cpu_output(&self, peer: String) {
        // A bounded blocking send here would stall rendering. Retry via the caller
        // if the queue is full instead.
        let _ = self.work.try_send(Work::CpuOutput(peer));
    }
    pub fn retain(&self, peers: Vec<String>) {
        let _ = self.work.try_send(Work::Retain(peers));
    }
    pub fn request_keyframe(&self) {
        self.force.store(true, Ordering::Relaxed);
        let _ = self.work.try_send(Work::Wake);
    }
    pub fn take_encoded(&self) -> Option<Encoded> {
        self.shared.lock().unwrap().encoded.pop_front()
    }
    pub fn take_decoded(&self) -> HashMap<String, DecodedFrame> {
        std::mem::take(&mut self.shared.lock().unwrap().decoded)
    }
    pub fn take_keyframe_requests(&self) -> HashSet<String> {
        std::mem::take(&mut self.shared.lock().unwrap().keyframe_requests)
    }
    pub fn diagnostics(&self) -> String {
        let s = self.shared.lock().unwrap();
        format!(
            "raw_coalesced={} encoded_drops={} encoder_skips={} encoded_queue={} raw_pending={}",
            s.raw_coalesced,
            s.encoded_drops,
            s.encoder_skips,
            s.encoded.len(),
            s.raw.is_some()
        )
    }
    pub fn backend(&self) -> String {
        self.shared.lock().unwrap().backend.clone()
    }
    pub fn take_error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.take()
    }
    pub fn clear_pending(&self) {
        let mut s = self.shared.lock().unwrap();
        s.raw = None;
        s.encoded.clear();
        let _ = self.work.try_send(Work::Reset);
    }
}
const SHADER: &[u8] = br#"
struct V { float4 pos : SV_POSITION; float2 uv : TEXCOORD; };
V vs(uint id : SV_VertexID) { V o; float2 uv=float2((id<<1)&2,id&2); o.pos=float4(uv.x*2-1,1-uv.y*2,0,1);o.uv=uv;return o; }
Texture2D image:register(t0); SamplerState linearSampler:register(s0);
float4 ps(V i):SV_TARGET { return float4(image.Sample(linearSampler,i.uv).rgb,1); }
"#;
// Three bounded staging textures overlap GPU copying with UI presentation.
struct StagingSlot {
    texture: ID3D11Texture2D,
    submitted: Option<(Instant, u64)>,
}
pub struct Readback {
    graphics: GraphicsDevice,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    target: Option<ID3D11Texture2D>,
    staging: Vec<StagingSlot>,
    pending: VecDeque<usize>,
    busy_polls: u64,
    submission_drops: u64,
    rtv: Option<ID3D11RenderTargetView>,
    size: Dimensions,
    started: Instant,
    settings: StreamConfig,
    read_latency: LatencyMeter,
    copy_latency: LatencyMeter,
    submit_latency: LatencyMeter,
}
impl Readback {
    pub fn new(graphics: GraphicsDevice) -> Result<Self> {
        unsafe {
            let mut vs = None;
            let mut ps = None;
            D3DCompile(
                SHADER.as_ptr().cast(),
                SHADER.len(),
                None,
                None,
                None,
                s!("vs"),
                s!("vs_4_0"),
                0,
                0,
                &mut vs,
                None,
            )?;
            D3DCompile(
                SHADER.as_ptr().cast(),
                SHADER.len(),
                None,
                None,
                None,
                s!("ps"),
                s!("ps_4_0"),
                0,
                0,
                &mut ps,
                None,
            )?;
            let vs = vs.unwrap();
            let ps = ps.unwrap();
            let mut vertex = None;
            let mut pixel = None;
            let mut sampler = None;
            graphics.device.CreateVertexShader(
                std::slice::from_raw_parts(vs.GetBufferPointer().cast(), vs.GetBufferSize()),
                None,
                Some(&mut vertex),
            )?;
            graphics.device.CreatePixelShader(
                std::slice::from_raw_parts(ps.GetBufferPointer().cast(), ps.GetBufferSize()),
                None,
                Some(&mut pixel),
            )?;
            graphics.device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MaxLOD: f32::MAX,
                    ComparisonFunc: D3D11_COMPARISON_NEVER,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            Ok(Self {
                graphics,
                vertex: vertex.unwrap(),
                pixel: pixel.unwrap(),
                sampler: sampler.unwrap(),
                target: None,
                staging: Vec::new(),
                pending: VecDeque::new(),
                busy_polls: 0,
                submission_drops: 0,
                rtv: None,
                size: Dimensions::default(),
                started: Instant::now(),
                settings: StreamConfig::default(),
                read_latency: LatencyMeter::new("gpu_readback_ready"),
                copy_latency: LatencyMeter::new("readback_cpu_copy"),
                submit_latency: LatencyMeter::new("gpu_readback_submit"),
            })
        }
    }
    pub fn set_quality(&mut self, quality: VideoQuality) {
        self.set_config(quality.into());
    }
    pub fn set_config(&mut self, settings: StreamConfig) {
        self.settings = settings;
    }
    pub fn clear_pending(&mut self) {
        self.pending.clear();
        self.staging.clear();
        self.target = None;
        self.rtv = None;
        self.size = Dimensions::default();
    }
    pub fn diagnostics(&self) -> String {
        format!(
            "readback_pending={} readback_busy_polls={} readback_submission_drops={}",
            self.pending.len(),
            self.busy_polls,
            self.submission_drops
        )
    }
    /// Poll only completed GPU copies. WAS_STILL_DRAWING is backpressure, never a wait.
    pub fn poll(&mut self) -> Result<Option<RawFrame>> {
        let Some(&index) = self.pending.front() else {
            return Ok(None);
        };
        let slot = &self.staging[index];
        let (submitted, micros) = slot.submitted.unwrap();
        let c = &self.graphics.context;
        let started = Instant::now();
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            match c.Map(
                &slot.texture,
                0,
                D3D11_MAP_READ,
                D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                Some(&mut mapped),
            ) {
                Err(error) if error.code() == DXGI_ERROR_WAS_STILL_DRAWING => {
                    self.busy_polls += 1;
                    return Ok(None);
                }
                result => result?,
            }
            let stride = self.size.width as usize * 4;
            let mut bytes = vec![0; stride * self.size.height as usize];
            for row in 0..self.size.height as usize {
                std::ptr::copy_nonoverlapping(
                    (mapped.pData as *const u8).add(row * mapped.RowPitch as usize),
                    bytes.as_mut_ptr().add(row * stride),
                    stride,
                );
            }
            c.Unmap(&slot.texture, 0);
            self.staging[index].submitted = None;
            self.pending.pop_front();
            let ready_ms = submitted.elapsed().as_secs_f64() * 1000.;
            let copy_ms = started.elapsed().as_secs_f64() * 1000.;
            self.read_latency.record(ready_ms);
            self.copy_latency.record(copy_ms);
            if diagnostics::enabled() && self.settings.log_per_frame {
                tracing::info!(ready_ms, copy_ms, micros, "gpu_readback_completed");
            }
            Ok(Some(RawFrame {
                queued_at: Instant::now(),
                size: self.size,
                bytes,
                micros,
            }))
        }
    }
    /// Queue GPU work; no resource is mapped until a later nonblocking poll.
    /// If the bounded ring is full, skip this raw frame before H.264 dependencies exist.
    pub fn submit(&mut self, input: &ID3D11Texture2D, source: Dimensions) -> Result<bool> {
        let configured = self.settings.bounds();
        let bounds = Dimensions::new(
            configured.width.max(configured.height),
            configured.width.min(configured.height),
        );
        let read_started = Instant::now();
        let ratio = (bounds.width as f64 / source.width.max(source.height).max(1) as f64)
            .min(bounds.height as f64 / source.width.min(source.height).max(1) as f64)
            .min(1.0);
        let size = Dimensions::new(
            ((source.width as f64 * ratio) as u32 & !1).max(2),
            ((source.height as f64 * ratio) as u32 & !1).max(2),
        );
        unsafe {
            if self.size != size {
                self.target = None;
                self.staging.clear();
                self.pending.clear();
                self.rtv = None;
                self.size = size;
                let mut desc = D3D11_TEXTURE2D_DESC {
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
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                    ..Default::default()
                };
                self.graphics
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut self.target))?;
                self.graphics.device.CreateRenderTargetView(
                    self.target.as_ref().unwrap(),
                    None,
                    Some(&mut self.rtv),
                )?;
                desc.Usage = D3D11_USAGE_STAGING;
                desc.BindFlags = 0;
                desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                for _ in 0..3 {
                    let mut texture = None;
                    self.graphics
                        .device
                        .CreateTexture2D(&desc, None, Some(&mut texture))?;
                    self.staging.push(StagingSlot {
                        texture: texture.unwrap(),
                        submitted: None,
                    });
                }
            }
            let Some(index) = self
                .staging
                .iter()
                .position(|slot| slot.submitted.is_none())
            else {
                self.submission_drops += 1;
                return Ok(false);
            };
            let mut srv = None;
            self.graphics
                .device
                .CreateShaderResourceView(input, None, Some(&mut srv))?;
            let c = &self.graphics.context;
            c.OMSetRenderTargets(Some(std::slice::from_ref(&self.rtv)), None);
            c.OMSetBlendState(None, Some(&[0.0; 4]), u32::MAX);
            c.OMSetDepthStencilState(None, 0);
            c.RSSetState(None);
            c.RSSetViewports(Some(&[D3D11_VIEWPORT {
                Width: size.width as f32,
                Height: size.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
                ..Default::default()
            }]));
            c.IASetInputLayout(None);
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            c.VSSetShader(&self.vertex, None);
            c.PSSetShader(&self.pixel, None);
            c.PSSetShaderResources(0, Some(&[srv]));
            c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            c.Draw(3, 0);
            c.PSSetShaderResources(0, Some(&[None]));
            c.OMSetRenderTargets(None, None);
            let slot = &mut self.staging[index];
            c.CopyResource(&slot.texture, self.target.as_ref().unwrap());
            slot.submitted = Some((read_started, self.started.elapsed().as_micros() as u64));
            self.pending.push_back(index);
            // Submit copies even if the window is occluded and no Present will flush.
            c.Flush();
            self.submit_latency
                .record(read_started.elapsed().as_secs_f64() * 1000.);
            Ok(true)
        }
    }
}
pub(crate) struct CodecApartment(bool);
impl CodecApartment {
    pub(crate) fn new() -> Self {
        Self(unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
            .is_ok()
        })
    }
}
impl Drop for CodecApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                windows::Win32::System::Com::CoUninitialize();
            }
        }
    }
}

#[cfg(test)]
mod readback_tests {
    use super::*;
    use std::time::Duration;

    fn texture(graphics: &GraphicsDevice, size: Dimensions, bgra: [u8; 4]) -> ID3D11Texture2D {
        let pixels = bgra.repeat((size.width * size.height) as usize);
        let desc = D3D11_TEXTURE2D_DESC {
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
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr().cast(),
            SysMemPitch: size.width * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        // SAFETY: initial data stays alive during CreateTexture2D; the returned texture owns its copy.
        unsafe {
            graphics
                .device
                .CreateTexture2D(&desc, Some(&data), Some(&mut texture))
                .unwrap();
        }
        texture.unwrap()
    }
    fn receive(readback: &mut Readback) -> RawFrame {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(frame) = readback.poll().unwrap() {
                return frame;
            }
            assert!(Instant::now() < deadline, "GPU readback never completed");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn pipelined_gpu_readback_preserves_pixels_pts_bounds_and_resize() {
        let _com = CodecApartment::new();
        let graphics = GraphicsDevice::new().unwrap();
        let mut readback = Readback::new(graphics.clone()).unwrap();
        let small = Dimensions::new(64, 64);
        let input = texture(&graphics, small, [31, 63, 127, 255]);
        for _ in 0..3 {
            assert!(readback.submit(&input, small).unwrap());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!readback.submit(&input, small).unwrap());
        assert_eq!(readback.pending.len(), 3);
        assert_eq!(readback.submission_drops, 1);
        let pts_before_poll = readback.started.elapsed().as_micros() as u64;
        std::thread::sleep(Duration::from_millis(20));
        let mut previous = None;
        for _ in 0..3 {
            let frame = receive(&mut readback);
            assert_eq!(frame.size, small);
            assert!(
                frame
                    .bytes
                    .chunks_exact(4)
                    .all(|pixel| pixel == [31, 63, 127, 255])
            );
            assert!(
                frame.micros <= pts_before_poll,
                "PTS must describe submission, not completion"
            );
            if let Some(pts) = previous {
                assert!(frame.micros > pts);
            }
            previous = Some(frame.micros);
        }
        assert!(readback.poll().unwrap().is_none());
        assert!(readback.submit(&input, small).unwrap());
        // A resize must retire pending copies rather than report old pixels with new dimensions.
        let large = Dimensions::new(128, 64);
        let resized = texture(&graphics, large, [17, 35, 71, 255]);
        assert!(readback.submit(&resized, large).unwrap());
        let frame = receive(&mut readback);
        assert_eq!(frame.size, large);
        assert!(
            frame
                .bytes
                .chunks_exact(4)
                .all(|pixel| pixel == [17, 35, 71, 255])
        );
        assert!(readback.submit(&resized, large).unwrap());
        readback.clear_pending();
        assert!(readback.poll().unwrap().is_none());
        assert!(readback.submit(&input, small).unwrap());
        assert_eq!(receive(&mut readback).size, small);
    }
}
