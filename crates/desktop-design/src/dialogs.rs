use crate::config::ThemeUi;
use crate::{
    app::{App, Dialog, Intent, label, rect, text},
    config::{Theme, color},
};
use egui::{Color32, Id, Stroke, StrokeKind, Ui, pos2, vec2};
impl App {
    pub fn draw_dialog(&mut self, ctx: &egui::Context) {
        if self.dialog == Dialog::None {
            return;
        }
        let mode = self.dialog;
        let p = self.palette();
        let caption = if self.fullscreen {
            0.
        } else {
            crate::chrome::HEIGHT
        };
        let viewport = ctx.content_rect();
        let screen = egui::Rect::from_min_max(viewport.min + vec2(0., caption), viewport.max);
        let size = match mode {
            Dialog::Name => vec2(420., 272.),
            Dialog::Share => vec2(
                (screen.width() - 80.).min(920.),
                (screen.height() - 80.).min(660.),
            ),
            Dialog::Appearance => vec2((screen.width() - 60.).min(760.), (screen.height() - 64.).min(520.)),
            Dialog::Crop=>vec2(620.,(screen.height()-80.).min(580.)),
            _ => vec2(430., 244.),
        };
        let response = egui::Modal::new(Id::new("prototype-dialog"))
            .area(
                egui::Modal::default_area(Id::new("prototype-dialog"))
                    .anchor(egui::Align2::CENTER_CENTER, vec2(0., caption / 2.))
                    .fade_in(false),
            )
            .backdrop_color(Color32::from_black_alpha(165))
            .frame(
                egui::Frame::new()
                    .fill(p.side)
                    .stroke(Stroke::new(1., p.border))
                    .corner_radius(14)
                    .inner_margin(20),
            )
            .show(ctx, |ui| {
                let (_, r) = ui.allocate_space(size);
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(r)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                child.set_clip_rect(r);
                match mode {
                    Dialog::Name => self.name_dialog(&mut child, r),
                    Dialog::Share => self.share_dialog(&mut child, r),
                    Dialog::Appearance => self.appearance_dialog(&mut child, r),
                    Dialog::Crop => self.crop_dialog(&mut child, r),
                    Dialog::Room => self.room_dialog(&mut child, r),
                    _ => {}
                }
            });
        self.regions.push(("modal".into(), response.response.rect));
        if mode != Dialog::Name && response.should_close() {
            if mode==Dialog::Crop{self.crop=None;self.dialog=Dialog::Appearance;}else{self.dialog = Dialog::None;}
        }
    }
    fn heading(&mut self, ui: &mut Ui, r: egui::Rect, title: &str, subtitle: &str) {
        let p = self.palette();
        text(ui, r.min, title, 23., p.text);
        text(ui, r.min + vec2(0., 34.), subtitle, 13., p.muted);
        if self.button(
            ui,
            "close-dialog",
            rect(r.right() - 33., r.top(), 33., 30.),
            "X",
            false,
        ) {
            self.dialog = Dialog::None;
        }
    }
    fn name_dialog(&mut self, ui: &mut Ui, r: egui::Rect) {
        let p = self.palette();
        text(ui, r.min, p2pshare_media::tr!("What should we call you?"), 25., p.text);
        text(
            ui,
            r.min + vec2(0., 39.),
            p2pshare_media::tr!("Choose the name people will see in your room."),
            14.,
            p.muted,
        );
        text(ui, r.min + vec2(0., 87.), p2pshare_media::tr!("DISPLAY NAME"), 11., p.muted);
        let field = rect(r.left(), r.top() + 109., r.width(), 40.);
        self.hits.insert("name-input".into(), field);
        let response = ui.put(
            field,
            egui::TextEdit::singleline(&mut self.name_input)
                .vertical_align(egui::Align::Center)
                .id(Id::new("first-name"))
                .hint_text(p2pshare_media::tr!("Your name"))
                .char_limit(32)
                .font(egui::TextStyle::Body),
        );
        self.hits.insert("actual-name-input".into(), response.rect);
        if self.name_input.is_empty() && !ui.input(|i| i.pointer.any_down()) {
            response.request_focus();
        }
        text(
            ui,
            r.min + vec2(0., 163.),
            p2pshare_media::tr!("Saved in config.toml. You can change it later."),
            12.,
            p.muted,
        );
        if !self.notice.is_empty() {
            label(
                ui,
                rect(r.left(), r.top() + 192., r.width(), 25.),
                &p2pshare_media::i18n::status(&self.notice).into_owned(),
                12.,
                p.red,
            );
        }
        let valid =
            !self.name_input.trim().is_empty() && !self.name_input.chars().any(char::is_control);
        let clicked = ui
            .add_enabled_ui(valid, |ui| {
                self.button(
                    ui,
                    "continue",
                    rect(r.left(), r.bottom() - 40., r.width(), 40.),
                    if self.live {
                        p2pshare_media::tr!("Continue")
                    } else {
                        p2pshare_media::tr!("Continue to preview")
                    },
                    true,
                )
            })
            .inner;
        if valid && (clicked || ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            self.config.username = self.name_input.trim().into();
            if self.save() {
                self.dialog = Dialog::None;
            }
        }
    }
    fn share_dialog(&mut self, ui: &mut Ui, r: egui::Rect) {
        self.heading(
            ui,
            r,
            p2pshare_media::tr!("Share your screen"),
            p2pshare_media::tr!("Pick a window, set your quality, and let everyone take a look."),
        );
        let p = self.palette();
        let left_width = r.width() - 312.;
        let left = rect(r.left(), r.top() + 76., left_width, r.height() - 137.);
        let right = rect(r.right() - 286., r.top() + 76., 286., r.height() - 137.);
        ui.painter().vline(
            right.left() - 14.,
            r.top() + 76. ..=r.bottom() - 58.,
            Stroke::new(1., p.border),
        );
        if self.button(
            ui,
            "tab-windows",
            rect(left.left(), left.top(), 131., 32.),
            p2pshare_media::tr!("Applications"),
            !self.screens_tab,
        ) {
            self.screens_tab = false;
        }
        if self.button(
            ui,
            "tab-screens",
            rect(left.left() + 139., left.top(), 109., 32.),
            p2pshare_media::tr!("Displays"),
            self.screens_tab,
        ) {
            self.screens_tab = true;
        }
        let search = rect(left.left(), left.top() + 43., left.width(), 34.);
        self.hits.insert("search".into(), search);
        ui.put(
            search,
            egui::TextEdit::singleline(&mut self.search)
                .vertical_align(egui::Align::Center)
                .hint_text(p2pshare_media::tr!("Find a window..."))
                .char_limit(80),
        );
        let list = rect(
            left.left(),
            left.top() + 90.,
            left.width(),
            left.height() - 90.,
        );
        let matches: Vec<_> = self
            .sources
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.screen == self.screens_tab
                    && format!("{} {}", s.title, s.app)
                        .to_lowercase()
                        .contains(&self.search.to_lowercase())
            })
            .map(|(i, _)| i)
            .collect();
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_width(list.width());
            ui.set_height(list.height());
            egui::ScrollArea::vertical()
                .id_salt("source-scroll")
                .auto_shrink([false, false])
                .max_height(list.height())
                .show(ui, |ui| {
                    let origin = ui.cursor().min;
                    let w = (list.width() - 18.) / 2.;
                    let h = w * 9. / 16. + 43.;
                    let rows = matches.len().div_ceil(2);
                    ui.set_min_size(vec2(
                        list.width() - 8.,
                        (rows as f32 * (h + 10.) - 10.).max(100.),
                    ));
                    if matches.is_empty() {
                        text(
                            ui,
                            origin + vec2(12., 27.),
                            p2pshare_media::tr!("No matching windows"),
                            17.,
                            p.text,
                        );
                        text(
                            ui,
                            origin + vec2(12., 56.),
                            p2pshare_media::tr!("Try another name or switch tabs."),
                            13.,
                            p.muted,
                        );
                    }
                    for (n, &i) in matches.iter().enumerate() {
                        let s = self.sources[i].clone();
                        let card = rect(
                            origin.x + (n % 2) as f32 * (w + 10.),
                            origin.y + (n / 2) as f32 * (h + 10.),
                            w,
                            h,
                        );
                        let selected = self.selected == Some(i);
                        ui.painter().rect_filled(card, 7., p.surface);
                        let preview = rect(
                            card.left() + 4.,
                            card.top() + 4.,
                            w - 8.,
                            (w - 8.) * 9. / 16.,
                        );
                        self.paint_media(ui, preview, s.texture, s.dimensions, s.scene);
                        label(
                            ui,
                            rect(card.left() + 10., card.bottom() - 37., w - 20., 19.),
                            &s.title,
                            13.,
                            p.text,
                        );
                        label(
                            ui,
                            rect(card.left() + 10., card.bottom() - 19., w - 20., 16.),
                            &s.app,
                            11.,
                            p.muted,
                        );
                        ui.painter().rect_stroke(
                            card,
                            7.,
                            Stroke::new(
                                if selected { 2. } else { 1. },
                                if selected { p.accent } else { p.border },
                            ),
                            StrokeKind::Inside,
                        );
                        let response =
                            ui.interact(card, Id::new(("source", i)), egui::Sense::click());
                        self.hits
                            .insert(format!("source-{i}"), card.intersect(list));
                        if response.clicked() {
                            self.selected = Some(i);
                        }
                        if response.hovered() {
                            response.on_hover_text(format!("{} — {}", s.title, s.app));
                        }
                        let paint_badge = |text_str: &str, align_right: bool| {
                            let galley = ui.painter().layout_no_wrap(
                                text_str.to_owned(),
                                egui::FontId::proportional(11.),
                                p.accent_text,
                            );
                            let bw = (galley.size().x + 14.).max(44.);
                            let bx = if align_right {
                                preview.right() - 6. - bw
                            } else {
                                preview.left() + 6.
                            };
                            let badge = rect(bx, preview.top() + 6., bw, 22.);
                            ui.painter().rect_filled(badge, 5., p.accent);
                            ui.painter().galley(
                                pos2(
                                    badge.center().x - galley.size().x / 2.,
                                    badge.center().y - galley.size().y / 2.,
                                ),
                                galley,
                                p.accent_text,
                            );
                        };
                        if s.grouped {
                            paint_badge(p2pshare_media::tr!("Group"), false);
                        }
                        if selected {
                            paint_badge(p2pshare_media::tr!("Selected"), true);
                        }
                    }
                });
        });
        let x = right.left();
        let y = right.top();
        let w = right.width();
        text(ui, pos2(x, y), p2pshare_media::tr!("RESOLUTION"), 11., p.muted);
        let rr = rect(x, y + 20., w, 32.);
        self.hits.insert("resolution".into(), rr);
        let resolution_text = format!("{}p", self.draft.stream.height);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rr), |ui| {
            egui::ComboBox::from_id_salt("resolution")
                .width(w)
                .selected_text("")
                .show_ui(ui, |ui| {
                    for (ww, hh) in [
                        (854, 480),
                        (1280, 720),
                        (1920, 1080),
                        (2560, 1440),
                        (3840, 2160),
                    ] {
                        let response =
                            ui.selectable_label(self.draft.stream.height == hh, format!("{hh}p"));
                        self.hits.insert(format!("resolution-{hh}"), response.rect);
                        if response.clicked() {
                            self.draft.stream.width = ww;
                            self.draft.stream.height = hh;
                        }
                    }
                });
        });
        ui.painter().text(
            rr.center(),
            egui::Align2::CENTER_CENTER,
            resolution_text,
            egui::FontId::proportional(14.),
            p.text,
        );
        text(ui, pos2(x, y + 56.), p2pshare_media::tr!("FRAME RATE"), 11., p.muted);
        let fr = rect(x, y + 76., w, 32.);
        self.hits.insert("fps".into(), fr);
        let fps_text = format!("{} FPS", self.draft.stream.fps);
        ui.scope_builder(egui::UiBuilder::new().max_rect(fr), |ui| {
            egui::ComboBox::from_id_salt("fps")
                .width(w)
                .selected_text("")
                .show_ui(ui, |ui| {
                    for fps in [24, 30, 60, 90, 120] {
                        let res = ui.selectable_value(
                            &mut self.draft.stream.fps,
                            fps,
                            format!("{fps} FPS"),
                        );
                        self.hits.insert(format!("fps-{fps}"), res.rect);
                    }
                });
        });
        ui.painter().text(
            fr.center(),
            egui::Align2::CENTER_CENTER,
            fps_text,
            egui::FontId::proportional(14.),
            p.text,
        );
        text(ui, pos2(x, y + 112.), p2pshare_media::tr!("BITRATE"), 11., p.muted);
        let br = rect(x, y + 132., w, 32.);
        self.hits.insert("bitrate".into(), br);
        ui.put(
            br,
            egui::DragValue::new(&mut self.draft.stream.bitrate_kbps)
                .range(100..=50000)
                .speed(250)
                .suffix(" kbps"),
        );
        text(ui, pos2(x, y + 168.), p2pshare_media::tr!("CODEC"), 11., p.muted);
        let cr = rect(x, y + 188., w, 32.);
        ui.scope_builder(egui::UiBuilder::new().max_rect(cr), |ui| {
            egui::ComboBox::from_id_salt("codec")
                .width(w)
                .selected_text(match self.draft.stream.codec.as_str() {
                    "auto" => p2pshare_media::tr!("Auto · HEVC preferred"),
                    "hevc" => p2pshare_media::tr!("HEVC · Efficient"),
                    "avc" => p2pshare_media::tr!("AVC · Compatible"),
                    _ => "AVC",
                })
                .show_ui(ui, |ui| {
                    for (value, title) in [
                        ("avc", p2pshare_media::tr!("AVC · Compatible")),
                        ("hevc", p2pshare_media::tr!("HEVC · Efficient")),
                        ("auto", p2pshare_media::tr!("Auto · HEVC preferred")),
                    ] {
                        ui.selectable_value(&mut self.draft.stream.codec, value.into(), title);
                    }
                });
        });
        text(ui, pos2(x, y + 224.), p2pshare_media::tr!("PREVIEW REFRESH"), 11., p.muted);
        let pr = rect(x, y + 244., w, 32.);
        self.hits.insert("preview-interval".into(), pr);
        ui.put(pr, egui::DragValue::new(&mut self.draft.stream.preview_interval_secs).range(10..=60).suffix(p2pshare_media::tr!(" seconds")).speed(1))
            .on_hover_text(p2pshare_media::tr!("JPEG preview every 10–60 seconds. Maximum width and height: 512 pixels. Viewers choose when to watch live video."));
        text(ui, pos2(x, y + 280.), p2pshare_media::tr!("PLAYBACK BUFFER"), 11., p.muted);
        let dr = rect(x, y + 300., w - 80., 32.);
        self.hits.insert("delay".into(), dr);
        ui.put(
            dr,
            egui::DragValue::new(&mut self.draft.playback.buffer_ms)
                .range(0..=1000)
                .speed(10)
                .suffix(" ms"),
        );
        if self.button(
            ui,
            "delay-reset",
            rect(x + w - 72., y + 300., 72., 32.),
            p2pshare_media::tr!("Reset"),
            false,
        ) {
            self.draft.playback.buffer_ms = 0;
        }
        text(
            ui,
            pos2(x, y + 339.),
            p2pshare_media::tr!("0–1000 ms · affects what this device watches."),
            11.,
            p.muted,
        );
        text(
            ui,
            pos2(x, y + 356.),
            p2pshare_media::tr!("Adds receive delay to absorb brief stutters."),
            11.,
            p.muted,
        );
        let audio = rect(x, y + 382., w, 27.);
        self.hits.insert("audio".into(), audio);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(audio)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x=10.;
                ui.add(egui::Checkbox::new(&mut self.draft.stream.audio,p2pshare_media::tr!("Share audio")));
                let cursor=ui.add(egui::Checkbox::new(&mut self.draft.stream.show_cursor,p2pshare_media::tr!("Show mouse pointer")));
                self.hits.insert("show-cursor".into(),cursor.rect);
            },
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect(x,y+420.,w,32.)).layout(egui::Layout::left_to_right(egui::Align::Center)),|ui|{
            let filter=ui.add(egui::Checkbox::new(&mut self.draft.stream.exclude_audio_processes,p2pshare_media::tr!("Exclude app audio")))
                .on_hover_text(p2pshare_media::tr!("Whole-display audio only. Excluded executable names are configured in config.toml."));
            self.hits.insert("exclude-app-audio".into(),filter.rect);
        });
        ui.painter().hline(
            r.left()..=r.right(),
            r.bottom() - 51.,
            Stroke::new(1., p.border),
        );
        let selected = self
            .selected
            .map(|i| self.sources[i].title.as_ref())
            .unwrap_or(p2pshare_media::tr!("Choose a window to continue"));
        label(
            ui,
            rect(r.left(), r.bottom() - 37., r.width() - 300., 27.),
            selected,
            13.,
            p.muted,
        );
        if self.button(
            ui,
            "cancel-share",
            rect(r.right() - 272., r.bottom() - 36., 90., 36.),
            p2pshare_media::tr!("Cancel"),
            false,
        ) {
            self.dialog = Dialog::None;
        }
        let valid = self.settings_only || (self.selected.is_some() && self.joined);
        let clicked = ui
            .add_enabled_ui(valid, |ui| {
                self.button(
                    ui,
                    "start-share",
                    rect(r.right() - 172., r.bottom() - 36., 172., 36.),
                    if self.settings_only {
                        p2pshare_media::tr!("Save settings")
                    } else {
                        p2pshare_media::tr!("Share selected screen")
                    },
                    true,
                )
            })
            .inner;
        if clicked && valid {
            let old = self.config.clone();
            self.config.stream = self.draft.stream.clone();
            self.config.playback = self.draft.playback.clone();
            if self.save() {
                if (!self.settings_only || self.members[0].scene.is_some())
                    && let Some(id) = self.selected
                {
                    if self.live {
                        self.intents.push(Intent::Start(self.sources[id].id));
                    } else {
                        self.members[0].scene = Some(self.sources[id].scene);
                        self.members[0].title = self.sources[id].title.to_string();
                        self.joined = true;
                    }
                    self.focus = Some(0);
                }
                self.dialog = Dialog::None;
            } else {
                self.config = old;
            }
        }
    }
    fn appearance_dialog(&mut self, ui: &mut Ui, r: egui::Rect) {
        if self.button(
            ui,
            "close-dialog",
            rect(r.right() - 33., r.top(), 33., 30.),
            "X",
            false,
        ) {
            self.dialog = Dialog::None;
        }
        use crate::diagnostics::Tab;
        use p2pshare_media::tr;
        let p = self.palette();
        label(ui, rect(r.left(), r.top(), r.width() - 50., 28.), tr!("Settings"), 20., p.text);
        for (index, (tab, title)) in [
            (Tab::Profile, tr!("Profile")),
            (Tab::Appearance, tr!("Appearance")),
            (Tab::ShareGroups, tr!("Share groups")),
            (Tab::Diagnostics, tr!("Diagnostics")),
        ].into_iter().enumerate() {
            if self.button(ui, title, rect(r.left(), r.top() + 48. + index as f32 * 39., 136., 34.), title, self.settings_tab == tab) {
                self.settings_tab = tab;
            }
        }
        let x = r.left() + 150.;
        ui.painter().vline(x, (r.top() + 42.)..=r.bottom(), Stroke::new(1., p.border));
        let r = egui::Rect::from_min_max(pos2(x + 16., r.top() + 44.), r.max);
        if self.settings_tab == Tab::Diagnostics {
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| self.diagnostics_panel(ui));
            return;
        }
        ui.painter().hline(r.left()..=r.right(), r.bottom() - 48., Stroke::new(1., p.border));
        let bottom_pad = if self.notice.is_empty() { 56. } else { 76. };
        if self.settings_tab == Tab::Profile {
            self.profile_row(ui, rect(r.left(), r.top(), r.width(), 110.));
            text(ui, r.min + vec2(0., 122.), tr!("DISPLAY NAME"), 11., p.muted);
            let field = rect(r.left(), r.top() + 143., r.width(), 34.);
            self.hits.insert("profile-name".into(), field);
            ui.put(field, egui::TextEdit::singleline(&mut self.name_input)
                .vertical_align(egui::Align::Center).char_limit(32));
            text(ui, r.min + vec2(0., 192.), tr!("PROCESS PRIORITY"), 11., p.muted);
            let prio_rect = rect(r.left(), r.top() + 212., 250., 32.);
            self.hits.insert("process-priority".into(), prio_rect);
            let priority_label = match self.draft.process_priority {
                p2pshare_media::config::ProcessPriority::AboveNormal => tr!("Above normal (recommended)"),
                p2pshare_media::config::ProcessPriority::Normal => tr!("Normal"),
            };
            ui.scope_builder(egui::UiBuilder::new().max_rect(prio_rect), |ui| {
                egui::ComboBox::from_id_salt("settings-process-priority")
                    .width(prio_rect.width())
                    .selected_text(priority_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.draft.process_priority,
                            p2pshare_media::config::ProcessPriority::AboveNormal,
                            tr!("Above normal (recommended)"),
                        );
                        ui.selectable_value(
                            &mut self.draft.process_priority,
                            p2pshare_media::config::ProcessPriority::Normal,
                            tr!("Normal"),
                        );
                    });
            });
        } else if self.settings_tab == Tab::ShareGroups {
            let content = egui::Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - bottom_pad));
            self.share_groups_panel(ui, content);
        } else {
            let content = egui::Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - bottom_pad));
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(content));
                egui::ScrollArea::vertical().max_height(content.height()).show(ui, |ui| {
                    let (_, panel) = ui.allocate_space(vec2((content.width() - 18.).max(0.), 358.));
                    self.appearance_panel(ui, panel);
                });
            });
        }
        if !self.notice.is_empty() {
            label(
                ui,
                rect(r.left(), r.bottom() - 72., r.width(), 20.),
                &p2pshare_media::i18n::status(&self.notice).into_owned(),
                12.,
                p.red,
            );
        }
        if self.live
            && self.settings_tab == Tab::Profile
            && self.button(
                ui,
                "firewall-setup",
                rect(r.left(), r.bottom() - 37., 180., 37.),
                p2pshare_media::tr!("Windows Firewall"),
                false,
            )
        {
            self.intents.push(Intent::Firewall);
        }
        if self.settings_tab == Tab::ShareGroups
            && self.button(
                ui,
                "share-group-reset",
                rect(r.left(), r.bottom() - 37., 164., 37.),
                tr!("Reset defaults"),
                false,
            )
        {
            self.draft.share_groups = p2pshare_media::config::default_share_groups();
            self.share_group_selected = 0;
            self.share_group_focus_name = false;
            self.notice.clear();
        }
        if self.button(
            ui,
            "cancel-theme",
            rect(r.right() - 201., r.bottom() - 37., 98., 37.),
            p2pshare_media::tr!("Cancel"),
            false,
        ) {
            self.dialog = Dialog::None;
        }
        let valid =
            !self.name_input.trim().is_empty() && !self.name_input.chars().any(char::is_control);
        let save = ui
            .add_enabled_ui(valid, |ui| {
                self.button(
                    ui,
                    "save-theme",
                    rect(r.right() - 93., r.bottom() - 37., 93., 37.),
                    p2pshare_media::tr!("Save"),
                    true,
                )
            })
            .inner;
        if save {
            let previous = self.config.clone();
            self.config.theme = self.draft.theme.clone();
            self.config.language = self.draft.language;
            self.config.username = self.name_input.trim().into();
            self.config.process_priority = self.draft.process_priority;
            self.config.share_groups = self.draft.share_groups.clone();
            for group in &mut self.config.share_groups {
                group.name = group.name.trim().into();
            }
            if self.save() {
                if let Some(avatar)=self.pending_avatar.take(){self.intents.push(Intent::SaveAvatar(avatar));}
                self.dialog = Dialog::None;
            } else {
                self.config = previous;
            }
        }
    }
    fn add_share_group_exe(&mut self, group_idx: usize, raw: &str) {
        let mut candidate = raw.trim().to_owned();
        if candidate.is_empty() {
            return;
        }
        if !candidate.contains('.') {
            candidate.push_str(".exe");
        }
        if !p2pshare_media::config::valid_process_exe_name(&candidate) {
            self.notice = "Share group processes must be executable filenames ending in .exe".into();
            return;
        }
        let Some(group) = self.draft.share_groups.get_mut(group_idx) else {
            return;
        };
        if group.processes.len() >= 16 {
            self.notice = "Each share group must contain 1..16 executable filenames".into();
            return;
        }
        if !group
            .processes
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&candidate))
        {
            group.processes.push(candidate);
        }
        self.notice.clear();
    }
    fn add_empty_share_group(&mut self) {
        let next = self.draft.share_groups.len() + 1;
        self.draft.share_groups.push(p2pshare_media::config::ShareGroup {
            name: format!("{} {next}", p2pshare_media::tr!("Group")),
            merge_in_picker: true,
            processes: Vec::new(),
        });
        self.share_group_selected = self.draft.share_groups.len() - 1;
        self.share_group_focus_name = true;
        self.notice.clear();
    }
    fn share_groups_panel(&mut self, ui: &mut Ui, content: egui::Rect) {
        use p2pshare_media::tr;
        let p = self.palette();
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(content));

            // 1. Top bar: scrollable group tabs on the left, pinned "+ New group" on the right.
            let top_rect = rect(content.left(), content.top(), content.width(), 34.);
            let tabs_w = (content.width() - 132.).max(120.);
            let tabs_rect = rect(top_rect.left(), top_rect.top() + 2., tabs_w, 30.);
            ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(tabs_rect));
                egui::ScrollArea::horizontal()
                    .id_salt("share-group-tabs-scroll")
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.;
                            let len = self.draft.share_groups.len();
                            for i in 0..len {
                                let active = self.share_group_selected == i;
                                let title = self.draft.share_groups[i].name.clone();
                                let label_text = if title.trim().is_empty() {
                                    tr!("Group name")
                                } else {
                                    title.as_str()
                                };
                                let btn = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(label_text)
                                            .size(12.5)
                                            .color(if active { p.accent_text } else { p.text }),
                                    )
                                    .min_size(vec2(0., 30.))
                                    .fill(if active { p.accent } else { p.surface })
                                    .stroke(Stroke::new(
                                        1.,
                                        if active { p.accent } else { p.border },
                                    ))
                                    .corner_radius(7),
                                );
                                self.hits.insert(format!("share-group-tab-{i}"), btn.rect);
                                if btn.clicked() {
                                    self.share_group_selected = i;
                                    self.share_group_focus_name = false;
                                    self.notice.clear();
                                }
                            }
                        });
                    });
            });

            let can_add_group = self.draft.share_groups.len() < 16;
            let new_btn_rect = rect(top_rect.right() - 122., top_rect.top() + 2., 122., 30.);
            let add_group = ui
                .add_enabled_ui(can_add_group, |ui| {
                    let hovered = ui.rect_contains_pointer(new_btn_rect) && can_add_group;
                    let accent_border =
                        Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 150);
                    let resp = ui.interact(
                        new_btn_rect,
                        Id::new("share-group-new-btn"),
                        egui::Sense::click(),
                    );
                    ui.painter().rect_filled(
                        new_btn_rect,
                        7.,
                        if hovered { p.hover } else { p.surface },
                    );
                    ui.painter().rect_stroke(
                        new_btn_rect,
                        7.,
                        Stroke::new(1., if can_add_group { accent_border } else { p.border }),
                        StrokeKind::Inside,
                    );
                    let g = ui.painter().layout_no_wrap(
                        tr!("+ New group").into(),
                        egui::FontId::proportional(12.5),
                        if can_add_group { p.text } else { p.muted },
                    );
                    ui.painter().galley(
                        pos2(
                            new_btn_rect.center().x - g.size().x / 2.,
                            new_btn_rect.center().y - g.size().y / 2.,
                        ),
                        g,
                        if can_add_group { p.text } else { p.muted },
                    );
                    resp
                })
                .inner;
            self.hits.insert("share-group-new".into(), new_btn_rect);
            if add_group.clicked() && can_add_group {
                self.add_empty_share_group();
            }

            let body_top = top_rect.bottom() + 8.;
            let body_max_rect = egui::Rect::from_min_max(
                pos2(content.left(), body_top),
                pos2(content.right(), content.bottom().max(body_top + 60.)),
            );

            // 2. Empty groups state.
            if self.draft.share_groups.is_empty() {
                let empty_rect = rect(
                    body_max_rect.left(),
                    body_max_rect.top(),
                    content.width(),
                    164.,
                );
                ui.painter().rect_filled(empty_rect, 10., p.bg);
                ui.painter().rect_stroke(
                    empty_rect,
                    10.,
                    Stroke::new(1., p.border),
                    StrokeKind::Inside,
                );
                ui.painter().text(
                    pos2(empty_rect.center().x, empty_rect.top() + 44.),
                    egui::Align2::CENTER_CENTER,
                    tr!("No share groups yet"),
                    egui::FontId::proportional(15.),
                    p.text,
                );
                let sub_galley = ui.painter().layout(
                    tr!("Create a group to switch video and audio automatically between a launcher and a game.").into(),
                    egui::FontId::proportional(12.5),
                    p.muted,
                    (empty_rect.width() - 64.).max(140.),
                );
                ui.painter().galley(
                    pos2(
                        empty_rect.center().x - sub_galley.size().x / 2.,
                        empty_rect.top() + 66.,
                    ),
                    sub_galley,
                    p.muted,
                );
                let cta_rect = rect(
                    empty_rect.center().x - 76.,
                    empty_rect.bottom() - 48.,
                    152.,
                    32.,
                );
                let cta = ui.put(
                    cta_rect,
                    egui::Button::new(
                        egui::RichText::new(tr!("+ New group"))
                            .size(13.)
                            .color(p.accent_text),
                    )
                    .fill(p.accent)
                    .stroke(Stroke::new(1., p.accent))
                    .corner_radius(7),
                );
                if cta.clicked() {
                    self.add_empty_share_group();
                }
                return;
            }

            let idx = self
                .share_group_selected
                .min(self.draft.share_groups.len() - 1);
            self.share_group_selected = idx;

            // 3. Active Group Editor Card (flush-aligned to content.width(), hugs content height).
            let mut delete_group = false;
            let mut swap_pair: Option<(usize, usize)> = None;
            let mut remove_proc: Option<usize> = None;
            let mut selected_running: Option<String> = None;
            let mut submit_manual = false;

            let inner_w = (content.width() - 30.).max(200.);
            ui.scope_builder(egui::UiBuilder::new().max_rect(body_max_rect), |ui| {
                egui::Frame::new()
                    .fill(p.bg)
                    .stroke(Stroke::new(1., p.border))
                    .corner_radius(10)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.set_width(inner_w);
                        ui.spacing_mut().item_spacing = vec2(8., 0.);

                        // Section A: Group name + Delete button + Merge checkbox.
                        ui.label(egui::RichText::new(tr!("GROUP NAME")).size(11.).color(p.muted));
                        ui.add_space(4.);
                        let (name_row, _) =
                            ui.allocate_exact_size(vec2(inner_w, 32.), egui::Sense::hover());
                        let del_w = 82.;
                        let name_rect = rect(
                            name_row.left(),
                            name_row.top(),
                            (inner_w - del_w - 8.).max(120.),
                            32.,
                        );
                        let del_rect = rect(name_row.right() - del_w, name_row.top(), del_w, 32.);
                        let name_edit = styled_text_edit(
                            ui,
                            name_rect,
                            ("share-group-name", idx),
                            &mut self.draft.share_groups[idx].name,
                            48,
                            tr!("Group name"),
                            p,
                        );
                        if self.share_group_focus_name {
                            name_edit.request_focus();
                            self.share_group_focus_name = false;
                        }
                        let del_hovered = ui.rect_contains_pointer(del_rect);
                        let del_btn = ui.interact(
                            del_rect,
                            Id::new(("share-group-delete-btn", idx)),
                            egui::Sense::click(),
                        );
                        ui.painter().rect_filled(
                            del_rect,
                            6.,
                            if del_hovered {
                                Color32::from_rgba_unmultiplied(p.red.r(), p.red.g(), p.red.b(), 28)
                            } else {
                                p.surface
                            },
                        );
                        ui.painter().rect_stroke(
                            del_rect,
                            6.,
                            Stroke::new(
                                1.,
                                if del_hovered {
                                    Color32::from_rgba_unmultiplied(p.red.r(), p.red.g(), p.red.b(), 150)
                                } else {
                                    p.border
                                },
                            ),
                            StrokeKind::Inside,
                        );
                        let del_galley = ui.painter().layout_no_wrap(
                            tr!("Delete").into(),
                            egui::FontId::proportional(13.),
                            p.red,
                        );
                        ui.painter().galley(
                            pos2(
                                del_rect.center().x - del_galley.size().x / 2.,
                                del_rect.center().y - del_galley.size().y / 2.,
                            ),
                            del_galley,
                            p.red,
                        );
                        self.hits.insert("share-group-delete".into(), del_rect);
                        if del_btn.clicked() {
                            delete_group = true;
                        }

                        ui.add_space(7.);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.;
                            let merge = ui.add(egui::Checkbox::new(
                                &mut self.draft.share_groups[idx].merge_in_picker,
                                tr!("Merge in picker"),
                            ));
                            self.hits.insert("share-group-merge".into(), merge.rect);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(tr!(
                                        "Show as one card in the screen selector"
                                    ))
                                    .size(11.5)
                                    .color(p.muted),
                                )
                                .truncate(),
                            );
                        });

                        ui.add_space(9.);
                        let (sep, _) =
                            ui.allocate_exact_size(vec2(inner_w, 1.), egui::Sense::hover());
                        ui.painter().hline(
                            sep.left()..=sep.right(),
                            sep.center().y,
                            Stroke::new(1., p.border),
                        );
                        ui.add_space(9.);

                        // Section B: Priority order list (compact auto-shrink, scrollable at 4+ apps).
                        let (hdr, _) =
                            ui.allocate_exact_size(vec2(inner_w, 16.), egui::Sense::hover());
                        ui.painter().text(
                            hdr.left_center(),
                            egui::Align2::LEFT_CENTER,
                            tr!("PRIORITY ORDER"),
                            egui::FontId::proportional(11.),
                            p.muted,
                        );
                        ui.add_space(5.);

                        // Reserve height inside the card for Add App (58px) + bottom padding (14px).
                        let list_max_h = (body_max_rect.bottom() - ui.cursor().top() - 74.).max(72.);
                        egui::ScrollArea::vertical()
                            .id_salt("share-group-processes-scroll")
                            .max_height(list_max_h)
                            .auto_shrink([false, true])
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                            )
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.;
                                let row_w = ui.available_width().min(inner_w).max(160.);
                                let procs = &self.draft.share_groups[idx].processes;
                                if procs.is_empty() {
                                    let (empty_row, _) = ui.allocate_exact_size(
                                        vec2(row_w, 40.),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(empty_row, 7., p.surface);
                                    ui.painter().rect_stroke(
                                        empty_row,
                                        7.,
                                        Stroke::new(1., p.border),
                                        StrokeKind::Inside,
                                    );
                                    ui.painter().text(
                                        pos2(empty_row.left() + 12., empty_row.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        tr!("No apps in this group yet — pick a running app or type an .exe below."),
                                        egui::FontId::proportional(12.5),
                                        p.muted,
                                    );
                                }
                                let count = procs.len();
                                for (pi, proc_name) in procs.iter().enumerate() {
                                    let (row_rect, _) = ui.allocate_exact_size(
                                        vec2(row_w, 38.),
                                        egui::Sense::hover(),
                                    );
                                    let is_primary = pi == 0;
                                    let primary_stroke = Color32::from_rgba_unmultiplied(
                                        p.green.r(),
                                        p.green.g(),
                                        p.green.b(),
                                        95,
                                    );
                                    ui.painter().rect_filled(row_rect, 7., p.surface);
                                    ui.painter().rect_stroke(
                                        row_rect,
                                        7.,
                                        Stroke::new(
                                            1.,
                                            if is_primary { primary_stroke } else { p.border },
                                        ),
                                        StrokeKind::Inside,
                                    );

                                    // Rank pill.
                                    let rank_rect = rect(
                                        row_rect.left() + 8.,
                                        row_rect.center().y - 11.,
                                        22.,
                                        22.,
                                    );
                                    ui.painter().rect_filled(
                                        rank_rect,
                                        5.,
                                        if is_primary {
                                            Color32::from_rgba_unmultiplied(
                                                p.green.r(),
                                                p.green.g(),
                                                p.green.b(),
                                                32,
                                            )
                                        } else {
                                            p.bg
                                        },
                                    );
                                    ui.painter().rect_stroke(
                                        rank_rect,
                                        5.,
                                        Stroke::new(
                                            1.,
                                            if is_primary {
                                                Color32::from_rgba_unmultiplied(
                                                    p.green.r(),
                                                    p.green.g(),
                                                    p.green.b(),
                                                    110,
                                                )
                                            } else {
                                                p.border
                                            },
                                        ),
                                        StrokeKind::Inside,
                                    );
                                    ui.painter().text(
                                        rank_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        format!("{}", pi + 1),
                                        egui::FontId::proportional(11.5),
                                        if is_primary { p.green } else { p.muted },
                                    );

                                    // Right action buttons: Up, Down, Remove.
                                    let del_rect = rect(
                                        row_rect.right() - 8. - 26.,
                                        row_rect.center().y - 13.,
                                        26.,
                                        26.,
                                    );
                                    let down_rect = rect(
                                        del_rect.left() - 4. - 26.,
                                        del_rect.top(),
                                        26.,
                                        26.,
                                    );
                                    let up_rect = rect(
                                        down_rect.left() - 4. - 26.,
                                        down_rect.top(),
                                        26.,
                                        26.,
                                    );

                                    let up_enabled = pi > 0;
                                    let up_hovered = up_enabled && ui.rect_contains_pointer(up_rect);
                                    let up_resp = ui.interact(
                                        up_rect,
                                        Id::new(("share-group-up", idx, pi)),
                                        if up_enabled {
                                            egui::Sense::click()
                                        } else {
                                            egui::Sense::hover()
                                        },
                                    );
                                    ui.painter().rect_filled(
                                        up_rect,
                                        5.,
                                        if !up_enabled {
                                            Color32::TRANSPARENT
                                        } else if up_hovered {
                                            p.hover
                                        } else {
                                            p.bg
                                        },
                                    );
                                    ui.painter().rect_stroke(
                                        up_rect,
                                        5.,
                                        Stroke::new(
                                            1.,
                                            if up_enabled {
                                                p.border
                                            } else {
                                                p.border.gamma_multiply(0.45)
                                            },
                                        ),
                                        StrokeKind::Inside,
                                    );
                                    crate::icons::arrow_up(
                                        ui,
                                        up_rect.shrink(5.),
                                        if up_enabled {
                                            p.text
                                        } else {
                                            p.muted.gamma_multiply(0.45)
                                        },
                                    );
                                    self.hits.insert(format!("share-group-up-{pi}"), up_rect);
                                    if up_enabled {
                                        if up_resp.on_hover_text(tr!("Move up")).clicked() {
                                            swap_pair = Some((pi - 1, pi));
                                        }
                                    }

                                    let down_enabled = pi + 1 < count;
                                    let down_hovered =
                                        down_enabled && ui.rect_contains_pointer(down_rect);
                                    let down_resp = ui.interact(
                                        down_rect,
                                        Id::new(("share-group-down", idx, pi)),
                                        if down_enabled {
                                            egui::Sense::click()
                                        } else {
                                            egui::Sense::hover()
                                        },
                                    );
                                    ui.painter().rect_filled(
                                        down_rect,
                                        5.,
                                        if !down_enabled {
                                            Color32::TRANSPARENT
                                        } else if down_hovered {
                                            p.hover
                                        } else {
                                            p.bg
                                        },
                                    );
                                    ui.painter().rect_stroke(
                                        down_rect,
                                        5.,
                                        Stroke::new(
                                            1.,
                                            if down_enabled {
                                                p.border
                                            } else {
                                                p.border.gamma_multiply(0.45)
                                            },
                                        ),
                                        StrokeKind::Inside,
                                    );
                                    crate::icons::arrow_down(
                                        ui,
                                        down_rect.shrink(5.),
                                        if down_enabled {
                                            p.text
                                        } else {
                                            p.muted.gamma_multiply(0.45)
                                        },
                                    );
                                    self.hits
                                        .insert(format!("share-group-down-{pi}"), down_rect);
                                    if down_enabled {
                                        if down_resp.on_hover_text(tr!("Move down")).clicked() {
                                            swap_pair = Some((pi, pi + 1));
                                        }
                                    }

                                    let del_hovered = ui.rect_contains_pointer(del_rect);
                                    let del_resp = ui.interact(
                                        del_rect,
                                        Id::new(("share-group-del", idx, pi)),
                                        egui::Sense::click(),
                                    );
                                    ui.painter().rect_filled(
                                        del_rect,
                                        5.,
                                        if del_hovered {
                                            Color32::from_rgba_unmultiplied(
                                                p.red.r(),
                                                p.red.g(),
                                                p.red.b(),
                                                30,
                                            )
                                        } else {
                                            p.bg
                                        },
                                    );
                                    ui.painter().rect_stroke(
                                        del_rect,
                                        5.,
                                        Stroke::new(
                                            1.,
                                            if del_hovered {
                                                Color32::from_rgba_unmultiplied(
                                                    p.red.r(),
                                                    p.red.g(),
                                                    p.red.b(),
                                                    150,
                                                )
                                            } else {
                                                p.border
                                            },
                                        ),
                                        StrokeKind::Inside,
                                    );
                                    crate::icons::close_x(
                                        ui,
                                        del_rect.shrink(6.),
                                        if del_hovered { p.red } else { p.muted },
                                    );
                                    self.hits.insert(format!("share-group-del-{pi}"), del_rect);
                                    if del_resp.on_hover_text(tr!("Remove")).clicked() {
                                        remove_proc = Some(pi);
                                    }

                                    // Middle area: .exe name + Primary pill + live Open status indicator.
                                    let text_left = rank_rect.right() + 10.;
                                    let right_limit = up_rect.left() - 10.;
                                    let running_match = self
                                        .running_apps
                                        .iter()
                                        .find(|app| app.exe.eq_ignore_ascii_case(proc_name));
                                    let primary_galley = is_primary.then(|| {
                                        ui.painter().layout_no_wrap(
                                            tr!("Primary").into(),
                                            egui::FontId::proportional(11.),
                                            p.green,
                                        )
                                    });
                                    let open_galley = running_match.map(|_| {
                                        ui.painter().layout_no_wrap(
                                            tr!("Open").into(),
                                            egui::FontId::proportional(11.),
                                            p.muted,
                                        )
                                    });
                                    let primary_w = primary_galley
                                        .as_ref()
                                        .map_or(0., |g| g.size().x + 14.);
                                    let open_w = open_galley
                                        .as_ref()
                                        .map_or(0., |g| 11. + g.size().x + 2.);
                                    let badges_reserve = (if primary_galley.is_some() {
                                        8. + primary_w
                                    } else {
                                        0.
                                    }) + (if open_galley.is_some() {
                                        8. + open_w
                                    } else {
                                        0.
                                    });
                                    let max_name_w =
                                        (right_limit - text_left - badges_reserve).max(60.);
                                    let name_area =
                                        rect(text_left, row_rect.top(), max_name_w, row_rect.height());
                                    let name_resp = ui
                                        .scope_builder(
                                            egui::UiBuilder::new()
                                                .max_rect(name_area)
                                                .layout(egui::Layout::left_to_right(
                                                    egui::Align::Center,
                                                )),
                                            |ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(proc_name)
                                                            .size(13.5)
                                                            .color(p.text),
                                                    )
                                                    .truncate(),
                                                )
                                            },
                                        )
                                        .inner;
                                    let mut next_x =
                                        name_resp.rect.right().min(text_left + max_name_w) + 8.;
                                    if let Some(pg) = primary_galley {
                                        let pill = rect(
                                            next_x,
                                            row_rect.center().y - 10.,
                                            primary_w,
                                            20.,
                                        );
                                        ui.painter().rect_filled(
                                            pill,
                                            10.,
                                            Color32::from_rgba_unmultiplied(
                                                p.green.r(),
                                                p.green.g(),
                                                p.green.b(),
                                                28,
                                            ),
                                        );
                                        ui.painter().rect_stroke(
                                            pill,
                                            10.,
                                            Stroke::new(
                                                1.,
                                                Color32::from_rgba_unmultiplied(
                                                    p.green.r(),
                                                    p.green.g(),
                                                    p.green.b(),
                                                    95,
                                                ),
                                            ),
                                            StrokeKind::Inside,
                                        );
                                        ui.painter().galley(
                                            pos2(
                                                pill.center().x - pg.size().x / 2.,
                                                pill.center().y - pg.size().y / 2.,
                                            ),
                                            pg,
                                            p.green,
                                        );
                                        next_x = pill.right() + 8.;
                                    }
                                    if let (Some(og), Some(app)) = (open_galley, running_match) {
                                        let open_rect = rect(
                                            next_x,
                                            row_rect.center().y - 10.,
                                            open_w,
                                            20.,
                                        );
                                        ui.painter().circle_filled(
                                            pos2(open_rect.left() + 3., open_rect.center().y),
                                            3.,
                                            p.green,
                                        );
                                        ui.painter().galley(
                                            pos2(
                                                open_rect.left() + 11.,
                                                open_rect.center().y - og.size().y / 2.,
                                            ),
                                            og,
                                            p.muted,
                                        );
                                        if !app.title.trim().is_empty() {
                                            ui.interact(
                                                open_rect,
                                                Id::new(("share-group-open", idx, pi)),
                                                egui::Sense::hover(),
                                            )
                                            .on_hover_text(&app.title);
                                        }
                                    }
                                }
                            });

                        // Section C: Add App toolbar (running app ComboBox + manual .exe input + Add).
                        ui.add_space(9.);
                        ui.label(egui::RichText::new(tr!("ADD APP")).size(11.).color(p.muted));
                        ui.add_space(4.);
                        let (add_row, _) =
                            ui.allocate_exact_size(vec2(inner_w, 32.), egui::Sense::hover());
                        let add_w = 88.;
                        let rem = (inner_w - add_w - 16.).max(180.);
                        let combo_w = (rem * 0.52).round();
                        let input_w = rem - combo_w;
                        let combo_rect = rect(add_row.left(), add_row.top(), combo_w, 32.);
                        let input_rect =
                            rect(combo_rect.right() + 8., add_row.top(), input_w, 32.);
                        let add_btn_rect = rect(add_row.right() - add_w, add_row.top(), add_w, 32.);

                        let running = self.running_apps.clone();
                        let existing_procs = self.draft.share_groups[idx].processes.clone();
                        self.hits.insert("share-group-running-apps".into(), combo_rect);
                        ui.scope_builder(egui::UiBuilder::new().max_rect(combo_rect), |ui| {
                            let visuals = ui.visuals_mut();
                            for w in [
                                &mut visuals.widgets.inactive,
                                &mut visuals.widgets.hovered,
                                &mut visuals.widgets.active,
                            ] {
                                w.corner_radius = egui::CornerRadius::same(6);
                            }
                            let popup_w = (combo_w + 100.).clamp(280., 350.);
                            egui::ComboBox::from_id_salt("share-group-running-apps")
                                .width(combo_w)
                                .height(200.)
                                .truncate()
                                .selected_text(tr!("Pick a running app…"))
                                .show_ui(ui, |ui| {
                                    ui.set_min_width(popup_w);
                                    ui.set_max_width(popup_w);
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                    ui.spacing_mut().item_spacing.y = 2.;
                                    ui.spacing_mut().button_padding = vec2(8., 4.);
                                    ui.spacing_mut().interact_size.y = 24.;
                                    if running.is_empty() {
                                        ui.add_enabled(
                                            false,
                                            egui::Label::new(
                                                egui::RichText::new(tr!("No open apps detected"))
                                                    .color(p.muted),
                                            ),
                                        );
                                    } else {
                                        ui.with_layout(
                                            egui::Layout::top_down_justified(egui::Align::Min),
                                            |ui| {
                                                for app in &running {
                                                    let already_added = existing_procs
                                                        .iter()
                                                        .any(|e| e.eq_ignore_ascii_case(&app.exe));
                                                    let display = format_running_app_label(app);
                                                    if already_added {
                                                        let added_text = if display == app.exe {
                                                            format!("{} ({})", app.exe, tr!("Added"))
                                                        } else if let Some(rest) =
                                                            display.strip_prefix(&app.exe)
                                                        {
                                                            format!(
                                                                "{} ({}){rest}",
                                                                app.exe,
                                                                tr!("Added")
                                                            )
                                                        } else {
                                                            format!(
                                                                "{display} ({})",
                                                                tr!("Added")
                                                            )
                                                        };
                                                        ui.add_enabled_ui(false, |ui| {
                                                            let mut resp = ui.add(
                                                                egui::Button::selectable(
                                                                    false,
                                                                    egui::RichText::new(added_text)
                                                                        .color(p.muted),
                                                                )
                                                                .min_size(vec2(0., 24.))
                                                                .truncate(),
                                                            );
                                                            if !app.title.trim().is_empty() {
                                                                resp = resp
                                                                    .on_disabled_hover_text(&display);
                                                            }
                                                            let _ = resp;
                                                        });
                                                    } else {
                                                        let mut resp = ui.add(
                                                            egui::Button::selectable(
                                                                false,
                                                                &display,
                                                            )
                                                            .min_size(vec2(0., 24.))
                                                            .truncate(),
                                                        );
                                                        if !app.title.trim().is_empty() {
                                                            resp = resp.on_hover_text(&display);
                                                        }
                                                        if resp.clicked() {
                                                            selected_running =
                                                                Some(app.exe.clone());
                                                        }
                                                    }
                                                }
                                            },
                                        );
                                    }
                                    self.hits
                                        .insert("share-group-running-popup".into(), ui.min_rect());
                                });
                        });

                        let exe_edit = styled_text_edit(
                            ui,
                            input_rect,
                            "share-group-exe-input",
                            &mut self.share_group_exe_input,
                            120,
                            tr!("or type app.exe"),
                            p,
                        );
                        if exe_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit_manual = true;
                        }

                        let has_input = !self.share_group_exe_input.trim().is_empty();
                        let add_hovered = ui.rect_contains_pointer(add_btn_rect);
                        let add_btn = ui.interact(
                            add_btn_rect,
                            Id::new("share-group-add-btn"),
                            egui::Sense::click(),
                        );
                        ui.painter().rect_filled(
                            add_btn_rect,
                            6.,
                            if has_input {
                                p.accent
                            } else if add_hovered {
                                p.hover
                            } else {
                                p.surface
                            },
                        );
                        ui.painter().rect_stroke(
                            add_btn_rect,
                            6.,
                            Stroke::new(1., if has_input { p.accent } else { p.border }),
                            StrokeKind::Inside,
                        );
                        let add_fg = if has_input { p.accent_text } else { p.muted };
                        let add_galley = ui.painter().layout_no_wrap(
                            tr!("Add").into(),
                            egui::FontId::proportional(13.),
                            add_fg,
                        );
                        ui.painter().galley(
                            pos2(
                                add_btn_rect.center().x - add_galley.size().x / 2.,
                                add_btn_rect.center().y - add_galley.size().y / 2.,
                            ),
                            add_galley,
                            add_fg,
                        );
                        self.hits.insert("share-group-add".into(), add_btn_rect);
                        if add_btn.clicked() {
                            submit_manual = true;
                        }
                    });
            });

            if delete_group {
                self.draft.share_groups.remove(idx);
                self.share_group_selected = idx.saturating_sub(1);
                self.share_group_focus_name = false;
                self.notice.clear();
                return;
            }
            if let Some((a, b)) = swap_pair {
                self.draft.share_groups[idx].processes.swap(a, b);
                self.notice.clear();
            } else if let Some(pi) = remove_proc {
                self.draft.share_groups[idx].processes.remove(pi);
                self.notice.clear();
            }
            if let Some(exe) = selected_running {
                self.add_share_group_exe(idx, &exe);
            } else if submit_manual {
                let typed = self.share_group_exe_input.clone();
                self.add_share_group_exe(idx, &typed);
                if self.notice.is_empty() {
                    self.share_group_exe_input.clear();
                }
            }
        });
    }
    fn appearance_panel(&mut self, ui: &mut Ui, r: egui::Rect) {
        let p = self.palette();
        use p2pshare_media::{i18n::Language, tr};
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            ui.label(tr!("Language"));
            let language = self.draft.language.get_or_insert_with(Language::default);
            let selected = match language { Language::English => "English", Language::PtBr => "Português (Brasil)" };
            egui::ComboBox::from_id_salt("settings-language").selected_text(selected).show_ui(ui, |ui| {
                ui.selectable_value(language, Language::English, "English");
                ui.selectable_value(language, Language::PtBr, "Português (Brasil)");
            });
        });
        if self.button(
            ui,
            "theme-default",
            rect(r.right() - 210., r.top() + 36., 99., 34.),
            "Midnight",
            false,
        ) {
            self.draft.theme = Theme::default();
        }
        if self.button(
            ui,
            "theme-light",
            rect(r.right() - 101., r.top() + 36., 101., 34.),
            "Daylight",
            false,
        ) {
            self.draft.theme = Theme {
                background: "#F1F3F8".into(),
                sidebar: "#E7EBF3".into(),
                surface: "#FFFFFF".into(),
                hover: "#DEE3EF".into(),
                border: "#BAC2D3".into(),
                text: "#172036".into(),
                muted: "#4D5668".into(),
                accent: "#535ACF".into(),
                accent_text: "#FFFFFF".into(),
                success: "#126946".into(),
                danger: "#A21B38".into(),
                video: "#101117".into(),
            };
        }
        let colors = [
            (p2pshare_media::tr!("Background"), &mut self.draft.theme.background),
            (p2pshare_media::tr!("Sidebar"), &mut self.draft.theme.sidebar),
            (p2pshare_media::tr!("Surface"), &mut self.draft.theme.surface),
            (p2pshare_media::tr!("Hover"), &mut self.draft.theme.hover),
            (p2pshare_media::tr!("Border"), &mut self.draft.theme.border),
            (p2pshare_media::tr!("Text"), &mut self.draft.theme.text),
            (p2pshare_media::tr!("Secondary text"), &mut self.draft.theme.muted),
            (p2pshare_media::tr!("Accent"), &mut self.draft.theme.accent),
            (p2pshare_media::tr!("Accent text"), &mut self.draft.theme.accent_text),
            (p2pshare_media::tr!("Connected"), &mut self.draft.theme.success),
            ("Stop / errors", &mut self.draft.theme.danger),
            (p2pshare_media::tr!("Video background"), &mut self.draft.theme.video),
        ];
        for (n, (title, value)) in colors.into_iter().enumerate() {
            let col = n / 6;
            let row = n % 6;
            let x = r.left() + col as f32 * (r.width() / 2. + 8.);
            let y = r.top() + 80. + row as f32 * 39.;
            text(ui, pos2(x, y + 6.), title, 13., p.text);
            let area = rect(x + r.width() / 2. - 79., y, 54., 29.);
            let mut c = color(value).unwrap();
            ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                if ui.color_edit_button_srgba(&mut c).changed() {
                    *value = format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
                }
            });
        }
        let contrast = contrast_ratio(
            self.draft.theme.palette().text,
            self.draft.theme.palette().bg,
        );
        text(
            ui,
            pos2(r.left(), r.top() + 332.),
            &p2pshare_media::trf!(
                "Text contrast  {contrast:.1}:1{}",
                if contrast < 4.5 {
                    p2pshare_media::tr!(" · Increase contrast for readability")
                } else {
                    p2pshare_media::tr!(" · Readable")
                }
            ,contrast=contrast),
            12.,
            if contrast < 4.5 { p.red } else { p.muted },
        );
    }
    fn room_dialog(&mut self, ui: &mut Ui, r: egui::Rect) {
        self.heading(
            ui,
            r,
            "Find your people",
            p2pshare_media::tr!("Use the same room name on every device."),
        );
        let p = self.palette();
        text(ui, r.min + vec2(0., 83.), p2pshare_media::tr!("ROOM NAME"), 11., p.muted);
        let rr = rect(r.left(), r.top() + 105., r.width(), 37.);
        self.hits.insert("room-input".into(), rr);
        ui.put(
            rr,
            egui::TextEdit::singleline(&mut self.room_input)
                .vertical_align(egui::Align::Center)
                .char_limit(64),
        );
        text(
            ui,
            r.min + vec2(0., 156.),
            if self.live {
                p2pshare_media::tr!("Anyone using this room name can join.")
            } else {
                "Prototype only: no network requests are made."
            },
            12.,
            p.muted,
        );
        let valid = (3..=64).contains(&self.room_input.trim().len())
            && self
                .room_input
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || " _-".contains(c));
        if ui
            .add_enabled_ui(valid, |ui| {
                self.button(
                    ui,
                    "join",
                    rect(r.left(), r.bottom() - 37., r.width(), 37.),
                    if self.live {
                        p2pshare_media::tr!("Join room")
                    } else {
                        "Preview this room"
                    },
                    true,
                )
            })
            .inner
        {
            self.room = self
                .room_input
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            if self.live {
                self.intents.push(Intent::Join(self.room.clone()));
            } else {
                self.joined = true;
            }
            self.dialog = Dialog::None;
        }
    }
}
fn styled_text_edit(
    ui: &mut Ui,
    target_rect: egui::Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    buf: &mut String,
    char_limit: usize,
    hint: &str,
    p: crate::config::Palette,
) -> egui::Response {
    ui.scope_builder(egui::UiBuilder::new().max_rect(target_rect), |ui| {
        let visuals = ui.visuals_mut();
        visuals.extreme_bg_color = p.surface;
        visuals.selection.stroke = Stroke::new(1.5, p.accent);
        for w in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            w.corner_radius = egui::CornerRadius::same(6);
        }
        ui.put(
            target_rect,
            egui::TextEdit::singleline(buf)
                .id(Id::new(id))
                .vertical_align(egui::Align::Center)
                .margin(egui::Margin::symmetric(10, 6))
                .char_limit(char_limit)
                .hint_text(egui::RichText::new(hint).color(p.muted)),
        )
    })
    .inner
}
fn format_running_app_label(app: &crate::app::RunningApp) -> String {
    let title = app.title.trim();
    let stem = app
        .exe
        .strip_suffix(".exe")
        .or_else(|| app.exe.strip_suffix(".EXE"))
        .unwrap_or(&app.exe);
    if title.is_empty()
        || title.eq_ignore_ascii_case(&app.exe)
        || title.eq_ignore_ascii_case(stem)
    {
        app.exe.clone()
    } else {
        format!("{} · {title}", app.exe)
    }
}
fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    fn l(c: Color32) -> f32 {
        let v = [c.r(), c.g(), c.b()].map(|x| {
            let f = x as f32 / 255.;
            if f <= 0.04045 {
                f / 12.92
            } else {
                ((f + 0.055) / 1.055).powf(2.4)
            }
        });
        v[0] * 0.2126 + v[1] * 0.7152 + v[2] * 0.0722
    }
    let x = l(a);
    let y = l(b);
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]mod cursor_tests {
 use super::*;
 #[test]fn pointer_checkbox_toggles_draft_and_stays_in_the_dialog(){
  let ctx=egui::Context::default();let mut app=App::new(Default::default(),None);app.open_share();
  let render=|a:&mut App,events|ctx.run_ui(egui::RawInput{screen_rect:Some(rect(0.,0.,1440.,932.)),events,..Default::default()},|ui|a.ui(ui));
  render(&mut app,vec![]);render(&mut app,vec![]);let checkbox=app.hits["show-cursor"];assert!(app.regions.iter().find(|(n,_)|n=="modal").unwrap().1.contains_rect(checkbox));
  let at=checkbox.center();for pressed in [true,false]{render(&mut app,vec![egui::Event::PointerMoved(at),egui::Event::PointerButton{pos:at,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()}]);}
  assert!(!app.draft.stream.show_cursor);assert!(app.config.stream.show_cursor);
 }
}

