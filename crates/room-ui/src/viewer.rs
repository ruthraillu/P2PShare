//! Fullscreen presentation state and overlay controls. No platform media dependencies.
use crate::{RoomUi, layout::safe_rect, video::paint_video};
use egui::{Sense, pos2};
#[derive(Default)]
pub(crate) struct ViewerState {
    pub(crate) screen: Option<u64>,
    // None means reveal on the next frame; the deadline uses egui time for testability.
    pub(crate) controls_until: Option<f64>,
}
impl RoomUi {
    pub(crate) fn fullscreen_view(&mut self, root: &mut egui::Ui) {
        let Some(p) = self
            .participants
            .iter()
            .find(|p| Some(p.id) == self.viewer.screen && p.sharing)
            .cloned()
        else {
            self.viewer.screen = None;
            return;
        };
        let ctx = root.ctx().clone();
        let now = ctx.input(|i| i.time);
        let deadline = *self.viewer.controls_until.get_or_insert(now + 3.0);
        let mut tapped = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::BLACK))
            .show(root, |ui| {
                let rect = ui.max_rect();
                self.snapshot.viewer_rect = Some(rect);
                self.snapshot.content_rect = Some(rect);
                paint_video(ui.painter(), rect, &p);
                tapped = ui
                    .interact(rect, ui.id().with("viewer_surface"), Sense::click())
                    .clicked();
            });
        if tapped {
            self.viewer.controls_until = Some(if deadline > now { now } else { now + 3.0 });
        }
        if self.viewer.controls_until.is_some_and(|t| t > now) {
            let safe = safe_rect(root.max_rect(), self.safe_insets).shrink(8.0);
            let area = egui::Area::new(egui::Id::new("viewer_controls"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(safe.left(), (safe.bottom() - 68.0).max(safe.top())))
                .constrain_to(safe);
            let response = area.show(&ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_black_alpha(225))
                    .inner_margin(8)
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_width((safe.width() - 16.0).max(160.0));
                        ui.horizontal(|ui| {
                            let exit = ui.add_sized([48.0, 44.0], egui::Button::new(p2pshare_media::tr!("Back")));
                            self.snapshot.exit_fullscreen_button = Some(exit.rect);
                            if exit.clicked() {
                                self.viewer.screen = None;
                            }
                            self.volume_controls(ui, p.id);
                        });
                    });
            });
            if response.response.contains_pointer() && ctx.input(|i| i.pointer.any_down()) {
                self.viewer.controls_until = Some(now + 3.0);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // The image always stays fitted to the available display, including rotation.
    }
}
