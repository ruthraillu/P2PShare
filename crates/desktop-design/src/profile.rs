//! Profile editing and circular avatar paint, independent of file dialogs and transport.
use crate::app::{App, Dialog, Intent, label, rect};
use egui::{Color32, Rect, TextureId, pos2, vec2};
pub struct Crop {
    pub image: p2pshare_media::avatar::Source,
    pub texture: egui::TextureHandle,
    pub zoom: f32,
    pub offset: egui::Vec2,
}
pub fn circle(p: &egui::Painter, center: egui::Pos2, r: f32, texture: TextureId, uv: Rect) {
    let mut mesh = egui::Mesh::with_texture(texture);
    mesh.vertices.push(egui::epaint::Vertex {
        pos: center,
        uv: uv.center(),
        color: Color32::WHITE,
    });
    for i in 0..=64 {
        let a = i as f32 * std::f32::consts::TAU / 64.;
        let d = vec2(a.cos(), a.sin());
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + d * r,
            uv: uv.center() + d * uv.size() * 0.5,
            color: Color32::WHITE,
        });
        if i > 0 {
            mesh.indices.extend([0, i, i + 1]);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}
impl App {
    pub fn load_profile_image(
        &mut self,
        ctx: &egui::Context,
        image: p2pshare_media::avatar::Source,
    ) {
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        self.crop = Some(Crop {
            image,
            texture: ctx.load_texture("profile-source", color, egui::TextureOptions::LINEAR),
            zoom: 1.,
            offset: vec2(0.5, 0.5),
        });
        self.dialog = Dialog::Crop;
    }
    pub(crate) fn profile_row(&mut self, ui: &mut egui::Ui, r: Rect) {
        let p = self.palette();
        ui.painter().rect_filled(r, 8., p.surface);
        let center = r.min + vec2(42., 50.);
        ui.painter().circle_filled(center, 34., p.accent);
        if let Some(texture) = self.avatars.get(&0) {
            circle(
                ui.painter(),
                center,
                34.,
                *texture,
                Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
            );
        } else {
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                self.name_input
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string(),
                egui::FontId::proportional(28.),
                p.accent_text,
            );
        }
        label(
            ui,
            rect(r.left() + 94., r.top() + 10., r.width() - 100., 22.),
            p2pshare_media::tr!("PROFILE IMAGE"),
            11.,
            p.muted,
        );
        label(
            ui,
            rect(r.left() + 94., r.top() + 27., r.width() - 100., 20.),
            p2pshare_media::tr!("Choose the image other users see."),
            12.,
            p.muted,
        );
        let has = self.avatars.contains_key(&0);
        if self.button(
            ui,
            "choose-avatar",
            rect(r.left() + 94., r.top() + 52., 126., 32.),
            if has { p2pshare_media::tr!("Change image") } else { p2pshare_media::tr!("Choose image") },
            false,
        ) {
            self.intents.push(Intent::ChooseAvatar);
        }
        if has
            && self.button(
                ui,
                "remove-avatar",
                rect(r.left() + 232., r.top() + 52., 84., 32.),
                p2pshare_media::tr!("Remove"),
                false,
            )
        {
            self.pending_avatar = Some(None);
            self.avatars.remove(&0);
            self.profile_preview = None;
        }
    }
    pub(crate) fn crop_dialog(&mut self, ui: &mut egui::Ui, r: Rect) {
        let p = self.palette();
        label(
            ui,
            rect(r.left(), r.top(), r.width(), 28.),
            p2pshare_media::tr!("Adjust profile image"),
            20.,
            p.text,
        );
        let size = (r.height() - 168.).min(r.width() - 180.).max(100.);
        let area = rect(r.left(), r.top() + 46., size, size);
        if let Some(crop) = &mut self.crop {
            let side = crop.image.width().min(crop.image.height()) as f32 / crop.zoom;
            let response = ui.allocate_rect(area, egui::Sense::drag());
            if response.dragged() {
                let d = ui.input(|i| i.pointer.delta());
                crop.offset.x = (crop.offset.x
                    - d.x / size * side / (crop.image.width() as f32 - side).max(1.))
                .clamp(0., 1.);
                crop.offset.y = (crop.offset.y
                    - d.y / size * side / (crop.image.height() as f32 - side).max(1.))
                .clamp(0., 1.);
            }
            let uv = Rect::from_min_size(
                pos2(
                    (crop.image.width() as f32 - side) * crop.offset.x / crop.image.width() as f32,
                    (crop.image.height() as f32 - side) * crop.offset.y
                        / crop.image.height() as f32,
                ),
                vec2(
                    side / crop.image.width() as f32,
                    side / crop.image.height() as f32,
                ),
            );
            ui.painter().rect_filled(area, 6., p.surface);
            circle(
                ui.painter(),
                area.center(),
                size * 0.5,
                crop.texture.id(),
                uv,
            );
            ui.painter()
                .circle_stroke(area.center(), size * 0.5, egui::Stroke::new(1.5, p.accent));
            circle(
                ui.painter(),
                pos2(r.right() - 70., area.top() + 74.),
                48.,
                crop.texture.id(),
                uv,
            );
            label(
                ui,
                rect(r.right() - 138., area.top() + 137., 136., 24.),
                p2pshare_media::tr!("Preview"),
                13.,
                p.muted,
            );
            label(
                ui,
                rect(area.left(), area.bottom() + 6., size, 22.),
                p2pshare_media::tr!("Drag image to reposition"),
                12.,
                p.muted,
            );
            label(
                ui,
                rect(r.left(), r.bottom() - 87., 52., 28.),
                p2pshare_media::tr!("Zoom"),
                13.,
                p.text,
            );
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = (r.width() - 68.).max(80.);
                ui.put(
                    rect(r.left() + 62., r.bottom() - 87., r.width() - 62., 28.),
                    egui::Slider::new(&mut crop.zoom, 1.0..=8.0).show_value(false),
                );
            });
        }
        if self.button(
            ui,
            "cancel-avatar-crop",
            rect(r.right() - 236., r.bottom() - 36., 100., 34.),
            p2pshare_media::tr!("Cancel"),
            false,
        ) {
            self.crop = None;
            self.dialog = Dialog::Appearance;
        }
        if self.button(
            ui,
            "use-avatar-crop",
            rect(r.right() - 124., r.bottom() - 36., 124., 34.),
            p2pshare_media::tr!("Use image"),
            true,
        ) {
            if let Some(crop) = &self.crop {
                match p2pshare_media::avatar::crop(
                    &crop.image,
                    crop.zoom,
                    crop.offset.x,
                    crop.offset.y,
                ) {
                    Ok(bytes) => {
                        let image = p2pshare_media::avatar::validate(&bytes).unwrap();
                        let texture = ui.ctx().load_texture(
                            "avatar-staged",
                            egui::ColorImage::from_rgba_unmultiplied([256, 256], image.as_raw()),
                            egui::TextureOptions::LINEAR,
                        );
                        self.avatars.insert(0, texture.id());
                        self.profile_preview = Some(texture);
                        self.pending_avatar = Some(Some(bytes));
                        self.crop = None;
                        self.dialog = Dialog::Appearance;
                    }
                    Err(e) => self.notice = e.to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_is_staged_until_profile_save() {
        let ctx = egui::Context::default();
        let mut c = crate::config::Config::default();
        c.username = "Alex".into();
        let mut app = App::live(c, "grupo".into(), false);
        app.joined = true;
        let image = p2pshare_media::avatar::Source::from_raw(
            400,
            300,
            [80u8, 100, 140, 255].repeat(400 * 300),
        )
        .unwrap();
        app.load_profile_image(&ctx, image);
        app.crop.as_mut().unwrap().zoom = 2.;
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 1440., 932.)),
                    events,
                    ..Default::default()
                },
                |ui| app.ui(ui),
            )
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        let at = app.hits["use-avatar-crop"].center();
        for pressed in [true, false] {
            frame(
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
        assert!(app.dialog == Dialog::Appearance);
        assert!(app.avatars.contains_key(&0));
        assert_eq!(
            p2pshare_media::avatar::validate(
                app.pending_avatar.as_ref().unwrap().as_ref().unwrap()
            )
            .unwrap()
            .dimensions(),
            (256, 256)
        );
        assert!(
            !app.intents
                .iter()
                .any(|i| matches!(i, Intent::SaveAvatar(_)))
        );
    }
}