#[cfg(test)]mod audio_exclusion_toggle_tests{
 use super::*;
 #[test]fn exclusion_toggle_is_localized_and_fits_in_sharing_settings(){
  for language in [p2pshare_media::i18n::Language::English,p2pshare_media::i18n::Language::PtBr]{
   let ctx=egui::Context::default();let mut config=crate::config::Config::default();config.language=Some(language);let mut app=App::new(config,None);app.open_share();
   let render=|a:&mut App,events|ctx.run_ui(egui::RawInput{screen_rect:Some(rect(0.,0.,1440.,932.)),events,..Default::default()},|ui|a.ui(ui));for _ in 0..3{render(&mut app,vec![]);}
   let checkbox=app.hits["exclude-app-audio"];assert!(app.regions.iter().find(|(n,_)|n=="modal").unwrap().1.contains_rect(checkbox));assert!(!checkbox.intersects(app.hits["show-cursor"]),"filter {checkbox:?}; cursor {:?}",app.hits["show-cursor"]);
   assert!((checkbox.left()-app.hits["audio"].left()).abs()<1.);let at=checkbox.center();for pressed in [true,false]{render(&mut app,vec![egui::Event::PointerMoved(at),egui::Event::PointerButton{pos:at,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()}]);}
    assert!(app.draft.stream.exclude_audio_processes);assert!(!app.config.stream.exclude_audio_processes);
   }
  }
}

