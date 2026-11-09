//! Compact stream metadata and controls, independent of capture and transport.
use crate::app::{App, Intent, label, rect};
use egui::{FontId, Rect, Stroke, Ui, pos2, vec2};

impl App {
    fn stream_owner_avatar(&mut self,ui:&mut Ui,area:Rect,peer:u64,name:&str)->f32 {
        let p=self.palette();let center=pos2(area.left()+26.,area.center().y);
        let avatar=Rect::from_center_size(center,vec2(32.,32.));
        ui.painter().circle_filled(center,16.,p.accent);
        if let Some(texture)=self.avatars.get(&peer){crate::profile::circle(ui.painter(),center,16.,*texture,Rect::from_min_max(pos2(0.,0.),pos2(1.,1.)));}
        else{ui.painter().text(center,egui::Align2::CENTER_CENTER,name.chars().next().unwrap_or('?').to_uppercase().to_string(),FontId::proportional(14.),p.accent_text);}
        self.regions.push((format!("stream-owner-avatar-{peer}"),avatar));
        self.hits.insert(format!("stream-owner-avatar-{peer}"),avatar);
        if ui.interact(avatar,ui.id().with(("stream-owner-avatar",peer)),egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){self.profile_view=Some(peer);self.profile_show_names=false;}
        area.left()+52.
    }
    pub(crate) fn stream_footer(&mut self, ui: &mut Ui, area: Rect, id: usize, small: bool) {
        let palette = self.palette();
        let peer = self.members[id].id;
        let name = if id == 0 {
            self.config.username.clone()
        } else {
            self.members[id].name.clone()
        };
        self.regions.push((format!("stream-footer-{id}"), area));
        let center = area.center().y;
        if small {
            label(
                ui,
                rect(area.left() + 9., center - 11., area.width() - 18., 22.),
                &name,
                12.,
                palette.text,
            );
            return;
        }
        if self.popped_out.is_none() && self.detached_peers.contains(&peer) {
            let button = rect(area.right() - 134., center - 16., 124., 32.);
            let start=if button.left()-area.left()>=118.{self.stream_owner_avatar(ui,area,peer,&name)}else{area.left()+10.};
            let width = (button.left() - start - 10.).max(1.);
            label(
                ui,
                rect(start, center - 18., width, 18.),
                &name,
                14.,
                palette.text,
            );
            label(
                ui,
                rect(start, center, width, 18.),
                &self.members[id].stats,
                11.,
                palette.muted,
            );
            if self.button(
                ui,
                &format!("show-popout-{peer}"),
                button,
                p2pshare_media::tr!("Show window"),
                false,
            ) {
                self.intents.push(Intent::PopOut(peer));
            }
            return;
        }
        let watching = self.live && id != 0 && self.watching.contains(&peer);
        let preview = self.live && id != 0 && !watching;
        let compact = area.width() < 560.;
        let (stop_width, full_width, gap, pad) = if compact {
            (100., 28., 6., 10.)
        } else {
            (124., 32., 8., 12.)
        };
        let has_stop = watching || id == 0;
        let has_volume = id != 0 && (!self.live || watching);
        let full = rect(
            area.right() - pad - full_width,
            center - 16.,
            full_width,
            32.,
        );
        let popout = rect(
            full.left() - gap - full_width,
            center - 16.,
            full_width,
            32.,
        );
        // Main view: pop-out; detached view: per-window always-on-top toggle.
        let action_left = popout.left();
        let stop = rect(
            action_left - gap - stop_width,
            center - 16.,
            stop_width,
            32.,
        );
        let volume_width = if area.width() < 420. {
            52.
        } else if compact {
            180.
        } else {
            224.
        };
        let volume = rect(
            (if has_stop { stop.left() } else { action_left }) - gap - volume_width,
            center - 16.,
            volume_width,
            32.,
        );
        let text_end = if has_volume {
            volume.left()
        } else if has_stop {
            stop.left()
        } else {
            action_left
        } - 10.;
        // Reserve horizontal space only: the existing 44px footer stays unchanged.
        let text_start = if text_end-area.left()>=108. {
            self.stream_owner_avatar(ui,area,peer,&name)
        } else {area.left()+10.};
        let available = (text_end - text_start).max(1.);
        let stats = if preview {
            String::new()
        } else if self.live {
            self.members[id].stats.replace(" x ", "×")
        } else if compact {
            format!(
                "{}p · {} · {:.1} Mbps",
                self.config.stream.height,
                self.config.stream.codec.to_uppercase(),
                self.config.stream.bitrate_kbps as f32 / 1000.
            )
        } else if id == 0 {
            format!(
                "{}p · {} · {} FPS · {:.1} Mbps",
                self.config.stream.height,
                self.config.stream.codec.to_uppercase(),
                self.config.stream.fps,
                self.config.stream.bitrate_kbps as f32 / 1000.
            )
        } else {
            "1080p · AVC · 20 Mbps · 60 FPS".into()
        };
        let text_font = if compact { 12. } else { 14. };
        let stats_font = if compact { 10. } else { 11. };
        // Both lines share the same left edge and width. Keep the
        // two-line block centered within the existing compact footer height.
        let name_rect = if stats.is_empty() {
            rect(text_start, center - 12., available, 24.)
        } else {
            rect(text_start, center - 18., available, 18.)
        };
        self.regions.push((format!("stream-name-{id}"), name_rect));
        let count_width=42.0;
        let title_rect=Rect::from_min_max(name_rect.min,pos2((name_rect.right()-count_width-6.0).max(name_rect.left()+1.0),name_rect.bottom()));
        label(ui, title_rect, &name, text_font, palette.text);
        let title_width=ui.painter().layout_no_wrap(name.clone(),FontId::proportional(text_font),palette.text).size().x;
        let eye=rect((title_rect.left()+title_width+12.0).min(title_rect.right()+6.0),name_rect.center().y-11.0,count_width,22.0);
        let names=self.stream_viewers.get(&peer).cloned();
        let response=ui.allocate_rect(eye,egui::Sense::click());
        ui.painter().rect_filled(eye,5.0,palette.hover);
        let c=pos2(eye.left()+11.0,eye.center().y);let stroke=Stroke::new(1.1,palette.accent);
        ui.painter().add(egui::Shape::line(vec![c+vec2(-7.0,0.0),c+vec2(-3.0,-4.0),c+vec2(3.0,-4.0),c+vec2(7.0,0.0),c+vec2(3.0,4.0),c+vec2(-3.0,4.0),c+vec2(-7.0,0.0)],stroke));ui.painter().circle_filled(c,2.0,palette.accent);
        ui.painter().text(pos2(eye.right()-10.0,eye.center().y),egui::Align2::CENTER_CENTER,names.as_ref().map(|n|n.len().to_string()).unwrap_or_else(||"—".into()),FontId::proportional(11.0),palette.text);
        self.hits.insert(format!("stream-viewers-{peer}"),eye);
        egui::Popup::from_toggle_button_response(&response)
            .frame(egui::Frame::new().fill(palette.side).stroke(Stroke::new(1.0,palette.border)).corner_radius(10).inner_margin(egui::Margin::symmetric(16,14)))
            .show(|ui| {
                ui.set_width(238.0);
                ui.spacing_mut().item_spacing=vec2(0.0,0.0);
                let (header,_)=ui.allocate_exact_size(vec2(238.0,20.0),egui::Sense::hover());
                ui.painter().text(header.left_center(),egui::Align2::LEFT_CENTER,p2pshare_media::tr!("Viewers"),FontId::proportional(14.0),palette.text);
                ui.painter().text(header.right_center(),egui::Align2::RIGHT_CENTER,names.as_ref().map(|n|n.len().to_string()).unwrap_or_else(||"—".into()),FontId::proportional(13.0),palette.muted);
                ui.add_space(8.0);
                if let Some(names)=&names {
                    if names.is_empty(){ui.label(egui::RichText::new(p2pshare_media::tr!("No viewers")).color(palette.muted));}
                    egui::ScrollArea::vertical().max_height(324.0).min_scrolled_height((names.len() as f32 * 54.0).min(324.0)).auto_shrink([false,true]).show(ui,|ui| {
                        for (index,name) in names.iter().enumerate() {
                            let (row,_)=ui.allocate_exact_size(vec2(ui.available_width(),54.0),egui::Sense::hover());
                            let center=pos2(row.left()+20.0,row.center().y);
                            ui.painter().circle_filled(center,20.0,palette.accent.gamma_multiply(0.65));
                            ui.painter().text(center,egui::Align2::CENTER_CENTER,name.chars().next().unwrap_or('?').to_uppercase().to_string(),FontId::proportional(17.0),palette.text);
                            if let Some(texture)=self.members.iter().find(|m|m.name==*name||(m.id==0&&self.config.username==*name)).and_then(|m|self.avatars.get(&m.id)) {
                                crate::profile::circle(ui.painter(),center,20.0,*texture,Rect::from_min_max(pos2(0.,0.),pos2(1.,1.)));
                            }
                            let name_rect=Rect::from_min_max(pos2(row.left()+52.0,row.top()),row.max);
                            label(ui,name_rect,name,16.0,palette.text);
                            if ui.interact(row,ui.id().with(("viewer-name",peer,index)),egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){if let Some(member)=self.members.iter().find(|m|m.name==*name||(m.id==0&&self.config.username==*name)){self.profile_view=Some(member.id);self.profile_show_names=false;ui.close();}}
                            self.regions.push((format!("viewer-avatar-{peer}-{index}"),Rect::from_center_size(center,vec2(40.0,40.0))));
                            self.regions.push((format!("viewer-name-{peer}-{index}"),name_rect));
                        }
                    });
                } else {
                    ui.label(egui::RichText::new(p2pshare_media::tr!("Viewer information unavailable")).color(palette.muted));
                    ui.label(egui::RichText::new(p2pshare_media::tr!("The sharer needs an updated app.")).small().color(palette.muted));
                }
            });
        if ui.interact(Rect::from_min_max(title_rect.min,pos2((title_rect.left()+title_width).min(title_rect.right()),title_rect.bottom())),ui.id().with(("owner-profile",peer)),egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){self.profile_view=Some(peer);self.profile_show_names=false;}
        ui.interact(
            title_rect,
            egui::Id::new(("stream-name-tooltip", peer, small)),
            egui::Sense::hover(),
        )
        .on_hover_text(&name);
        if !stats.is_empty() {
            let stats_rect = rect(text_start, center, available, 18.);
            self.regions.push((format!("stats-{id}"), stats_rect));
            label(ui, stats_rect, &stats, stats_font, palette.muted);
            ui.interact(
                stats_rect,
                egui::Id::new(("stream-stats-tooltip", peer)),
                egui::Sense::hover(),
            )
            .on_hover_text(p2pshare_media::trf!(
                "{stats}\nBitrate is the sharer’s configured video target."
            ,stats=stats));
        }
        ui.scope(|ui| {
            if compact {
                ui.style_mut().override_font_id = Some(FontId::proportional(12.));
            }
            if id == 0
                && self.button(
                    ui,
                    "local-preview",
                    stop,
                    if self.local_preview_hidden {
                        p2pshare_media::tr!("Show preview")
                    } else {
                        p2pshare_media::tr!("Hide preview")
                    },
                    false,
                )
            {
                self.local_preview_hidden = !self.local_preview_hidden;
            }
            if has_volume {
                self.volume_control(ui, volume, id, "footer");
            }
            if watching && self.button(ui, &format!("watch-{peer}"), stop, p2pshare_media::tr!("Stop watching"), false) {
                self.intents.push(Intent::Watch {
                    peer,
                    enabled: false,
                });
                self.focus = None;
            }
            if self.popped_out.is_none() {
                let clicked = ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size = popout.size();
                        ui.spacing_mut().button_padding = vec2(0., 0.);
                        self.button(ui, &format!("popout-{peer}"), popout, "", false)
                    })
                    .inner;
                let actual = self.hits[&format!("popout-{peer}")];
                crate::icons::pop_out(ui, actual.shrink(6.), palette.text);
                if clicked {
                    if self.live && id != 0 && !watching {
                        self.intents.push(Intent::Watch {
                            peer,
                            enabled: true,
                        });
                    }
                    self.intents.push(Intent::PopOut(peer));
                }
            }
            if self.popped_out.is_some() {
                let clicked = ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size = popout.size();
                        ui.spacing_mut().button_padding = vec2(0., 0.);
                        self.button(ui, &format!("pin-{peer}"), popout, "", self.always_on_top)
                    })
                    .inner;
                let actual = self.hits[&format!("pin-{peer}")];
                crate::icons::pin(
                    ui,
                    actual.shrink(6.),
                    if self.always_on_top {
                        palette.accent_text
                    } else {
                        palette.text
                    },
                );
                if clicked {
                    self.always_on_top = !self.always_on_top;
                }
            }
            let clicked = ui
                .scope(|ui| {
                    ui.spacing_mut().interact_size = full.size();
                    ui.spacing_mut().button_padding = vec2(0., 0.);
                    self.button(ui, &format!("full-{peer}"), full, "", false)
                })
                .inner;
            if clicked {
                self.focus = Some(id);
                self.fullscreen = true;
                if self.live && id != 0 && !watching {
                    self.intents.push(Intent::Watch {
                        peer,
                        enabled: true,
                    });
                }
            }
        });
        let full = self.hits[&format!("full-{peer}")];
        let c = full.center();
        self.regions.push((
            format!("fullscreen-icon-{peer}"),
            Rect::from_center_size(c, vec2(12., 10.)),
        ));
        for (sx, sy) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
            let corner = c + vec2(sx * 6., sy * 5.);
            ui.painter().line_segment(
                [corner + vec2(-sx * 4., 0.), corner],
                Stroke::new(1.4, palette.text),
            );
            ui.painter().line_segment(
                [corner, corner + vec2(0., -sy * 4.)],
                Stroke::new(1.4, palette.text),
            );
        }
    }
    pub(crate) fn volume_control(&mut self, ui: &mut Ui, area: Rect, member: usize, context: &str) {
        let peer = self.members[member].id;
        let p = self.palette();
        if self.members[member].volume > 0 {
            self.members[member].volume_before_silence = self.members[member].volume;
        }
        let icon = Rect::from_center_size(pos2(area.left() + 12., area.center().y), vec2(20., 20.));
        let speaker_hit = Rect::from_center_size(icon.center(), vec2(24., 32.));
        let response = ui.interact(
            speaker_hit,
            egui::Id::new(("speaker-mute", context, peer)),
            egui::Sense::click(),
        );
        if response.clicked() {
            if self.members[member].muted || self.members[member].volume == 0 {
                self.members[member].muted = false;
                if self.members[member].volume == 0 {
                    self.members[member].volume = self.members[member].volume_before_silence.max(1);
                }
            } else {
                self.members[member].muted = true;
            }
        }
        let muted = self.members[member].muted || self.members[member].volume == 0;
        if response.hovered() {
            ui.painter().rect_filled(speaker_hit, 4., p.hover);
        }
        crate::icons::speaker(ui, icon, if muted { p.red } else { p.text }, muted);
        self.hits
            .insert(format!("speaker-{context}-{peer}"), response.rect);
        self.regions
            .push((format!("speaker-{context}-{peer}"), icon));
        response.on_hover_text(if muted {
            p2pshare_media::tr!("Unmute this screen")
        } else {
            p2pshare_media::tr!("Mute this screen")
        });
        if area.width() < 80. {
            let dropdown = rect(area.right() - 24., area.center().y - 16., 24., 32.);
            let response = ui
                .push_id(("volume-menu", context, peer), |ui| {
                    ui.spacing_mut().button_padding = vec2(0., 0.);
                    ui.spacing_mut().interact_size = dropdown.size();
                    ui.put(dropdown, egui::Button::new(""))
                })
                .inner;
            let c = dropdown.center();
            ui.painter().add(egui::Shape::line(
                vec![c + vec2(-3., -2.), c + vec2(0., 1.), c + vec2(3., -2.)],
                Stroke::new(1.3, p.text),
            ));
            self.hits
                .insert(format!("volume-{context}-{peer}"), response.rect);
            self.regions
                .push((format!("volume-{context}-{peer}"), area));
            egui::Popup::menu(&response)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.label(p2pshare_media::tr!("Screen volume"));
                    let (popup, _) = ui.allocate_exact_size(vec2(264., 36.), egui::Sense::hover());
                    self.volume_control(ui, popup, member, "popup");
                });
            response.on_hover_text(p2pshare_media::tr!("Adjust screen volume"));
            return;
        }
        let slider = rect(
            area.left() + 32.,
            area.center().y - 12.,
            area.width() - 78.,
            24.,
        );
        let response = ui
            .scope(|ui| {
                ui.spacing_mut().slider_width = slider.width();
                ui.spacing_mut().interact_size.y = slider.height();
                ui.visuals_mut().widgets.inactive.bg_fill = p.border;
                ui.visuals_mut().selection.bg_fill = p.green;
                ui.push_id(("screen-volume", context, peer), |ui| {
                    ui.put(
                        slider,
                        egui::Slider::new(&mut self.members[member].volume, 0..=100)
                            .show_value(false),
                    )
                })
                .inner
            })
            .inner;
        let mut changed = response.changed();
        if response.hovered() {
            let ticks = ui.input_mut(|input| {
                let mut ticks = 0i32;
                input.events.retain(|event| {
                    if let egui::Event::MouseWheel { delta, .. } = event
                        && delta.y != 0.
                    {
                        ticks += if delta.y > 0. { 1 } else { -1 };
                        false
                    } else {
                        true
                    }
                });
                if ticks != 0 {
                    input.smooth_scroll_delta.y = 0.;
                }
                ticks
            });
            if ticks != 0 {
                self.members[member].volume = (i32::from(self.members[member].volume)
                    + ticks * i32::from(self.config.volume_step_percent))
                .clamp(0, 100) as u8;
                changed = true;
            }
        }
        if changed {
            self.members[member].muted = false;
            if self.members[member].volume > 0 {
                self.members[member].volume_before_silence = self.members[member].volume;
            }
            // Repaint after gain changes, including 0% crossing into the muted icon.
            ui.ctx().request_repaint();
        }
        self.hits
            .insert(format!("volume-{context}-{peer}"), response.rect);
        self.regions
            .push((format!("volume-{context}-{peer}"), area));
        response.on_hover_text(p2pshare_media::trf!(
            "Screen volume. Mouse wheel: {}% per tick.",
            self.config.volume_step_percent
        ));
        let number = rect(area.right() - 40., area.center().y - 12., 40., 24.);
        self.regions
            .push((format!("volume-number-{context}-{peer}"), number));
        label(
            ui,
            number,
            &format!("{}%", self.members[member].volume),
            14.,
            p.text,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeUi;
    #[test]
    fn slider_unmutes_this_screen_and_fullscreen_uses_same_level() {
        let ctx = egui::Context::default();
        let mut app = App::new(Default::default(), None);
        app.live = true;
        app.members[1].muted = true;
        let peer = app.members[1].id;
        app.watching.insert(peer);
        let footer = rect(0., 100., 760., 40.);
        let input = |events| egui::RawInput {
            screen_rect: Some(rect(0., 0., 900., 600.)),
            events,
            ..Default::default()
        };
        for _ in 0..2 {
            let _ = ctx.run_ui(input(vec![]), |ui| app.stream_footer(ui, footer, 1, false));
        }
        let point = app.hits[&format!("volume-footer-{peer}")].center();
        for pressed in [true, false] {
            let events = vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            let _ = ctx.run_ui(input(events), |ui| app.stream_footer(ui, footer, 1, false));
        }
        let level = app.members[1].volume;
        assert!(
            (20..=80).contains(&level),
            "Slider did not respond: {level}"
        );
        assert!(!app.members[1].muted);
        assert_eq!(app.members[2].volume, 100);
        app.hits.clear();
        app.regions.clear();
        let viewport = rect(0., 0., 900., 600.);
        let _ = ctx.run_ui(
            input(vec![egui::Event::PointerMoved(pos2(450., 580.))]),
            |ui| app.fullscreen_controls(ui, viewport, 1),
        );
        assert!(viewport.contains_rect(app.hits[&format!("volume-fullscreen-{peer}")]));
        assert_eq!(app.members[1].volume, level);
        assert!(!app.hits.contains_key("full-mute"));
        assert!(!app.hits.contains_key(&format!("watch-{peer}")));
    }
    #[test]
    fn speaker_toggle_and_hover_wheel_use_configured_step() {
        let ctx = egui::Context::default();
        let mut app = App::new(Default::default(), None);
        app.live = true;
        app.config.theme.apply(&ctx);
        app.watching.insert(1);
        app.members[1].volume = 50;
        let footer = rect(0., 100., 760., 44.);
        let frame = |app: &mut App, events| {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 900., 600.)),
                    events,
                    ..Default::default()
                },
                |ui| app.stream_footer(ui, footer, 1, false),
            );
        };
        for _ in 0..2 {
            frame(&mut app, vec![]);
        }
        let icon = app.hits["speaker-footer-1"].center();
        let click = |app: &mut App, point| {
            for pressed in [true, false] {
                frame(
                    app,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        click(&mut app, icon);
        assert!(app.members[1].muted);
        assert_eq!(app.members[1].volume, 50);
        click(&mut app, icon);
        assert!(!app.members[1].muted);
        assert_eq!(app.members[1].volume, 50);
        let slider = app.hits["volume-footer-1"].center();
        let wheel = |app: &mut App, point, y| {
            frame(
                app,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        phase: egui::TouchPhase::Move,
                        delta: vec2(0., y),
                        modifiers: Default::default(),
                    },
                ],
            )
        };
        wheel(&mut app, slider, 3.);
        assert_eq!(app.members[1].volume, 52);
        app.config.volume_step_percent = 7;
        wheel(&mut app, slider, -3.);
        assert_eq!(app.members[1].volume, 45);
        wheel(&mut app, pos2(10., 10.), 3.);
        assert_eq!(app.members[1].volume, 45);
        app.members[1].volume = 99;
        wheel(&mut app, slider, 3.);
        assert_eq!(app.members[1].volume, 100);
        app.members[1].volume = 3;
        wheel(&mut app, slider, -3.);
        assert_eq!(app.members[1].volume, 0);
        click(&mut app, icon);
        assert!(!app.members[1].muted);
        assert_eq!(app.members[1].volume, 3);
        assert_eq!(app.members[2].volume, 100);
    }
    #[test]
    fn watched_screen_controls_fit_narrow_and_wide_tiles() {
        for width in [280., 360., 480., 760.] {
            let ctx = egui::Context::default();
            let mut app = App::new(Default::default(), None);
            app.live = true;
            app.config.theme.apply(&ctx);
            let peer = app.members[1].id;
            app.watching.insert(peer);
            let footer = rect(0., 100., width, 40.);
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 900., 600.)),
                    ..Default::default()
                },
                |ui| app.stream_footer(ui, footer, 1, false),
            );
            let volume = app.hits[&format!("volume-footer-{peer}")];
            let full = app.hits[&format!("full-{peer}")];
            let stop = app.hits[&format!("watch-{peer}")];
            assert!(footer.contains_rect(volume), "{width}: {volume:?}");
            assert!((volume.center().y - footer.center().y).abs() < 0.1);
            assert!(volume.right() <= stop.left() && stop.right() <= full.left());
            assert!(
                (full.right() - (footer.right() - if width < 560. { 10. } else { 12. })).abs()
                    < 0.1
            );
            assert!(!app.hits.contains_key(&format!("mute-{peer}")));
            let glyph = app
                .regions
                .iter()
                .find(|(name, _)| name == &format!("fullscreen-icon-{peer}"))
                .unwrap()
                .1;
            assert!(glyph.center().distance(full.center()) < 0.01);
            if width >= 460. {
                assert!(volume.width() >= 100.);
            }
        }
    }
}

