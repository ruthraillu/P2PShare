//! Phone pages: editable controls are aligned explicitly, independent of font metrics.
use crate::{Action, RoomUi, theme::*};
use egui::{RichText, Ui, vec2};
fn field(ui: &mut Ui, label: &str, value: &mut String, limit: usize) {
    ui.label(RichText::new(label).color(MUTED));
    ui.add_sized(
        [ui.available_width(), 44.0],
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .vertical_align(egui::Align::Center)
            .margin(vec2(10.0, 0.0)),
    );
}
impl RoomUi {
    pub(crate) fn mobile_pages(&mut self, root: &mut Ui, actions: &mut Vec<Action>) -> bool {
        if self.in_room && !self.room_dialog && !self.members_open {
            return false;
        }
        let landscape=root.max_rect().width()>root.max_rect().height();
        if self.in_room {
            if landscape { let layout=crate::layout::Layout::new(root.max_rect(),self.platform);self.mobile_navigation(root,&layout,actions); } else {self.mobile_bottom_navigation(root);}
            if let Some(p) = self
                .playback_participant()
                .cloned()
            {
                egui::Panel::bottom("return_to_stream")
                    .frame(egui::Frame::new().fill(PANEL).inner_margin(8))
                    .show(root, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            vec2(ui.available_width(), 48.0),
                            egui::Sense::click(),
                        );
                        let thumb = egui::Rect::from_min_size(rect.min, vec2(80.0, 45.0));
                        crate::video::paint_video(ui.painter(), thumb, &p);
                        ui.painter().text(
                            rect.left_center() + vec2(90.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            p2pshare_media::trf!("Watching {}  ›", p.name),
                            egui::FontId::proportional(14.0),
                            egui::Color32::WHITE,
                        );
                        if response.clicked() {
                            self.room_dialog = false;
                            self.members_open = false;
                        }
                    });
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(12)).show(root,|ui|{
            ui.visuals_mut().override_text_color=Some(egui::Color32::from_rgb(243,244,250));
            ui.visuals_mut().extreme_bg_color=PANEL;
            egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui,|ui|{
                if !self.in_room || self.room_dialog {
                    self.android_update_notice(ui, actions);
                }
                if self.signaling_setup {
                    ui.heading(p2pshare_media::tr!("Signaling server"));
                    ui.label(RichText::new(p2pshare_media::tr!("Enter the WebSocket URL your group uses.")).color(MUTED));
                    ui.add_space(12.0);
                    field(ui, "WebSocket URL", &mut self.signaling_server, 2048);
                    ui.add_space(12.0);
                    if ui.add_enabled(!self.signaling_server.trim().is_empty(),
                        egui::Button::new(p2pshare_media::tr!("Continue")).fill(ACCENT).min_size(vec2(ui.available_width(),44.0))).clicked()
                    {
                        actions.push(Action::ConfigureSignaling(self.signaling_server.clone()));
                    }
                    if !self.signaling_setup_error.is_empty() {
                        ui.label(RichText::new(&self.signaling_setup_error).color(egui::Color32::LIGHT_RED));
                    }
                } else if self.room_dialog {
                    ui.horizontal(|ui|{
                        if !self.in_room && ui.button("‹").clicked(){self.room_dialog=false;}
                        ui.heading(p2pshare_media::tr!("Settings"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{if ui.button(p2pshare_media::tr!("Save")).clicked(){actions.push(Action::SaveMobilePreferences{buffer_ms:self.playback_buffer_ms,server:self.signaling_server.clone()});}});
                    });
                    self.mobile_avatar_settings(ui,actions);
                    if landscape {
                        self.mobile_landscape_settings(ui,actions);
                    } else { self.mobile_profile_options(ui);if self.capture_supported && self.in_room && ui.button(p2pshare_media::tr!("Stream settings")).clicked(){actions.push(Action::StartCapture(0));}self.mobile_advanced_options(ui,actions); }
                    ui.label(RichText::new(p2pshare_media::i18n::status(&self.status)).small().color(MUTED));
                } else if self.members_open && self.in_room {
                    ui.horizontal(|ui|{ui.heading(p2pshare_media::tr!("Users"));if landscape {ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{if ui.button(RichText::new(p2pshare_media::tr!("Leave room")).color(egui::Color32::LIGHT_RED)).clicked(){actions.push(Action::LeaveRoom);self.members_open=false;self.focused=None;self.watching.clear();}});}});ui.label(RichText::new(p2pshare_media::trf!("{} · {} users",self.room_name,self.participants.len())).color(MUTED));
                    let people=self.participants.clone();let columns=if ui.available_width()>600.0 {2}else{1};
                    let width=(ui.available_width()-10.0*(columns-1) as f32)/columns as f32;
                    for row in people.chunks(columns) {ui.horizontal_top(|ui| {for person in row {self.mobile_member(ui,person,width);}});}
                    ui.add_space(16.0);if !landscape && ui.add_sized([ui.available_width(),44.0],egui::Button::new(RichText::new(p2pshare_media::tr!("Leave room")).color(egui::Color32::LIGHT_RED))).clicked(){actions.push(Action::LeaveRoom);self.members_open=false;self.focused=None;self.watching.clear();}
                } else {
                    ui.horizontal(|ui|{ui.heading("P2PShare");if ui.button(p2pshare_media::tr!("Settings")).clicked(){self.room_dialog=true;}});
                    ui.add_space(26.0);ui.heading(p2pshare_media::tr!("Join a room"));ui.label(RichText::new(p2pshare_media::tr!("Use the same room name as your friends.")).color(MUTED));ui.add_space(14.0);
                    if ui.available_width()>600.0 {ui.columns(2,|columns| {field(&mut columns[0],p2pshare_media::tr!("Room name"),&mut self.room_name,64);field(&mut columns[1],p2pshare_media::tr!("Display name"),&mut self.name,32);});}
                    else {field(ui,p2pshare_media::tr!("Room name"),&mut self.room_name,64);ui.add_space(12.0);field(ui,p2pshare_media::tr!("Display name"),&mut self.name,32);}
                    ui.add_space(12.0);
                    if ui.add_enabled(!self.busy&&!self.room_name.trim().is_empty()&&!self.name.trim().is_empty(),egui::Button::new(p2pshare_media::tr!("Join room")).fill(ACCENT).min_size(vec2(ui.available_width(),44.0))).clicked(){actions.push(Action::NamedRoom{room:self.room_name.clone(),name:self.name.clone()});}
                    if !self.signaling_setup_error.is_empty() {
                        ui.label(RichText::new(&self.signaling_setup_error).color(egui::Color32::LIGHT_RED));
                    }
                    ui.label(RichText::new(p2pshare_media::i18n::status(&self.status)).small().color(MUTED));
                }
            });
        });
        true
    }
    fn android_update_notice(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let Some(version) = self.android_update_version.as_deref() else { return };
        let label = format!(
            "{} · v{}  ›  {}",
            p2pshare_media::tr!("Update available"),
            version,
            p2pshare_media::tr!("View release")
        );
        let button = egui::Button::new(RichText::new(label).size(13.0).color(ACCENT))
            .fill(PANEL)
            .stroke(egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.55)));
        if ui.add_sized([ui.available_width(), 40.0], button).clicked() {
            actions.push(Action::ViewAndroidUpdate);
        }
        ui.add_space(6.0);
    }

    fn mobile_landscape_settings(&mut self,ui:&mut Ui,actions:&mut Vec<Action>) {
        ui.spacing_mut().interact_size.y=28.0;
        for index in 0..if self.capture_supported&&self.in_room{5}else{4} {
            ui.add_space(6.0);
            egui::Frame::new().fill(PANEL).corner_radius(8).inner_margin(10).show(ui,|ui|{
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui|{
                    ui.set_min_height(26.0);
                    ui.label([p2pshare_media::tr!("Display name"),p2pshare_media::tr!("Playback buffer"),p2pshare_media::tr!("Debug logging"),p2pshare_media::tr!("Copy log"),p2pshare_media::tr!("Stream settings")][index]);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{match index {
                        0=>{ui.add_sized([ui.available_width().min(260.0),30.0],egui::TextEdit::singleline(&mut self.name).char_limit(32).horizontal_align(egui::Align::Max).vertical_align(egui::Align::Center).margin(vec2(10.0,0.0)));},
                        1=>{ui.add(egui::DragValue::new(&mut self.playback_buffer_ms).range(0..=1000).suffix(" ms"));},
                        2=>{
                            let (r,response)=ui.allocate_exact_size(vec2(40.0,24.0),egui::Sense::click());
                            if response.clicked(){self.logging_enabled=!self.logging_enabled;actions.push(Action::DebugLogging(self.logging_enabled));}
                            let track=r.shrink2(vec2(1.0,2.0));ui.painter().rect_filled(track,10.0,if self.logging_enabled{ACCENT}else{MUTED.gamma_multiply(0.4)});
                            ui.painter().circle_filled(egui::pos2(if self.logging_enabled{track.right()-10.0}else{track.left()+10.0},track.center().y),8.0,egui::Color32::WHITE);
                        },
                        4=>{if ui.button(p2pshare_media::tr!("Open")).clicked(){actions.push(Action::StartCapture(0));}},
                        _=>{if ui.button(p2pshare_media::tr!("Copy")).clicked(){actions.push(Action::CopyDebugLog);}}
                    }});
                });
            });
        }
        self.mobile_diagnostics(ui, actions);
        ui.add_space(6.0);
        egui::Frame::new().fill(PANEL).corner_radius(8).inner_margin(10).show(ui,|ui|{
            ui.set_min_width(ui.available_width());
            egui::CollapsingHeader::new(p2pshare_media::tr!("Advanced")).show(ui,|ui|{
                field(ui,p2pshare_media::tr!("Signaling server"),&mut self.signaling_server,2048);
                ui.label(RichText::new(p2pshare_media::tr!("Use the same server on every device. Restart after changing it.")).small().color(MUTED));
                if ui.button(p2pshare_media::tr!("Edit config.toml")).clicked(){actions.push(Action::EditConfig);}
            });
        });
    }
    fn mobile_profile_options(&mut self,ui:&mut Ui) {
                    ui.add_space(10.0);ui.label(RichText::new(p2pshare_media::tr!("PROFILE")).color(MUTED));field(ui,p2pshare_media::tr!("Display name"),&mut self.name,32);
                    ui.add_space(12.0);ui.label(RichText::new(p2pshare_media::tr!("PLAYBACK")).color(MUTED));
                    ui.horizontal(|ui|{ui.label(p2pshare_media::tr!("Playback buffer"));ui.add(egui::DragValue::new(&mut self.playback_buffer_ms).range(0..=1000).suffix(" ms"));});
                    ui.label(RichText::new(p2pshare_media::tr!("Extra synchronized audio/video delay")).small().color(MUTED));
    }
    fn mobile_advanced_options(&mut self,ui:&mut Ui,actions:&mut Vec<Action>) {
                    ui.add_space(12.0);ui.label(RichText::new(p2pshare_media::tr!("DIAGNOSTICS")).color(MUTED));
                    if ui.add_sized([ui.available_width(),44.0],egui::Checkbox::new(&mut self.logging_enabled,p2pshare_media::tr!("Debug logging"))).changed(){actions.push(Action::DebugLogging(self.logging_enabled));}
                    if ui.add_sized([ui.available_width(),44.0],egui::Button::new(p2pshare_media::tr!("Copy log"))).clicked(){actions.push(Action::CopyDebugLog);}
                    self.mobile_diagnostics(ui, actions);
                    ui.add_space(12.0);egui::CollapsingHeader::new(p2pshare_media::tr!("Advanced")).show(ui,|ui|{
                        field(ui,p2pshare_media::tr!("Signaling server"),&mut self.signaling_server,2048);
                        ui.label(RichText::new(p2pshare_media::tr!("Use the same server on every device. Restart after changing it.")).small().color(MUTED));
                        if ui.button(p2pshare_media::tr!("Edit config.toml")).clicked(){actions.push(Action::EditConfig);}
                    });
    }
    fn mobile_diagnostics(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        use p2pshare_media::{tr, i18n::status};
        let state = &mut self.diagnostic_panel;
        ui.add_space(8.);
        egui::CollapsingHeader::new(tr!("Recent runs")).default_open(true).show(ui, |ui| {
            if state.runs.is_empty() {
                ui.label(tr!("No diagnostic runs yet."));
            }
            for run in &state.runs {
                ui.radio_value(&mut state.selected, Some(run.id.clone()),
                    format!("{} · {}", run.time, status(run.status)));
            }
            if !state.configured {
                ui.label(tr!("Set a secure signaling server to send diagnostics."));
            }
            let selected = state.selected.as_ref().filter(|id| state.runs.iter().any(|run| &run.id == *id));
            if ui.add_enabled(state.configured && !state.sending && selected.is_some(),
                egui::Button::new(tr!("Send diagnostics"))).clicked()
                && let Some(id) = selected
            {
                state.sending = true;
                state.receipt = None;
                actions.push(Action::SendDiagnosticRun(id.clone()));
            }
            if state.sending {
                ui.label(tr!("Sending diagnostics…"));
            }
            match &state.receipt {
                Some(Ok(receipt)) => {
                    ui.label(tr!("Diagnostics sent"));
                    if let Some(timestamp) = &receipt.timestamp { ui.label(timestamp); }
                    ui.label(format!("{} {}", tr!("Report ID"), receipt.id));
                }
                Some(Err(error)) => { ui.label(status(error)); }
                None => {}
            }
        });
    }
    fn mobile_member(&mut self,ui:&mut Ui,p:&crate::Participant,width:f32) {
        use egui::{Rect,pos2,Align2,FontId,Color32,Sense};
        let (rect,_)=ui.allocate_exact_size(vec2(width,64.0),Sense::hover());
        if self.watching.contains(&p.id){ui.painter().rect_filled(rect,8.0,PANEL);}
        let center=rect.left_center()+vec2(26.0,0.0);
        self.mobile_avatar(ui,Rect::from_center_size(center,vec2(40.,40.)),p.id,&p.name);
        let profile=Rect::from_min_max(rect.min,pos2(rect.right()-100.,rect.bottom()));
        self.profiles.hits.insert(format!("member-{}",p.id),profile);
        if ui.interact(profile,ui.id().with(("member-profile",p.id)),Sense::click()).clicked(){self.profiles.open(p.id);}
        let label_rect=Rect::from_min_max(rect.min+vec2(58.0,0.0),pos2(rect.right()-100.0,rect.bottom()));
        let owners:Vec<_>=self.participants.iter().filter(|owner|self.profiles.viewers.get(&owner.id).is_some_and(|v|v.contains(&p.id))).collect();
        let status=if p.sharing{if p.local{p2pshare_media::tr!("You · Sharing screen").into()}else{p2pshare_media::tr!("Sharing a screen").into()}}else if owners.len()==1{p2pshare_media::trf!("Watching {}",owners[0].name)}else if owners.len()>1{p2pshare_media::trf!("Watching {} screens",owners.len())}else if p.local{p2pshare_media::tr!("You").into()}else{p2pshare_media::tr!("In room").into()};
        let painter=ui.painter_at(label_rect);
        painter.text(label_rect.left_center()-vec2(0.0,10.0),Align2::LEFT_CENTER,&p.name,FontId::proportional(14.0),Color32::WHITE);
        painter.text(label_rect.left_center()+vec2(0.0,10.0),Align2::LEFT_CENTER,&status,FontId::proportional(11.0),MUTED);
        if p.sharing && !p.local {
            let button=Rect::from_center_size(pos2(rect.right()-48.0,rect.center().y),vec2(90.0,44.0));
            if self.watching.contains(&p.id){ui.painter().text(button.center(),Align2::CENTER_CENTER,p2pshare_media::tr!("Watching"),FontId::proportional(13.0),ACCENT);}
            else if ui.put(button,egui::Button::new(p2pshare_media::tr!("Watch"))).clicked(){self.watching.clear();self.watching.insert(p.id);self.focused=Some(p.id);self.mobile_controls_until=0.0;self.members_open=false;}
        }
    }
}
