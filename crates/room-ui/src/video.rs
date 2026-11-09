//! Texture painting only: no capture, codecs or networking ownership.
use crate::{Participant, RoomUi, theme::*};
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
pub(crate) fn fit_video(screen: Rect, size: p2pshare_media::Dimensions) -> Rect {
    let ratio = size.width.max(1) as f32 / size.height.max(1) as f32;
    let fit = if screen.width() / screen.height().max(1.0) > ratio {
        vec2(screen.height() * ratio, screen.height())
    } else {
        vec2(screen.width(), screen.width() / ratio)
    };
    Rect::from_center_size(screen.center(), fit)
}
pub(crate) fn paint_video(painter: &egui::Painter, screen: Rect, p: &Participant) {
    if let Some(texture) = p.texture {
        painter.image(
            texture,
            fit_video(screen, p.size),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        painter.text(
            screen.center(),
            Align2::CENTER_CENTER,
            p2pshare_media::tr!("Waiting for a frame…"),
            FontId::proportional(15.0),
            MUTED,
        );
    }
}
impl RoomUi {
    pub(crate) fn tile(&mut self, ui: &mut Ui, p: &Participant, size: Vec2, focused: bool) {
        if self.platform == crate::Platform::Android {
            self.phone_tile(ui, p, size, focused);
            return;
        }
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        self.snapshot.tiles.push((p.id, rect));
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10.0, TILE);
        let screen = Rect::from_min_max(rect.min + vec2(2.0, 2.0), rect.max - vec2(2.0, 46.0));
        painter.rect_filled(screen, 8.0, Color32::from_rgb(12, 14, 20));
        paint_video(&painter, screen, p);
        painter
            .with_clip_rect(Rect::from_min_max(
                pos2(rect.left() + 12.0, rect.bottom() - 44.0),
                pos2(
                    rect.right()
                        - if !p.local && self.watching.contains(&p.id) {
                            220.0
                        } else {
                            92.0
                        },
                    rect.bottom(),
                ),
            ))
            .text(
                rect.left_bottom() + vec2(12.0, -22.0),
                Align2::LEFT_CENTER,
                format!("{}{}", p.name, if p.local { p2pshare_media::tr!(" (you)") } else { "" }),
                FontId::proportional(14.0),
                Color32::WHITE,
            );
        let mute_rect = Rect::from_min_size(
            pos2(rect.right() - 84.0, rect.bottom() - 44.0),
            vec2(80.0, 44.0),
        );
        let mute = ui
            .add_enabled_ui(!p.local, |ui| {
                ui.put(
                    mute_rect,
                    egui::Button::new(if p.local {
                        p2pshare_media::tr!("Local")
                    } else if self.audio.is_muted(p.id) {
                        p2pshare_media::tr!("Unmute")
                    } else {
                        p2pshare_media::tr!("Mute")
                    }),
                )
            })
            .inner;
        self.snapshot.audio_buttons.push((p.id, mute.rect));
        if mute.clicked() {
            self.toggle_audio(p.id);
        }
        mute.on_disabled_hover_text(p2pshare_media::tr!("Local preview does not replay your own audio."))
            .on_hover_text(p2pshare_media::tr!("Mute this screen on your device."));
        if response.hovered() || focused {
            painter.rect_stroke(
                rect.shrink(1.0),
                10.0,
                Stroke::new(2.0, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        response
            .clone()
            .on_hover_text(p2pshare_media::trf!("{}\nClick or tap to focus this screen", p.name));
        let mut stopped = false;
        if !p.local {
            let watching = self.watching.contains(&p.id);
            let r = if watching {
                Rect::from_min_size(
                    pos2(mute_rect.left() - 132., mute_rect.top() + 6.),
                    vec2(124., 32.),
                )
            } else {
                Rect::from_center_size(screen.center(), vec2(128., 38.))
            };
            if ui
                .put(
                    r,
                    egui::Button::new(if watching {
                        p2pshare_media::tr!("Stop watching")
                    } else {
                        p2pshare_media::tr!("Watch screen")
                    })
                    .fill(ACCENT),
                )
                .clicked()
            {
                if watching {
                    self.watching.remove(&p.id);
                    self.focused = None;
                    stopped = true;
                } else {
                    self.watching.insert(p.id);
                    self.focused = Some(p.id);
                }
            }
        }
        if !stopped
            && response.clicked()
            && !ui.input(|i| {
                i.pointer
                    .interact_pos()
                    .is_some_and(|pos| mute_rect.contains(pos))
            })
        {
            self.focused = Some(p.id);
        }
    }
}

impl RoomUi {
    fn phone_tile(&mut self, ui: &mut Ui, p: &Participant, size: Vec2, _focused: bool) {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("phone_tile", p.id))
                .max_rect(rect),
        );
        let ui = &mut child;
        self.snapshot.tiles.push((p.id, rect));
        let watching = self.watching.contains(&p.id);
        let footer = if watching { 100.0 } else { 54.0 };
        let screen = Rect::from_min_max(
            rect.min,
            pos2(rect.right(), (rect.bottom() - footer).max(rect.top() + 1.0)),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 9.0, TILE);
        painter.rect_filled(screen, 9.0, egui::Color32::BLACK);
        if let Some(texture) = p.texture {
            painter.add(
                egui::epaint::RectShape::filled(
                    fit_video(screen, p.size),
                    egui::CornerRadius {
                        nw: 9,
                        ne: 9,
                        sw: 0,
                        se: 0,
                    },
                    Color32::WHITE,
                )
                .with_texture(texture, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
            );
        } else {
            paint_video(&painter, screen, p);
        }

        if watching {
            if ui
                .interact(screen, ui.id().with((p.id, "video")), Sense::click())
                .clicked()
            {
                self.focused = Some(p.id);
            }
        } else {
            let watch = ui.put(
                Rect::from_center_size(screen.center(), vec2(120.0, 44.0)),
                egui::Button::new(p2pshare_media::tr!("Watch")).fill(ACCENT),
            );
            self.snapshot.watch_buttons.push((p.id, watch.rect));
            if watch.clicked() {
                self.watching.insert(p.id);
                self.focused = Some(p.id);
            }
        }
        let details = Rect::from_min_max(
            pos2(rect.left() + 10.0, screen.bottom() + 5.0),
            pos2(rect.right() - 10.0, screen.bottom() + 49.0),
        );
        let text_right = if watching {
            details.right() - 122.0
        } else {
            details.right()
        };
        let text_painter = painter.with_clip_rect(Rect::from_min_max(
            details.min,
            pos2(text_right.max(details.left()), details.bottom()),
        ));
        text_painter.text(
            details.left_top() + vec2(0.0, 11.0),
            Align2::LEFT_CENTER,
            &p.name,
            FontId::proportional(14.0),
            Color32::WHITE,
        );
        let detail = self
            .stream_details
            .get(&p.id)
            .cloned()
            .unwrap_or_else(|| format!("{} × {}", p.size.width, p.size.height));
        text_painter.text(
            details.left_top() + vec2(0.0, 31.0),
            Align2::LEFT_CENTER,
            detail,
            FontId::proportional(11.0),
            MUTED,
        );
        if watching {
            let full_rect = Rect::from_min_size(
                pos2(details.right() - 44.0, details.top()),
                vec2(44.0, 44.0),
            );
            let full = ui.put(full_rect, egui::Button::new(""));
            self.snapshot.fullscreen_button = Some(full.rect);
            let c = full.rect.center();
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                painter.add(egui::Shape::line(
                    vec![
                        c + vec2(x * 4.0, y * 8.0),
                        c + vec2(x * 8.0, y * 8.0),
                        c + vec2(x * 8.0, y * 4.0),
                    ],
                    Stroke::new(1.7, Color32::WHITE),
                ));
            }
            if full.clicked() {
                self.enter_fullscreen(p.id);
            }
            let stop = ui.put(
                Rect::from_min_size(
                    pos2(full_rect.left() - 68.0, details.top()),
                    vec2(60.0, 44.0),
                ),
                egui::Button::new(p2pshare_media::tr!("Stop")),
            );
            self.snapshot.stop_button = Some(stop.rect);
            if stop.clicked() {
                self.watching.remove(&p.id);
                self.focused = None;
            }
        }
        if watching {
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_min_max(
                    pos2(rect.left() + 6.0, rect.bottom() - 48.0),
                    rect.right_bottom() - vec2(6.0, 2.0),
                )),
                |ui| {
                    ui.horizontal(|ui| self.volume_controls(ui, p.id));
                },
            );
        }
    }
}
