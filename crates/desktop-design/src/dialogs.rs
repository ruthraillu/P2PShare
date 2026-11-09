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
            Dialog::Appearance => vec2(760., (screen.height() - 80.).min(600.)),
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
                ui.set_min_size(size);
                ui.set_max_size(size);
                let r = egui::Rect::from_min_size(ui.min_rect().min, size);
                ui.set_clip_rect(r);
                self.regions.push(("modal".into(), r.expand(20.)));
                match mode {
                    Dialog::Name => self.name_dialog(ui, r),
                    Dialog::Share => self.share_dialog(ui, r),
                    Dialog::Appearance => self.appearance_dialog(ui, r),
                    Dialog::Crop=>self.crop_dialog(ui,r),
                    Dialog::Room => self.room_dialog(ui, r),
                    _ => {}
                }
            });
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
                        if selected {
                            let badge = rect(preview.right() - 70., preview.top() + 6., 64., 22.);
                            ui.painter().rect_filled(badge, 5., p.accent);
                            label(
                                ui,
                                badge.shrink2(vec2(6., 0.)),
                                p2pshare_media::tr!("Selected"),
                                11.,
                                p.accent_text,
                            );
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
        let palette=self.palette();label(ui,rect(r.left(),r.top(),r.width()-50.,28.),p2pshare_media::tr!("Appearance & profile"),20.,palette.text);
        self.profile_row(ui,rect(r.left(),r.top()+40.,r.width(),110.));
        let r=egui::Rect::from_min_max(r.min+vec2(0.,160.),r.max);
        let p = self.palette();
        text(ui, r.min + vec2(0., 15.), p2pshare_media::tr!("DISPLAY NAME"), 11., p.muted);
        let field = rect(r.left(), r.top() + 36., r.width() - 222., 34.);
        self.hits.insert("profile-name".into(), field);
        ui.put(
            field,
            egui::TextEdit::singleline(&mut self.name_input)
                .vertical_align(egui::Align::Center)
                .char_limit(32),
        );
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
            let y = r.top() + 93. + row as f32 * 39.;
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
            pos2(r.left(), r.top() + 345.),
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
        if !self.notice.is_empty() {
            label(
                ui,
                rect(r.left(), r.bottom() - 82., r.width(), 25.),
                &p2pshare_media::i18n::status(&self.notice).into_owned(),
                12.,
                p.red,
            );
        }
        if self.live
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
        if self.button(
            ui,
            "cancel-theme",
            rect(r.right() - 256., r.bottom() - 37., 98., 37.),
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
                    rect(r.right() - 148., r.bottom() - 37., 148., 37.),
                    p2pshare_media::tr!("Save appearance"),
                    true,
                )
            })
            .inner;
        if save {
            let previous = self.config.clone();
            self.config.theme = self.draft.theme.clone();
            self.config.username = self.name_input.trim().into();
            if self.save() {
                if let Some(avatar)=self.pending_avatar.take(){self.intents.push(Intent::SaveAvatar(avatar));}
                self.dialog = Dialog::None;
            } else {
                self.config = previous;
            }
        }
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
