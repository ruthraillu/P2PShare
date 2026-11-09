//! Android profiles, crop editor and per-stream viewer sheets. No transport or file I/O.
use crate::{Action, Participant, RoomUi, theme::*};
use egui::{Align2, Color32, FontId, Rect, Sense, TextureHandle, TextureId, Ui, pos2, vec2};
use std::{collections::HashMap, sync::Arc};
#[derive(Default)]
pub struct Profiles {
    pub avatars: HashMap<u64, TextureId>,
    pub names: HashMap<u64, Vec<String>>,
    pub images: HashMap<u64, Vec<Arc<[u8]>>>,
    pub viewers: HashMap<u64, Vec<u64>>,
    pub selected: Option<u64>,
    pub viewers_for: Option<u64>,
    pub hits: HashMap<String, Rect>,
    pub crop: Option<Crop>,
    pub error: String,
    just_opened: bool,
    all_names: bool,
    cached: Option<u64>,
    textures: Vec<TextureHandle>,
    enlarged: Option<TextureId>,
}
pub struct Crop {
    image: p2pshare_media::avatar::Source,
    texture: TextureHandle,
    zoom: f32,
    x: f32,
    y: f32,
}
impl Profiles {
    pub fn open(&mut self, id: u64) {
        self.selected = Some(id);
        self.viewers_for = None;
        self.cached = None;
        self.all_names = false;
        self.just_opened = true;
        self.enlarged = None;
    }
    pub fn invalidate(&mut self) {
        self.cached = None;
        // An enlarged image may reference a texture owned by the invalidated cache.
        self.enlarged = None;
    }
    pub fn handles_back(&self) -> bool {
        self.selected.is_some()
            || self.viewers_for.is_some()
            || self.crop.is_some()
            || self.enlarged.is_some()
    }
    pub fn back(&mut self) -> bool {
        if self.enlarged.take().is_some() {
            return true;
        }
        if self.crop.take().is_some() {
            return true;
        }
        if self.selected.take().is_some() {
            self.textures.clear();
            self.cached = None;
            return true;
        }
        self.viewers_for.take().is_some()
    }
    pub fn load_image(&mut self, ctx: &egui::Context, bytes: &[u8]) {
        match p2pshare_media::avatar::source(bytes) {
            Ok(image) => {
                let texture = ctx.load_texture(
                    "avatar-crop",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                );
                self.crop = Some(Crop {
                    image,
                    texture,
                    zoom: 1.,
                    x: 0.5,
                    y: 0.5,
                });
                self.just_opened = true;
                self.error.clear();
            }
            Err(e) => self.error = e.to_string(),
        }
    }
}
pub fn circle(p: &egui::Painter, c: egui::Pos2, r: f32, t: TextureId, uv: Rect) {
    let mut mesh = egui::Mesh::with_texture(t);
    mesh.vertices.push(egui::epaint::Vertex {
        pos: c,
        uv: uv.center(),
        color: Color32::WHITE,
    });
    for i in 0..=64 {
        let a = i as f32 * std::f32::consts::TAU / 64.;
        let d = vec2(a.cos(), a.sin());
        mesh.vertices.push(egui::epaint::Vertex {
            pos: c + d * r,
            uv: uv.center() + d * uv.size() * 0.5,
            color: Color32::WHITE,
        });
        if i > 0 {
            mesh.indices.extend([0, i, i + 1]);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}
impl RoomUi {
    pub fn mobile_avatar(&self, ui: &Ui, r: Rect, id: u64, name: &str) {
        let p = ui.painter_at(r);
        p.circle_filled(r.center(), r.width() / 2., avatar_color(id));
        if let Some(t) = self.profiles.avatars.get(&id) {
            circle(
                &p,
                r.center(),
                r.width() / 2.,
                *t,
                Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
            );
        } else {
            p.text(
                r.center(),
                Align2::CENTER_CENTER,
                initials(name),
                FontId::proportional(r.width() * 0.4),
                Color32::WHITE,
            );
        }
    }
    pub(crate) fn mobile_avatar_settings(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::Frame::new()
            .fill(PANEL)
            .corner_radius(10)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if ui.available_width() > 560. {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(48., 48.), Sense::hover());
                        self.mobile_avatar(ui, r, 0, &self.name);
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&self.name)
                                    .size(16.)
                                    .color(Color32::WHITE),
                            );
                            ui.label(egui::RichText::new(p2pshare_media::tr!("Profile image")).size(12.).color(MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.profiles.avatars.contains_key(&0)
                                && ui
                                    .button(egui::RichText::new(p2pshare_media::tr!("Remove")).color(Color32::LIGHT_RED))
                                    .clicked()
                            {
                                actions.push(Action::RemoveAvatar);
                            }
                            if ui
                                .button(if self.profiles.avatars.contains_key(&0) {
                                    p2pshare_media::tr!("Change image")
                                } else {
                                    p2pshare_media::tr!("Choose image")
                                })
                                .clicked()
                            {
                                actions.push(Action::ChooseAvatar);
                            }
                        });
                    });
                } else {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(64., 64.), Sense::hover());
                        self.mobile_avatar(ui, r, 0, &self.name);
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&self.name)
                                    .size(18.)
                                    .color(Color32::WHITE),
                            );
                            ui.label(egui::RichText::new(p2pshare_media::tr!("Profile image")).size(12.).color(MUTED));
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .button(if self.profiles.avatars.contains_key(&0) {
                                        p2pshare_media::tr!("Change image")
                                    } else {
                                        p2pshare_media::tr!("Choose image")
                                    })
                                    .clicked()
                                {
                                    actions.push(Action::ChooseAvatar);
                                }
                                if self.profiles.avatars.contains_key(&0)
                                    && ui
                                        .button(
                                            egui::RichText::new(p2pshare_media::tr!("Remove")).color(Color32::LIGHT_RED),
                                        )
                                        .clicked()
                                {
                                    actions.push(Action::RemoveAvatar);
                                }
                            });
                        });
                    });
                }
                if !self.profiles.error.is_empty() {
                    ui.label(egui::RichText::new(&self.profiles.error).color(Color32::LIGHT_RED));
                }
            });
        ui.add_space(8.);
    }
    pub(crate) fn mobile_stream_footer(&mut self, ui: &mut Ui, r: Rect, p: &Participant) {
        ui.painter().rect_filled(r, 0., TILE);
        let avatar = Rect::from_center_size(pos2(r.left() + 23., r.center().y), vec2(32., 32.));
        self.mobile_avatar(ui, avatar, p.id, &p.name);
        let eye = Rect::from_center_size(pos2(r.right() - 25., r.center().y), vec2(44., 36.));
        let text_end=if self.watching.contains(&p.id){let volume=Rect::from_min_size(pos2(eye.left()-44.,r.top()),vec2(44.,r.height()));self.mobile_volume_button(ui,volume,p.id);volume.left()-4.}else{eye.left()-4.};
        let text = Rect::from_min_max(
            pos2(r.left() + 46., r.top()),
            pos2(text_end.max(r.left() + 47.), r.bottom()),
        );
        let painter = ui.painter_at(text);
        painter.text(
            pos2(text.left(), r.center().y - 8.),
            Align2::LEFT_CENTER,
            &p.name,
            FontId::proportional(14.),
            Color32::WHITE,
        );
        let detail = self
            .stream_details
            .get(&p.id)
            .cloned()
            .unwrap_or_else(|| format!("{}×{}", p.size.width, p.size.height));
        painter.text(
            pos2(text.left(), r.center().y + 10.),
            Align2::LEFT_CENTER,
            detail,
            FontId::proportional(11.),
            MUTED,
        );
        let profile = Rect::from_min_max(r.min, pos2(text_end, r.bottom()));
        self.profiles
            .hits
            .insert(format!("stream-profile-{}", p.id), profile);
        if ui
            .interact(
                profile,
                ui.id().with(("stream-profile", p.id)),
                Sense::click(),
            )
            .clicked()
        {
            self.profiles.open(p.id);
        }
        let c = pos2(eye.left() + 12., eye.center().y);
        ui.painter().add(egui::Shape::line(
            vec![
                c + vec2(-7., 0.),
                c + vec2(-3., -4.),
                c + vec2(3., -4.),
                c + vec2(7., 0.),
                c + vec2(3., 4.),
                c + vec2(-3., 4.),
                c + vec2(-7., 0.),
            ],
            egui::Stroke::new(1.2, ACCENT),
        ));
        ui.painter().circle_filled(c, 2., ACCENT);
        ui.painter().text(
            pos2(eye.right() - 10., eye.center().y),
            Align2::CENTER_CENTER,
            self.profiles
                .viewers
                .get(&p.id)
                .map(|v| v.len().to_string())
                .unwrap_or_else(|| "—".into()),
            FontId::proportional(12.),
            Color32::WHITE,
        );
        self.profiles
            .hits
            .insert(format!("stream-viewers-{}", p.id), eye);
        if ui
            .interact(eye, ui.id().with(("stream-viewers", p.id)), Sense::click())
            .clicked()
        {
            self.profiles.viewers_for = Some(p.id);
            self.profiles.just_opened = true;
        }
    }
    pub(crate) fn mobile_profile_dialogs(
        &mut self,
        ctx: &egui::Context,
        actions: &mut Vec<Action>,
    ) {
        self.profiles
            .hits
            .retain(|k, _| k.starts_with("stream-") || k.starts_with("member-"));
        let safe = crate::layout::safe_rect(ctx.content_rect(), self.safe_insets).shrink(8.);
        let landscape = safe.width() > safe.height();
        let enlarged_before = self.profiles.enlarged;
        let just_opened = std::mem::take(&mut self.profiles.just_opened);
        if let Some(id) = self.profiles.viewers_for {
            if !self.participants.iter().any(|p| p.id == id && p.sharing) {
                self.profiles.viewers_for = None;
            } else {
                let m = egui::Modal::new(egui::Id::new("mobile-viewers"))
                    .frame(
                        egui::Frame::new()
                            .fill(PANEL)
                            .corner_radius(12)
                            .inner_margin(16),
                    )
                    .show(ctx, |ui| {
                        ui.set_width((safe.width() - 32.).min(300.));
                        ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(p2pshare_media::tr!("Viewers"))
                                    .size(16.)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("×").clicked() {
                                        self.profiles.viewers_for = None;
                                    }
                                },
                            );
                        });
                        egui::ScrollArea::vertical()
                            .max_height((safe.height() - 90.).min(330.))
                            .show(ui, |ui| {
                                if let Some(ids) = self.profiles.viewers.get(&id).cloned() {
                                    if ids.is_empty() {
                                        ui.label(egui::RichText::new(p2pshare_media::tr!("No viewers")).color(MUTED));
                                    }
                                    for id in ids {
                                        if let Some(p) =
                                            self.participants.iter().find(|p| p.id == id).cloned()
                                        {
                                            let (r, response) = ui.allocate_exact_size(
                                                vec2(ui.available_width(), 54.),
                                                Sense::click(),
                                            );
                                            self.mobile_avatar(
                                                ui,
                                                Rect::from_center_size(
                                                    pos2(r.left() + 20., r.center().y),
                                                    vec2(40., 40.),
                                                ),
                                                id,
                                                &p.name,
                                            );
                                            ui.painter_at(r).text(
                                                pos2(r.left() + 52., r.center().y),
                                                Align2::LEFT_CENTER,
                                                &p.name,
                                                FontId::proportional(16.),
                                                Color32::WHITE,
                                            );
                                            if response.clicked() {
                                                self.profiles.open(id);
                                            }
                                        }
                                    }
                                } else {
                                    ui.label(p2pshare_media::tr!("Viewer information unavailable"));
                                    ui.label(
                                        egui::RichText::new(p2pshare_media::tr!("The sharer needs an updated app."))
                                            .color(MUTED),
                                    );
                                }
                            });
                    });
                if !just_opened && m.should_close() {
                    self.profiles.viewers_for = None;
                }
            }
        }
        if let Some(id) = self.profiles.selected {
            let Some(person) = self.participants.iter().find(|p| p.id == id).cloned() else {
                self.profiles.back();
                return;
            };
            if self.profiles.cached != Some(id) {
                self.profiles.textures.clear();
                for (i, b) in self
                    .profiles
                    .images
                    .get(&id)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    if let Ok(image) = p2pshare_media::avatar::validate(b) {
                        self.profiles.textures.push(ctx.load_texture(
                            format!("profile-old-{id}-{i}"),
                            egui::ColorImage::from_rgba_unmultiplied([256, 256], image.as_raw()),
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                self.profiles.cached = Some(id);
            }
            let modal_id = egui::Id::new("mobile-profile");
            let mut modal = egui::Modal::new(modal_id).frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(egui::Stroke::new(1., Color32::from_rgb(55, 60, 76)))
                    .corner_radius(16)
                    .inner_margin(16),
            );
            if !landscape {
                modal = modal.area(egui::Modal::default_area(modal_id).anchor(
                    Align2::CENTER_BOTTOM,
                    vec2(0., safe.bottom() - ctx.content_rect().bottom()),
                ));
            }
            let m = modal.show(ctx, |ui| {
                ui.set_width((safe.width() - 32.).min(440.));
                ui.spacing_mut().item_spacing = vec2(8., 6.);
                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(p2pshare_media::tr!("Profile")).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").clicked() {
                            self.profiles.selected = None;
                        }
                    });
                });
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    let size = if landscape { 64. } else { 88. };
                    let (r, res) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                    self.mobile_avatar(ui, r, id, &person.name);
                    if res.clicked() {
                        self.profiles.enlarged = self.profiles.avatars.get(&id).copied();
                    }
                    ui.add_space(6.);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&person.name)
                                    .size(22.)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .wrap(),
                        );
                        ui.label(
                            egui::RichText::new(if person.sharing {
                                p2pshare_media::tr!("Sharing screen")
                            } else {
                                p2pshare_media::tr!("In room")
                            })
                            .size(12.)
                            .color(MUTED),
                        );
                    });
                });
                ui.add_space(8.);
                ui.separator();
                let h = (safe.height() - if landscape { 230. } else { 240. }).clamp(70., 440.);
                egui::ScrollArea::vertical()
                    .id_salt("profile-history")
                    .max_height(h)
                    .min_scrolled_height(h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let names = self.profiles.names.get(&id).cloned().unwrap_or_default();
                        if !names.is_empty() {
                            ui.label(
                                egui::RichText::new(p2pshare_media::trf!("Previous names   {}", names.len()))
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.add_space(6.);
                            for n in names.iter().take(if self.profiles.all_names {
                                usize::MAX
                            } else {
                                3
                            }) {
                                ui.add(egui::Label::new(egui::RichText::new(n).size(15.)).wrap());
                            }
                            let label = if self.profiles.all_names {
                                p2pshare_media::tr!("Show fewer names").into()
                            } else {
                                p2pshare_media::trf!("Show all {} names", names.len())
                            };
                            if names.len() > 3 {
                                let b = ui.add_sized(
                                    [ui.available_width(), 36.],
                                    egui::Button::new(label),
                                );
                                self.profiles.hits.insert("all-names".into(), b.rect);
                                if b.clicked() {
                                    self.profiles.all_names = !self.profiles.all_names;
                                }
                            }
                        }
                        if !self.profiles.textures.is_empty() {
                            ui.add_space(14.);
                            ui.label(
                                egui::RichText::new(p2pshare_media::trf!(
                                    "Previous avatars   {}",
                                    self.profiles.textures.len()
                                ))
                                .strong()
                                .color(Color32::WHITE),
                            );
                            ui.add_space(8.);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(12., 12.);
                                for t in &self.profiles.textures {
                                    let (r, res) =
                                        ui.allocate_exact_size(vec2(56., 56.), Sense::click());
                                    circle(
                                        ui.painter(),
                                        r.center(),
                                        28.,
                                        t.id(),
                                        Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                                    );
                                    if res.clicked() {
                                        self.profiles.enlarged = Some(t.id());
                                    }
                                }
                            });
                        }
                        if names.is_empty() && self.profiles.textures.is_empty() {
                            ui.label(
                                egui::RichText::new(p2pshare_media::tr!("No previous names or avatars seen yet."))
                                    .color(MUTED),
                            );
                        }
                    });
                ui.separator();
                ui.label(
                    egui::RichText::new(p2pshare_media::tr!("History saved on this device"))
                        .size(12.)
                        .color(MUTED),
                );
            });
            if !just_opened && !self.profiles.just_opened && m.should_close() {
                self.profiles.back();
            }
        } else {
            self.profiles.textures.clear();
            self.profiles.cached = None;
        }
        if let Some(texture) = self.profiles.enlarged {
            let m = egui::Modal::new(egui::Id::new("mobile-avatar-large")).show(ctx, |ui| {
                let side = (safe.height() - 90.).min(safe.width() - 40.).min(320.);
                ui.add(egui::Image::new((texture, vec2(side, side))));
                if ui.button(p2pshare_media::tr!("Close")).clicked() {
                    self.profiles.enlarged = None;
                }
            });
            if enlarged_before.is_some() && m.should_close() {
                self.profiles.enlarged = None;
            }
        }
        if let Some(crop) = &mut self.profiles.crop {
            let mut close = false;
            let m = egui::Modal::new(egui::Id::new("mobile-avatar-crop"))
                .frame(
                    egui::Frame::new()
                        .fill(PANEL)
                        .inner_margin(16)
                        .corner_radius(12),
                )
                .show(ctx, |ui| {
                    ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                    ui.set_width((safe.width() - 40.).min(340.));
                    ui.label(
                        egui::RichText::new(p2pshare_media::tr!("Crop profile image"))
                            .size(18.)
                            .color(Color32::WHITE),
                    );
                    let side = (safe.height() - 165.)
                        .min(ui.available_width())
                        .clamp(64., 280.);
                    let uv = |c: &Crop| {
                        let s = c.image.width().min(c.image.height()) as f32 / c.zoom;
                        let w = c.image.width() as f32;
                        let h = c.image.height() as f32;
                        Rect::from_min_size(
                            pos2((w - s) * c.x / w, (h - s) * c.y / h),
                            vec2(s / w, s / h),
                        )
                    };
                    let (r, res) = ui.allocate_exact_size(vec2(side, side), Sense::drag());
                    ui.painter()
                        .image(crop.texture.id(), r, uv(crop), Color32::WHITE);
                    ui.painter().circle_stroke(
                        r.center(),
                        side / 2.,
                        egui::Stroke::new(1.5, ACCENT),
                    );
                    if res.dragged() {
                        let s = crop.image.width().min(crop.image.height()) as f32 / crop.zoom;
                        let delta = res.drag_delta();
                        crop.x = (crop.x
                            - delta.x / side * s / (crop.image.width() as f32 - s).max(1.))
                        .clamp(0., 1.);
                        crop.y = (crop.y
                            - delta.y / side * s / (crop.image.height() as f32 - s).max(1.))
                        .clamp(0., 1.);
                    }
                    ui.add(egui::Slider::new(&mut crop.zoom, 1.0..=8.0).text(p2pshare_media::tr!("Zoom")));
                    ui.horizontal(|ui| {
                        if ui.button(p2pshare_media::tr!("Cancel")).clicked() {
                            close = true;
                        }
                        if ui.button(p2pshare_media::tr!("Use image")).clicked() {
                            match p2pshare_media::avatar::crop(
                                &crop.image,
                                crop.zoom,
                                crop.x,
                                crop.y,
                            ) {
                                Ok(bytes) => {
                                    actions.push(Action::SaveAvatar(bytes));
                                    close = true
                                }
                                Err(e) => self.profiles.error = e.to_string(),
                            }
                        }
                    });
                });
            if close || (!just_opened && m.should_close()) {
                self.profiles.crop = None;
            }
        }
    }
}
