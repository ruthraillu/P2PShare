//! Compact non-fullscreen per-stream volume popup; leaves video gestures to scrolling.
use crate::{RoomUi, theme::*};
use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};
fn speaker(ui: &Ui, r: Rect, muted: bool) {
    let c = r.center();
    let color = if muted {
        Color32::LIGHT_RED
    } else {
        Color32::from_rgb(169, 173, 255)
    };
    ui.painter().add(egui::Shape::closed_line(
        vec![
            c + vec2(-9., -4.),
            c + vec2(-5., -4.),
            c + vec2(1., -9.),
            c + vec2(1., 9.),
            c + vec2(-5., 4.),
            c + vec2(-9., 4.),
        ],
        egui::Stroke::new(1.4, color),
    ));
    if muted {
        ui.painter().line_segment(
            [c + vec2(-11., 11.), c + vec2(11., -11.)],
            egui::Stroke::new(1.7, color),
        );
    } else {
        ui.painter().add(egui::Shape::line(
            vec![
                c + vec2(6., -8.),
                c + vec2(10., -4.),
                c + vec2(11., 0.),
                c + vec2(10., 4.),
                c + vec2(6., 8.),
            ],
            egui::Stroke::new(1.4, color),
        ));
    }
}
impl RoomUi {
    pub(crate) fn mobile_volume_button(&mut self, ui: &mut Ui, r: Rect, id: u64) {
        let key = format!("stream-volume-{id}");
        self.profiles.hits.insert(key, r);
        if self.mobile_volume_popup.is_some_and(|(peer, _)| peer == id) {
            self.mobile_volume_popup = Some((id, r));
            ui.painter()
                .rect_filled(r.shrink2(vec2(2., 3.)), 8., Color32::from_rgb(52, 55, 70));
        }
        speaker(
            ui,
            r,
            self.participant_audio_muted(id) || self.participant_volume(id) == 0,
        );
        if ui
            .interact(r, ui.id().with(("stream-volume", id)), Sense::click())
            .clicked()
        {
            self.mobile_volume_popup =
                if self.mobile_volume_popup.is_some_and(|(peer, _)| peer == id) {
                    None
                } else {
                    Some((id, r))
                };
        }
    }
    pub(crate) fn mobile_volume_dialog(&mut self, ctx: &egui::Context) {
        if self.pip_mode || self.is_fullscreen() || self.profiles.handles_back() {
            self.mobile_volume_popup = None;
            return;
        }
        let Some((id, anchor)) = self.mobile_volume_popup else {
            return;
        };
        if !self.watching.contains(&id)
            || !self
                .profiles
                .hits
                .contains_key(&format!("stream-volume-{id}"))
        {
            self.mobile_volume_popup = None;
            return;
        }
        let safe = crate::layout::safe_rect(ctx.content_rect(), self.safe_insets).shrink(8.);
        let width = 280.0_f32.min(safe.width());
        let at = pos2(
            (anchor.right() - width).clamp(safe.left(), (safe.right() - width).max(safe.left())),
            (anchor.top() - 72.).clamp(safe.top(), (safe.bottom() - 64.).max(safe.top())),
        );
        let result = egui::Area::new(egui::Id::new("mobile-volume-popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .constrain_to(safe)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(egui::Stroke::new(1., Color32::from_rgb(55, 60, 76)))
                    .corner_radius(11)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        let (body, _) =
                            ui.allocate_exact_size(vec2(width - 18., 48.), Sense::hover());
                        let mute = Rect::from_min_size(body.min, vec2(44., 48.));
                        speaker(
                            ui,
                            mute,
                            self.participant_audio_muted(id) || self.participant_volume(id) == 0,
                        );
                        self.snapshot.audio_buttons.push((id, mute));
                        if ui
                            .interact(mute, ui.id().with("popup-mute"), Sense::click())
                            .clicked()
                        {
                            self.toggle_audio(id);
                        }
                        let slider = Rect::from_min_max(
                            pos2(mute.right() + 4., body.top()),
                            pos2(body.right() - 44., body.bottom()),
                        );
                        self.snapshot.volume_sliders.push((id, slider));
                        let res = ui.interact(
                            slider,
                            ui.id().with("popup-level"),
                            Sense::click_and_drag(),
                        );
                        let left = slider.left() + 7.;
                        let length = (slider.width() - 14.).max(1.);
                        if (res.clicked() || res.dragged())
                            && let Some(p) = res.interact_pointer_pos()
                        {
                            let next = (((p.x - left) / length).clamp(0., 1.) * 100.).round() as u8;
                            if next != self.participant_volume(id) {
                                self.audio.set_volume(id, next);
                            }
                            if next > 0 && self.participant_audio_muted(id) {
                                self.audio.toggle(id);
                            }
                        }
                        let muted =
                            self.participant_audio_muted(id) || self.participant_volume(id) == 0;
                        let level = if muted {
                            0
                        } else {
                            self.participant_volume(id)
                        };
                        let y = slider.center().y;
                        let x = left + length * level as f32 / 100.;
                        let track =
                            Rect::from_min_max(pos2(left, y - 2.), pos2(left + length, y + 2.));
                        ui.painter()
                            .rect_filled(track, 2., Color32::from_rgb(67, 72, 88));
                        if level > 0 {
                            ui.painter().rect_filled(
                                Rect::from_min_max(track.min, pos2(x, track.bottom())),
                                2.,
                                Color32::from_rgb(149, 154, 255),
                            );
                        }
                        ui.painter().circle_filled(
                            pos2(x, y),
                            6.,
                            Color32::from_rgb(194, 196, 255),
                        );
                        ui.painter().text(
                            pos2(body.right() - 1., body.center().y),
                            Align2::RIGHT_CENTER,
                            format!("{level}%"),
                            FontId::proportional(16.),
                            if muted {
                                Color32::LIGHT_RED
                            } else {
                                Color32::WHITE
                            },
                        );
                    });
            });
        self.snapshot.volume_popup = Some(result.response.rect);
        if ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !result.response.rect.contains(p) && !anchor.contains(p))
        }) {
            self.mobile_volume_popup = None;
        }
    }
}
