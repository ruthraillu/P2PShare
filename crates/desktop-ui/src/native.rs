#[path = "design_adapter.rs"]
mod design_adapter;
#[path="profile.rs"]
mod profile;
#[path = "detached.rs"]
mod detached;
#[path = "detached_fit.rs"]
mod detached_fit;
#[path = "video_texture.rs"]
mod video_texture;
use p2pshare_codecs_windows::CodecWorker;
use p2pshare_media::{CaptureState, Dimensions};
use p2pshare_platform_windows::{GraphicsDevice, WindowsCapture, enumerate_sources};
use p2pshare_room_ui::{Action, RoomUi};
use p2pshare_session::{Command, Session};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
        System::WinRT::*,
    },
    core::{Interface, Result},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

fn app_icon() -> winit::window::Icon {
    static ICON: std::sync::OnceLock<winit::window::Icon> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        winit::window::Icon::from_rgba(
            include_bytes!("../../../assets/app-icon-64.rgba").to_vec(),
            64,
            64,
        )
        .expect("Embedded app icon is valid RGBA")
    })
    .clone()
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}
pub fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // SAFETY: initialized and released on this thread; winit owns the Win32 message pump.
    unsafe {
        RoInitialize(RO_INIT_SINGLETHREADED)?;
    }
    let _apartment = Apartment;
    let event_loop = EventLoop::new()?;
    let mut app = Runner {
        view: None,
        popouts: HashMap::new(),
        error: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error.into());
    }
    Ok(())
}
// egui-winit deliberately reports repaint=true for RedrawRequested. That
// response means paint THIS event, not queue another. Live video is scheduled
// in about_to_wait; static room polls use its deadline instead of a feedback loop.
fn input_requests_redraw(event: &WindowEvent, repaint: bool) -> bool {
    repaint && !matches!(event, WindowEvent::RedrawRequested)
}

