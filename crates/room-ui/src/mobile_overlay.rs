//! Tap-revealed controls and per-stream swipe volume, with no media pipeline ownership.
use crate::{Action, Participant, RoomUi, theme::*};
use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};
impl RoomUi {
    pub(crate) fn mobile_fullscreen(&mut self, root: &mut Ui, actions: &mut Vec<Action>) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::BLACK))
            .show(root, |ui| {
                let rect = ui.max_rect();
                self.snapshot.viewer_rect = Some(rect);
                self.snapshot.content_rect = Some(rect);
                if let Some(p) = self
                    .participants
                    .iter()
                    .find(|p| Some(p.id) == self.focused)
                    .cloned()
                {
                    crate::video::paint_video(ui.painter(), rect, &p);
                    self.mobile_overlay(ui, rect, &p, actions);
                }
            });
    }
    pub(crate) fn mobile_overlay(
        &mut self,
        ui: &mut Ui,
        video: Rect,
        p: &Participant,
        actions: &mut Vec<Action>,
    ) {
        // Overlay widgets must not move the gallery row's allocation cursor.
        let mut overlay_ui = ui.new_child(egui::UiBuilder::new()
            .id_salt(("mobile_overlay", p.id)).max_rect(video));
        let ui = &mut overlay_ui;
        let now = ui.input(|i| i.time);
        let surface = ui.interact(
            video,
            ui.id().with(("swipe_video", p.id)),
            if self.is_fullscreen(){Sense::click_and_drag()}else{Sense::click()},
        );
        if self.is_fullscreen() {self.mobile_volume_drag(ui,&surface,video,p);}
        if !self.is_fullscreen() && self.mobile_swipe.is_some() && ui.input(|i|i.pointer.primary_down()) {self.mobile_controls_until=now+3.0;}
        if surface.clicked() {
            self.mobile_controls_until = if self.mobile_controls_until > now {
                0.0
            } else {
                now + 3.0
            };
        }
        if self.mobile_volume_until>now {
            let hud=Rect::from_center_size(pos2(video.right()-42.0,video.center().y),vec2(72.0,44.0));
            ui.painter().rect_filled(hud,8.0,PANEL);
            ui.painter().text(hud.center(),Align2::CENTER_CENTER,format!("{}%",self.participant_volume(p.id)),FontId::proportional(18.0),Color32::WHITE);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.mobile_controls_until <= now {
            return;
        }
        let safe = if self.is_fullscreen() {
            crate::layout::safe_rect(video, self.safe_insets)
        } else {
            video
        }
        .shrink(4.0);
        let width = 248.0_f32.min(safe.width());
        let rect = Rect::from_center_size(
            pos2(safe.center().x, safe.bottom() - 26.0),
            vec2(width, 48.0),
        );
        ui.painter().rect_filled(rect, 10.0, PANEL);
        let unit = width / 5.0;
        let muted = self.participant_audio_muted(p.id) || self.participant_volume(p.id) == 0;
        let color = if muted {
            Color32::LIGHT_RED
        } else {
            Color32::WHITE
        };
        let mute_rect = Rect::from_min_size(rect.min, vec2(unit * 2.0, 48.0));
        let response = ui.interact(mute_rect, ui.id().with("overlay_mute"), Sense::click_and_drag());
        self.snapshot.audio_buttons.push((p.id, mute_rect));
        self.mobile_volume_drag(ui,&response,video,p);
        let c = pos2(rect.left() + unit * 0.45, rect.center().y);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                c + vec2(-9.0, -4.0),
                c + vec2(-4.0, -4.0),
                c + vec2(3.0, -9.0),
                c + vec2(3.0, 9.0),
                c + vec2(-4.0, 4.0),
                c + vec2(-9.0, 4.0),
            ],
            color,
            egui::Stroke::NONE,
        ));
        if muted {
            ui.painter().line_segment(
                [c + vec2(-10.0, 11.0), c + vec2(11.0, -11.0)],
                egui::Stroke::new(2.0, color),
            );
        }
        ui.painter().text(
            pos2(rect.left() + unit * 1.35, rect.center().y),
            Align2::CENTER_CENTER,
            format!(
                "{}%",
                if muted {
                    0
                } else {
                    self.participant_volume(p.id)
                }
            ),
            FontId::proportional(15.0),
            color,
        );
        if response.clicked() {
            self.toggle_audio(p.id);
            self.mobile_controls_until = now + 3.0;
        }
        for (index, _label) in [(2, "PiP"), (3, "■"), (4, "⛶")] {
            let r = Rect::from_min_size(
                pos2(rect.left() + unit * index as f32, rect.top()),
                vec2(unit, 48.0),
            );
            let b = ui.put(r, egui::Button::new("").frame(false));
            let c = r.center();
            let painter = ui.painter();
            if index == 3 {
                painter.rect_filled(
                    Rect::from_center_size(c, vec2(12.0, 12.0)),
                    2.0,
                    Color32::LIGHT_RED,
                );
            } else if index == 2 {
                let b = Rect::from_center_size(c, vec2(22.0, 15.0));
                painter.rect_stroke(
                    b,
                    2.0,
                    egui::Stroke::new(1.5, Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
                painter.rect_filled(Rect::from_min_size(c, vec2(9.0, 6.0)), 1.0, Color32::WHITE);
            } else {
                for x in [-1.0, 1.0] {
                    for y in [-1.0, 1.0] {
                        painter.add(egui::Shape::line(
                            vec![
                                c + vec2(x * 4.0, y * 8.0),
                                c + vec2(x * 8.0, y * 8.0),
                                c + vec2(x * 8.0, y * 4.0),
                            ],
                            egui::Stroke::new(1.5, Color32::WHITE),
                        ));
                    }
                }
            }
            match index {
                2 => self.snapshot.pip_button = Some(r),
                3 => self.snapshot.stop_button = Some(r),
                _ => {
                    self.snapshot.fullscreen_button = Some(r);
                    if self.is_fullscreen() {
                        self.snapshot.exit_fullscreen_button = Some(r);
                    }
                }
            }
            if b.clicked() {
                self.mobile_controls_until = now + 3.0;
                match index {
                    2 => actions.push(Action::PictureInPicture),
                    3 => {
                        self.watching.remove(&p.id);
                        self.focused = None;
                        self.viewer.screen = None;
                    }
                    _ => {
                        if self.is_fullscreen() {
                            self.viewer.screen = None;
                        } else {
                            self.enter_fullscreen(p.id);
                        }
                    }
                }
            }
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    fn mobile_volume_drag(&mut self,ui:&mut Ui,surface:&egui::Response,video:Rect,p:&Participant) {
        let now=ui.input(|i|i.time);
        if surface.drag_started() {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                self.mobile_swipe = Some((p.id, self.participant_volume(p.id), origin.y, video.height().max(100.0)));
            }
        }
        if surface.dragged()
            && let Some((id, start, origin_y, height)) = self.mobile_swipe
            && id == p.id
            && let Some(pointer) = surface.interact_pointer_pos()
        {
            // drag_delta() is per-frame. Anchor to the press position and freeze sensitivity
            // for the gesture so variable frame rates and stationary frames cannot undo it.
            let change = (origin_y - pointer.y) / height * 100.0;
            let volume = (start as f32 + change).round().clamp(0.0, 100.0) as u8;
            if volume != self.participant_volume(p.id) {
                self.audio.set_volume(p.id, volume);
                if volume > 0 && self.participant_audio_muted(p.id) {
                    self.toggle_audio(p.id);
                }
            }
            self.mobile_volume_until=now+1.0;
            if self.mobile_controls_until>now {self.mobile_controls_until=now+3.0;}
        }
        if surface.drag_stopped() {
            self.mobile_swipe = None;
        }
    }
}
