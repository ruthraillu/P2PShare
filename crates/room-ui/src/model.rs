use crate::{audio::AudioPreferences, layout::Platform, viewer::ViewerState};
use egui::{Rect, TextureId};
use p2pshare_media::{CaptureSource, Dimensions};
#[derive(Clone, Debug)]
pub struct Participant {
    pub id: u64,
    pub name: String,
    pub local: bool,
    pub sharing: bool,
    pub texture: Option<TextureId>,
    pub size: Dimensions,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    SetLanguage(p2pshare_media::i18n::Language),
    JoinRoom {
        invite: String,
        name: String,
    },
    NamedRoom {
        room: String,
        name: String,
    },
    EditRoom(String),
    EditConfig,
    ChooseAvatar,
    SaveAvatar(Vec<u8>),
    RemoveAvatar,
    SaveMobilePreferences { buffer_ms: u32, server: String },
    ConfigureSignaling(String),
    ViewAndroidUpdate,
    PictureInPicture,
    LeaveRoom,
    Rename(String),
    EditName(String),
    CopyInvite(String),
    PasteInvite,
    RefreshSources,
    Watch {
        participant: u64,
        enabled: bool,
    },
    StartCapture(u64),
    StopCapture,
    ViewerMode {
        fullscreen: bool,
        handles_back: bool,
    },
    DebugLogging(bool),
    CopyDebugLog,
    SendDiagnosticRun(String),
    VideoQuality(p2pshare_media::VideoQuality),
    AudioVolume {
        participant: u64,
        percent: u8,
    },
    AudioMute {
        participant: u64,
        muted: bool,
    },
}
#[derive(Default)]
pub struct UiSnapshot {
    pub tiles: Vec<(u64, Rect)>,
    pub grid_button: Option<Rect>,
    pub navigation_buttons: Vec<(u8,Rect)>,
    pub share_button: Option<Rect>,
    pub fullscreen_button: Option<Rect>,
    pub pip_button: Option<Rect>,
    pub exit_fullscreen_button: Option<Rect>,
    pub viewer_rect: Option<Rect>,
    pub content_rect: Option<Rect>,
    pub audio_buttons: Vec<(u64, Rect)>,
    pub volume_sliders: Vec<(u64, Rect)>,
    pub volume_popup:Option<Rect>,
    pub watch_buttons: Vec<(u64, Rect)>,
    pub stop_button: Option<Rect>,
    pub fullscreen: bool,
    pub mobile: bool,
}
pub struct DiagnosticRun {
    pub id: String,
    pub time: String,
    pub status: &'static str,
}
pub struct DiagnosticReceipt {
    pub id: String,
    pub timestamp: Option<String>,
}
#[derive(Default)]
pub struct DiagnosticPanel {
    pub runs: Vec<DiagnosticRun>,
    pub selected: Option<String>,
    pub sending: bool,
    pub receipt: Option<Result<DiagnosticReceipt, String>>,
    pub configured: bool,
}