// Only transient presentation/access errors are retried. Device removal and
// programming errors still go through the normal fatal diagnostic path.
fn transient_display_error(code:i32)->bool{matches!(code as u32,0x887A000A|0x887A0022|0x887A0026|0x887A0028)}
#[cfg(test)]mod display_transition_tests{
 #[test]fn retry_only_recoverable_display_errors(){for code in [0x887A000Au32,0x887A0022,0x887A0026,0x887A0028]{assert!(super::transient_display_error(code as i32));}for code in [0,0x887A0005u32,0x887A0006,0x80070057]{assert!(!super::transient_display_error(code as i32));}}
}
struct Runner {
    view: Option<View>,
    popouts: HashMap<u64, detached::Detached>,
    error: Option<String>,
}
impl Runner {
    fn failed(&mut self, event_loop: &ActiveEventLoop, error: impl std::fmt::Display) {
        tracing::error!(%error, "Desktop UI failure");
        self.error = Some(error.to_string());
        event_loop.exit();
    }
}
impl ApplicationHandler for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.view.is_some() {
            return;
        }
        let result = (|| -> std::result::Result<View, Box<dyn std::error::Error>> {
            let window = event_loop.create_window(
                Window::default_attributes()
                    .with_title("P2PShare • Screen lounge")
                    .with_window_icon(Some(app_icon()))
                    // Keep the uninitialized Win32 surface out of the compositor.
                    .with_visible(
                        std::env::args().any(|a| a == "--ui-smoke" || a == "--capture-ui-smoke"),
                    )
                    .with_decorations(
                        std::env::args().any(|a| a == "--ui-smoke" || a == "--capture-ui-smoke"),
                    )
                    .with_inner_size(LogicalSize::new(1280.0, 832.0))
                    .with_min_inner_size(
                        if std::env::args().any(|a| a == "--ui-smoke" || a == "--capture-ui-smoke")
                        {
                            LogicalSize::new(360.0, 480.0)
                        } else {
                            LogicalSize::new(900.0, 652.0)
                        },
                    ),
            )?;
            View::new(window)
        })();
        match result {
            Ok(mut view) => {
                if !view.smoke && !view.capture_smoke {
                    // The first egui pass measures floating dialogs/font layout. Paint
                    // again while hidden so the first visible frame is complete.
                    for _ in 0..2 {
                        view.force_present = true;
                        if let Err(error) = view.render(event_loop) {
                            self.failed(event_loop, error);
                            return;
                        }
                    }
                    if let Err(error) = view.window_memory.restore(view.hwnd, true) {
                        tracing::warn!(%error, "window_state_restore_failed");
                    }
                    view.window.set_visible(true);
                    view.request_redraw();
                    tracing::info!("startup_ui_ready");
                }
                self.view = Some(view);
            }
            Err(error) => self.failed(event_loop, error),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(peer) = self
            .popouts
            .iter()
            .find_map(|(peer, v)| (v.window.id() == id).then_some(*peer))
        {
            if let Err(error) = self.child_event(event_loop, peer, event) {
                tracing::warn!(%error, peer, "popout_window_failed");
                self.popouts.remove(&peer);
                if let Some(parent) = &mut self.view {
                    parent.design.detached_peers.remove(&peer);
                    parent.force_present = true;
                    parent.request_redraw();
                }
            }
            return;
        }
        let Some(view) = &mut self.view else {
            return;
        };
        if id != view.window.id() {
            return;
        }
        if matches!(&event, WindowEvent::RedrawRequested) {
            // Timer/input polls can reuse the compositor's last frame, but an
            // unsolicited OS paint must refresh even if egui shapes are unchanged.
            view.force_present |= !std::mem::take(&mut view.redraw_requested);
        }
        let response = view.input.on_window_event(&view.window, &event);
        if input_requests_redraw(&event, response.repaint) {
            view.request_redraw();
        }
        if matches!(
            &event,
            WindowEvent::Focused(true)
                | WindowEvent::Occluded(false)
                | WindowEvent::ScaleFactorChanged { .. }
        ) {
            view.force_present = true;
        }
        let painted = matches!(&event, WindowEvent::RedrawRequested);
        let result = match event {
            WindowEvent::CloseRequested => {
                view.close(event_loop);
                Ok(())
            }
            WindowEvent::Resized(size) => {
                view.window_memory
                    .capture(view.hwnd, view.window.fullscreen().is_some());
                view.resize(size)
            }
            WindowEvent::Moved(_) => {
                view.window_memory
                    .capture(view.hwnd, view.window.fullscreen().is_some());
                Ok(())
            }
            WindowEvent::RedrawRequested => view.render(event_loop),
            WindowEvent::Occluded(value) => {
                view.occluded = value;
                if value {
                    view.thumbnails.set_sources(Vec::new());
                    // Keep completed static snapshots; cancel any unfinished capture.
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            if transient_display_error(error.code().0){
                tracing::warn!(%error,"display_transition_retry");
                view.target=None;let retry=Instant::now()+Duration::from_millis(250);view.graphics_retry=Some(retry);view.next_redraw=retry;
                return;
            }
            self.failed(event_loop, error);
            return;
        }
        if painted && let Err(error) = self.update_popouts(event_loop) {
            tracing::warn!(%error, "popout_update_failed");
        }
    }
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(view) = &mut self.view {
            view.window_memory
                .capture(view.hwnd, view.window.fullscreen().is_some());
            view.window_memory.flush(true);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let popout_live = self.popouts.values().any(|child| {
            !child.occluded && child.target.is_some() && child.app.has_visible_live_video()
        });
        if let Some(view) = &mut self.view {
            view.window_memory.flush(false);
            if (view.ui.in_room || view.ui.capture_running)
                && ((view.smoke || view.capture_smoke || view.design.has_visible_live_video())
                    && !view.occluded
                    && view.target.is_some()
                    || popout_live)
            {
                // Present(1) already limits rendering to display refresh. An additional
                // timer after Present skips refreshes and makes frame delivery uneven.
                view.request_redraw();
                event_loop.set_control_flow(ControlFlow::Poll);
                return;
            }
            let interval = if view.ui.in_room || view.ui.capture_running {
                if view.smoke || view.capture_smoke || view.design.has_visible_live_video() {
                    // Keep the old receive/decode polling cadence for live streams,
                    // including an occluded/minimized window.
                    Duration::from_micros(1_000_000 / view.stream_config.fps as u64)
                } else {
                    // JPEG-only/hidden previews need room updates, not continuous redraws.
                    // Input events still request an immediate redraw.
                    Duration::from_millis(50)
                }
            } else {
                Duration::from_millis(200)
            };
            if Instant::now() >= view.next_redraw {
                view.request_redraw();
                view.next_redraw = Instant::now() + interval;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(view.next_redraw));
        }
    }
}
struct View {
    window: Window,
    close_requested: bool,
    updater: crate::updater::Updater,
    popout_requests: Vec<u64>,
    maximized_before_fullscreen: bool,
    window_memory: crate::window_memory::WindowMemory,
    hwnd: HWND,
    graphics: GraphicsDevice,
    swap: IDXGISwapChain1,
    target: Option<ID3D11RenderTargetView>,
    graphics_retry:Option<Instant>,
    renderer: egui_directx11::Renderer,
    ctx: egui::Context,
    input: egui_winit::State,
    capture: crate::local_preview::LocalPreview<WindowsCapture>,
    ui: RoomUi,
    design: p2pshare_desktop_design::app::App,
    app_config: p2pshare_session::AppConfig,
    firewall: crate::firewall::Firewall,
    config_path: std::path::PathBuf,
    thumbnails: crate::thumbnails::Thumbnails,
    preview_textures: HashMap<u64, egui::TextureHandle>,
    picker_open: bool,
    active_source: Option<u64>,
    session: Arc<Session>,
    codecs: CodecWorker,
    publisher: crate::publisher::Publisher,
    audio: Option<crate::audio::AudioRuntime>,
    stream_config: p2pshare_media::config::StreamConfig,
    capture_meter: p2pshare_media::diagnostics::FrameMeter,
    preview_hidden_last: bool,
    telemetry_at: Instant,
    cpu_profile: crate::cpu_profile::CpuProfiler,
    preview_meter: p2pshare_media::diagnostics::FrameMeter,
    update_latency: p2pshare_media::diagnostics::LatencyMeter,
    draw_latency: p2pshare_media::diagnostics::LatencyMeter,
    present_latency: p2pshare_media::diagnostics::LatencyMeter,
    remote_textures: HashMap<String, Option<video_texture::VideoTexture>>,
    room_previews: HashMap<String, egui::TextureHandle>,
    avatars:profile::Store,
    preview_epoch: u64,
    last_members: Vec<String>,
    published_size: p2pshare_media::Dimensions,
    invite_out: Option<std::path::PathBuf>,
    test_receiver: Option<Session>,
    test_joined: bool,
    test_decoded: bool,
    registered: Option<(ID3D11Texture2D, egui::TextureId)>,
    next_redraw: Instant,
    force_present: bool,
    redraw_requested: bool,
    presented_frames: u64,
    presented_last_frame: bool,
    skipped_presentations: u64,
    occluded: bool,
    smoke: bool,
    capture_smoke: bool,
    viewer_smoke: bool,
    pacing_smoke: bool,
    independence_checked: bool,
    stage_at: Instant,
    smoke_stage: u8,
    started: Instant,
    test_source: Option<crate::test_source::TestSource>,
}
impl View {
    fn request_redraw(&mut self) {
        self.redraw_requested = true;
        self.window.request_redraw();
    }
    fn new(window: Window) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
            return Err("Expected a Windows window handle".into());
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        let state_path = p2pshare_media::portable::data_dir()?.join("window-state.toml");
        let remember = !std::env::args().any(|a| a == "--ui-smoke" || a == "--capture-ui-smoke");
        let window_memory = crate::window_memory::WindowMemory::new(state_path, remember);
        if let Err(error) = window_memory.restore(hwnd, false) {
            tracing::warn!(%error,"window_state_restore_failed");
        }
        let graphics = GraphicsDevice::new()?;
        let size = window.inner_size();
        // SAFETY: live window and COM D3D11 objects; the swap chain outlives its back buffer.
        let swap = unsafe {
            let dxgi: IDXGIDevice = graphics.device.cast()?;
            // Keep only one queued presentation instead of the default three:
            // input/capture should follow the displayed frame, not a GPU backlog.
            dxgi.cast::<IDXGIDevice1>()?.SetMaximumFrameLatency(1)?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: size.width,
                Height: size.height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                Scaling: DXGI_SCALING_STRETCH,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForHwnd(&graphics.device, hwnd, &desc, None, None)?;
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
            swap
        };
        let renderer = egui_directx11::Renderer::new(&graphics.device)?;
        let ctx = egui::Context::default();
        RoomUi::configure(&ctx);
        let input = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(16384),
        );
        let capture =
            crate::local_preview::LocalPreview::new(WindowsCapture::new(graphics.clone()));
        let smoke = std::env::args().any(|a| a == "--ui-smoke");
        let capture_smoke = std::env::args().any(|a| a == "--capture-ui-smoke");
        let test_source = if capture_smoke {
            Some(crate::test_source::TestSource::new()?)
        } else {
            None
        };
        if std::env::args().any(|a| a == "--hd-capture-smoke")
            && let Some(source) = &test_source
        {
            source.prepare_hd();
        }
        let config_path = p2pshare_media::portable::config_path()?;
        let app_config = design_adapter::load_config(&config_path)?;
        let mut design = p2pshare_desktop_design::app::App::live(
            design_adapter::ui_config(&app_config),
            app_config.room_name.clone(),
            !app_config.profile_setup_complete || app_config.username.is_empty(),
        );
        design.language_prompt=app_config.language.is_none();
        design.signaling_setup = app_config.signaling_url.is_empty().then(String::new);
        let thumbnails = crate::thumbnails::Thumbnails::new()?;
        let mut ui = RoomUi::new(true);
        ui.room_name = app_config.room_name.clone();
        let args: Vec<String> = std::env::args().collect();
        let arg = |name: &str| args.windows(2).find(|a| a[0] == name).map(|a| a[1].clone());
        ui.name = arg("--name").unwrap_or(if capture_smoke {
            "Capture publisher".into()
        } else {
            app_config.username.clone()
        });
        let data = arg("--data-dir")
            .map(std::path::PathBuf::from)
            .unwrap_or(p2pshare_media::portable::data_dir()?);
        let session = if capture_smoke {
            Session::start_isolated(data.join("identity.key"))?
        } else {
            Session::start_cloudflare(
                data.join("identity.key"),
                app_config.signaling_url.clone(),
                false,
            )?
        };
        session.set_playback_config(app_config.playback)?;
        let test_receiver = if capture_smoke {
            session.command(Command::Create("Capture publisher".into()));
            Some(Session::start_isolated(data.join("receiver.key"))?)
        } else {
            None
        };
        if args.iter().any(|a| a == "--create-room") {
            session.command(Command::NamedRoom {
                room: ui.room_name.clone(),
                name: ui.name.clone(),
            });
        }
        if let Some(invite) = arg("--join") {
            session.command(Command::Join {
                invite,
                name: arg("--name").unwrap_or("Windows".into()),
            });
        }
        let codecs = CodecWorker::with_display_device(if capture_smoke {
            None
        } else {
            Some(graphics.device.clone())
        })?;
        let stream_config = app_config.stream;
        codecs.set_config(stream_config);
        tracing::info!(?stream_config, "stream_configuration");
        let session = Arc::new(session);
        let publisher = crate::publisher::Publisher::new(session.clone(), stream_config)?;
        let audio = crate::audio::AudioRuntime::new(session.clone(), app_config.audio,app_config.audio_exclusions.clone())?;
        ui.video_config_external = true;
        ui.encoder = format!(
            "{:?} • {}x{} / {} FPS / {} kbps (config.toml)",
            stream_config.codec,
            stream_config.width,
            stream_config.height,
            stream_config.fps,
            stream_config.bitrate_kbps
        );
        let invite_out = arg("--invite-out").map(std::path::PathBuf::from);
        let mut view = Self {
            window,
            close_requested: false,
            updater: crate::updater::Updater::new(p2pshare_media::portable::data_dir()?, !smoke && !capture_smoke),
            popout_requests: Vec::new(),
            maximized_before_fullscreen: false,
            window_memory,
            hwnd,
            graphics,
            swap,
            target: None,
            graphics_retry:None,
            renderer,
            ctx,
            input,
            capture,
            ui,
            design,
            app_config,
            firewall: crate::firewall::Firewall::new(
                std::env::current_exe()?,
                !smoke && !capture_smoke,
            ),
            config_path,
            thumbnails,
            preview_textures: HashMap::new(),
            picker_open: false,
            active_source: None,
            session,
            codecs,
            publisher,
            audio: Some(audio),
            stream_config,
            capture_meter: p2pshare_media::diagnostics::FrameMeter::new("local_preview_capture")
                .with_fps(stream_config.fps),
            preview_hidden_last: false,
            telemetry_at: Instant::now(),
            cpu_profile: crate::cpu_profile::CpuProfiler::new(),
            preview_meter: p2pshare_media::diagnostics::FrameMeter::new("windows_preview")
                .with_fps(60),
            update_latency: p2pshare_media::diagnostics::LatencyMeter::new("windows_update"),
            draw_latency: p2pshare_media::diagnostics::LatencyMeter::new("windows_draw"),
            present_latency: p2pshare_media::diagnostics::LatencyMeter::new("windows_present"),
            invite_out,
            test_receiver,
            test_joined: false,
            test_decoded: false,
            remote_textures: HashMap::new(),
            room_previews: HashMap::new(),
            avatars:Default::default(),
            preview_epoch: 0,
            last_members: vec![],
            published_size: p2pshare_media::Dimensions::default(),
            registered: None,
            next_redraw: Instant::now(),
            force_present: true,
            redraw_requested: false,
            presented_frames: 0,
            presented_last_frame: false,
            skipped_presentations: 0,
            occluded: false,
            smoke,
            capture_smoke,
            viewer_smoke: std::env::args().any(|a| a == "--viewer-ui-smoke"),
            pacing_smoke: std::env::args().any(|a| a == "--pacing-smoke"),
            independence_checked: false,
            stage_at: Instant::now(),
            smoke_stage: 0,
            started: Instant::now(),
            test_source,
        };
        view.create_target()?;
        if capture_smoke {
            let sources = enumerate_sources(Some(hwnd))?;
            if let Some(source) = sources
                .iter()
                .find(|s| s.title == "P2PShare egui test source")
            {
                view.capture.start(source)?;
                view.publisher.start(source.clone())?;
                view.ui.capture_running = true;
            } else {
                return Err("Synthetic capture source not enumerated".into());
            }
        }
        Ok(view)
    }
    fn set_viewer_fullscreen(&mut self, fullscreen: bool) {
        if fullscreen == self.window.fullscreen().is_some() {
            return;
        }
        self.window_memory
            .capture(self.hwnd, self.window.fullscreen().is_some());
        if fullscreen {
            self.maximized_before_fullscreen = self.window.is_maximized();
            let monitor = self.window.current_monitor();
            // winit's undecorated WM_NCCALCSIZE handler clamps maximized HWNDs
            // to rcWork, even in borderless fullscreen, leaving a taskbar-sized
            // non-client strip. Restore first so the client covers rcMonitor.
            if self.maximized_before_fullscreen {
                self.window.set_maximized(false);
            }
            self.window
                .set_fullscreen(Some(winit::window::Fullscreen::Borderless(monitor)));
        } else {
            self.window.set_fullscreen(None);
            if self.maximized_before_fullscreen {
                self.window.set_maximized(true);
            }
            self.maximized_before_fullscreen = false;
        }
        self.request_redraw();
    }
    fn close(&mut self, event_loop: &ActiveEventLoop) {
        self.window_memory
            .capture(self.hwnd, self.window.fullscreen().is_some());
        self.window_memory.flush(true);
        let _ = self.capture.stop();
        event_loop.exit();
    }
    fn create_target(&mut self) -> Result<()> {
        unsafe {
            let texture: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            self.graphics
                .device
                .CreateRenderTargetView(&texture, None, Some(&mut self.target))
        }
    }
    fn resize(&mut self, size: PhysicalSize<u32>) -> Result<()> {
        self.force_present = true;
        if size.width == 0 || size.height == 0 {
            self.target = None;
            return Ok(());
        }
        unsafe {
            self.graphics.context.OMSetRenderTargets(None, None);
            self.target = None;
            self.swap.ResizeBuffers(
                0,
                size.width,
                size.height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.create_target()
    }
    fn clear_capture(&mut self) {
        let _ = self.capture.stop();
        if let Some(audio) = &self.audio {
            audio.set_source(None);
        }
        if let Some((_, id)) = self.registered.take() {
            self.renderer.unregister_user_texture(id);
        }
        self.ui.clear_local_frame();
        self.session.command(Command::Unpublish);
        self.codecs.clear_pending();
        self.publisher.stop();
        self.published_size = p2pshare_media::Dimensions::default();
    }
    fn poll_capture(&mut self) -> Result<()> {
        if !self.ui.capture_running {
            return Ok(());
        }
        if let Err(error) = self.capture.set_visible(!self.design.local_preview_hidden) {
            tracing::warn!(%error, "local_preview_restart_failed");
            self.ui.status = "Local preview unavailable; screen sharing continues".into();
            return Ok(());
        }
        if self.design.local_preview_hidden {
            // Release the capture session and GPU texture, while keeping sharing,
            // its dimensions, audio and remote viewers intact.
            if let Some((_, id)) = self.registered.take() {
                self.renderer.unregister_user_texture(id);
                if let Some(member) = self.ui.participants.iter_mut().find(|p| p.local) {
                    member.texture = None;
                }
            }
            return Ok(());
        }
        match self.capture.state() {
            CaptureState::Paused => {
                self.ui.status = "Shared window minimized • preview paused".into();
                return Ok(());
            }
            CaptureState::Stopped | CaptureState::SourceClosed => return Ok(()),
            CaptureState::Capturing => {}
        }
        if let Some(frame) = self.capture.poll_frame()? {
            self.capture_meter.frame(0);
            if self
                .registered
                .as_ref()
                .is_none_or(|(texture, _)| *texture != frame.texture)
            {
                if let Some((_, id)) = self.registered.take() {
                    self.renderer.unregister_user_texture(id);
                }
                let mut srv = None;
                // SAFETY: capture texture is shader-readable and owned until the next capture poll.
                unsafe {
                    self.graphics.device.CreateShaderResourceView(
                        &frame.texture,
                        None,
                        Some(&mut srv),
                    )?;
                }
                let id = self
                    .renderer
                    .register_user_texture(srv.ok_or_else(windows::core::Error::empty)?);
                self.registered = Some((frame.texture.clone(), id));
            }
            if let Some((_, id)) = &self.registered {
                self.ui.set_local_frame(*id, frame.metadata.size);
            }
        }
        Ok(())
    }
    fn action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::SetLanguage(_)=>{},
            Action::ChooseAvatar|Action::SaveAvatar(_)|Action::RemoveAvatar=>{},
            Action::DebugLogging(_) | Action::CopyDebugLog => {}
            Action::Watch {
                participant,
                enabled,
            } => {
                for member in self.session.snapshot().members {
                    if p2pshare_room_ui::peer_ui_id(&member.id) == participant {
                        self.session.set_watching(&member.id, enabled);
                        if !enabled
                            && let Some(p) = self
                                .ui
                                .participants
                                .iter_mut()
                                .find(|p| p.id == participant)
                        {
                            p.texture = None;
                        }
                    }
                }
            }
            Action::ViewerMode { fullscreen, .. } => {
                self.set_viewer_fullscreen(fullscreen);
            }
            Action::VideoQuality(quality) => {
                let _ = quality; // Portable desktop settings are loaded from config.toml.
            }
                Action::AudioVolume { participant, percent } => {
                    for member in self.session.snapshot().members {
                        if p2pshare_room_ui::peer_ui_id(&member.id) == participant { self.session.set_audio_volume(&member.id, percent); }
                    }
                }
                Action::AudioMute { participant, muted } => {
                for member in self.session.snapshot().members {
                    if p2pshare_room_ui::peer_ui_id(&member.id) == participant {
                        self.session.set_audio_muted(&member.id, muted);
                    }
                }
            }
            Action::CopyInvite(_)
            | Action::PasteInvite
            | Action::EditName(_)
            | Action::EditRoom(_)
            | Action::EditConfig | Action::PictureInPicture | Action::SaveMobilePreferences { .. }
            | Action::ConfigureSignaling(_) | Action::ViewAndroidUpdate => {}
            Action::NamedRoom { room, name } => {
                self.session.command(Command::NamedRoom { room, name })
            }
            Action::JoinRoom { invite, name } => {
                if let Ok(parsed) = p2pshare_session::CloudflareInvite::parse(&invite)
                    && let Some(room) = parsed.room
                {
                    self.ui.room_name = room;
                }
                self.session.command(Command::Join { invite, name })
            }
            Action::LeaveRoom => self.session.command(Command::Leave),
            Action::Rename(name) => {
                let path = self.config_path.clone();
                if let Err(error) = p2pshare_session::AppConfig::save_username(&path, &name) {
                    tracing::warn!(%error,"username_config_save_failed");
                }
                self.session.command(Command::Rename(name));
            }
            Action::RefreshSources => {
                self.ui.sources = enumerate_sources(Some(self.hwnd))?;
            }
            Action::StartCapture(id) => {
                if self.ui.capture_running && self.active_source==Some(id){let health=self.publisher.health();if !health.source_closed&&health.error.is_none(){return Ok(());}}
                if let Some(source) = self.ui.sources.iter().find(|s| s.id == id).cloned() {
                    // Publisher validates the replacement before releasing the old capture.
                    // Keep the room publication and viewers intact throughout the handoff.
                    self.publisher.start(source.clone()).map_err(|error|windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),error.to_string()))?;
                    self.capture.stop()?;
                    if let Some((_,id))=self.registered.take(){self.renderer.unregister_user_texture(id);}
                    self.capture.set_cursor_capture(self.stream_config.show_cursor)?;
                    self.capture.set_visible(!self.design.local_preview_hidden)?;
                    if let Err(error)=self.capture.start(&source){tracing::warn!(%error,"source_switch_local_preview_failed");}
                    self.active_source=Some(id);
                    if let Some(audio) = &self.audio {
                        audio.set_source(Some(&source));
                    }
                    self.ui.capture_running = true;
                    self.ui.status = format!("Starting {}…", source.title);
                }
            }
            Action::StopCapture => {
                self.active_source = None;
                self.capture.stop()?;
                self.clear_capture();
                self.ui.status = "Screen sharing stopped".into();
            }
        }
        Ok(())
    }
    fn poll_network(&mut self) {
        let state = self.session.snapshot();
        if let Some(receiver) = &self.test_receiver {
            if !self.test_joined
                && let Some(invite) = &state.invite
            {
                receiver.command(Command::Join {
                    invite: invite.clone(),
                    name: "Capture receiver".into(),
                });
                self.test_joined = true;
            }
            for frame in receiver.take_video() {
                let size = receiver
                    .snapshot()
                    .members
                    .iter()
                    .find(|m| m.id == frame.peer)
                    .map(|m| Dimensions::new(m.width, m.height))
                    .unwrap_or_default();
                let _ =
                    self.codecs
                        .decode_codec("verification".into(), frame.data, frame.codec, size);
            }
        }
        if let Some(invite) = &state.invite
            && let Some(path) = self.invite_out.take()
        {
            let _ = std::fs::write(path, invite);
        }
        if !state.busy && !state.in_room && self.ui.capture_running {
            let _ = self.capture.stop();
            self.clear_capture();
        }
        self.ui.apply_snapshot(&room_state(&state));
        for (peer, bytes) in self.session.take_previews() {
            match p2pshare_media::preview::decode(&bytes) {
                Ok(preview) => {
                    self.preview_epoch = self.preview_epoch.wrapping_add(1);
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [preview.width as usize, preview.height as usize],
                        &preview.rgba,
                    );
                    let texture = self.room_previews.entry(peer.clone()).or_insert_with(|| {
                        self.ctx.load_texture(
                            format!("screen-{peer}"),
                            image.clone(),
                            egui::TextureOptions::LINEAR,
                        )
                    });
                    texture.set(image, egui::TextureOptions::LINEAR);
                }
                Err(error) => tracing::warn!(%error, "invalid_remote_jpeg_preview"),
            }
        }
        self.room_previews
            .retain(|id, _| state.members.iter().any(|m| &m.id == id));
        for (peer, texture) in &self.room_previews {
            if !self.session.is_watching(peer) {
                let size = texture.size();
                self.ui.set_remote_frame(
                    peer,
                    texture.id(),
                    Dimensions::new(size[0] as u32, size[1] as u32),
                );
            }
        }
        let health = self.publisher.health();
        self.ui.encoder = health.backend;
        self.ui.capture_fps = health.fps;
        if health.source_closed && self.ui.capture_running {
            self.clear_capture();
            self.ui.status = "The shared window closed. Choose another screen.".into();
        }
        if let Some(error) = health.error {
            if self.ui.capture_running {
                let _ = self.capture.stop();
                self.clear_capture();
            }
            self.ui.status = error;
        }
        let members: Vec<_> = state
            .members
            .iter()
            .filter(|m| m.id != state.local_id && m.sharing)
            .map(|m| m.id.clone())
            .collect();
        self.remote_textures.retain(|p, texture| {
            let keep = members.contains(p)
                && (self.session.is_watching(p) || self.smoke || self.capture_smoke);
            if !keep && let Some(texture) = texture {
                self.renderer.unregister_user_texture(texture.id);
            }
            keep
        });
        let members: Vec<_> = members
            .into_iter()
            .filter(|p| self.session.is_watching(p) || self.smoke || self.capture_smoke)
            .collect();
        if members != self.last_members {
            self.codecs.retain(members.clone());
            self.last_members = members;
        }
        for frame in self.session.take_video() {
            let peer = frame.peer;
            let size = state
                .members
                .iter()
                .find(|m| m.id == peer)
                .map(|m| Dimensions::new(m.width, m.height))
                .unwrap_or_default();
            if !self
                .codecs
                .decode_codec(peer.clone(), frame.data, frame.codec, size)
            {
                self.session.command(Command::RequestKeyframe(peer));
            }
        }
        for (peer, frame) in self.codecs.take_decoded() {
            if peer == "verification" {
                let count = frame
                    .rgba
                    .chunks_exact(4)
                    .filter(|p| {
                        p[0].abs_diff(38) < 12 && p[1].abs_diff(124) < 12 && p[2].abs_diff(159) < 12
                    })
                    .count();
                if count > frame.rgba.len() / 40
                    && (!std::env::args().any(|a| a == "--hd-capture-smoke")
                        || self.smoke_stage > 0
                        || frame.size.width
                            >= self
                                .stream_config
                                .width
                                .max(self.stream_config.height)
                                .saturating_sub(32)
                                .min(1900))
                {
                    self.test_decoded = true;
                    tracing::info!(width=frame.size.width,height=frame.size.height,matching_pixels=count,backend=%self.publisher.health().backend,"WGC -> H264 -> SRTP -> remote decode passed");
                }
                continue;
            }
            if !self.last_members.contains(&peer) {
                continue;
            }
            match video_texture::VideoTexture::upload(
                self.remote_textures.entry(peer.clone()).or_default(),
                &self.graphics,
                &mut self.renderer,
                &frame,
            ) {
                Ok(id) => self.ui.set_remote_frame(&peer, id, frame.size),
                Err(error) => {
                    tracing::warn!(%error, peer, "video_texture_upload_failed");
                    if frame.gpu.is_some() {
                        self.codecs.use_cpu_output(peer);
                    }
                }
            }
        }
        for peer in self.codecs.take_keyframe_requests() {
            self.session.command(Command::RequestKeyframe(peer));
        }
        if let Some(error) = self.codecs.take_error() {
            self.ui.status = error;
        }
    }
    fn render(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        self.presented_last_frame = false;
        let cpu_update = crate::cpu_profile::CpuProfiler::thread_stamp();
        let update_started = Instant::now();
        self.poll_network();
        if let Some(retry)=self.graphics_retry{
            if Instant::now()<retry{return Ok(());}
            self.resize(self.window.inner_size())?;self.graphics_retry=None;
            tracing::info!("display_transition_recovered");
        }
        if self.preview_hidden_last != self.design.local_preview_hidden {
            self.preview_hidden_last = self.design.local_preview_hidden;
            self.capture_meter =
                p2pshare_media::diagnostics::FrameMeter::new("local_preview_capture")
                    .with_fps(self.stream_config.fps);
            tracing::info!(
                hidden = self.preview_hidden_last,
                "local_preview_visibility_changed"
            );
        }
        if !self.design.local_preview_hidden {
            self.capture_meter.report();
        }
        if self.telemetry_at.elapsed() >= Duration::from_secs(1) {
            let health = self.publisher.health();
            let state = self.session.snapshot();
            tracing::info!(capture_running=self.ui.capture_running, viewers=state.viewer_count,
                local_preview_hidden=self.design.local_preview_hidden,
                local_preview_capture_active=self.capture.state()!=CaptureState::Stopped,
                video_mode=if !self.ui.capture_running {"not_sharing"} else if self.session.needs_live_video() {"streaming"} else {"previews_only_no_viewers"},
                backend=%health.backend,published_frames=health.published_frames,
                metrics=%self.session.video_diagnostics(),codec_queues=%health.queues, readback=%health.readback, "stream_health");
            let (reused, rebuilt) = self.renderer.geometry_cache_stats();
            tracing::info!(reused, rebuilt, "ui_geometry_cache");
            tracing::info!(
                presented = self.presented_frames,
                skipped_unchanged = self.skipped_presentations,
                "ui_presentations"
            );
            let watching = state
                .members
                .iter()
                .filter(|m| self.session.is_watching(&m.id))
                .count();
            self.cpu_profile.sample(
                self.ui.capture_running,
                watching,
                self.capture.state() != CaptureState::Stopped,
                self.design.local_preview_hidden,
            );
            self.telemetry_at = Instant::now();
        }
        if let Err(error) = self.poll_capture() {
            tracing::error!(%error, "Capture stopped");
            let _ = self.capture.stop();
            self.clear_capture();
            self.ui.status = format!("Capture error: {error}");
        }
        if self.target.is_none() || self.occluded {
            self.sync_design();
            return Ok(());
        }
        self.update_latency
            .record(update_started.elapsed().as_secs_f64() * 1000.);
        let cpu_draw = self
            .cpu_profile
            .record_stage(crate::cpu_profile::UiStage::Update, cpu_update);
        let draw_started = Instant::now();
        let mut actions = Vec::new();
        let legacy = self.smoke || self.capture_smoke;
        if !legacy {
            self.sync_design();
            self.updater.poll(&mut self.design.update);
        }
        let output = self
            .ctx
            .run_ui(self.input.take_egui_input(&self.window), |ui| {
                if legacy {
                    actions = self.ui.ui(ui);
                } else {
                    self.design.ui(ui);
                    self.firewall.show(
                        ui.ctx(),
                        self.hwnd.0 as usize,
                        self.design.dialog == p2pshare_desktop_design::app::Dialog::None
                            && !self.design.fullscreen && !self.design.update_open,
                    );
                }
            });

        let (output, platform, _) = egui_directx11::split_output(output);
        self.input.handle_platform_output(&self.window, platform);
        let target = self.target.as_ref().unwrap();
        // Poll room state/input as before, but don't submit unchanged static windows
        // to the driver/compositor. Live video preserves the existing vsync path.
        let paint = legacy
            || self.force_present
            || self.design.has_visible_live_video()
            || self.renderer.needs_render(target, &self.ctx, &output)?;
        if paint {
            unsafe {
                self.graphics
                    .context
                    .ClearRenderTargetView(target, &[0.09, 0.09, 0.12, 1.0]);
            }
            self.renderer
                .render(&self.graphics.context, target, &self.ctx, output)?;
        } else {
            self.skipped_presentations += 1;
        }
        let _ = self
            .cpu_profile
            .record_stage(crate::cpu_profile::UiStage::Draw, cpu_draw);
        self.draw_latency
            .record(draw_started.elapsed().as_secs_f64() * 1000.);
        // Apply actions after drawing: this frame may still reference the old capture texture.
        for action in actions {
            if let Err(error) = self.action(action) {
                tracing::error!(%error, "Capture action failed");
                self.ui.status = format!("Capture error: {error}");
            }
        }
        if !legacy {
            self.apply_design();
            if self.close_requested {
                self.close(event_loop);
                return Ok(());
            }
        }
        if self.viewer_smoke && self.capture_smoke && self.smoke_stage == 0 && !self.test_decoded {
            if self.started.elapsed() > Duration::from_secs(15) {
                return Err(windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    "Viewer diagnostic never received a real decoded frame",
                ));
            }
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                    self.test_source.as_ref().map(|s| s.hwnd),
                    None,
                    false,
                );
            }
        }
        if self.pacing_smoke
            && !self.independence_checked
            && self.test_decoded
            && self.started.elapsed() >= Duration::from_secs(3)
        {
            let before = self.publisher.health().published_frames;
            // Only an opt-in owned-source diagnostic: simulate a blocked GUI/Present.
            std::thread::sleep(Duration::from_millis(350));
            let frames = self
                .publisher
                .health()
                .published_frames
                .saturating_sub(before);
            if frames < 4 {
                return Err(windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    format!("Publisher stopped with blocked preview: {frames} frames"),
                ));
            }
            self.independence_checked = true;
            tracing::info!(
                frames,
                blocked_ui_ms = 350,
                "publisher_continued_while_preview_blocked"
            );
        }
        if (self.smoke || self.capture_smoke)
            && (!self.viewer_smoke || self.smoke_stage > 0 || self.test_decoded)
            && self.stage_at.elapsed()
                > Duration::from_secs(if self.pacing_smoke && self.smoke_stage == 0 {
                    12
                } else {
                    1
                })
        {
            let folder = self.config_path.parent().unwrap().join("data/artifacts");
            let _ = std::fs::create_dir_all(&folder);
            let name = match self.smoke_stage {
                0 => "egui-grid.bmp",
                1 => "egui-focus.bmp",
                _ => "egui-mobile.bmp",
            };
            let capture_name = format!("egui-capture-{}.bmp", self.smoke_stage);
            self.save_bmp(&folder.join(if self.capture_smoke {
                &capture_name
            } else {
                name
            }))?;
            match self.smoke_stage {
                0 => {
                    self.ui.focused = Some(0);
                    if self.viewer_smoke {
                        self.ui.enter_fullscreen(0);
                        self.action(Action::ViewerMode {
                            fullscreen: true,
                            handles_back: true,
                        })?;
                    }
                }
                1 => {
                    if self.viewer_smoke
                        && (!self.ui.is_fullscreen() || self.window.fullscreen().is_none())
                    {
                        return Err(windows::core::Error::new(
                            windows::core::HRESULT(0x80004005u32 as i32),
                            "Viewer fullscreen did not render",
                        ));
                    }
                    if let Some(source) = &self.test_source {
                        source.resize();
                    }
                    if self.viewer_smoke {
                        self.ui.back();
                        self.action(Action::ViewerMode {
                            fullscreen: false,
                            handles_back: true,
                        })?;
                        self.ui.set_platform(p2pshare_room_ui::Platform::Android);
                        self.ui.set_safe_insets([28.0, 0.0, 24.0, 16.0]);
                    }
                    self.ui.focused = None;
                    let _ = self.window.request_inner_size(if self.viewer_smoke {
                        LogicalSize::new(900.0, 500.0)
                    } else {
                        LogicalSize::new(430.0, 900.0)
                    });
                }
                2 if self.capture_smoke => {
                    if self.viewer_smoke
                        && (self.ui.is_fullscreen()
                            || self.window.fullscreen().is_some()
                            || !self.ui.snapshot.mobile)
                    {
                        return Err(windows::core::Error::new(
                            windows::core::HRESULT(0x80004005u32 as i32),
                            "Viewer Back/landscape transition failed",
                        ));
                    }
                    if self.registered.is_none() {
                        return Err(windows::core::Error::new(
                            windows::core::HRESULT(0x80004005u32 as i32),
                            "Capture UI received no frames",
                        ));
                    }
                    self.action(Action::StopCapture)?;
                    if self.registered.is_some() || self.ui.capture_running {
                        return Err(windows::core::Error::new(
                            windows::core::HRESULT(0x80004005u32 as i32),
                            "Capture UI did not release stopped texture",
                        ));
                    }
                }
                3 if self.capture_smoke => {
                    self.action(Action::RefreshSources)?;
                    let source = self
                        .ui
                        .sources
                        .iter()
                        .find(|s| s.title == "P2PShare egui test source")
                        .ok_or_else(windows::core::Error::empty)?
                        .id;
                    self.action(Action::StartCapture(source))?;
                }
                _ => {
                    if self.capture_smoke && (!self.test_decoded || self.registered.is_none()) {
                        return Err(windows::core::Error::new(
                            windows::core::HRESULT(0x80004005u32 as i32),
                            "Capture UI restart or encrypted H.264 remote decode failed",
                        ));
                    }
                    self.capture.stop()?;
                    event_loop.exit();
                }
            }
            self.smoke_stage += 1;
            self.stage_at = Instant::now();
        }
        if paint {
            let cpu_present = crate::cpu_profile::CpuProfiler::thread_stamp();
            let present_started = Instant::now();
            // SAFETY: the swap chain and its back buffer remain alive on the UI thread.
            unsafe {
                self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
            }
            let _ = self
                .cpu_profile
                .record_stage(crate::cpu_profile::UiStage::Present, cpu_present);
            self.present_latency
                .record(present_started.elapsed().as_secs_f64() * 1000.);
            self.presented_frames += 1;
            self.presented_last_frame = true;
            self.force_present = false;
            self.preview_meter.frame(0);
        }
        self.preview_meter.report();
        Ok(())
    }
    // Diagnostic snapshots only. Normal preview never maps pixels back to the CPU.
    fn save_bmp(&self, path: &Path) -> Result<()> {
        unsafe {
            let texture: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            self.graphics
                .device
                .CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = staging.unwrap();
            self.graphics.context.CopyResource(&staging, &texture);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            self.graphics
                .context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
            let pixel_bytes = desc.Width * desc.Height * 4;
            let mut data = Vec::with_capacity(54 + pixel_bytes as usize);
            data.extend_from_slice(b"BM");
            data.extend_from_slice(&(54 + pixel_bytes).to_le_bytes());
            data.extend_from_slice(&[0; 4]);
            data.extend_from_slice(&54u32.to_le_bytes());
            data.extend_from_slice(&40u32.to_le_bytes());
            data.extend_from_slice(&(desc.Width as i32).to_le_bytes());
            data.extend_from_slice(&(-(desc.Height as i32)).to_le_bytes());
            data.extend_from_slice(&1u16.to_le_bytes());
            data.extend_from_slice(&32u16.to_le_bytes());
            data.extend_from_slice(&[0; 24]);
            for y in 0..desc.Height as usize {
                data.extend_from_slice(std::slice::from_raw_parts(
                    (map.pData as *const u8).add(y * map.RowPitch as usize),
                    desc.Width as usize * 4,
                ));
            }
            self.graphics.context.Unmap(&staging, 0);
            std::fs::write(path, data).map_err(|e| {
                windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    e.to_string(),
                )
            })?;
        }
        Ok(())
    }
}
impl Drop for View {
    fn drop(&mut self) {
        let _ = self.capture.stop();
    }
}

