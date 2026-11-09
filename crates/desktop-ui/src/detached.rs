//! Detached stream windows share the parent's session, decoded textures and D3D device.
//! Only input, window state and presentation are local to each window.
use super::*;
use p2pshare_desktop_design::app::{App, Intent};
use winit::window::{Fullscreen, WindowLevel};

pub(super) struct Detached {
    avatar_textures:HashMap<u64,(std::sync::Arc<[u8]>,egui::TextureHandle)>,

    pub window: Window,
    fit: super::detached_fit::Guard,
    pub target: Option<ID3D11RenderTargetView>,
    pub app: App,
    pub occluded: bool,
    swap: IDXGISwapChain1,
    renderer: egui_directx11::Renderer,
    ctx: egui::Context,
    source_texture: Option<egui::TextureId>,
    texture_alias: Option<egui::TextureId>,
    input: egui_winit::State,
    peer: u64,
    last_full: bool,
    last_top: bool,
    was_maximized: bool,
    force_present: bool,
    preview_epoch: u64,
    presented_frames: u64,
    input_events: u64,
    frame_meter: p2pshare_media::diagnostics::FrameMeter,
    metrics_at: Instant,
}
impl Detached {
    fn model(parent: &App, peer: u64) -> Option<App> {
        let index = parent
            .members
            .iter()
            .position(|m| m.id == peer && m.scene.is_some())?;
        let mut app = App::live(parent.config.clone(), parent.room.clone(), false);
        app.members = parent.members.clone();
        app.joined = parent.joined;
        app.watching = parent.watching.clone();
        app.stream_viewers=parent.stream_viewers.clone();app.profile_names=parent.profile_names.clone();app.profile_images=parent.profile_images.clone();app.avatars.clear();
        app.local_preview_hidden = parent.local_preview_hidden;
        app.popped_out = Some(index);
        app.focus = Some(index);
        Some(app)
    }
    fn new(
        window: Window,
        peer: u64,
        design: &App,
        graphics: &GraphicsDevice,
    ) -> anyhow::Result<Self> {
        let app =
            Self::model(design, peer).ok_or_else(|| anyhow::anyhow!("Screen no longer shared"))?;
        let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
            anyhow::bail!("Expected Win32 window");
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        let dimensions = app.members[app.popped_out.unwrap()].dimensions;
        let fit = super::detached_fit::Guard::install(&window, dimensions)?;
        let size = window.inner_size();
        let swap = unsafe {
            let dxgi: IDXGIDevice = graphics.device.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let swap = factory.CreateSwapChainForHwnd(
                &graphics.device,
                hwnd,
                &DXGI_SWAP_CHAIN_DESC1 {
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
                    ..Default::default()
                },
                None,
                None,
            )?;
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
            swap
        };
        let ctx = egui::Context::default();
        RoomUi::configure(&ctx);
        let renderer = egui_directx11::Renderer::new(&graphics.device)?;
        let source_texture = app.members[app.popped_out.unwrap()].texture;
        let input = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(16384),
        );
        let mut view = Self {
            window,
            fit,
            target: None,
            app,
            avatar_textures:Default::default(),
            occluded: false,
            swap,
            renderer,
            ctx,
            source_texture,
            texture_alias: None,
            input,
            peer,
            last_full: false,
            last_top: false,
            was_maximized: false,
            force_present: true,
            preview_epoch: 0,
            presented_frames: 0,
            input_events: 0,
            frame_meter: p2pshare_media::diagnostics::FrameMeter::new("detached_video"),
            metrics_at: Instant::now(),
        };
        view.make_target(graphics)?;
        Ok(view)
    }
    fn make_target(&mut self, graphics: &GraphicsDevice) -> Result<()> {
        unsafe {
            let texture: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            graphics
                .device
                .CreateRenderTargetView(&texture, None, Some(&mut self.target))
        }
    }
    fn resize(&mut self, graphics: &GraphicsDevice, size: PhysicalSize<u32>) -> Result<()> {
        self.force_present = true;
        unsafe {
            graphics.context.OMSetRenderTargets(None, None);
        }
        self.target = None;
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        unsafe {
            self.swap.ResizeBuffers(
                0,
                size.width,
                size.height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.make_target(graphics)
    }
    // Input never presents a frame. Queue it for the shared playback loop,
    // which polls/decode-updates once and supplies exactly one vsync wait.
    fn on_event(&mut self, graphics: &GraphicsDevice, event: &WindowEvent) -> anyhow::Result<bool> {
        let response = self.input.on_window_event(&self.window, event);
        if matches!(
            event,
            WindowEvent::CursorMoved { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::KeyboardInput { .. }
        ) {
            self.input_events += 1;
        }
        let expose = matches!(
            event,
            WindowEvent::RedrawRequested
                | WindowEvent::Focused(true)
                | WindowEvent::Occluded(false)
                | WindowEvent::ScaleFactorChanged { .. }
        );
        self.force_present |= expose;
        match event {
            WindowEvent::Resized(size) => self.resize(graphics, *size)?,
            WindowEvent::Occluded(value) => self.occluded = *value,
            _ => {}
        }
        Ok(input_requests_redraw(event, response.repaint)
            || expose
            || matches!(event, WindowEvent::Resized(_) | WindowEvent::Occluded(_)))
    }
    fn sync(&mut self, parent: &App) {
        self.app.members = parent.members.clone();
        self.app.config = parent.config.clone();
        self.app.joined = parent.joined;
        self.app.watching = parent.watching.clone();
        self.avatar_textures.retain(|id,_|parent.avatar_data.contains_key(id));
        for (id,bytes) in &parent.avatar_data {
            if self.avatar_textures.get(id).is_none_or(|(old,_)|!std::sync::Arc::ptr_eq(old,bytes)) {
                if let Ok(image)=p2pshare_media::avatar::validate(bytes){let texture=self.ctx.load_texture(format!("detached-avatar-{id}"),egui::ColorImage::from_rgba_unmultiplied([256,256],image.as_raw()),egui::TextureOptions::LINEAR);self.avatar_textures.insert(*id,(bytes.clone(),texture));}
            }
        }

        self.app.stream_viewers=parent.stream_viewers.clone();self.app.profile_names=parent.profile_names.clone();self.app.profile_images=parent.profile_images.clone();self.app.avatars.clear();for (id,(_,texture)) in &self.avatar_textures{self.app.avatars.insert(*id,texture.id());}
        self.app.local_preview_hidden = parent.local_preview_hidden;
        let index = self.app.members.iter().position(|m| m.id == self.peer);
        // An absent peer must never cause this window to switch to someone else's screen.
        self.app.popped_out = Some(index.unwrap_or(self.app.members.len()));
        self.app.focus = index;
        if let Some(i) = index {
            self.fit.refresh(
                &self.window,
                self.app.members[i].dimensions,
                self.window.scale_factor() as f32 * self.ctx.zoom_factor(),
                self.app.fullscreen,
            );
        }
        self.source_texture = index.and_then(|i| self.app.members[i].texture);
        if index.is_none_or(|i| self.app.members[i].scene.is_none()) {
            self.app.fullscreen = false;
        }
        self.app.maximized = self.window.is_maximized();
        self.app.window_focused = self.window.has_focus();
    }
    fn paint(
        &mut self,
        graphics: &GraphicsDevice,
        source_renderer: &egui_directx11::Renderer,
        wait_vsync: bool,
    ) -> anyhow::Result<()> {
        if self.occluded || self.target.is_none() {
            return Ok(());
        }
        let imported = self.source_texture.and_then(|id| {
            self.renderer
                .import_texture(source_renderer, id, self.texture_alias)
        });
        if imported.is_none()
            && let Some(id) = self.texture_alias.take()
        {
            self.renderer.unregister_user_texture(id);
        }
        self.texture_alias = imported;
        if let Some(member) = self
            .app
            .popped_out
            .and_then(|i| self.app.members.get_mut(i))
        {
            member.texture = imported;
        }
        let ctx = &self.ctx;
        let output = ctx.run_ui(self.input.take_egui_input(&self.window), |ui| {
            ui.push_id(("detached-stream", self.peer), |ui| self.app.ui(ui));
        });
        let (output, platform, _) = egui_directx11::split_output(output);
        self.input.handle_platform_output(&self.window, platform);
        let target = self.target.as_ref().unwrap();
        let renderer = &mut self.renderer;
        let result = (|| -> anyhow::Result<()> {
            if self.force_present
                || self.app.has_visible_live_video()
                || renderer.needs_render(target, ctx, &output)?
            {
                unsafe {
                    graphics
                        .context
                        .ClearRenderTargetView(target, &[0., 0., 0., 1.]);
                }
                renderer.render(&graphics.context, target, ctx, output)?;
                // Only one visible window supplies the vsync wait for all presentations.
                unsafe {
                    self.swap
                        .Present(u32::from(wait_vsync), DXGI_PRESENT(0))
                        .ok()?;
                }
                self.presented_frames += 1;
                self.frame_meter.frame(0);
            }
            Ok(())
        })();
        if result.is_ok() {
            self.force_present = false;
        }
        if self.metrics_at.elapsed() >= Duration::from_secs(1) {
            self.frame_meter.report();
            let (reused, rebuilt) = self.renderer.geometry_cache_stats();
            tracing::info!(
                peer = self.peer,
                presented = self.presented_frames,
                input_events = self.input_events,
                reused,
                rebuilt,
                "detached_presentations"
            );
            self.metrics_at = Instant::now();
        }
        result
    }
    fn draw(&mut self, parent: &mut View, wait_vsync: bool) -> anyhow::Result<bool> {
        self.sync(&parent.design);
        // The parent consumes shared texture deltas. A paused/JPEG-only child
        // still needs to present when the same texture receives a new image.
        self.force_present |= self.preview_epoch != parent.preview_epoch;
        self.preview_epoch = parent.preview_epoch;
        self.paint(&parent.graphics, &parent.renderer, wait_vsync)?;
        let mut close = false;
        for intent in std::mem::take(&mut self.app.intents) {
            if let Intent::Window(action) = intent {
                use p2pshare_desktop_design::chrome::{ResizeEdge, WindowAction};
                use winit::window::ResizeDirection;
                match action {
                    WindowAction::Close => close = true,
                    WindowAction::Minimize => self.window.set_minimized(true),
                    WindowAction::ToggleMaximize => {
                        self.window.set_maximized(!self.window.is_maximized())
                    }
                    WindowAction::Drag => self.window.drag_window()?,
                    WindowAction::Resize(edge) => self.window.drag_resize_window(match edge {
                        ResizeEdge::North => ResizeDirection::North,
                        ResizeEdge::NorthEast => ResizeDirection::NorthEast,
                        ResizeEdge::East => ResizeDirection::East,
                        ResizeEdge::SouthEast => ResizeDirection::SouthEast,
                        ResizeEdge::South => ResizeDirection::South,
                        ResizeEdge::SouthWest => ResizeDirection::SouthWest,
                        ResizeEdge::West => ResizeDirection::West,
                        ResizeEdge::NorthWest => ResizeDirection::NorthWest,
                    })?,
                }
            } else {
                parent.design.intents.push(intent);
            }
        }
        // Copy only user preferences back. Textures and session membership remain parent-owned.
        for member in &self.app.members {
            if let Some(main) = parent.design.members.iter_mut().find(|m| m.id == member.id) {
                main.muted = member.muted;
                main.volume = member.volume;
                main.volume_before_silence = member.volume_before_silence;
            }
        }
        parent.design.local_preview_hidden = self.app.local_preview_hidden;
        parent.apply_design();
        let fullscreen_changed = self.last_full != self.app.fullscreen;
        if fullscreen_changed {
            if self.app.fullscreen {
                self.was_maximized = self.window.is_maximized();
                if self.was_maximized {
                    self.window.set_maximized(false);
                }
                self.window
                    .set_fullscreen(Some(Fullscreen::Borderless(self.window.current_monitor())));
            } else {
                self.window.set_fullscreen(None);
                if self.was_maximized {
                    self.window.set_maximized(true);
                }
                self.was_maximized = false;
            }
            self.last_full = self.app.fullscreen;
        }
        if self.last_top != self.app.always_on_top || fullscreen_changed {
            self.window.set_window_level(if self.app.always_on_top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            });
            self.last_top = self.app.always_on_top;
        }
        Ok(close)
    }
}
impl Runner {
    pub(super) fn update_popouts(&mut self, ev: &ActiveEventLoop) -> anyhow::Result<()> {
        let Some(parent) = &mut self.view else {
            return Ok(());
        };
        if !parent.design.joined {
            self.popouts.clear();
            parent.design.detached_peers.clear();
            return Ok(());
        }
        for peer in std::mem::take(&mut parent.popout_requests) {
            if let Some(child) = self.popouts.get(&peer) {
                child.window.set_minimized(false);
                child.window.focus_window();
                continue;
            }
            let Some(member) = parent
                .design
                .members
                .iter()
                .find(|m| m.id == peer && m.scene.is_some())
            else {
                continue;
            };
            let window = ev.create_window(
                Window::default_attributes()
                    .with_title(format!("{} · P2PShare", member.name))
                    .with_window_icon(Some(super::app_icon()))
                    .with_visible(false)
                    .with_decorations(false)
                    .with_inner_size(LogicalSize::new(960., 600.))
                    .with_min_inner_size(LogicalSize::new(500., 300.)),
            )?;
            let mut child = Detached::new(window, peer, &parent.design, &parent.graphics)?;
            // Prime layout and font resources before exposing the window.
            for _ in 0..2 {
                child.draw(parent, false)?;
            }
            parent.design.detached_peers.insert(peer);
            if parent
                .design
                .focus
                .is_some_and(|i| parent.design.members.get(i).is_some_and(|m| m.id == peer))
            {
                parent.design.focus = None;
                parent.design.fullscreen = false;
            }
            parent.force_present = true;
            parent.request_redraw();
            child.window.set_visible(true);
            self.popouts.insert(peer, child);
            tracing::info!(peer, "stream_popout_opened");
        }
        let mut closed = Vec::new();
        let mut wait_vsync = !parent.presented_last_frame;
        for (peer, child) in &mut self.popouts {
            let child_wait = wait_vsync
                && !child.occluded
                && child.target.is_some()
                && child.app.has_visible_live_video();
            if child_wait {
                wait_vsync = false;
            }
            match child.draw(parent, child_wait) {
                Ok(true) => closed.push(*peer),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(%error, peer, "popout_draw_failed");
                    closed.push(*peer);
                }
            }
        }
        for peer in closed {
            self.popouts.remove(&peer);
            parent.design.detached_peers.remove(&peer);
            parent.force_present = true;
            parent.request_redraw();
        }
        Ok(())
    }
    pub(super) fn child_event(
        &mut self,
        _ev: &ActiveEventLoop,
        peer: u64,
        event: WindowEvent,
    ) -> anyhow::Result<()> {
        if matches!(event, WindowEvent::CloseRequested) {
            self.popouts.remove(&peer);
            if let Some(parent) = &mut self.view {
                parent.design.detached_peers.remove(&peer);
                parent.force_present = true;
                parent.request_redraw();
            }
            return Ok(());
        }
        let Some(parent) = &mut self.view else {
            return Ok(());
        };
        let Some(child) = self.popouts.get_mut(&peer) else {
            return Ok(());
        };
        if child.on_event(&parent.graphics, &event)? {
            parent.request_redraw();
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn verify_hidden_views(ev: &ActiveEventLoop) {
    let graphics = GraphicsDevice::new().unwrap();
    let ctx = egui::Context::default();
    let mut renderer = egui_directx11::Renderer::new(&graphics.device).unwrap();
    let texture = ctx.load_texture(
        "shared-popout-test",
        egui::ColorImage::filled([96, 54], egui::Color32::from_rgb(24, 140, 220)),
        egui::TextureOptions::LINEAR,
    );
    let mut parent = App::new(Default::default(), None);
    parent.live = true;
    for member in &mut parent.members {
        member.scene = Some(0);
        member.texture = Some(texture.id());
        member.dimensions = [96, 54];
        parent.watching.insert(member.id);
    }
    let root_output = ctx.run_ui(egui::RawInput::default(), |_| {});
    let (root_output, _, _) = egui_directx11::split_output(root_output);
    // A hidden test target suffices to flush source texture deltas before imports.
    let mut tex = None;
    let mut target = None;
    unsafe {
        graphics
            .device
            .CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: 64,
                    Height: 64,
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
                },
                None,
                Some(&mut tex),
            )
            .unwrap();
        graphics
            .device
            .CreateRenderTargetView(tex.as_ref().unwrap(), None, Some(&mut target))
            .unwrap();
    }
    renderer
        .render(
            &graphics.context,
            target.as_ref().unwrap(),
            &ctx,
            root_output,
        )
        .unwrap();
    let mut children = Vec::new();
    for peer in 1..=4 {
        let window = ev
            .create_window(
                Window::default_attributes()
                    .with_visible(false)
                    .with_decorations(false)
                    .with_inner_size(LogicalSize::new(960., 600.)),
            )
            .unwrap();
        children.push(Detached::new(window, peer, &parent, &graphics).unwrap());
    }
    // Verify initial native fit and the actual installed WM_SIZING callback,
    // without showing or moving any real desktop window.
    for child in &children {
        let inner = child.window.inner_size();
        let footer = (44. * child.window.scale_factor()).round() as u32;
        assert!((inner.width as f64 - (inner.height - footer) as f64 * 16. / 9.).abs() < 2.);
        let RawWindowHandle::Win32(h) = child.window.window_handle().unwrap().as_raw() else {
            panic!("Win32 required");
        };
        let mut bounds = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: 960,
            bottom: 700,
        };
        unsafe {
            use windows::Win32::{
                Foundation::{LPARAM, WPARAM},
                UI::WindowsAndMessaging::{SendMessageW, WM_SIZING},
            };
            SendMessageW(
                HWND(h.hwnd.get() as *mut _),
                WM_SIZING,
                Some(WPARAM(8)),
                Some(LPARAM(
                    (&mut bounds as *mut windows::Win32::Foundation::RECT) as isize,
                )),
            );
        }
        let outer = child.window.outer_size();
        let extra_w = outer.width.saturating_sub(inner.width) as i32;
        let extra_h = outer.height.saturating_sub(inner.height) as i32;
        let w = bounds.right - bounds.left - extra_w;
        let h = bounds.bottom - bounds.top - extra_h - footer as i32;
        assert!((w as f64 - h as f64 * 16. / 9.).abs() < 2.);
    }
    let presented = children[0].presented_frames;
    for n in 0..1000 {
        children[0]
            .on_event(
                &graphics,
                &WindowEvent::CursorMoved {
                    device_id: winit::event::DeviceId::dummy(),
                    position: winit::dpi::PhysicalPosition::new(100. + (n % 600) as f64, 200.),
                },
            )
            .unwrap();
    }
    assert_eq!(children[0].input_events, 1000);
    assert_eq!(
        children[0].presented_frames, presented,
        "Mouse input must not present outside the playback loop"
    );
    children[0].paint(&graphics, &renderer, false).unwrap();
    assert_eq!(
        children[0].presented_frames,
        presented + 1,
        "The queued input burst must share one presentation"
    );
    // Repeated interleaved views must share fonts/images without stealing geometry caches.
    for _ in 0..3 {
        for child in &mut children {
            child.paint(&graphics, &renderer, false).unwrap();
            assert_eq!(child.window.is_visible(), Some(false));
            assert_eq!(
                child.app.members[child.app.popped_out.unwrap()].id,
                child.peer
            );
            assert!(child.app.regions.iter().any(|(name, _)| name == "popout"));
        }
    }
    assert!(
        children
            .iter()
            .all(|c| c.renderer.geometry_cache_stats().0 >= 1),
        "Each child must reuse its own geometry"
    );
    // Read actual GPU output to catch missing shared textures or viewport rendering errors.
    for child in &children {
        unsafe {
            let rendered: ID3D11Texture2D = child
                .target
                .as_ref()
                .unwrap()
                .GetResource()
                .unwrap()
                .cast()
                .unwrap();
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            rendered.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            graphics
                .device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            graphics.context.CopyResource(&staging, &rendered);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            graphics
                .context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let at = mapped
                .pData
                .cast::<u8>()
                .add((desc.Height / 2 * mapped.RowPitch + desc.Width / 2 * 4) as usize);
            let pixel = std::slice::from_raw_parts(at, 4).to_vec();
            graphics.context.Unmap(&staging, 0);
            assert!(
                pixel[0].abs_diff(220) < 3
                    && pixel[1].abs_diff(140) < 3
                    && pixel[2].abs_diff(24) < 3,
                "Shared frame missing in peer {}: {pixel:?}",
                child.peer
            );
        }
    }
    let key = "speaker-footer-1";
    // Exercise the real footer through the child viewport's own input state.
    let speaker = children[0]
        .app
        .hits
        .get(key)
        .or_else(|| children[0].app.hits.get("mute-1"))
        .copied()
        .expect("Speaker button in detached view");
    let point = speaker.center();
    for pressed in [true, false] {
        children[0].input.egui_input_mut().events.extend([
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]);
        children[0].paint(&graphics, &renderer, false).unwrap();
        // Real playback repaints the parent and other windows between press/release.
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerGone],
                ..Default::default()
            },
            |ui| parent.ui(ui),
        );
        for child in children.iter_mut().skip(1) {
            child.paint(&graphics, &renderer, false).unwrap();
        }
    }
    assert!(
        children[0]
            .app
            .members
            .iter()
            .find(|m| m.id == 1)
            .unwrap()
            .muted
    );
    assert!(
        !children[1]
            .app
            .members
            .iter()
            .find(|m| m.id == 1)
            .unwrap()
            .muted
    );
    parent.members.retain(|m| m.id != 1);
    children[0].sync(&parent);
    assert!(
        children[0]
            .app
            .members
            .get(children[0].app.popped_out.unwrap())
            .is_none()
    );
    children[0].paint(&graphics, &renderer, false).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detached_peer_identity_survives_members_reordering_and_departure() {
        let mut parent = App::new(Default::default(), None);
        let child = Detached::model(&parent, 2).unwrap();
        assert_eq!(child.members[child.popped_out.unwrap()].id, 2);
        parent.members.swap(1, 2);
        let child = Detached::model(&parent, 2).unwrap();
        assert_eq!(child.members[child.popped_out.unwrap()].id, 2);
        parent.members.retain(|m| m.id != 2);
        assert!(Detached::model(&parent, 2).is_none());
    }
}
