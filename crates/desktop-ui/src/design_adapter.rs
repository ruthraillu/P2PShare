//! Adapter between presentation intents and the existing media/session runtime.
use super::*;
use p2pshare_desktop_design::{
    app::{Dialog, Intent, Member, Source},
    config::{Config, Playback, Stream},
};
use p2pshare_media::config::Codec;
use p2pshare_session::AppConfig;
pub(super) fn ui_config(c: &AppConfig) -> Config {
    Config {
        language:c.language,
        username: c.username.clone(),
        volume_step_percent: c.volume_step_percent,
        theme: c.theme.clone(),
        stream: Stream {
            codec: match c.stream.codec {
                Codec::Auto => "auto",
                Codec::Avc => "avc",
                Codec::Hevc => "hevc",
            }
            .into(),
            bitrate_kbps: c.stream.bitrate_kbps,
            fps: c.stream.fps,
            width: c.stream.width,
            height: c.stream.height,
            audio: c.audio.enabled,
            exclude_audio_processes:c.audio_exclusions.enabled,
            show_cursor: c.stream.show_cursor,
            preview_interval_secs: c.stream.preview_interval_secs,
        },
        playback: Playback {
            buffer_ms: c.playback.buffer_ms,
        },
    }
}
fn merged_config(current: &AppConfig, input: &Config) -> anyhow::Result<AppConfig> {
    input.validate()?;
    let mut c = current.clone();
    c.username = input.username.trim().into();
    c.language=input.language;
    c.volume_step_percent = input.volume_step_percent;
    c.profile_setup_complete = true;
    c.theme = input.theme.clone();
    c.stream.codec = match input.stream.codec.as_str() {
        "auto" => Codec::Auto,
        "hevc" => Codec::Hevc,
        _ => Codec::Avc,
    };
    c.stream.bitrate_kbps = input.stream.bitrate_kbps;
    c.stream.fps = input.stream.fps;
    c.stream.width = input.stream.width;
    c.stream.height = input.stream.height;
    c.stream.preview_interval_secs = input.stream.preview_interval_secs;
    c.audio.enabled = input.stream.audio;
    c.audio_exclusions.enabled=input.stream.exclude_audio_processes;
    c.stream.show_cursor = input.stream.show_cursor;
    c.playback.buffer_ms = input.playback.buffer_ms;
    c.validate()?;
    Ok(c)
}
fn members(ui: &RoomUi, old: &[Member]) -> Vec<Member> {
    let preferences: HashMap<_, _> = old
        .iter()
        .map(|m| (m.id, (m.muted, m.volume, m.volume_before_silence)))
        .collect();
    let mut members: Vec<Member> = ui
        .participants
        .iter()
        .map(|m| Member {
            id: m.id,
            name: m.name.clone(),
            scene: if m.sharing { Some(0) } else { None },
            texture: m.texture,
            // A retained decoded texture keeps its actual aspect ratio between
            // frames, even if an announcement reports a different rounded size.
            dimensions: if !m.local && matches!(m.texture, Some(egui::TextureId::User(_))) {
                old.iter()
                    .find(|p| p.id == m.id && p.texture == m.texture)
                    .map(|p| p.dimensions)
                    .unwrap_or([m.size.width, m.size.height])
            } else {
                [m.size.width, m.size.height]
            },
            muted: preferences.get(&m.id).map(|p| p.0).unwrap_or(false),
            volume: preferences.get(&m.id).map(|p| p.1).unwrap_or(100),
            volume_before_silence: preferences.get(&m.id).map(|p| p.2).unwrap_or(100),
            title: if m.local {
                "Your screen".into()
            } else {
                "Screen share".into()
            },
            stats: if m.size.is_empty() {
                "Connecting video…".into()
            } else {
                format!("{} x {}", m.size.width, m.size.height)
            },
        })
        .collect();
    members.sort_by_key(|m| m.id != 0);
    if members.is_empty() {
        members.push(Member {
            id: 0,
            name: ui.name.clone(),
            ..Default::default()
        });
    }
    members
}
pub(super) fn load_config(path: &std::path::Path) -> anyhow::Result<AppConfig> {
    let legacy=path.parent().unwrap().join("data/config.json");
    if !path.exists()&&legacy.exists(){let staged=legacy.with_extension("toml");AppConfig::load_or_create(&staged,"Windows")?;std::fs::rename(staged,path)?;}
    let fresh = !path.exists() && !path.with_extension("json").exists();
    let mut config = AppConfig::load_or_create(path, "Windows")?;
    if fresh {
        config.profile_setup_complete = false;
        std::fs::write(path, config.document()?)?;
    }
    Ok(config)
}
impl View {
    pub(super) fn sync_design(&mut self) {
        self.sync_avatars();
        self.design.stream_viewers.clear();
        let roster=self.session.snapshot();
        for source in &roster.members {
            if let Some(viewers)=self.session.stream_viewers(&source.id){
                let names=viewers.iter().filter_map(|id|roster.members.iter().find(|m|&m.id==id).map(|m|m.name.clone())).collect();
                self.design.stream_viewers.insert(if source.id==roster.local_id{0}else{p2pshare_room_ui::peer_ui_id(&source.id)},names);
            }
        }
        self.design.maximized = self.window.is_maximized();
        self.design.window_focused = self.window.has_focus();
        self.design.watching = self
            .session
            .snapshot()
            .members
            .iter()
            .filter(|m| self.session.is_watching(&m.id))
            .map(|m| p2pshare_room_ui::peer_ui_id(&m.id))
            .collect();
        let focus = self
            .design
            .focus
            .and_then(|i| self.design.members.get(i))
            .map(|m| m.id);
        let old = std::mem::take(&mut self.design.members);
        self.design.members = members(&self.ui, &old);
        let state = self.session.snapshot();
        for member in &mut self.design.members {
            if member.id != 0 && !self.design.watching.contains(&member.id) {
                member.stats.clear();
            } else if let Some(peer) = state.members.iter().find(|p| {
                (member.id == 0 && p.id == state.local_id)
                    || (member.id != 0 && p2pshare_room_ui::peer_ui_id(&p.id) == member.id)
            }) {
                if peer.width > 0 && peer.height > 0 {
                    member.stats = format!("{}×{}", peer.width, peer.height);
                }
                if let Some(info) = self.session.stream_info(&peer.id) {
                    let codec = info.codec.map_or("Auto", |c| c.label());
                    member.stats.push_str(&format!(
                        " · {codec} · {:.1} Mbps",
                        info.bitrate_kbps as f32 / 1000.
                    ));
                } else if member.id != 0 {
                    member.stats.push_str(&format!(
                        " · {}",
                        self.session.video_codec(&peer.id).label()
                    ));
                }
                if member.id == 0 {
                    member
                        .stats
                        .push_str(&format!(" · {} FPS", self.stream_config.fps));
                }
            }
        }
        self.design.focus = focus.and_then(|id| {
            self.design
                .members
                .iter()
                .position(|m| m.id == id && m.scene.is_some())
        });
        if self.design.focus.is_none() {
            self.design.fullscreen = false;
        }
        self.design.joined = self.ui.in_room;
        self.design.status = self.ui.status.clone();
        while let Some((id, image)) = self.thumbnails.take() {
            if self.picker_open {
                if let Some(t) = self.preview_textures.get_mut(&id) {
                    t.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.preview_textures.insert(
                        id,
                        self.ctx.load_texture(
                            format!("source-preview-{id}"),
                            image,
                            egui::TextureOptions::LINEAR,
                        ),
                    );
                }
            }
        }
        for source in &mut self.design.sources {
            source.texture = self.preview_textures.get(&source.id).map(|t| t.id());
            source.dimensions = [360, 202];
        }
    }
    fn save_design(&mut self, input: &Config) -> anyhow::Result<()> {
        let c = merged_config(&self.app_config, input)?;
        // Persist first. A full document always preserves native encoder and audio tuning.
        let pending = p2pshare_media::portable::config_pending_path()?;
        std::fs::write(&pending, c.document()?)?;
        std::fs::rename(pending, &self.config_path)?;
        let stream_changed = c.stream != self.app_config.stream;
        let audio_changed =
            c.audio != self.app_config.audio || c.audio_exclusions!=self.app_config.audio_exclusions || c.playback != self.app_config.playback;
        if stream_changed {
            self.publisher.configure(c.stream)?;
            self.capture.set_cursor_capture(c.stream.show_cursor)?;
            if !crate::publisher::cursor_only_change(self.app_config.stream,c.stream){self.codecs.set_config(c.stream);}
            self.stream_config = c.stream;
            self.capture_meter =
                p2pshare_media::diagnostics::FrameMeter::new("local_preview_capture")
                    .with_fps(c.stream.fps);
        }
        if audio_changed {
            drop(self.audio.take());
            self.session.discard_playback();
            self.session.set_playback_config(c.playback)?;
            self.audio = Some(crate::audio::AudioRuntime::new(
                self.session.clone(),
                c.audio,
                c.audio_exclusions.clone(),
            )?);
        }
        if c.username != self.app_config.username {
            self.session.command(Command::Rename(c.username.clone()));
        }
        self.ui.name = c.username.clone();
        self.app_config = c;
        if audio_changed
            && self.ui.capture_running
            && let Some(audio) = &self.audio
        {
            let source = self
                .active_source
                .and_then(|id| self.ui.sources.iter().find(|s| s.id == id));
            audio.set_source(source);
        }
        tracing::info!("desktop_settings_saved");
        Ok(())
    }
    fn window_action(
        &mut self,
        action: p2pshare_desktop_design::chrome::WindowAction,
    ) -> anyhow::Result<()> {
        use p2pshare_desktop_design::chrome::{ResizeEdge, WindowAction};
        use winit::window::ResizeDirection;
        self.window_memory
            .capture(self.hwnd, self.window.fullscreen().is_some());
        match action {
            WindowAction::Minimize => {
                self.window_memory.flush(true);
                self.window.set_minimized(true);
            }
            WindowAction::ToggleMaximize => self.window.set_maximized(!self.window.is_maximized()),
            WindowAction::Close => self.close_requested = true,
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
        Ok(())
    }
    pub(super) fn apply_design(&mut self) {
        let mut failed = false;
        for intent in std::mem::take(&mut self.design.intents) {
            if failed {
                break;
            }
            let result: anyhow::Result<()> = (|| {
                match intent {
                    Intent::Language(language)=>{
                        let mut config=self.app_config.clone();config.language=Some(language);
                        let pending=p2pshare_media::portable::config_pending_path()?;std::fs::write(&pending,config.document()?)?;std::fs::rename(pending,&self.config_path)?;
                        self.app_config=config;self.design.config.language=Some(language);self.design.draft.language=Some(language);
                    },
                    Intent::SetSignalingServer(url) => {
                        let mut config = self.app_config.clone();
                        config.set_signaling_url(&url)?;
                        let pending = p2pshare_media::portable::config_pending_path()?;
                        std::fs::write(&pending, config.document()?)?;
                        std::fs::rename(pending, &self.config_path)?;
                        self.session.command(Command::SetSignalingServer(config.signaling_url.clone()));
                        self.app_config = config;
                        self.design.signaling_setup = None;
                        self.design.notice.clear();
                    }
                    Intent::DownloadUpdate => {
                        if let Some(notice) = &mut self.design.update {
                            self.updater.start_download(notice);
                        }
                    }
                    Intent::InstallUpdate => match self.updater.install() {
                        Ok(()) => self.close_requested = true,
                        Err(error) => {
                            if let Some(notice) = &mut self.design.update {
                                notice.error = Some(error.to_string());
                            }
                        }
                    },
                    Intent::Window(action) => self.window_action(action)?,
                    Intent::PopOut(peer) => self.popout_requests.push(peer),
                    Intent::Firewall => self.firewall.request(),
                    Intent::Watch { peer, enabled } => {
                        for member in self.session.snapshot().members {
                            if p2pshare_room_ui::peer_ui_id(&member.id) == peer {
                                self.session.set_watching(&member.id, enabled);
                                if !enabled
                                    && let Some(p) =
                                        self.ui.participants.iter_mut().find(|p| p.id == peer)
                                {
                                    p.texture = None;
                                }
                            }
                        }
                    }
                    Intent::Save(config) => self.save_design(&config)?,
                    Intent::ChooseAvatar=>self.choose_avatar()?,
                    Intent::SaveAvatar(bytes)=>self.save_avatar(bytes)?,
                    Intent::SetNickname { peer, nickname } => self.save_nickname(peer, nickname),
                    Intent::Join(room) => {
                        self.app_config.room_name = room.clone();
                        std::fs::write(&self.config_path, self.app_config.document()?)?;
                        self.ui.room_name = room.clone();
                        self.action(Action::NamedRoom {
                            room,
                            name: self.app_config.username.clone(),
                        })?;
                    }
                    Intent::Leave => {
                        self.action(Action::LeaveRoom)?;
                    }
                    Intent::Start(id) => {
                        self.action(Action::StartCapture(id))?;
                    }
                    Intent::Stop => {
                        self.action(Action::StopCapture)?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                tracing::error!(%error,"desktop_ui_action_failed");
                self.design.language_prompt=self.app_config.language.is_none();
                self.design.notice = format!("Could not apply settings: {error}");
                self.ui.status = self.design.notice.clone();
                self.design.config = ui_config(&self.app_config);
                failed = true;
            }
        }
        let peers = self.session.snapshot().members;
        for m in &self.design.members {
            if m.id != 0 {
                for peer in &peers {
                    if p2pshare_room_ui::peer_ui_id(&peer.id) == m.id {
                        if self.session.audio_muted(&peer.id) != m.muted {
                            self.session.set_audio_muted(&peer.id, m.muted);
                        }
                        if self.session.audio_volume(&peer.id) != m.volume {
                            self.session.set_audio_volume(&peer.id, m.volume);
                        }
                    }
                }
            }
        }
        self.set_viewer_fullscreen(self.design.fullscreen);
        let open = self.design.dialog == Dialog::Share;
        if open && !self.picker_open {
            self.preview_textures.clear();
            match enumerate_sources(Some(self.hwnd)) {
                Ok(sources) => {
                    self.ui.sources = sources;
                    self.design.selected = None;
                    self.design.sources = self
                        .ui
                        .sources
                        .iter()
                        .map(|s| Source {
                            id: s.id,
                            title: s.title.clone().into(),
                            app: match s.kind {
                                p2pshare_media::SourceKind::Monitor => "Display",
                                _ => "Application",
                            }
                            .into(),
                            scene: 0,
                            screen: s.kind == p2pshare_media::SourceKind::Monitor,
                            texture: None,
                            dimensions: [360, 202],
                        })
                        .collect();
                }
                Err(e) => {
                    self.ui.status = format!("Could not list windows: {e}");
                }
            }
        }
        if open && !self.picker_open {
            // One bounded snapshot batch per opening, shared by both tabs/search.
            // Filter changes only change which cached thumbnails are drawn.
            self.thumbnails
                .set_sources(self.ui.sources.iter().take(96).cloned().collect());
        } else if !open && self.picker_open {
            self.thumbnails.set_sources(Vec::new());
            self.preview_textures.clear();
        }
        self.picker_open = open;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_member_is_first_and_mute_tracks_peer_identity() {
        let mut ui = RoomUi::new(true);
        ui.participants.insert(
            0,
            p2pshare_room_ui::Participant {
                id: 42,
                name: "Remote".into(),
                local: false,
                sharing: true,
                texture: None,
                size: Dimensions::new(1280, 720),
            },
        );
        let old = vec![Member {
            id: 42,
            muted: true,
            volume: 37,
            ..Default::default()
        }];
        let out = members(&ui, &old);
        assert_eq!(out[0].id, 0);
        assert_eq!(out[1].id, 42);
        assert!(out[1].muted);
        assert_eq!(out[1].volume, 37);
        assert_eq!(out[0].volume, 100);
        assert_eq!(out[1].dimensions, [1280, 720]);
        ui.participants.remove(0);
        assert_eq!(members(&ui, &old).len(), 1);
    }
    #[test]
    fn retained_remote_texture_keeps_decoded_aspect_between_announcements() {
        let mut ui = RoomUi::new(true);
        let mut participant = ui.participants[0].clone();
        participant.id = 42;
        participant.local = false;
        participant.sharing = true;
        participant.texture = Some(egui::TextureId::User(42));
        participant.size = Dimensions::new(1920, 1080);
        ui.participants.push(participant);
        let old = vec![Member {
            id: 42,
            texture: Some(egui::TextureId::User(42)),
            dimensions: [1920, 1032],
            ..Default::default()
        }];
        let current = members(&ui, &old);
        assert_eq!(
            current.iter().find(|m| m.id == 42).unwrap().dimensions,
            [1920, 1032]
        );
        ui.participants.last_mut().unwrap().texture = Some(egui::TextureId::User(43));
        let current = members(&ui, &old);
        assert_eq!(
            current.iter().find(|m| m.id == 42).unwrap().dimensions,
            [1920, 1080]
        );
    }
    #[test]
    fn mouse_redraws_between_capture_frames_keep_preview_quad_fixed() {
        use p2pshare_desktop_design::app::App;
        use p2pshare_room_ui::{MemberState, RoomState};
        let state = RoomState {
            local_id: "local".into(),
            members: vec![MemberState {
                id: "local".into(),
                name: "You".into(),
                sharing: true,
                width: 818,
                height: 720,
            }],
            in_room: true,
            busy: false,
            invite: None,
            saved_invite: None,
            status: String::new(),
        };
        let mut room = RoomUi::new(true);
        room.capture_running = true;
        let mut app = App::live(Default::default(), "test-room".into(), false);
        app.joined = true;
        let ctx = egui::Context::default();
        let texture = egui::TextureId::User(9);
        let mut expected = None;
        for frame in 0..30 {
            room.apply_snapshot(&state);
            if frame % 5 == 0 {
                room.set_local_frame(texture, Dimensions::new(819, 721));
            }
            app.members = members(&room, &app.members);
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1920., 1032.),
                    )),
                    events: vec![egui::Event::PointerMoved(egui::pos2(
                        250. + frame as f32 * 20.,
                        400.,
                    ))],
                    ..Default::default()
                },
                |ui| app.ui(ui),
            );
            let positions = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == texture => {
                        Some(mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>())
                    }
                    _ => None,
                })
                .expect("Local preview quad missing");
            if let Some(expected) = &expected {
                assert_eq!(&positions, expected, "Preview moved on redraw {frame}");
            } else {
                expected = Some(positions);
            }
        }
    }
    #[test]
    fn first_start_prompt_survives_relaunch_and_completed_profile_is_preserved() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/desktop-config-tests")
            .join(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
                    .to_string(),
            );
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("config.toml");
        let c = load_config(&path).unwrap();
        assert!(!c.profile_setup_complete);
        assert!(!load_config(&path).unwrap().profile_setup_complete);
        let mut input = ui_config(&c);
        input.username = "Alex".into();
        let out = merged_config(&c, &input).unwrap();
        std::fs::write(&path, out.document().unwrap()).unwrap();
        let reopened = load_config(&path).unwrap();
        assert!(reopened.profile_setup_complete);
        assert_eq!(reopened.username, "Alex");
    }
    #[test]
    fn appearance_and_share_settings_preserve_native_options() {
        let mut c = AppConfig::default();
        c.stream.encoder = p2pshare_media::config::EncoderBackend::Software;
        c.stream.software_encode_threads = p2pshare_media::config::ThreadCount::Count(3);
        c.stream.software_decode_threads = p2pshare_media::config::ThreadCount::Auto;
        c.audio.bitrate_kbps = 256;
        c.signaling_url = "wss://example.com".into();
        let mut ui = ui_config(&c);
        ui.stream.fps = 120;
        ui.playback.buffer_ms = 1000;
        ui.theme.accent = "#123456".into();
        let out = merged_config(&c, &ui).unwrap();
        assert_eq!(out.stream.encoder, c.stream.encoder);
        assert_eq!(out.stream.software_encode_threads, c.stream.software_encode_threads);
        assert_eq!(out.stream.software_decode_threads, c.stream.software_decode_threads);
        assert_eq!(out.audio.bitrate_kbps, 256);
        assert_eq!(out.signaling_url, c.signaling_url);
        assert_eq!(out.stream.fps, 120);
        assert_eq!(out.playback.buffer_ms, 1000);
        assert_eq!(out.theme.accent, "#123456");
        assert!(out.profile_setup_complete);
    }
}

