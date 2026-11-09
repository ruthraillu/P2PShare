use crate::{Action, RoomUi, theme::*};
use egui::RichText;
impl RoomUi {
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context, mobile: bool, actions: &mut Vec<Action>) {
        if self.members_open && mobile {
            egui::Window::new(p2pshare_media::tr!("Users"))
                .collapsible(false)
                .resizable(false)
                .default_width(270.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 88.0).max(80.0))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("mobile_members")
                                .max_height((ctx.content_rect().height() - 200.0).max(160.0))
                                .show(ui, |ui| self.sidebar(ui, actions));
                            if ui.button(p2pshare_media::tr!("Close")).clicked() {
                                self.members_open = false;
                            }
                        });
                });
        }
        if self.source_picker {
            egui::Window::new(p2pshare_media::tr!("Share your screen"))
                .collapsible(false)
                .resizable(true)
                .default_width(480.0_f32.min(ctx.content_rect().width() - 24.0))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 88.0).max(80.0))
                        .show(ui, |ui| {
                            ui.label(p2pshare_media::tr!("Choose a monitor or application to publish to this room."));
                            if ui.button(p2pshare_media::tr!("Refresh sources")).clicked() {
                                actions.push(Action::RefreshSources);
                            }
                            egui::ScrollArea::vertical()
                                .max_height(300.0)
                                .show(ui, |ui| {
                                    for source in &self.sources {
                                        if ui
                                            .selectable_label(
                                                self.selected_source == Some(source.id),
                                                format!(
                                                    "{}  •  {} × {}",
                                                    source.title,
                                                    source.size.width,
                                                    source.size.height
                                                ),
                                            )
                                            .clicked()
                                        {
                                            self.selected_source = Some(source.id);
                                        }
                                    }
                                });
                            if self.sources.is_empty() {
                                ui.label(
                                    p2pshare_media::tr!("No capture sources found. Refresh after opening a window."),
                                );
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        self.selected_source.is_some_and(|id| {
                                            self.sources.iter().any(|s| s.id == id)
                                        }),
                                        egui::Button::new(p2pshare_media::tr!("Start sharing")).fill(ACCENT),
                                    )
                                    .clicked()
                                {
                                    if let Some(id) = self.selected_source {
                                        actions.push(Action::StartCapture(id));
                                    }
                                    self.source_picker = false;
                                }
                                if ui.button(p2pshare_media::tr!("Cancel")).clicked() {
                                    self.source_picker = false;
                                }
                            });
                        });
                });
        }
        if self.room_dialog {
            egui::Window::new(p2pshare_media::tr!("Settings"))
                .collapsible(false)
                .default_width(440.0_f32.min(ctx.content_rect().width() - 24.0))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 88.0).max(80.0))
                        .show(ui, |ui| {
                            ui.label(p2pshare_media::tr!("Display name"));
                            if cfg!(target_os = "android") {
                                ui.horizontal(|ui| {
                                    ui.label(&self.name);
                                    if ui.button(p2pshare_media::tr!("Change name")).clicked() {
                                        actions.push(Action::EditName(self.name.clone()));
                                    }
                                });
                            } else {
                                ui.add(egui::TextEdit::singleline(&mut self.name).char_limit(32));
                            }
                            if !self.capture_supported { ui.label(p2pshare_media::tr!("Watch screens shared by people in your room.")); } else if self.video_config_external { ui.label(p2pshare_media::tr!("Stream settings: config.toml beside the EXE (restart to apply).")); } else {
                            ui.label(p2pshare_media::tr!("Sharing quality"));
                            let before = self.quality;
                            ui.add_enabled_ui(!self.capture_running, |ui| {
                                egui::ComboBox::from_id_salt("sharing_quality").selected_text(self.quality.label()).show_ui(ui, |ui| {
                                    for quality in p2pshare_media::VideoQuality::ALL { ui.selectable_value(&mut self.quality, quality, quality.label()); }
                                });
                            });
                            if before != self.quality { actions.push(Action::VideoQuality(self.quality)); }
                            if self.capture_running { ui.label(p2pshare_media::tr!("Stop sharing to change quality.")); }
                            }
                            if !self.in_room {
                                ui.label(p2pshare_media::tr!("Room name"));
                                if cfg!(target_os="android") {
                                    ui.label(&self.room_name);
                                    if ui.button(p2pshare_media::tr!("Change room name")).clicked(){actions.push(Action::EditRoom(self.room_name.clone()));}
                                }else{ui.add(egui::TextEdit::singleline(&mut self.room_name).char_limit(64));}
                                ui.label(p2pshare_media::tr!("Anyone using this room name and signaling server can connect."));
                                if ui.add_enabled(!self.busy&&!self.room_name.trim().is_empty(),egui::Button::new(p2pshare_media::tr!("Join room")).fill(ACCENT)).clicked(){
                                    actions.push(Action::NamedRoom{room:self.room_name.clone(),name:self.name.clone()});self.room_dialog=false;
                                }
                                if let Some(invite)=&self.saved_invitation && ui.add_enabled(!self.busy,egui::Button::new(p2pshare_media::tr!("Rejoin last room"))).clicked(){actions.push(Action::JoinRoom{invite:invite.clone(),name:self.name.clone()});self.room_dialog=false;}
                            }else{ui.label(p2pshare_media::trf!("Connected to {}",self.room_name));}
                            if cfg!(target_os="android")&&ui.button(p2pshare_media::tr!("Edit config.toml")).clicked(){actions.push(Action::EditConfig);}
                            if (self.in_room || self.invitation.is_some()) && ui.button(p2pshare_media::tr!("Leave room")).clicked() {
                                actions.push(Action::StopCapture);
                                actions.push(Action::LeaveRoom);
                                self.room_dialog = false;
                            }
                            ui.label(RichText::new(p2pshare_media::i18n::status(&self.status)).small().color(MUTED));
                            if ui.button(p2pshare_media::tr!("Diagnostics & logs")).clicked() { self.diagnostics = true; self.room_dialog = false; }
                            if ui.button(p2pshare_media::tr!("Close")).clicked() {
                                self.room_dialog = false;
                            }
                        });
                });
        }
        if self.diagnostics {
            egui::Window::new(p2pshare_media::tr!("Build details"))
                .collapsible(false)
                .default_width(340.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 88.0).max(80.0))
                        .show(ui, |ui| {
                            ui.label(p2pshare_media::trf!(
                                "P2PShare {} • viewer diagnostics",
                                env!("CARGO_PKG_VERSION")
                            ));
                            ui.label(p2pshare_media::tr!("Up to 6 devices • encrypted WebRTC video and Opus audio."));
                            ui.label(if self.platform == crate::Platform::Desktop {
                                "Windows: WGC / D3D11 capture"
                            } else {
                                p2pshare_media::tr!("Android: viewer only • MediaCodec")
                            });
                            if !self.encoder.is_empty() {
                                ui.label(&self.encoder);
                            }
                            if self.capture_running {
                                ui.label(p2pshare_media::trf!("Local capture: {:.1} FPS", self.capture_fps));
                            }
                            if !self.playback_diagnostics.is_empty() {
                                ui.label(&self.playback_diagnostics);
                            }
                            if self.platform == crate::Platform::Android {
                                if ui
                                    .button(if self.logging_enabled {
                                        p2pshare_media::tr!("Disable logging")
                                    } else {
                                        p2pshare_media::tr!("Enable logging")
                                    })
                                    .clicked()
                                {
                                    self.logging_enabled = !self.logging_enabled;
                                    actions.push(Action::DebugLogging(self.logging_enabled));
                                }
                                if ui.button(p2pshare_media::tr!("Copy log")).clicked() {
                                    actions.push(Action::CopyDebugLog);
                                }
                                ui.label(p2pshare_media::tr!("Logging keeps a bounded recent session buffer."));
                            }
                            ui.label(p2pshare_media::tr!("Tap a screen to focus. All screens returns to the grid."));
                            if ui.button(p2pshare_media::tr!("Close")).clicked() {
                                self.diagnostics = false;
                            }
                        });
                });
        }
    }
}
