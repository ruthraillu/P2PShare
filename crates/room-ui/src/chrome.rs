//! Navigation and room controls. Phone layout never depends on orientation width alone.
use crate::{Action, RoomUi, layout::Layout, theme::*};
use egui::{RichText, Ui, vec2};
impl RoomUi {
    pub(crate) fn chrome(&mut self, root: &mut Ui, layout: &Layout, actions: &mut Vec<Action>) {
        if self.platform == crate::Platform::Android {
            self.phone_chrome(root);
            return;
        }
        if !layout.mobile {
            egui::Panel::left("rail")
                .exact_size(64.0)
                .frame(egui::Frame::new().fill(BG).inner_margin(10))
                .show(root, |ui| {
                    ui.label(RichText::new("PS").size(22.0).color(ACCENT));
                });
        }
        egui::Panel::top("room_header")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(if layout.mobile { 6 } else { 14 }),
            )
            .show(root, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(&self.room_name)
                            .size(if layout.mobile { 16.0 } else { 22.0 })
                            .strong(),
                    );
                    ui.label(
                        RichText::new(p2pshare_media::trf!("{} members", self.participants.len()))
                            .small()
                            .color(MUTED),
                    );
                    if layout.mobile && ui.button(p2pshare_media::tr!("Members")).clicked() {
                        self.members_open = !self.members_open;
                        self.room_dialog = false;
                    }
                    if ui.button(p2pshare_media::tr!("Details")).clicked() {
                        self.diagnostics = !self.diagnostics;
                    }
                });
                if !layout.mobile {
                    ui.label(
                        RichText::new(if self.busy {
                            p2pshare_media::tr!("CONNECTING")
                        } else if self.in_room {
                            p2pshare_media::tr!("ENCRYPTED ROOM")
                        } else {
                            p2pshare_media::tr!("NO ROOM")
                        })
                        .small()
                        .color(GREEN),
                    );
                }
            });
        if !layout.mobile {
            egui::Panel::left("members")
                .exact_size(218.0)
                .frame(egui::Frame::new().fill(PANEL).inner_margin(16))
                .show(root, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("member_scroll")
                        .show(ui, |ui| self.sidebar(ui, actions));
                });
        }
        egui::Panel::bottom("room_controls")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(if layout.mobile { 6 } else { 14 }),
            )
            .show(root, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.capture_supported {
                        let label = if self.capture_running {
                            if layout.mobile {
                                p2pshare_media::tr!("Stop")
                            } else {
                                p2pshare_media::tr!("Stop sharing")
                            }
                        } else if layout.mobile {
                            p2pshare_media::tr!("Share")
                        } else {
                            p2pshare_media::tr!("Share screen")
                        };
                        let share = ui.add_enabled(
                            self.in_room && !self.busy,
                            egui::Button::new(label).min_size(vec2(64.0, 44.0)).fill(
                                if self.capture_running {
                                    egui::Color32::from_rgb(160, 58, 72)
                                } else {
                                    ACCENT
                                },
                            ),
                        );
                        self.snapshot.share_button = Some(share.rect);
                        if share.clicked() {
                            if self.capture_running {
                                actions.push(Action::StopCapture);
                            } else if self.platform == crate::Platform::Android {
                                actions.push(Action::StartCapture(0));
                            } else {
                                self.source_picker = true;
                                actions.push(Action::RefreshSources);
                            }
                        }
                    }
                    let grid = ui.button(if layout.mobile { "Grid" } else { p2pshare_media::tr!("All screens") });
                    self.snapshot.grid_button = Some(grid.rect);
                    if grid.clicked() {
                        self.focused = None;
                    }
                    if let Some(id) = self.focused {
                        let fullscreen = ui.button(p2pshare_media::tr!("Full screen"));
                        self.snapshot.fullscreen_button = Some(fullscreen.rect);
                        if fullscreen.clicked() {
                            self.enter_fullscreen(id);
                        }
                    }
                    if ui.button(p2pshare_media::tr!("Room")).clicked() {
                        self.room_dialog = true;
                    }
                    if !layout.mobile && self.in_room && ui.button(p2pshare_media::tr!("Leave room")).clicked() {
                        actions.push(Action::StopCapture);
                        actions.push(Action::LeaveRoom);
                    }
                });
                if !layout.landscape {
                    ui.label(RichText::new(p2pshare_media::i18n::status(&self.status)).size(12.0).color(MUTED));
                }
            });
    }
}

impl RoomUi {
    fn phone_chrome(&mut self, root: &mut Ui) {
        egui::Panel::bottom("phone_navigation")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(6))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let width = ((ui.available_width() - 20.0) / 3.0).max(44.0);
                    let screens = ui.add_sized(
                        [width, 44.0],
                        egui::Button::new(p2pshare_media::tr!("Screens"))
                            .selected(!self.members_open && !self.room_dialog),
                    );
                    self.snapshot.grid_button = Some(screens.rect);
                    if screens.clicked() {
                        self.focused = None;
                        self.members_open = false;
                        self.room_dialog = false;
                    }
                    if ui
                        .add_sized(
                            [width, 44.0],
                            egui::Button::new(p2pshare_media::trf!("Users · {}", self.participants.len())),
                        )
                        .clicked()
                    {
                        self.members_open = !self.members_open;
                        self.room_dialog = false;
                    }
                    if ui
                        .add_sized([width, 44.0], egui::Button::new(p2pshare_media::tr!("Settings")))
                        .clicked()
                    {
                        self.room_dialog = true;
                        self.members_open = false;
                    }
                });
            });
        if self.focused.is_none() {
            egui::Panel::top("phone_room")
                .frame(egui::Frame::new().fill(PANEL).inner_margin(8))
                .show(root, |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("# {}", self.room_name)).strong(),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.room_dialog = true;
                    }
                });
        }
    }
}