#[cfg(test)]mod cursor_config_tests {
 use super::*;
 #[test]fn cursor_defaults_on_and_ui_changes_round_trip_in_native_config(){
  let old=p2pshare_session::AppConfig::parse(r#"username = "Test""#).unwrap();assert!(old.stream.show_cursor);
  let mut ui=ui_config(&old);ui.stream.show_cursor=false;
  let changed=merged_config(&old,&ui).unwrap();let loaded=p2pshare_session::AppConfig::parse(&changed.document().unwrap()).unwrap();
  assert!(!loaded.stream.show_cursor);assert!(!ui_config(&loaded).stream.show_cursor);
 }
}

#[cfg(test)]mod audio_exclusion_config_tests{
 use super::*;
 #[test]fn audio_toggle_preserves_custom_process_list_and_video_settings(){
  let mut original=AppConfig::default();original.audio_exclusions.processes=vec!["custom.exe".into()];let mut ui=ui_config(&original);ui.stream.exclude_audio_processes=true;
  let changed=merged_config(&original,&ui).unwrap();assert!(changed.audio_exclusions.enabled);assert_eq!(changed.audio_exclusions.processes,original.audio_exclusions.processes);assert_eq!(changed.stream,original.stream);
  let parsed=AppConfig::parse(&changed.document().unwrap()).unwrap();assert!(ui_config(&parsed).stream.exclude_audio_processes);
 }
}