pub struct RoomUi {
    pub diagnostic_panel: DiagnosticPanel,
    pub language:p2pshare_media::i18n::Language,
    pub language_prompt:bool,
    pub profiles:crate::mobile_profiles::Profiles,
    pub participants: Vec<Participant>,
    pub watching: std::collections::HashSet<u64>,
    pub focused: Option<u64>,
    pub(crate) mobile_volume_popup:Option<(u64,Rect)>,
    pub pip_mode: bool,
    pub(crate) mobile_controls_until: f64,
    pub(crate) mobile_volume_until: f64,
    pub(crate) mobile_swipe: Option<(u64,u8,f32,f32)>,
    pub sources: Vec<CaptureSource>,
    pub status: String,
    pub capture_supported: bool,
    pub capture_running: bool,
    pub capture_fps: f64,
    pub quality: p2pshare_media::VideoQuality,
    pub encoder: String,
    pub playback_diagnostics: String,
    pub stream_details: std::collections::HashMap<u64, String>,
    pub logging_enabled: bool,
    pub video_config_external: bool,
    pub snapshot: UiSnapshot,
    pub name: String,
    pub room_name: String,
    pub playback_buffer_ms: u32,
    pub signaling_server: String,
    pub signaling_setup: bool,
    pub signaling_setup_error: String,
    pub android_update_version: Option<String>,
    pub in_room: bool,
    pub busy: bool,
    pub invitation: Option<String>,
    pub(crate) saved_invitation: Option<String>,
    pub join_input: String,
    pub(crate) room_dialog: bool,
    pub(crate) last_network_status: String,
    pub(crate) source_picker: bool,
    pub(crate) selected_source: Option<u64>,
    pub(crate) members_open: bool,
    pub(crate) diagnostics: bool,
    pub(crate) platform: Platform,
    pub(crate) safe_insets: [f32; 4],
    pub(crate) viewer: ViewerState,
    pub(crate) audio: AudioPreferences,
    pub(crate) last_viewer_mode: (bool, bool),
}
impl RoomUi {
    pub fn new(capture_supported: bool) -> Self {
        Self {
            language:Default::default(),language_prompt:false,
            profiles:Default::default(),
            participants: vec![Participant {
                id: 0,
                name: "You".into(),
                local: true,
                sharing: false,
                texture: None,
                size: Dimensions::default(),
            }],
            focused: None,
            mobile_volume_popup:None,
            pip_mode: false,
            mobile_controls_until: 0.0,
            mobile_volume_until: 0.0,
            mobile_swipe: None,
            watching: Default::default(),
            sources: vec![],
            status: "Local room ready".into(),
            capture_supported,
            capture_running: false,
            capture_fps: 0.0,
            quality: p2pshare_media::VideoQuality::default(),
            encoder: String::new(),
            playback_diagnostics: String::new(),
            stream_details: Default::default(),
            logging_enabled: false,
            diagnostic_panel: DiagnosticPanel::default(),
            video_config_external: false,
            snapshot: UiSnapshot::default(),
            name: "You".into(),
            room_name: "grupo".into(),
            playback_buffer_ms: 0,
            signaling_server: String::new(),
            signaling_setup: false,
            signaling_setup_error: String::new(),
            android_update_version: None,
            in_room: false,
            busy: true,
            invitation: None,
            saved_invitation: None,
            join_input: String::new(),
            room_dialog: false,
            last_network_status: String::new(),
            source_picker: false,
            selected_source: None,
            members_open: false,
            diagnostics: false,
            platform: Platform::Desktop,
            safe_insets: [0.0; 4],
            viewer: ViewerState::default(),
            audio: AudioPreferences::default(),
            last_viewer_mode: (false, false),
        }
    }
    pub fn apply_snapshot(&mut self, state: &crate::RoomState) {
        self.in_room = state.in_room;
        self.busy = state.busy;
        self.invitation = state.invite.clone();
        self.saved_invitation = state.saved_invite.clone();
        if state.status != self.last_network_status {
            self.status = state.status.clone();
            self.last_network_status = state.status.clone();
        }
        let old = self.participants.clone();
        self.participants = state
            .members
            .iter()
            .map(|m| {
                let local = m.id == state.local_id;
                let id = if local { 0 } else { peer_ui_id(&m.id) };
                let previous = old.iter().find(|p| p.id == id);
                // Local preview is capture-owned; a delayed publish acknowledgement must not remove it.
                let sharing = m.sharing
                    || (local && self.capture_running && previous.is_some_and(|p| p.sharing));
                Participant {
                    id,
                    name: m.name.clone(),
                    local,
                    sharing,
                    texture: if sharing {
                        previous.and_then(|p| p.texture)
                    } else {
                        None
                    },
                    // A live local texture uses capture dimensions, not the encoded
                    // announcement (which can be scaled or rounded to even pixels).
                    // Metadata-only redraws must not change its aspect ratio.
                    size: if local
                        && self.capture_running
                        && previous.is_some_and(|p| p.texture.is_some() || !m.sharing)
                    {
                        previous.map(|p| p.size).unwrap_or_default()
                    } else {
                        Dimensions::new(m.width, m.height)
                    },
                }
            })
            .collect();
        if !self.participants.iter().any(|p| p.local) {
            self.participants.insert(
                0,
                Participant {
                    id: 0,
                    name: self.name.clone(),
                    local: true,
                    sharing: false,
                    texture: None,
                    size: Dimensions::default(),
                },
            );
        }
        self.audio.retain(self.participants.iter().map(|p| p.id));
        self.watching
            .retain(|id| self.participants.iter().any(|p| p.id == *id && p.sharing));
        self.reconcile_focus();
    }
    pub fn set_remote_frame(&mut self, peer: &str, texture: TextureId, size: Dimensions) {
        if let Some(p) = self
            .participants
            .iter_mut()
            .find(|p| p.id == peer_ui_id(peer) && !p.local && p.sharing)
        {
            p.texture = Some(texture);
            p.size = size;
        }
    }
    pub fn set_local_frame(&mut self, texture: TextureId, size: Dimensions) {
        if let Some(p) = self.participants.iter_mut().find(|p| p.local) {
            p.sharing = true;
            p.texture = Some(texture);
            p.size = size;
        }
    }
    pub fn clear_local_frame(&mut self) {
        self.capture_running = false;
        if let Some(p) = self.participants.iter_mut().find(|p| p.local) {
            p.sharing = false;
            p.texture = None;
        }
        self.reconcile_focus();
    }
    pub fn reconcile_focus(&mut self) {
        if self
            .focused
            .is_some_and(|id| !self.participants.iter().any(|p| p.id == id && p.sharing))
        {
            self.focused = None;
            self.viewer.screen = None;
        }
    }
    pub fn open_settings(&mut self) {self.room_dialog=true;self.members_open=false;}
    pub fn open_users(&mut self) {self.members_open=true;self.room_dialog=false;}
    pub fn close_mobile_page(&mut self) {self.members_open=false;self.room_dialog=false;}
    pub fn set_platform(&mut self, platform: Platform) {
        self.platform = platform;
    }
    /// Insets are logical egui points, in left/top/right/bottom order.
    pub fn set_safe_insets(&mut self, insets: [f32; 4]) {
        self.safe_insets = insets.map(|v| v.max(0.0));
    }
    pub fn is_fullscreen(&self) -> bool {
        self.viewer.screen.is_some()
    }
    pub fn audio_muted(&self, peer: &str) -> bool {
        self.audio.is_muted(peer_ui_id(peer))
    }
    /// Active media selection is independent of the gallery/focused presentation.
    pub fn playback_participant(&self) -> Option<&Participant> {
        self.participants.iter().find(|p| p.sharing && !p.local && self.watching.contains(&p.id)
            && self.focused.is_none_or(|id| p.id == id))
            .or_else(|| self.participants.iter().find(|p| p.sharing && !p.local && self.watching.contains(&p.id)))
    }
    pub fn participant_audio_muted(&self, id: u64) -> bool {
        self.audio.is_muted(id)
    }
    pub fn enter_fullscreen(&mut self, id: u64) {
        if self.participants.iter().any(|p| p.id == id && p.sharing) {
            self.focused = Some(id);
            self.viewer.screen = Some(id);
            self.viewer.controls_until = None;
            self.members_open = false;
            self.source_picker = false;
            self.room_dialog = false;
            self.diagnostics = false;
        }
    }
    pub fn handles_back(&self) -> bool {
        self.mobile_volume_popup.is_some() || self.profiles.handles_back() || self.is_fullscreen()
            || self.focused.is_some()
            || self.members_open
            || self.source_picker
            || self.room_dialog
            || self.diagnostics
    }
    /// Platform Back/Escape dismisses one presentation level and never leaves a room.
    pub fn back(&mut self) -> bool {
        if self.mobile_volume_popup.take().is_some(){return true;}
        if self.profiles.back(){return true;}
        if self.room_dialog {
            self.room_dialog = false;
        } else if self.source_picker {
            self.source_picker = false;
        } else if self.members_open {
            self.members_open = false;
        } else if self.diagnostics {
            self.diagnostics = false;
        } else if self.is_fullscreen() {
            self.viewer.screen = None;
            self.mobile_controls_until = 0.0;
        } else if self.focused.is_some() {
            self.focused = None;
            self.mobile_controls_until = 0.0;
        } else {
            return false;
        }
        true
    }
}
pub fn peer_ui_id(peer: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    peer.hash(&mut h);
    h.finish().max(1)
}
