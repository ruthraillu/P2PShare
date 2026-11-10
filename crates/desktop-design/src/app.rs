//! Desktop presentation and typed user intents. Native adapters supply media textures.
use crate::config::ThemeUi;
use crate::{
    config::{Config, Palette},
    scenes,
};
use egui::{
    Align2, Button, Color32, FontId, Id, Label, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use std::{collections::HashMap, path::PathBuf};
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    None,
    Share,
    Appearance,
    Crop,
    Room,
    Name,
}
#[derive(Clone)]
pub struct Member {
    pub id: u64,
    pub texture: Option<egui::TextureId>,
    pub dimensions: [u32; 2],
    pub stats: String,
    pub name: String,
    pub scene: Option<usize>,
    pub muted: bool,
    pub volume: u8,
    pub volume_before_silence: u8,
    pub title: String,
}
impl Default for Member {
    fn default() -> Self {
        Self {
            id: 0,
            texture: None,
            dimensions: [0, 0],
            stats: String::new(),
            name: String::new(),
            scene: None,
            muted: false,
            volume: 100,
            volume_before_silence: 100,
            title: String::new(),
        }
    }
}
#[derive(Clone)]
pub struct Source {
    pub id: u64,
    pub texture: Option<egui::TextureId>,
    pub dimensions: [u32; 2],
    pub title: std::borrow::Cow<'static, str>,
    pub app: std::borrow::Cow<'static, str>,
    pub scene: usize,
    pub screen: bool,
}
pub const SOURCES: [Source; 6] = [
    Source {
        id: 0,
        texture: None,
        dimensions: [1920, 1080],
        title: std::borrow::Cow::Borrowed("Night walk"),
        app: std::borrow::Cow::Borrowed("Video player"),
        scene: 0,
        screen: false,
    },
    Source {
        id: 0,
        texture: None,
        dimensions: [1920, 1080],
        title: std::borrow::Cow::Borrowed("Component library"),
        app: std::borrow::Cow::Borrowed("Design studio"),
        scene: 1,
        screen: false,
    },
    Source {
        id: 0,
        texture: None,
        dimensions: [1920, 1080],
        title: std::borrow::Cow::Borrowed("player.rs"),
        app: std::borrow::Cow::Borrowed("Code editor"),
        scene: 2,
        screen: false,
    },
    Source {
        id: 0,
        texture: None,
        dimensions: [1920, 1080],
        title: std::borrow::Cow::Borrowed("Weekly activity"),
        app: std::borrow::Cow::Borrowed("Browser"),
        scene: 3,
        screen: false,
    },
    Source {
        id: 0,
        texture: None,
        dimensions: [1920, 1080],
        title: std::borrow::Cow::Borrowed("Display 1"),
        app: std::borrow::Cow::Borrowed("1920 x 1080 · Primary"),
        scene: 1,
        screen: true,
    },
    Source {
        id: 0,
        texture: None,
        dimensions: [1920, 1080],
        title: std::borrow::Cow::Borrowed("Display 2"),
        app: std::borrow::Cow::Borrowed("1920 x 1080"),
        scene: 2,
        screen: true,
    },
];
#[derive(Clone)]
pub enum Intent {
    Language(p2pshare_media::i18n::Language),
    SetSignalingServer(String),
    DownloadUpdate,
    InstallUpdate,
    Save(Box<Config>),
    ChooseAvatar,
    SaveAvatar(Option<Vec<u8>>),
    SetNickname { peer: u64, nickname: Option<String> },
    Join(String),
    Leave,
    Firewall,
    Start(u64),
    Stop,
    PopOut(u64),
    Window(crate::chrome::WindowAction),
    Watch { peer: u64, enabled: bool },
}
pub struct App {
    pub language_prompt:bool,
    pub signaling_setup: Option<String>,
    pub update: Option<crate::updates::Notice>,
    pub update_open: bool,
    pub avatars:HashMap<u64,egui::TextureId>,
    pub profile_view:Option<u64>,
    pub profile_nicknames: HashMap<u64, String>,
    pub(crate) nickname_draft: Option<(u64, String)>,
    pub nickname_save_failed: bool,
    pub profile_names:HashMap<u64,Vec<String>>,
    pub profile_images:HashMap<u64,Vec<std::sync::Arc<[u8]>>>,
    pub profile_history_textures:Vec<egui::TextureHandle>,
    pub profile_history_key:Option<u64>,
    pub profile_show_names:bool,
    pub profile_enlarged:Option<egui::TextureId>,
    pub avatar_data:HashMap<u64,std::sync::Arc<[u8]>>,
    pub crop:Option<crate::profile::Crop>,
    pub pending_avatar:Option<Option<Vec<u8>>>,
    pub profile_preview:Option<egui::TextureHandle>,
    pub live: bool,
    pub maximized: bool,
    pub window_focused: bool,
    pub sources: Vec<Source>,
    pub intents: Vec<Intent>,
    pub status: String,
    pub settings_only: bool,
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub members: Vec<Member>,
    pub room: String,
    pub room_input: String,
    pub joined: bool,
    pub dialog: Dialog,
    pub focus: Option<usize>,
    pub fullscreen: bool,
    pub popped_out: Option<usize>,
    pub detached_peers: std::collections::HashSet<u64>,
    pub always_on_top: bool,
    pub(crate) caption_visible: bool,
    pub search: String,
    pub screens_tab: bool,
    pub selected: Option<usize>,
    pub draft: Config,
    pub name_input: String,
    pub notice: String,
    pub hits: HashMap<String, Rect>,
    pub regions: Vec<(String, Rect)>,
    pub saves: usize,
    pub watching: std::collections::HashSet<u64>,
    pub stream_viewers:HashMap<u64,Vec<String>>,
    pub local_preview_hidden: bool,
}
pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w.max(0.), h.max(0.)))
}
pub fn text(ui: &Ui, at: egui::Pos2, t: &str, size: f32, color: Color32) {
    ui.painter()
        .text(at, Align2::LEFT_TOP, t, FontId::proportional(size), color);
}
pub fn label(ui: &mut Ui, r: Rect, t: &str, size: f32, color: Color32) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(r)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add(
                Label::new(RichText::new(t).size(size).color(color))
                    .truncate()
                    .halign(egui::Align::Min),
            );
        },
    );
}
impl App {
    pub fn new(config: Config, path: Option<PathBuf>) -> Self {
        let first = config.username.trim().is_empty();
        let draft = config.clone();
        let members = [
            (p2pshare_media::tr!("You"), None, p2pshare_media::tr!("Your screen")),
            ("Maya", Some(0), "Night walk"),
            ("Theo", Some(1), "Component library"),
            ("Nora", Some(2), "Code editor"),
            ("Jules", Some(3), "Weekly activity"),
            ("Sam", None, p2pshare_media::tr!("Watching")),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, (n, s, t))| Member {
            id: id as u64,
            name: n.into(),
            scene: s,
            muted: false,
            title: t.into(),
            ..Member::default()
        })
        .collect();
        Self {
            language_prompt:false,
            signaling_setup: None,
            update: None,
            update_open: false,
            profile_nicknames: Default::default(),
            nickname_draft: None,
            nickname_save_failed: false,
            profile_view:None,profile_names:Default::default(),profile_images:Default::default(),profile_history_textures:Vec::new(),profile_history_key:None,profile_show_names:false,profile_enlarged:None,
            avatars:Default::default(),avatar_data:Default::default(),crop:None,pending_avatar:None,profile_preview:None,
            live: false,
            maximized: false,
            window_focused: true,
            sources: SOURCES.to_vec(),
            intents: Vec::new(),
            status: String::new(),
            settings_only: false,
            config,
            config_path: path,
            members,
            room: "screen-lounge".into(),
            room_input: "screen-lounge".into(),
            joined: true,
            dialog: if first { Dialog::Name } else { Dialog::None },
            focus: None,
            fullscreen: false,
            popped_out: None,
            detached_peers: Default::default(),
            always_on_top: false,
            caption_visible: false,
            search: String::new(),
            screens_tab: false,
            selected: None,
            draft,
            name_input: String::new(),
            notice: String::new(),
            hits: HashMap::new(),
            regions: Vec::new(),
            saves: 0,
            watching: Default::default(),
            stream_viewers:Default::default(),
            local_preview_hidden: false,
        }
    }
    pub fn live(config: Config, room: String, prompt: bool) -> Self {
        let mut app = Self::new(config, None);
        app.live = true;
        app.members.truncate(1);
        app.sources.clear();
        app.room = room;
        app.joined = false;
        app.dialog = if prompt { Dialog::Name } else { Dialog::None };
        app
    }
    /// Only actual video needs display-refresh redraws. JPEGs and hidden previews do not.
    pub fn has_visible_live_video(&self) -> bool {
        self.members.iter().enumerate().any(|(index, member)| {
            member.scene.is_some()
                && self.popped_out.is_none_or(|id| id == index)
                && (self.popped_out.is_some() || !self.detached_peers.contains(&member.id))
                && (!self.fullscreen || self.focus.unwrap_or(0) == index)
                && if index == 0 {
                    !self.local_preview_hidden
                } else {
                    !self.live || self.watching.contains(&member.id)
                }
        })
    }
    fn paint_stream(&self, ui: &Ui, area: Rect, member: usize) {
        if self.popped_out.is_none() && self.detached_peers.contains(&self.members[member].id) {
            ui.painter().text(
                area.center(),
                Align2::CENTER_CENTER,
                p2pshare_media::tr!("Playing in a separate window"),
                FontId::proportional(14.),
                self.palette().muted,
            );
        } else if member == 0 && self.local_preview_hidden {
            ui.painter().text(
                area.center(),
                Align2::CENTER_CENTER,
                p2pshare_media::tr!("Local preview hidden\nYour screen is still being shared."),
                FontId::proportional(14.),
                self.palette().muted,
            );
        } else {
            let source = &self.members[member];
            self.paint_media(
                ui,
                area,
                source.texture,
                source.dimensions,
                source.scene.unwrap_or_default(),
            );
        }
    }
    pub fn paint_media(
        &self,
        ui: &Ui,
        r: Rect,
        texture: Option<egui::TextureId>,
        dimensions: [u32; 2],
        scene: usize,
    ) {
        if !self.live {
            scenes::paint(ui.painter(), scenes::fitted(r), scene);
            return;
        }
        if let Some(texture) = texture {
            let [w, h] = dimensions;
            if w > 0 && h > 0 {
                let scale = (r.width() / w as f32).min(r.height() / h as f32);
                let fitted =
                    Rect::from_center_size(r.center(), vec2(w as f32 * scale, h as f32 * scale));
                ui.painter().image(
                    texture,
                    fitted,
                    Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                    Color32::WHITE,
                );
            }
        } else {
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                p2pshare_media::tr!("Waiting for preview"),
                FontId::proportional(13.),
                self.palette().muted,
            );
        }
    }
    pub fn palette(&self) -> Palette {
        if matches!(self.dialog,Dialog::Appearance|Dialog::Crop) {
            self.draft.theme.palette()
        } else {
            self.config.theme.palette()
        }
    }
    pub fn save(&mut self) -> bool {
        if self.live {
            if let Err(e) = self.config.validate() {
                self.notice = e.to_string();
                return false;
            }
            self.intents
                .push(Intent::Save(Box::new(self.config.clone())));
            self.saves += 1;
            return true;
        }
        match self.config.validate().and_then(|_| {
            if let Some(path) = &self.config_path {
                self.config.save(path)
            } else {
                Ok(())
            }
        }) {
            Ok(()) => {
                self.saves += 1;
                self.notice = "Saved to prototype config.toml".into();
                true
            }
            Err(e) => {
                self.notice = p2pshare_media::trf!("Could not save: {e}",e=e);
                false
            }
        }
    }
    pub fn button(&mut self, ui: &mut Ui, key: &str, r: Rect, title: &str, primary: bool) -> bool {
        let p = self.palette();
        let hovered = ui.rect_contains_pointer(r) && ui.is_enabled();
        let danger =
            (key == "leave" && self.joined) || (key == "share" && self.members[0].scene.is_some());
        let response = ui
            .scope_builder(
                egui::UiBuilder::new().id(Id::new(("desktop-button", key))),
                |ui| {
                    ui.put(
                        r,
                        Button::new(RichText::new(title).color(if primary {
                            p.accent_text
                        } else if danger {
                            p.red
                        } else {
                            p.text
                        }))
                        .truncate()
                        .fill(if primary {
                            p.accent
                        } else if hovered {
                            p.hover
                        } else {
                            p.surface
                        })
                        .stroke(Stroke::new(1., if primary { p.accent } else { p.border }))
                        .corner_radius(7),
                    )
                },
            )
            .inner;
        self.hits.insert(key.into(), response.rect);
        if key.starts_with("full-") {
            response.on_hover_text(p2pshare_media::tr!("Full screen")).clicked()
        } else if key.starts_with("pin-") {
            response
                .on_hover_text(if self.always_on_top {
                    p2pshare_media::tr!("Always on top enabled — click to disable")
                } else {
                    p2pshare_media::tr!("Keep this window always on top")
                })
                .clicked()
        } else if key.starts_with("popout-") {
            response.on_hover_text(p2pshare_media::tr!("Open in separate window")).clicked()
        } else {
            response.clicked()
        }
    }
    pub fn open_room(&mut self) {
        self.room_input = self.room.clone();
        self.dialog = Dialog::Room;
        self.notice.clear();
    }
    pub fn open_share(&mut self) {
        self.draft = self.config.clone();
        self.settings_only = false;
        self.dialog = Dialog::Share;
        self.search.clear();
        self.notice.clear();
    }
    pub fn open_appearance(&mut self) {
        self.draft = self.config.clone();
        self.name_input = self.config.username.clone();
        self.dialog = Dialog::Appearance;
        self.notice.clear();
    }
    pub fn ui(&mut self, ui: &mut Ui) {
        let _language=p2pshare_media::i18n::scope(self.config.language.unwrap_or_default());
        self.hits.clear();
        self.regions.clear();
        let effective = if matches!(self.dialog,Dialog::Appearance|Dialog::Crop) {
            &self.draft.theme
        } else {
            &self.config.theme
        };
        effective.apply(ui.ctx());
        if self.language_prompt {self.language_dialog(ui);return;}
        if self.signaling_setup.is_some() {
            self.signaling_dialog(ui);
            return;
        }
        if self.dialog == Dialog::None && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.fullscreen {
                self.fullscreen = false;
            } else {
                self.focus = None;
            }
        }
        let p = self.palette();
        let viewport = ui.max_rect();
        // Captions overlay the content so hiding/revealing never shifts the layout.
        let all = viewport;
        ui.painter().rect_filled(all, 0., p.bg);
        if self.fullscreen {
            self.immersive(ui, all);
        } else if let Some(id) = self.popped_out {
            self.regions.push(("popout".into(), all));
            if self.members.get(id).is_some_and(|m| m.scene.is_some()) {
                self.tile(ui, all, id, false);
            } else {
                ui.painter().text(
                    all.center(),
                    Align2::CENTER_CENTER,
                    p2pshare_media::tr!("Sharing has stopped"),
                    FontId::proportional(16.),
                    p.muted,
                );
            }
        } else {
            let sw = if all.width() < 1100. { 184. } else { 208. };
            let side = rect(all.left(), all.top(), sw, all.height());
            let main = rect(side.right(), all.top(), all.width() - sw, all.height());
            self.regions.push(("sidebar".into(), side));
            self.sidebar(ui, side);
            let foot = rect(main.left(), main.bottom() - 44., main.width(), 44.);
            self.regions.push(("footer".into(), foot));
            self.room_footer(ui, foot);
            let content = Rect::from_min_max(
                pos2(main.left() + 12., main.top() + 12.),
                pos2(main.right() - 12., foot.top() - 12.),
            );
            if self.joined {
                if let Some(id) = self.focus {
                    self.focus_view(ui, content, id);
                } else {
                    self.grid(ui, content);
                }
            } else {
                text(
                    ui,
                    content.center() - vec2(164., 44.),
                    p2pshare_media::tr!("A room for your screens."),
                    24.,
                    p.text,
                );
                text(
                    ui,
                    content.center() - vec2(164., 3.),
                    p2pshare_media::tr!("Choose a room name to get together."),
                    14.,
                    p.muted,
                );
                if self.button(
                    ui,
                    "join-room",
                    Rect::from_center_size(content.center() + vec2(0., 54.), vec2(180., 38.)),
                    p2pshare_media::tr!("Join a room"),
                    true,
                ) {
                    self.open_room();
                }
                if self.live && !self.status.is_empty() {
                    let painter = ui.painter_at(content);
                    let message = p2pshare_media::i18n::status(&self.status).into_owned();
                    let galley = painter.layout(
                        message,
                        FontId::proportional(14.),
                        p.muted,
                        (content.width() - 32.).max(1.),
                    );
                    let position = content.center() + vec2(-galley.size().x * 0.5, 86.);
                    painter.galley(position, galley, p.muted);
                }
            }
        }
        self.draw_dialog(ui.ctx());
        self.draw_profile_viewer(ui.ctx());
        self.draw_update_dialog(ui);
        if !self.fullscreen {
            self.window_chrome(ui.ctx(), viewport);
        }
    }
    fn sidebar(&mut self, ui: &mut Ui, r: Rect) {
        let p = self.palette();
        let x = r.left();
        let y = r.top();
        ui.painter().rect_filled(r, 0., p.side);
        text(
            ui,
            pos2(x + 16., y + 16.),
            &p2pshare_media::trf!(
                "USERS  /  {}",
                if self.joined { self.members.len() } else { 0 }
            ),
            11.,
            p.muted,
        );
        if self.joined {
            let list = rect(x, y + 43., r.width(), (r.height() - 43. - 100.).max(0.));
            ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("member-scroll")
                    .auto_shrink([false, false])
                    .max_height(list.height())
                    .show(ui, |ui| {
                        let origin = ui.cursor().top();
                        ui.set_min_height(self.members.len() as f32 * 53.);
                        for i in 0..self.members.len() {
                            let yy = origin + i as f32 * 53.;
                            let m = &self.members[i];
                            let name = if i == 0 {
                                if self.config.username.is_empty() {
                                    p2pshare_media::tr!("You")
                                } else {
                                    &self.config.username
                                }
                            } else {
                                &m.name
                            };
                            let initials = name
                                .chars()
                                .next()
                                .unwrap_or('?')
                                .to_uppercase()
                                .to_string();
                            ui.painter().circle_filled(
                                pos2(x + 33., yy + 21.),
                                20.,
                                if i == 0 { p.accent } else { p.surface },
                            );
                            let center = pos2(x + 33., yy + 21.);
                            let color = if i == 0 { p.accent_text } else { p.text };
                            let glyph = ui.painter().layout_no_wrap(
                                initials,
                                FontId::proportional(16.),
                                color,
                            );
                            let ink = if glyph.mesh_bounds.is_finite()
                                && glyph.mesh_bounds.is_positive()
                            {
                                glyph.mesh_bounds
                            } else {
                                glyph.rect
                            };
                            let at = center - ink.center().to_vec2();
                            if ui
                                .clip_rect()
                                .contains_rect(Rect::from_center_size(center, vec2(40., 40.)))
                            {
                                self.regions.push((
                                    format!("avatar-{i}"),
                                    Rect::from_center_size(center, vec2(40., 40.)),
                                ));
                                self.regions
                                    .push((format!("initial-{i}"), ink.translate(at.to_vec2())));
                            }
                            let profile_hit=rect(x+12.,yy,r.width()-24.,44.);
                            self.hits.insert(format!("profile-user-{}",m.id),profile_hit);
                            let profile = ui.interact(profile_hit, ui.id().with(("profile-user", m.id)), egui::Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if profile.clicked() {
                                self.profile_view = Some(m.id);
                                self.profile_show_names = false;
                            }
                            ui.painter().galley(at, glyph, color);
                            if let Some(texture)=self.avatars.get(&m.id){crate::profile::circle(ui.painter(),center,20.,*texture,Rect::from_min_max(pos2(0.,0.),pos2(1.,1.)));}
                            let name_area = rect(x + 65., yy, r.width() - 77., 22.);
                            if let Some(nickname) = self.profile_nicknames.get(&m.id) {
                                let mut job = egui::text::LayoutJob::default();
                                job.wrap.max_width = name_area.width();
                                job.wrap.max_rows = 1;
                                job.append(name, 0., egui::TextFormat::simple(FontId::proportional(14.), p.text));
                                job.append(
                                    &format!(" ({nickname})"),
                                    0.,
                                    egui::TextFormat::simple(
                                        FontId::proportional(14.),
                                        p.accent.lerp_to_gamma(p.text, 0.40),
                                    ),
                                );
                                let galley = ui.painter().layout_job(job);
                                if galley.elided {
                                    profile.on_hover_text(format!("{name} ({nickname})"));
                                }
                                let at = name_area.left_center() - vec2(0., galley.size().y / 2.);
                                ui.painter_at(name_area).galley(at, galley, p.text);
                            } else {
                                label(
                                    ui,
                                    name_area,
                                    &format!("{name}{}", if i == 0 { p2pshare_media::tr!(" (you)") } else { "" }),
                                    14.,
                                    p.text,
                                );
                            }
                            text(
                                ui,
                                pos2(x + 65., yy + 23.),
                                if m.scene.is_some() {
                                    p2pshare_media::tr!("Sharing screen")
                                } else {
                                    p2pshare_media::tr!("In room")
                                },
                                11.,
                                if m.scene.is_some() { p.green } else { p.muted },
                            );
                        }
                    });
            });
        }
        let bottom = r.bottom() - 44.;
        let room_area = rect(x + 12., bottom - 38., r.width() - 24., 32.);
        self.hits.insert("room".into(), room_area);
        if ui.rect_contains_pointer(room_area) {
            ui.painter().rect_filled(room_area, 5., p.hover);
        }
        text(ui, room_area.min + vec2(4., 0.), p2pshare_media::tr!("ROOM"), 9., p.muted);
        label(
            ui,
            rect(
                room_area.left() + 4.,
                room_area.top() + 11.,
                room_area.width() - 27.,
                20.,
            ),
            &format!("# {}", self.room),
            14.,
            p.text,
        );
        // Small pencil cue: this is an editable room field, not another action button.
        let c = pos2(room_area.right() - 10., room_area.center().y + 5.);
        let stroke = Stroke::new(1.2, p.muted);
        ui.painter().add(egui::epaint::PathShape::closed_line(
            vec![
                c + vec2(-5., 5.),
                c + vec2(-4., 1.),
                c + vec2(3., -6.),
                c + vec2(6., -3.),
                c + vec2(-1., 4.),
            ],
            stroke,
        ));
        ui.painter()
            .line_segment([c + vec2(1., -4.), c + vec2(4., -1.)], stroke);
        let response = ui
            .interact(room_area, Id::new("room-selector"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(p2pshare_media::trf!("{}\nClick to change room", self.room));
        if response.clicked() {
            self.open_room();
            self.notice.clear();
        }
        self.regions.push((
            "sidebar-footer".into(),
            rect(r.left(), bottom, r.width(), 44.),
        ));
        ui.painter()
            .hline(x..=r.right(), bottom, Stroke::new(1., p.border));
        self.profile_footer(ui, rect(r.left(), bottom, r.width(), 44.));
    }
    fn grid(&mut self, ui: &mut Ui, r: Rect) {
        let ids: Vec<_> = self
            .members
            .iter()
            .enumerate()
            .filter(|(_, m)| m.scene.is_some())
            .map(|(i, _)| i)
            .collect();
        let n = ids.len();
        if n == 0 {
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                p2pshare_media::tr!("No screens shared yet. Use Share screen to start."),
                FontId::proportional(16.),
                self.palette().muted,
            );
            return;
        }
        let gap = 12.;
        // Leave enough room for Stop watching, Mute and fullscreen on one row.
        let columns = (((r.width() + gap) / (280. + gap)).floor() as usize).max(1);
        let split = (if n <= 2 { n } else { n.div_ceil(2) }).min(columns);
        let rows = n.div_ceil(split);
        let h = (r.height() - gap * (rows - 1) as f32) / rows as f32;
        for (row, chunk) in ids.chunks(split).enumerate() {
            let w = (r.width() - gap * (chunk.len() - 1) as f32) / chunk.len() as f32;
            for (col, &id) in chunk.iter().enumerate() {
                self.tile(
                    ui,
                    rect(
                        r.left() + col as f32 * (w + gap),
                        r.top() + row as f32 * (h + gap),
                        w,
                        h,
                    ),
                    id,
                    false,
                );
            }
        }
    }
    fn tile(&mut self, ui: &mut Ui, r: Rect, id: usize, small: bool) {
        let p = self.palette();
        if self.members[id].scene.is_none() {
            return;
        }
        self.regions.push((format!("tile-{id}"), r));
        let window_player = self.popped_out.is_some();
        let radius = if window_player { 0 } else { 9 };
        ui.painter().rect_filled(r, radius, p.video);
        let detached =
            self.popped_out.is_none() && self.detached_peers.contains(&self.members[id].id);
        let watching = self.live && id != 0 && self.watching.contains(&self.members[id].id);
        let meta = if small { 30. } else { 44. };
        let video = Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - meta));
        self.paint_stream(
            ui,
            if window_player {
                video
            } else {
                video.shrink(1.)
            },
            id,
        );
        ui.painter().rect_filled(
            rect(r.left(), r.bottom() - meta, r.width(), meta),
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: radius,
                se: radius,
            },
            p.surface,
        );
        let click = ui.interact(
            video,
            Id::new(("stream", self.members[id].id, small)),
            Sense::click(),
        );
        self.hits.insert(format!("tile-{id}"), video);
        if click.clicked() && detached {
            self.intents.push(Intent::PopOut(self.members[id].id));
        }
        if click.clicked() && !detached {
            self.focus = Some(id);
            if self.live && id != 0 && !self.watching.contains(&self.members[id].id) {
                self.intents.push(Intent::Watch {
                    peer: self.members[id].id,
                    enabled: true,
                });
            }
        }
        if self.live
            && !detached
            && id != 0
            && !watching
            && self.button(
                ui,
                &format!("watch-{}", self.members[id].id),
                Rect::from_center_size(video.center(), vec2(128., 38.)),
                p2pshare_media::tr!("Watch screen"),
                true,
            )
        {
            self.intents.push(Intent::Watch {
                peer: self.members[id].id,
                enabled: true,
            });
            self.focus = Some(id);
        }
        if click.double_clicked() && !detached {
            self.focus = Some(id);
            self.fullscreen = true;
            if self.live && id != 0 && !self.watching.contains(&self.members[id].id) {
                self.intents.push(Intent::Watch {
                    peer: self.members[id].id,
                    enabled: true,
                });
            }
        }
        self.stream_footer(
            ui,
            rect(r.left(), r.bottom() - meta, r.width(), meta),
            id,
            small,
        );
        if !window_player {
            ui.painter().rect_stroke(
                r,
                9.,
                Stroke::new(
                    if self.focus == Some(id) { 2. } else { 1. },
                    if self.focus == Some(id) {
                        // A subtle theme-aware focus outline rather than a bright accent.
                        egui::Color32::from_rgb(
                            ((p.border.r() as u16 * 3 + p.accent.r() as u16) / 4) as u8,
                            ((p.border.g() as u16 * 3 + p.accent.g() as u16) / 4) as u8,
                            ((p.border.b() as u16 * 3 + p.accent.b() as u16) / 4) as u8,
                        )
                    } else {
                        p.border
                    },
                ),
                StrokeKind::Inside,
            );
        }
    }
    fn focus_view(&mut self, ui: &mut Ui, r: Rect, id: usize) {
        if self.members[id].scene.is_none() {
            self.focus = None;
            self.grid(ui, r);
            return;
        }
        let others: Vec<_> = self
            .members
            .iter()
            .enumerate()
            .filter(|(i, m)| *i != id && m.scene.is_some())
            .map(|(i, _)| i)
            .collect();
        let strip = if others.is_empty() { 0. } else { 126. };
        self.tile(
            ui,
            rect(r.left(), r.top(), r.width(), r.height() - strip),
            id,
            false,
        );
        if !others.is_empty() {
            let w = ((r.width() - 8. * (others.len() - 1) as f32) / others.len() as f32).min(210.);
            for (j, other) in others.into_iter().enumerate() {
                self.tile(
                    ui,
                    rect(r.left() + j as f32 * (w + 8.), r.bottom() - 114., w, 114.),
                    other,
                    true,
                );
            }
        }
    }
    fn immersive(&mut self, ui: &mut Ui, r: Rect) {
        let id = self.focus.unwrap_or(0);
        if id >= self.members.len() || self.members[id].scene.is_none() {
            self.fullscreen = false;
            self.focus = None;
            return;
        }
        ui.painter().rect_filled(r, 0., Color32::BLACK);
        self.paint_stream(ui, r, id);
        self.fullscreen_controls(ui, r, id);
    }
}