impl App {
    pub fn reset_nickname_draft(&mut self, peer: u64) {
        if let Some((id, draft)) = &mut self.nickname_draft
            && *id == peer
        {
            *draft = self.profile_nicknames.get(&peer).cloned().unwrap_or_default();
        }
    }

    fn draw_nickname_editor(&mut self, ui: &mut egui::Ui, id: u64) {
        use p2pshare_media::tr;
        let p = self.palette();
        let saved = self.profile_nicknames.get(&id);
        let Some((_, draft)) = &mut self.nickname_draft else { return };
        ui.add_space(6.);
        ui.label(egui::RichText::new(tr!("Nickname")).strong().color(p.text));
        let input = ui.add_sized(
            [ui.available_width(), 36.],
            egui::TextEdit::singleline(draft)
                .id_salt(("profile-nickname", id))
                .char_limit(32)
                .hint_text(tr!("Add a nickname"))
                .vertical_align(egui::Align::Center),
        );
        self.hits.insert("profile-nickname-input".into(), input.rect);
        ui.label(egui::RichText::new(tr!("Only you can see this nickname.")).size(12.).color(p.muted));
        let nickname = draft.trim();
        let can_save = !nickname.is_empty()
            && !nickname.chars().any(char::is_control)
            && saved.is_none_or(|value| value != nickname);
        ui.horizontal(|ui| {
            if saved.is_some() {
                let remove = ui.add(egui::Button::new(tr!("Remove nickname")).frame(false));
                self.hits.insert("profile-nickname-remove".into(), remove.rect);
                if remove.clicked() {
                    self.intents.push(Intent::SetNickname { peer: id, nickname: None });
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = egui::Button::new(egui::RichText::new(tr!("Save")).color(p.accent_text))
                    .fill(p.accent)
                    .min_size(vec2(96., 30.));
                let save = ui.add_enabled(can_save, button);
                self.hits.insert("profile-nickname-save".into(), save.rect);
                if save.clicked() {
                    self.intents.push(Intent::SetNickname { peer: id, nickname: Some(nickname.into()) });
                }
            });
        });
        if self.nickname_save_failed {
            ui.label(egui::RichText::new(tr!("Could not save nickname.")).color(p.red));
        }
        ui.separator();
    }

