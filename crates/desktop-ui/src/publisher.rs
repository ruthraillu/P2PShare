//! Capture and publication are independent of GUI Present/vsync and its D3D context.
use crate::local_preview::{SharedPreview, SharedPreviewProducer};
use anyhow::{Result, anyhow};
use p2pshare_codecs_windows::{CodecWorker, GpuEncoder, Readback};
use p2pshare_media::{
    CaptureBackend, CaptureSource, CaptureState, Dimensions, config::StreamConfig,
    diagnostics::FrameMeter,
};
use p2pshare_platform_windows::{GpuFrame, GraphicsDevice, WindowsCapture};
use p2pshare_session::{Command, Session, VideoFrame};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Default, Clone)]
pub struct Health {
    pub backend: String,
    pub queues: String,
    pub readback: String,
    pub fps: f64,
    pub error: Option<String>,
    pub published_frames: u64,
    pub source_closed: bool,
    pub waiting_for_window: bool,
    pub capture_hint: Option<String>,
}
enum Control {
    Start(CaptureSource, mpsc::SyncSender<Result<(), String>>),
    Stop(mpsc::SyncSender<()>),
    Configure(StreamConfig,mpsc::SyncSender<Result<(),String>>),
    Quit,
}
pub struct Publisher {
    control: mpsc::SyncSender<Control>,
    health: Arc<Mutex<Health>>,
    preview: Arc<SharedPreview>,
    group_mode: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Publisher {
    pub fn new(session: Arc<Session>, settings: StreamConfig) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel(2);
        let (ready, initialized) = mpsc::sync_channel(1);
        let health = Arc::new(Mutex::new(Health::default()));
        let shared = health.clone();
        let preview = SharedPreview::new();
        let worker_preview = preview.clone();
        let group_mode = Arc::new(AtomicBool::new(false));
        let worker_group_mode = group_mode.clone();
        let thread = std::thread::Builder::new()
            .name("p2pshare-publisher".into())
            .spawn(move || {
                let owner = session.clone();
                let result = run(
                    session,
                    settings,
                    rx,
                    shared.clone(),
                    worker_preview,
                    worker_group_mode,
                    &ready,
                );
                if let Err(error) = result {
                    owner.command(Command::Unpublish);
                    let text = format!("Capture publisher: {error:#}");
                    let _ = ready.try_send(Err(text.clone()));
                    tracing::error!(%error, "publisher_worker_error");
                    shared.lock().unwrap().error = Some(text);
                }
            })?;
        initialized
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| anyhow!(e))?
            .map_err(|e| anyhow!(e))?;
        Ok(Self {
            control: tx,
            health,
            preview,
            group_mode,
            thread: Some(thread),
        })
    }
    pub fn set_group_mode(&self, enabled: bool) {
        self.group_mode.store(enabled, Ordering::Release);
    }
    pub fn preview(&self) -> Arc<SharedPreview> {
        self.preview.clone()
    }
    pub fn set_preview_enabled(&self, enabled: bool) {
        self.preview.set_enabled(enabled);
    }
    pub fn start(&self, source: CaptureSource) -> Result<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.control.send(Control::Start(source, tx))?;
        rx.recv_timeout(Duration::from_secs(5))?
            .map_err(|e| anyhow!(e))
    }
    pub fn configure(&self,settings:StreamConfig)->Result<()> {
        let (tx,rx)=mpsc::sync_channel(1);self.control.send(Control::Configure(settings,tx))?;rx.recv_timeout(Duration::from_secs(5))?.map_err(|e|anyhow!(e))
    }
    pub fn stop(&self) {
        let (tx, rx) = mpsc::sync_channel(1);
        if self.control.send(Control::Stop(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }
    pub fn health(&self) -> Health {
        self.health.lock().unwrap().clone()
    }
    pub fn take_error(&self) -> Option<String> {
        self.health.lock().unwrap().error.take()
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::WinRT::RoUninitialize();
        }
    }
}
fn run(
    session: Arc<Session>,
    mut settings: StreamConfig,
    control: mpsc::Receiver<Control>,
    health: Arc<Mutex<Health>>,
    preview: Arc<SharedPreview>,
    group_mode: Arc<AtomicBool>,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<()> {
    // Capture, readback and their immediate context live only on this MTA worker.
    unsafe {
        windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        )?;
    }
    let _apartment = Apartment;
    let graphics = GraphicsDevice::new()?;
    let mut capture = WindowsCapture::new(graphics.clone());
    capture.set_cursor_capture(settings.show_cursor)?;
    let mut preview_producer = SharedPreviewProducer::new(preview);
    let mut preview_was_enabled = false;
    let mut last_min_interval: Option<Duration> = None;
    let mut readback = Readback::new(graphics.clone())?;
    readback.set_config(settings);
    let codecs = CodecWorker::new()?;
    codecs.set_config(settings);
    let _ = ready.send(Ok(()));
    tracing::info!("independent_capture_publisher_ready");
    let mut gpu: Option<GpuEncoder> = None;
    let mut source_size = Dimensions::default();
    let mut gpu_failed = false;
    let mut last_pts = 0;
    let mut gpu_meter = FrameMeter::new("encoded").with_fps(settings.fps);
    let mut active = false;
    let mut preview_renderer = None;
    let mut latest_frame: Option<GpuFrame> = None;
    let mut pending_keyframe = false;
    let mut was_live = false;
    let mut next_preview = Instant::now();
    let mut published = Dimensions::default();
    let mut interval = Duration::from_micros(1_000_000 / settings.fps as u64);
    let mut next_capture = Instant::now();
    let mut meter = FrameMeter::new("capture").with_fps(settings.fps);
    let mut window_frames = 0;
    let mut window_since = Instant::now();
    let mut reported = Instant::now();
    loop {
        let command = match control.recv_timeout(if active && session.needs_live_video() {
            Duration::from_millis(1)
        } else if active {
            Duration::from_millis(20)
        } else {
            Duration::from_millis(100)
        }) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Some(command) = command {
            let command=match command {
                Control::Configure(value,reply)=>{
                    if let Err(error)=capture.set_cursor_capture(value.show_cursor){let _=reply.send(Err(format!("Could not change mouse pointer capture: {error}")));continue;}
                    if cursor_only_change(settings,value){settings=value;latest_frame=None;preview_producer.clear();next_preview=Instant::now();let _=reply.send(Ok(()));continue;}
                    settings=value;interval=Duration::from_micros(1_000_000/settings.fps as u64);
                    gpu=None;gpu_failed=false;source_size=Dimensions::default();pending_keyframe=true;
                    readback.clear_pending();codecs.clear_pending();readback.set_config(settings);codecs.set_config(settings);
                    published=Dimensions::default();next_capture=Instant::now();next_preview=Instant::now();last_min_interval=None;
                    session.set_stream_info(p2pshare_session::StreamInfo{codec:if settings.codec==p2pshare_media::config::Codec::Auto{None}else{Some(settings.codec.format())},bitrate_kbps:settings.bitrate_kbps});
                    let _=reply.send(Ok(()));continue;
                },other=>other
            };
            let switching=active && matches!(&command,Control::Start(..));
            let mut replacement=None;
            if let Control::Start(source,reply)=&command {
                let mut next=WindowsCapture::new(graphics.clone());
                next.set_min_update_interval(last_min_interval);
                if let Err(error)=next.set_cursor_capture(settings.show_cursor).and_then(|_|next.start(source)){let _=reply.send(Err(format!("{error:#}")));continue;}
                replacement=Some(next);
            }
            health.lock().unwrap().source_closed = false;
            capture.stop()?;
            preview_producer.clear();
            gpu = None;
            source_size = Dimensions::default();
            latest_frame = None;
            pending_keyframe = false;
            was_live = false;
            gpu_failed = false;
            active = false;
            readback.clear_pending();
            codecs.clear_pending();
            published = Dimensions::default();
            if !switching {session.command(Command::Unpublish);}
            match command {
                Control::Configure(..)=>unreachable!(),
                Control::Quit => break,
                Control::Stop(reply) => {
                    let mut h = health.lock().unwrap();
                    h.waiting_for_window = false;
                    h.capture_hint = None;
                    let _ = reply.send(());
                }
                Control::Start(_source, reply) => {
                    if reply.send(Ok(())).is_ok() {
                        capture = replacement.expect("Prepared capture source");
                        active = true;
                        session.set_stream_info(p2pshare_session::StreamInfo {
                            codec: if settings.codec == p2pshare_media::config::Codec::Auto {
                                None
                            } else {
                                Some(settings.codec.format())
                            },
                            bitrate_kbps: settings.bitrate_kbps,
                        });
                        next_capture = Instant::now();
                        next_preview = Instant::now();
                        let mut h = health.lock().unwrap();
                        h.error = None;
                        h.waiting_for_window = capture.waiting_for_restore();
                        h.capture_hint = capture.status_hint().map(str::to_owned);
                    } else {
                        if let Some(mut next) = replacement {
                            let _ = next.stop();
                        }
                        active = false;
                        session.command(Command::Unpublish);
                        tracing::warn!("publisher_start_aborted_caller_timed_out");
                    }
                }
            }
        }
        if active
            && matches!(
                capture.state(),
                CaptureState::SourceClosed | CaptureState::Stopped
            )
        {
            {
                let mut h = health.lock().unwrap();
                h.source_closed = true;
                h.waiting_for_window = false;
                h.capture_hint = None;
            }
            capture.stop()?;
            preview_producer.clear();
            gpu = None;
            source_size = Dimensions::default();
            latest_frame = None;
            pending_keyframe = false;
            was_live = false;
            gpu_failed = false;
            active = false;
            readback.clear_pending();
            codecs.clear_pending();
            if !group_mode.load(Ordering::Acquire) {
                session.command(Command::Unpublish);
            }
        }
        if active {
            if let Some(mut raw) = readback.poll()? {
                raw.micros = session.media_micros().max(last_pts + 100);
                last_pts = raw.micros;
                if raw.size != published {
                    published = raw.size;
                    session.command(Command::Publish {
                        width: published.width,
                        height: published.height,
                    });
                }
                codecs.encode(raw);
            }
            let now = Instant::now();
            let live = session.needs_live_video();
            let preview_enabled = preview_producer.is_enabled();
            if !preview_enabled {
                preview_producer.clear();
            }
            let min_interval = if live {
                Some(Duration::from_micros(
                    (1_000_000 / u64::from(settings.fps.max(1)) / 2).max(1_000),
                ))
            } else if preview_enabled {
                Some(Duration::from_millis(16))
            } else {
                Some(Duration::from_millis(250))
            };
            if last_min_interval != min_interval {
                last_min_interval = min_interval;
                capture.set_min_update_interval(min_interval);
            }
            pending_keyframe |= session.needs_keyframe();
            if live != was_live {
                meter = FrameMeter::new("capture").with_fps(settings.fps);
                gpu_meter = FrameMeter::new("encoded").with_fps(settings.fps);
                window_frames = 0;
                window_since = now;
                tracing::info!(live, "publisher_live_encoding_changed");
            }
            if live && !was_live {
                pending_keyframe = true;
                next_capture = now;
            }
            was_live = live;
            let frame = if now >= next_capture {
                let waiting_before = capture.waiting_for_restore();
                let fresh = capture.poll_frame()?.filter(|frame|{
                    let source=frame.metadata.size;let output=source.fit(settings.bounds());
                    p2pshare_platform_windows::usable_capture_size(source)&&p2pshare_platform_windows::usable_capture_size(output)
                });
                if capture.waiting_for_restore() || waiting_before != capture.waiting_for_restore() {
                    let mut h = health.lock().unwrap();
                    h.waiting_for_window = capture.waiting_for_restore();
                    h.capture_hint = capture.status_hint().map(str::to_owned);
                }
                if let Some(frame) = &fresh {
                    if frame.metadata.sequence == 1 {
                        // Refresh room preview immediately after attach/reattach or fullscreen fallback.
                        next_preview = now;
                    }
                    // Keep the owned GPU copy on this thread. WGC may send no new
                    // frames for a static source when somebody clicks Watch.
                    latest_frame = Some(GpuFrame {
                        texture: frame.texture.clone(),
                        metadata: frame.metadata,
                    });
                    let mut h = health.lock().unwrap();
                    h.waiting_for_window = false;
                    h.capture_hint = capture.status_hint().map(str::to_owned);
                    drop(h);
                    if preview_enabled {
                        preview_producer.publish(&graphics, &frame.texture, frame.metadata.size);
                    }
                } else if preview_enabled
                    && !preview_was_enabled
                    && let Some(frame) = &latest_frame
                {
                    preview_producer.publish(&graphics, &frame.texture, frame.metadata.size);
                }
                fresh.or_else(|| {
                    // Keep the encoder fed at the configured FPS even when WGC
                    // reports no changes. Inter frames can preserve/refine detail;
                    // do not restart a still picture with isolated keyframes.
                    let refresh = live || now >= next_preview;
                    latest_frame
                        .as_ref()
                        .filter(|_| refresh)
                        .map(|frame| GpuFrame {
                            texture: frame.texture.clone(),
                            metadata: frame.metadata,
                        })
                })
            } else {
                None
            };
            preview_was_enabled = preview_enabled;
            if frame.is_none() && now >= next_capture {
                next_capture = now + if capture.waiting_for_restore() {
                    Duration::from_millis(50)
                } else if !live {
                    Duration::from_millis(33)
                } else {
                    Duration::from_millis(2)
                };
            }
            if let Some(frame) = frame {
                if live {
                    meter.frame(0);
                    window_frames += 1;
                }
                if source_size != frame.metadata.size {
                    source_size = frame.metadata.size;
                    gpu = None;
                    gpu_failed = false;
                    readback.clear_pending();
                    codecs.clear_pending();
                    pending_keyframe = true;
                }
                if now >= next_preview {
                    if preview_renderer.is_none() {
                        preview_renderer = Some((
                            egui_directx11::Renderer::new(&graphics.device)?,
                            egui::Context::default(),
                        ));
                    }
                    let (renderer, ctx) = preview_renderer.as_mut().unwrap();
                    match crate::thumbnails::jpeg_snapshot(
                        &graphics,
                        renderer,
                        ctx,
                        &frame.texture,
                    ) {
                        Ok(bytes) => {
                            tracing::info!(
                                bytes = bytes.len(),
                                interval_secs = settings.preview_interval_secs,
                                "share_jpeg_preview_encoded"
                            );
                            session.publish_preview(bytes);
                        }
                        Err(error) => tracing::warn!(%error, "share_jpeg_preview_failed"),
                    }
                    next_preview = now + Duration::from_secs(settings.preview_interval_secs as u64);
                }
                if !live {
                    // Announce the share and keep JPEGs current without opening a full
                    // encoder until a viewer explicitly subscribes.
                    let fit = source_size.fit(settings.bounds());
                    let size =
                        Dimensions::new((fit.width / 2 * 2).max(2), (fit.height / 2 * 2).max(2));
                    if published != size {
                        published = size;
                        session.command(Command::Publish {
                            width: size.width,
                            height: size.height,
                        });
                    }
                    gpu = None;
                    readback.clear_pending();
                    codecs.clear_pending();
                    next_capture = now + if preview_enabled {
                        Duration::from_millis(33)
                    } else {
                        Duration::from_millis(250)
                    };
                } else {
                    if gpu.is_none()
                        && !gpu_failed
                        && settings.encoder != p2pshare_media::config::EncoderBackend::Software
                    {
                        match GpuEncoder::new(graphics.clone(), source_size, settings) {
                            Ok(encoder) => gpu = Some(encoder),
                            Err(error) => {
                                gpu_failed = true;
                                tracing::warn!(
                                    error = format!("{error:#}"),
                                    "gpu_encoder_unavailable_using_readback"
                                );
                            }
                        }
                    }
                    if let Some(encoder) = &mut gpu {
                        let size = encoder.size();
                        if published != size {
                            published = size;
                            session.command(Command::Publish {
                                width: size.width,
                                height: size.height,
                            });
                        }
                        // New encoder/source, new viewer or recovery feedback only.
                        // Periodic IDRs visibly discard refined detail on static text.
                        let keyframe = std::mem::take(&mut pending_keyframe);
                        let pts = session.media_micros().max(last_pts + 100);
                        last_pts = pts;
                        match encoder.encode(&frame.texture, pts, keyframe) {
                            Ok(frames) => {
                                for encoded in frames {
                                    gpu_meter.frame(encoded.bytes.len());
                                    session.publish(VideoFrame {
                                        codec: encoded.codec,
                                        data: encoded.bytes.into(),
                                        micros: encoded.micros,
                                        keyframe: encoded.keyframe,
                                    });
                                    health.lock().unwrap().published_frames += 1;
                                }
                            }
                            Err(error) => {
                                encoder.runtime_failure(&error);
                                tracing::warn!(
                                    error = format!("{error:#}"),
                                    "gpu_encoder_failed_using_readback"
                                );
                                gpu = None;
                                gpu_failed = true;
                                codecs.request_keyframe();
                            }
                        }
                    }
                    if gpu.is_none() {
                        readback.submit(&frame.texture, frame.metadata.size)?;
                    }
                    next_capture += interval;
                    if next_capture <= now {
                        next_capture = now + interval;
                    }
                }
            }
            if live && gpu.is_none() && pending_keyframe {
                codecs.request_keyframe();
                pending_keyframe = false;
            }
            while let Some(frame) = codecs.take_encoded() {
                session.publish(VideoFrame {
                    codec: frame.codec,
                    data: frame.bytes.into(),
                    micros: frame.micros,
                    keyframe: frame.keyframe,
                });
                health.lock().unwrap().published_frames += 1;
            }
        }
        meter.report();
        if gpu.is_some() {
            gpu_meter.report();
        }
        if reported.elapsed() >= Duration::from_millis(250) {
            let waiting_for_window = active && capture.waiting_for_restore();
            let capture_hint = if active {
                capture.status_hint().map(str::to_owned)
            } else {
                None
            };
            let mut h = health.lock().unwrap();
            h.backend = if active && !session.needs_live_video() {
                "Idle (no viewers)".into()
            } else {
                gpu.as_ref()
                    .map_or_else(|| codecs.backend(), |g| g.backend())
            };
            h.queues = codecs.diagnostics();
            h.readback = readback.diagnostics();
            h.waiting_for_window = waiting_for_window;
            h.capture_hint = capture_hint;
            if window_since.elapsed() >= Duration::from_secs(1) {
                h.fps = window_frames as f64 / window_since.elapsed().as_secs_f64();
                window_frames = 0;
                window_since = Instant::now();
            }
            if let Some(error) = codecs.take_error() {
                h.error = Some(error);
            }
            reported = Instant::now();
        }
    }
    capture.stop()?;
    session.command(Command::Unpublish);
    Ok(())
}

/// Capture-only changes must leave encoder state and viewer subscriptions untouched.
pub(crate) fn cursor_only_change(mut before:StreamConfig,after:StreamConfig)->bool{before.show_cursor=after.show_cursor;before==after}
#[cfg(test)]mod cursor_tests{
 use super::*;
 #[test]fn cursor_toggle_does_not_require_reencoding_configuration(){let before=StreamConfig::default();let mut after=before;after.show_cursor=false;assert!(cursor_only_change(before,after));after.bitrate_kbps+=100;assert!(!cursor_only_change(before,after));}
}