#[cfg(test)]
mod watcher_tests {
 use super::*;
 #[test]
 fn popup_lists_viewers_and_reflects_stop_updates(){
  let ctx=egui::Context::default();let mut config=crate::config::Config::default();config.username="Alex".into();let mut app=App::live(config,"grupo".into(),false);app.joined=true;app.members[0].scene=Some(0);
  app.stream_viewers.insert(0,vec!["arthur".into(),"Maya".into(),"Theo".into()]);
  let render=|app:&mut App,events:Vec<egui::Event>|ctx.run_ui(egui::RawInput{screen_rect:Some(rect(0.,0.,1440.,932.)),events,..Default::default()},|ui|app.ui(ui));
  render(&mut app,vec![]);let at=app.hits["stream-viewers-0"].center();
  for pressed in [true,false]{render(&mut app,vec![egui::Event::PointerMoved(at),egui::Event::PointerButton{pos:at,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()}]);}
  let out=render(&mut app,vec![]);
  let text:Vec<_>=out.shapes.iter().filter_map(|s|if let egui::Shape::Text(t)=&s.shape{Some(t.galley.text())}else{None}).collect();
  assert!(text.contains(&"Viewers"));assert!(text.contains(&"Theo"));assert!(!text.iter().any(|s|s.starts_with("Watching")));
  let avatar=app.regions.iter().find(|(n,_)|n=="viewer-avatar-0-0").unwrap().1;
  let name=app.regions.iter().find(|(n,_)|n=="viewer-name-0-0").unwrap().1;
  assert_eq!(avatar.size(),vec2(40.0,40.0));assert_eq!(name.left()-avatar.right(),12.0);
  assert!((avatar.center().y-name.center().y).abs()<0.01);
  let last=app.regions.iter().find(|(n,_)|n=="viewer-avatar-0-2").unwrap().1;
  assert!(out.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Theo")&&s.clip_rect.contains_rect(last)),"Last viewer must fit without clipping");
  app.stream_viewers.insert(0,Vec::new());let out=render(&mut app,vec![]);
  assert!(out.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="No viewers")));
 }
}
