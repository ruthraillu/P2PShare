//! Per-viewer audio preferences. No fake audio pipeline or changes to publishers.
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub(crate) struct AudioPreferences {
    muted: HashSet<u64>,
    levels: HashMap<u64, u8>,
    pub(crate) volumes: Vec<(u64, u8)>,
    pub(crate) pending: Vec<(u64, bool)>,
}
impl AudioPreferences {
    pub(crate) fn is_muted(&self, id: u64) -> bool {
        self.muted.contains(&id)
    }
    pub(crate) fn toggle(&mut self, id: u64) {
        let muted = if self.muted.remove(&id) {
            false
        } else {
            self.muted.insert(id);
            true
        };
        self.pending.push((id, muted));
    }
    pub(crate) fn retain(&mut self, ids: impl Iterator<Item = u64>) {
        let ids: HashSet<_> = ids.collect();
        self.muted.retain(|id| ids.contains(id));
        self.levels.retain(|id, _| ids.contains(id));
    }
}
impl crate::RoomUi {
    pub(crate) fn toggle_audio(&mut self, id: u64) {
        if self.participant_volume(id) == 0 {
            self.audio.levels.insert(id, 100);
            self.audio.volumes.push((id, 100));
            if self.audio.is_muted(id) {
                self.audio.toggle(id);
            }
        } else {
            self.audio.toggle(id);
        }
    }
}

impl crate::RoomUi {
    pub fn participant_volume(&self, id: u64) -> u8 {
        self.audio.levels.get(&id).copied().unwrap_or(100)
    }
    pub(crate) fn volume_controls(&mut self, ui: &mut egui::Ui, id: u64) {
        let muted = self.audio.is_muted(id) || self.participant_volume(id) == 0;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::click());
        self.snapshot.audio_buttons.push((id, rect));
        let color = if muted {
            egui::Color32::from_rgb(235, 101, 113)
        } else {
            egui::Color32::from_rgb(220, 222, 230)
        };
        let p = ui.painter();
        let c = rect.center();
        p.rect_stroke(
            egui::Rect::from_center_size(c - egui::vec2(5.0, 0.0), egui::vec2(6.0, 8.0)),
            0.0,
            egui::Stroke::new(1.7, color),
            egui::StrokeKind::Inside,
        );
        p.add(egui::Shape::closed_line(
            vec![
                c + egui::vec2(-2.0, -4.0),
                c + egui::vec2(4.0, -9.0),
                c + egui::vec2(4.0, 9.0),
                c + egui::vec2(-2.0, 4.0),
            ],
            egui::Stroke::new(1.7, color),
        ));
        if muted {
            p.line_segment(
                [c + egui::vec2(-11.0, 11.0), c + egui::vec2(11.0, -11.0)],
                egui::Stroke::new(2.0, color),
            );
        } else {
            p.add(egui::Shape::line(
                vec![
                    c + egui::vec2(8.0, -7.0),
                    c + egui::vec2(11.0, 0.0),
                    c + egui::vec2(8.0, 7.0),
                ],
                egui::Stroke::new(1.7, color),
            ));
        }
        if response.clicked() {
            self.toggle_audio(id);
        }
        let mut volume = self.participant_volume(id);
        let available = (ui.available_width() - 50.0).clamp(44.0, 180.0);
        let (slider, response) =
            ui.allocate_exact_size(egui::vec2(available, 44.0), egui::Sense::click_and_drag());
        self.snapshot.volume_sliders.push((id, slider));
        let left = slider.left() + 8.0;
        let width = (slider.width() - 16.0).max(1.0);
        if (response.dragged() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let next = (((pos.x - left) / width).clamp(0.0, 1.0) * 100.0).round() as u8;
            if next != volume {
                volume = next;
                self.audio.levels.insert(id, volume);
                self.audio.volumes.push((id, volume));
            }
        }
        let center = slider.center().y;
        let track =
            egui::Rect::from_min_size(egui::pos2(left, center - 3.0), egui::vec2(width, 6.0));
        ui.painter()
            .rect_filled(track, 3.0, egui::Color32::from_rgb(67, 70, 84));
        let x = left + width * volume as f32 / 100.0;
        if volume > 0 {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(track.min, egui::pos2(x, track.bottom())),
                3.0,
                crate::theme::ACCENT,
            );
        }
        ui.painter().circle_filled(
            egui::pos2(x, center),
            7.0,
            egui::Color32::from_rgb(225, 227, 235),
        );
        ui.label(egui::RichText::new(format!("{volume}%")).size(16.0));
    }
}
impl AudioPreferences {
    pub(crate) fn set_volume(&mut self,id:u64,value:u8){self.levels.insert(id,value);self.volumes.push((id,value));}
}