    pub fn draw_profile_viewer(&mut self, ctx: &egui::Context) {
        let Some(id) = self.profile_view else {
            self.profile_history_textures.clear();
            self.profile_history_key = None;
            self.nickname_draft = None;
            self.nickname_save_failed = false;
            return;
        };
        let Some(member) = self.members.iter().find(|m| m.id == id) else {
            self.profile_view = None;
            self.nickname_draft = None;
            return;
        };
        let name = if id == 0 {
            self.config.username.clone()
        } else {
            member.name.clone()
        };
        let sharing = member.scene.is_some();
        if self.nickname_draft.as_ref().is_none_or(|(peer, _)| *peer != id) {
            self.nickname_draft = Some((id, self.profile_nicknames.get(&id).cloned().unwrap_or_default()));
            self.nickname_save_failed = false;
        }
        let p = self.palette();
        let just_opened=self.profile_history_key != Some(id);
        if just_opened {
            self.profile_history_textures.clear();
            self.profile_enlarged = None;
            for (i, bytes) in self
                .profile_images
                .get(&id)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Ok(image) = p2pshare_media::avatar::validate(bytes) {
                    self.profile_history_textures.push(ctx.load_texture(
                        format!("profile-history-{id}-{i}"),
                        egui::ColorImage::from_rgba_unmultiplied([256, 256], image.as_raw()),
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            self.profile_history_key = Some(id);
        }
        let names = self.profile_names.get(&id).cloned().unwrap_or_default();
        let modal = egui::Modal::new(egui::Id::new("profile-viewer"))
            .frame(
                egui::Frame::new()
                    .fill(p.side)
                    .stroke(egui::Stroke::new(1., p.border))
                    .corner_radius(14)
                    .inner_margin(24),
            )
            .show(ctx, |ui| {
                ui.set_width(392.);
                ui.spacing_mut().item_spacing=vec2(8.,6.);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(p2pshare_media::tr!("Profile")).color(p.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").clicked() {
                            self.profile_view = None;
                            self.nickname_draft = None;
                        }
                    });
                });
                ui.add_space(20.);
                ui.horizontal(|ui| {
                    let (r, res) = ui.allocate_exact_size(vec2(104., 104.), egui::Sense::click());
                    ui.painter().circle_filled(r.center(), 52., p.accent);
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        name.chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string(),
                        egui::FontId::proportional(40.),
                        p.text,
                    );
                    if let Some(t) = self.avatars.get(&id) {
                        circle(
                            ui.painter(),
                            r.center(),
                            52.,
                            *t,
                            Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                        );
                        if res.clicked() {
                            self.profile_enlarged = Some(*t);
                        }
                    }
                    ui.add_space(12.);
                    ui.vertical(|ui| {
                        ui.add_space(20.);
                        ui.add(
                            egui::Label::new(egui::RichText::new(&name).size(26.).strong().color(p.text)).wrap(),
                        );
                        ui.label(
                            egui::RichText::new(if sharing { p2pshare_media::tr!("Sharing screen") } else { p2pshare_media::tr!("In room") })
                                .color(p.muted),
                        );
                    });
                });
                ui.add_space(20.);
                ui.separator();
                let height = (ctx.content_rect().height() - 290.).clamp(100., 310.);
                egui::ScrollArea::vertical()
                    .max_height(height).min_scrolled_height(height).auto_shrink([false,false])
                    .show(ui, |ui| {
                        if id != 0 {
                            self.draw_nickname_editor(ui, id);
                        }
                        if !names.is_empty() {
                            ui.add_space(10.);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(p2pshare_media::tr!("Previous names")).size(15.).strong().color(p.text));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{ui.label(egui::RichText::new(names.len().to_string()).color(p.muted));});
                            });
                            ui.add_space(8.);
                            for name in names.iter().take(if self.profile_show_names {
                                usize::MAX
                            } else {
                                3
                            }) {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(name).size(16.)).wrap(),
                                );
                                ui.add_space(5.);
                            }
                            if names.len() > 3
                                && ui
                                    .add_sized(
                                        [ui.available_width(), 32.],
                                        egui::Button::new(if self.profile_show_names {
                                            p2pshare_media::tr!("Show fewer names").into()
                                        } else {
                                            p2pshare_media::trf!("Show all {} names", names.len())
                                        }),
                                    )
                                    .clicked()
                            {
                                self.profile_show_names = !self.profile_show_names;
                            }
                        }
                        if !self.profile_history_textures.is_empty() {
                            ui.add_space(20.);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(p2pshare_media::tr!("Previous avatars")).size(15.).strong().color(p.text));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{ui.label(egui::RichText::new(self.profile_history_textures.len().to_string()).color(p.muted));});
                            });
                            ui.add_space(12.);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(14., 14.);
                                for t in &self.profile_history_textures {
                                    let (r, response) = ui
                                        .allocate_exact_size(vec2(60., 60.), egui::Sense::click());
                                    circle(
                                        ui.painter(),
                                        r.center(),
                                        30.,
                                        t.id(),
                                        Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                                    );
                                    if response.clicked() {
                                        self.profile_enlarged = Some(t.id());
                                    }
                                }
                            });
                        }
                        if names.is_empty() && self.profile_history_textures.is_empty() {
                            ui.add_space(12.);
                            ui.label(
                                egui::RichText::new(p2pshare_media::tr!("No previous names or avatars seen yet."))
                                    .color(p.muted),
                            );
                        }
                    });
                ui.add_space(12.);
                ui.separator();
                ui.label(
                    egui::RichText::new(p2pshare_media::tr!("History saved on this device"))
                        .size(12.)
                        .color(p.muted),
                );
            });
        if !just_opened && modal.should_close() {
            self.profile_view = None;
            self.nickname_draft = None;
            self.profile_enlarged = None;
        }
        if let Some(texture) = self.profile_enlarged {
            let image = egui::Modal::new(egui::Id::new("profile-enlarged")).show(ctx, |ui| {
                ui.add(egui::Image::new((texture, vec2(256., 256.))));
                if ui.button(p2pshare_media::tr!("Close image")).clicked() {
                    self.profile_enlarged = None;
                }
            });
            if image.should_close() {
                self.profile_enlarged = None;
            }
        }
    }
}