#[cfg(test)]
mod share_groups_settings_tests {
    use super::*;
    use crate::{app::RunningApp, config::Config};
    use p2pshare_media::config::{ProcessPriority, ShareGroup};

    #[test]
    fn settings_tabs_edit_share_groups_and_process_priority() {
        let ctx = egui::Context::default();
        let mut cfg = Config::default();
        cfg.username = "Alex".into();
        let mut app = App::new(cfg, None);
        app.running_apps = vec![
            RunningApp {
                exe: "Discord.exe".into(),
                title: "Discord".into(),
            },
            RunningApp {
                exe: "LeagueClientUx.exe".into(),
                title: "League of Legends".into(),
            },
            RunningApp {
                exe: "VALORANT-Win64-Shipping.exe".into(),
                title: "VALORANT  ".into(),
            },
        ];
        app.open_appearance();
        let render = |a: &mut App, events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 1280., 832.)),
                    events,
                    ..Default::default()
                },
                |ui| a.ui(ui),
            )
        };
        let click = |a: &mut App, key: &str| {
            let at = a.hits[key].center();
            for pressed in [true, false] {
                render(
                    a,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };

        render(&mut app, vec![]);
        assert!(app.hits.contains_key("process-priority"));
        app.draft.process_priority = ProcessPriority::Normal;

        app.settings_tab = crate::diagnostics::Tab::ShareGroups;
        render(&mut app, vec![]);
        render(&mut app, vec![]);
        let modal = app.regions.iter().find(|(n, _)| n == "modal").unwrap().1;
        let cancel = app.hits["cancel-theme"];
        let save = app.hits["save-theme"];
        for key in [
            "share-group-new",
            "share-group-reset",
            "share-group-add",
            "share-group-up-1",
        ] {
            let hit = app.hits[key];
            assert!(modal.contains_rect(hit), "{key} outside modal: {hit:?}");
            assert!(!hit.intersects(cancel), "{key} overlaps cancel: {hit:?}");
            assert!(!hit.intersects(save), "{key} overlaps save: {hit:?}");
        }
        assert_eq!(
            app.draft.share_groups[0].processes,
            ["League of Legends.exe", "LeagueClientUx.exe"]
        );

        // Move LeagueClientUx.exe up to first priority.
        click(&mut app, "share-group-up-1");
        assert_eq!(
            app.draft.share_groups[0].processes,
            ["LeagueClientUx.exe", "League of Legends.exe"]
        );

        // Add custom app without explicit .exe extension.
        app.share_group_exe_input = "RiotClientUx".into();
        click(&mut app, "share-group-add");
        assert_eq!(
            app.draft.share_groups[0].processes,
            ["LeagueClientUx.exe", "League of Legends.exe", "RiotClientUx.exe"]
        );

        // Reset restores League default order.
        click(&mut app, "share-group-reset");
        assert_eq!(
            app.draft.share_groups,
            p2pshare_media::config::default_share_groups()
        );

        // Save persists process priority and share groups.
        app.draft.share_groups.push(ShareGroup {
            name: "Valorant".into(),
            merge_in_picker: true,
            processes: vec!["VALORANT-Win64-Shipping.exe".into()],
        });
        app.draft.process_priority = ProcessPriority::Normal;
        click(&mut app, "save-theme");
        assert_eq!(app.config.process_priority, ProcessPriority::Normal);
        assert_eq!(app.config.share_groups.len(), 2);
    }

    #[test]
    fn running_apps_dropdown_stays_compact_for_long_titles() {
        let long_title = "Very Long Window Title ".repeat(11);
        assert!(long_title.len() >= 250);
        assert_eq!(
            format_running_app_label(&RunningApp {
                exe: "Discord.exe".into(),
                title: "Discord".into(),
            }),
            "Discord.exe"
        );
        let labeled = format_running_app_label(&RunningApp {
            exe: "Antigravity.exe".into(),
            title: long_title.clone(),
        });
        assert!(labeled.starts_with("Antigravity.exe · "));

        let ctx = egui::Context::default();
        let mut cfg = Config::default();
        cfg.username = "Alex".into();
        let mut app = App::new(cfg, None);
        app.running_apps = vec![
            RunningApp {
                exe: "Antigravity.exe".into(),
                title: long_title,
            },
            RunningApp {
                exe: "LeagueClientUx.exe".into(),
                title: "League of Legends".into(),
            },
        ];
        app.open_appearance();
        app.settings_tab = crate::diagnostics::Tab::ShareGroups;
        let render = |a: &mut App, events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 1280., 832.)),
                    events,
                    ..Default::default()
                },
                |ui| a.ui(ui),
            )
        };
        render(&mut app, vec![]);
        render(&mut app, vec![]);

        let at = app.hits["share-group-running-apps"].center();
        for pressed in [true, false] {
            render(
                &mut app,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        render(&mut app, vec![]);
        let popup = app.hits["share-group-running-popup"];
        assert!(popup.width() <= 420., "popup width too wide: {popup:?}");
        assert!(popup.height() <= 220., "popup height too tall: {popup:?}");
    }

    #[test]
    fn settings_modal_size_is_constant_across_all_tabs() {
        use crate::diagnostics::Tab;
        use p2pshare_media::i18n::{Language, scope};
        let mut cfg = Config::default();
        cfg.username = "Alex".into();
        for language in [Language::English, Language::PtBr] {
            let _language = scope(language);
            let ctx = egui::Context::default();
            let mut app = App::new(cfg.clone(), None);
            app.open_appearance();
            let mut sizes = Vec::new();
            for tab in [Tab::Profile, Tab::Appearance, Tab::ShareGroups, Tab::Diagnostics] {
                app.settings_tab = tab;
                for _ in 0..2 {
                    let _ = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(rect(0., 0., 1280., 832.)),
                            ..Default::default()
                        },
                        |ui| app.ui(ui),
                    );
                }
                let modal = app.regions.iter().find(|(n, _)| n == "modal").unwrap().1;
                sizes.push(modal.size());
            }
            let first = sizes[0];
            for (idx, size) in sizes.iter().enumerate() {
                assert!(
                    (size.x - first.x).abs() < 0.5 && (size.y - first.y).abs() < 0.5,
                    "modal size changed on tab {idx} ({language:?}): first={first:?}, current={size:?}"
                );
            }
        }
    }
}
