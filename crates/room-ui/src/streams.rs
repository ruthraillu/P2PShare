//! Grid and focused layouts. Fullscreen uses a separate chrome-free renderer.
use crate::{Action, RoomUi, layout::Layout, theme::*};
use egui::{RichText, Ui, vec2};
impl RoomUi {
    pub(crate) fn streams(&mut self, root: &mut Ui, layout: &Layout, _actions: &mut Vec<Action>) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(if layout.mobile { 6 } else { 20 }),
            )
            .show(root, |ui| {
                self.snapshot.content_rect = Some(ui.max_rect());
                let participants = self.participants.clone();
                let streams: Vec<_> = participants.iter().filter(|p| p.sharing).collect();
                if streams.is_empty() {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(if layout.landscape { 4.0 } else { 20.0 });
                            ui.heading(if self.capture_supported {
                                "A space to share what you're doing"
                            } else {
                                p2pshare_media::tr!("Your screens, together")
                            });
                            ui.label(
                                RichText::new(if self.in_room {
                                    if self.capture_supported {
                                        p2pshare_media::tr!("Share your screen. Other devices can share too.")
                                    } else {
                                        p2pshare_media::tr!("Waiting for another device to share its screen.")
                                    }
                                } else {
                                    p2pshare_media::tr!("Join using the same room name on each device.")
                                })
                                .color(MUTED),
                            );
                            if !self.in_room {
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add_enabled(
                                            !self.busy,
                                            egui::Button::new(p2pshare_media::tr!("Join room")).fill(ACCENT),
                                        )
                                        .clicked()
                                    {
                                        self.room_dialog = true;
                                    }
                                });
                            }
                        });
                    });
                    return;
                }
                if !layout.landscape && self.platform != crate::Platform::Android {
                    ui.label(
                        RichText::new(if self.focused.is_some() {
                            p2pshare_media::tr!("FOCUSED SCREEN")
                        } else {
                            p2pshare_media::tr!("LIVE SCREENS")
                        })
                        .size(12.0)
                        .color(MUTED),
                    );
                }
                egui::ScrollArea::vertical()
                    .id_salt(("screens", self.focused))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(id) = self.focused {
                            if let Some(p) = streams.iter().find(|p| p.id == id) {
                                let filmstrip = !layout.mobile && streams.len() > 1;
                                let portrait_phone =
                                    self.platform == crate::Platform::Android && !layout.landscape;
                                let height = if portrait_phone {
                                    (ui.available_width() * p.size.height.max(1) as f32
                                        / p.size.width.max(1) as f32
                                        + 100.0)
                                        .min(ui.available_height())
                                        .max(150.0)
                                } else {
                                    (ui.available_height() - if filmstrip { 110.0 } else { 0.0 })
                                        .max(60.0)
                                };
                                self.tile(ui, p, vec2(ui.available_width(), height), true);
                                if portrait_phone {
                                    for other in streams.iter().filter(|other| other.id != id) {
                                        let footer = if self.watching.contains(&other.id) {
                                            100.0
                                        } else {
                                            54.0
                                        };
                                        let height = ui.available_width() * 9.0 / 16.0 + footer;
                                        self.tile(
                                            ui,
                                            other,
                                            vec2(ui.available_width(), height),
                                            false,
                                        );
                                    }
                                }
                                if filmstrip {
                                    egui::ScrollArea::horizontal().id_salt("filmstrip").show(
                                        ui,
                                        |ui| {
                                            ui.horizontal(|ui| {
                                                for p in &streams {
                                                    self.tile(
                                                        ui,
                                                        p,
                                                        vec2(160.0, 100.0),
                                                        p.id == id,
                                                    );
                                                }
                                            });
                                        },
                                    );
                                }
                            }
                        } else {
                            let width = ui.available_width();
                            let gap = 10.0;
                            let columns = ((width / if layout.landscape { 230.0 } else { 330.0 })
                                .floor() as usize)
                                .clamp(1, 3)
                                .min(streams.len());
                            let tile_width =
                                ((width - gap * (columns - 1) as f32) / columns as f32).max(1.0);
                            let rows = streams.len().div_ceil(columns);
                            let natural_height = tile_width * 9.0 / 16.0
                                + if streams.iter().any(|p| self.watching.contains(&p.id)) {
                                    100.0
                                } else {
                                    54.0
                                };
                            let tile_height = if self.platform == crate::Platform::Android {
                                natural_height
                            } else {
                                (tile_width * 9.0 / 16.0 + 46.0).min(
                                    ((ui.available_height() - gap * (rows - 1) as f32)
                                        / rows as f32)
                                        .max(if layout.landscape { 96.0 } else { 165.0 }),
                                )
                            };
                            for row in streams.chunks(columns) {
                                ui.horizontal(|ui| {
                                    for p in row {
                                        self.tile(ui, p, vec2(tile_width, tile_height), false);
                                    }
                                });
                            }
                        }
                    });
            });
    }
}