#[cfg(test)]
mod viewer_tests {
    use super::*;
    #[test]
    fn nickname_editor_emits_save_and_remove_for_the_clicked_peer() {
        let ctx = egui::Context::default();
        let mut app = App::live(Default::default(), "grupo".into(), false);
        app.joined = true;
        app.members.push(crate::app::Member {
            id: 42,
            name: "Rafael_M".into(),
            ..Default::default()
        });
        let render = |a: &mut App, events| ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(rect(0., 0., 950., 680.)),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| a.ui(ui),
        );
        let click = |app: &mut App, key: &str| {
            let pos = app.hits[key].center();
            for pressed in [true, false] {
                render(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default(),
                }]);
            }
        };
        for _ in 0..3 { render(&mut app, vec![]); }
        click(&mut app, "profile-user-42");
        assert_eq!(app.profile_view, Some(42));
        app.nickname_draft = Some((42, " Rafa ".into()));
        for _ in 0..3 { render(&mut app, vec![]); }
        click(&mut app, "profile-nickname-save");
        assert!(matches!(app.intents.pop(), Some(Intent::SetNickname { peer: 42, nickname: Some(n) }) if n == "Rafa"));
        app.profile_nicknames.insert(42, "Rafa".into());
        app.reset_nickname_draft(42);
        let output = render(&mut app, vec![]);
        assert!(output.shapes.iter().any(|s| matches!(
            &s.shape, egui::Shape::Text(t) if t.galley.text() == "Rafael_M (Rafa)"
        )));
        click(&mut app, "profile-nickname-remove");
        assert!(matches!(app.intents.pop(), Some(Intent::SetNickname { peer: 42, nickname: None })));
        app.profile_view = Some(0);
        render(&mut app, vec![]);
        assert!(!app.hits.contains_key("profile-nickname-input"));
    }

    #[test]
    fn sidebar_opens_profile_and_long_names_expand() {
        let ctx = egui::Context::default();
        let mut app = App::live(Default::default(), "grupo".into(), false);
        app.joined = true;
        app.profile_names
            .insert(0, (0..24).map(|i| format!("Past {i}")).collect());
        let render = |a: &mut App, events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 1440., 932.)),
                    events,
                    ..Default::default()
                },
                |ui| a.ui(ui),
            )
        };
        render(&mut app, vec![]);
        render(&mut app,vec![]);
        let at = app.hits["profile-user-0"].center();
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
        assert_eq!(app.profile_view, Some(0));
        let out = render(&mut app, vec![]);
        assert!(out.shapes.iter().any(
            |s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Show all 24 names")
        ));
    }
}