// Translate engine data once at the presentation boundary; UI code has no network dependency.
fn room_state(state: &p2pshare_session::Snapshot) -> p2pshare_room_ui::RoomState {
    p2pshare_room_ui::RoomState {
        local_id: state.local_id.clone(),
        in_room: state.in_room,
        busy: state.busy,
        invite: state.invite.clone(),
        saved_invite: state.saved_invite.clone(),
        status: if !state.in_room || state.discovery_status.is_empty() {
            state.status.clone()
        } else {
            format!("{} • {}", state.status, state.discovery_status)
        },
        members: state
            .members
            .iter()
            .map(|m| p2pshare_room_ui::MemberState {
                id: m.id.clone(),
                name: m.name.clone(),
                sharing: m.sharing,
                width: m.width,
                height: m.height,
            })
            .collect(),
    }
}

#[cfg(test)]
mod repaint_tests {
    use super::*;
    use winit::platform::windows::EventLoopBuilderExtWindows;

    #[test]
    fn redraw_is_consumed_while_real_input_still_requests_paint() {
        assert!(!input_requests_redraw(&WindowEvent::RedrawRequested, true));
        assert!(input_requests_redraw(&WindowEvent::Focused(true), true));
        assert!(input_requests_redraw(
            &WindowEvent::Resized(PhysicalSize::new(640, 360)),
            true
        ));
        assert!(!input_requests_redraw(&WindowEvent::Focused(false), false));
    }
    #[test]
    fn egui_adapter_redraw_does_not_self_schedule_in_hidden_window() {
        struct TestLoop {
            window: Option<Window>,
            input: Option<egui_winit::State>,
            started: Instant,
            next: Instant,
            redraws: u32,
            adapter_repaints: u32,
            followups: u32,
        }
        impl ApplicationHandler for TestLoop {
            fn resumed(&mut self, el: &ActiveEventLoop) {
                detached::verify_hidden_views(el);
                let window = el
                    .create_window(
                        Window::default_attributes()
                            .with_visible(false)
                            .with_title("P2PShare hidden redraw regression test")
                            .with_inner_size(PhysicalSize::new(64, 64)),
                    )
                    .unwrap();
                let input = egui_winit::State::new(
                    egui::Context::default(),
                    egui::ViewportId::ROOT,
                    &window,
                    Some(window.scale_factor() as f32),
                    None,
                    None,
                );
                self.window = Some(window);
                self.input = Some(input);
                self.started = Instant::now();
                self.next = self.started;
            }
            fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
                let window = self.window.as_ref().unwrap();
                let response = self.input.as_mut().unwrap().on_window_event(window, &event);
                if matches!(event, WindowEvent::RedrawRequested) {
                    self.redraws += 1;
                    self.adapter_repaints += u32::from(response.repaint);
                    if input_requests_redraw(&event, response.repaint) {
                        self.followups += 1;
                    }
                }
                if input_requests_redraw(&event, response.repaint) {
                    window.request_redraw();
                }
                // Bound the old faulty behavior: fail quickly instead of spending
                // the entire test timeout spinning at thousands of redraws/sec.
                if self.redraws > 100 {
                    el.exit();
                }
            }
            fn about_to_wait(&mut self, el: &ActiveEventLoop) {
                let now = Instant::now();
                let end = self.started + Duration::from_millis(350);
                if now >= end {
                    el.exit();
                    return;
                }
                if now >= self.next {
                    // Hidden Win32 windows don't get automatic paint messages.
                    // Post WM_PAINT to this owned hidden HWND so the real winit
                    // adapter/event handler is exercised without showing a window.
                    let window = self.window.as_ref().unwrap();
                    let RawWindowHandle::Win32(handle) = window.window_handle().unwrap().as_raw()
                    else {
                        panic!("Expected Win32 test window")
                    };
                    unsafe {
                        windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                            Some(HWND(handle.hwnd.get() as *mut _)),
                            windows::Win32::UI::WindowsAndMessaging::WM_PAINT,
                            windows::Win32::Foundation::WPARAM(0),
                            windows::Win32::Foundation::LPARAM(0),
                        )
                        .unwrap();
                    }
                    self.next = now + Duration::from_millis(50);
                }
                el.set_control_flow(ControlFlow::WaitUntil(self.next.min(end)));
            }
        }
        let mut builder = EventLoop::builder();
        builder.with_any_thread(true);
        let event_loop = builder.build().unwrap();
        let mut app = TestLoop {
            window: None,
            input: None,
            started: Instant::now(),
            next: Instant::now(),
            redraws: 0,
            adapter_repaints: 0,
            followups: 0,
        };
        event_loop.run_app(&mut app).unwrap();
        assert!(
            app.redraws > 0,
            "Hidden window did not exercise actual redraw events"
        );
        assert_eq!(
            app.adapter_repaints, app.redraws,
            "Test did not reproduce egui-winit's repaint response"
        );
        assert_eq!(app.followups, 0, "Redraw events self-scheduled");
        assert!(
            app.redraws < 30,
            "Static UI busy-looped: {} redraws",
            app.redraws
        );
        println!(
            "Hidden native window: {} redraws in {:?}; no self-scheduled redraws",
            app.redraws,
            app.started.elapsed()
        );
    }
}