#[cfg(test)]
mod detached_tests {
    use super::*;
    #[test]
    fn main_stops_painting_detached_video_and_restores_on_close() {
        let ctx = egui::Context::default();
        let mut app = App::live(Config::default(), "test".into(), false);
        app.joined = true;
        app.members.push(Member {
            id: 42,
            name: "Remote".into(),
            scene: Some(0),
            texture: Some(egui::TextureId::User(42)),
            dimensions: [1920, 1080],
            ..Default::default()
        });
        app.watching.insert(42);
        let frame = |app: &mut App| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 1280., 832.)),
                    ..Default::default()
                },
                |ui| app.ui(ui),
            );
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape,
                egui::epaint::Shape::Mesh(mesh) if mesh.texture_id == egui::TextureId::User(42))
            })
        };
        assert!(frame(&mut app));
        assert!(app.has_visible_live_video());
        app.detached_peers.insert(42);
        assert!(!frame(&mut app));
        assert!(!app.has_visible_live_video());
        assert!(app.hits.contains_key("show-popout-42"));
        assert!(!app.hits.contains_key("speaker-footer-42"));
        app.detached_peers.remove(&42);
        assert!(frame(&mut app));
        assert!(app.has_visible_live_video());
    }
}

#[cfg(test)]
mod join_status_tests {
    use super::*;

    #[test]
    fn pending_and_retrying_joins_show_status_in_empty_room() {
        let ctx = egui::Context::default();
        let mut app = App::live(Config::default(), "grupo".into(), false);
        for status in [
            "Connecting to Cloudflare signaling…",
            "Could not connect to the signaling server. Retrying…",
            "Room is full. Waiting for a spot…",
        ] {
            app.status = status.into();
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 950., 560.)),
                    ..Default::default()
                },
                |ui| app.ui(ui),
            );
            assert!(output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == status)
            }));
        }
    }
}
