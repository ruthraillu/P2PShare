//! Android presentation only. Stream selection, texture ownership and transport stay in adapters.
use crate::{Action, Participant, RoomUi, layout::Layout, theme::*, video::paint_video};
use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
impl RoomUi {
    pub(crate) fn pip_view(&mut self, root: &mut Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::BLACK))
            .show(root, |ui| {
                let rect = ui.max_rect();
                self.snapshot.viewer_rect = Some(rect);
                if let Some(p) = self.playback_participant() {
                    paint_video(ui.painter(), rect, p);
                }
            });
    }
    fn mobile_select(&mut self, id: u64) {
        // A phone watches one stream; all other tiles remain lightweight JPEG previews.
        self.watching.clear();
        self.watching.insert(id);
        self.focused = Some(id);
        self.mobile_controls_until=0.0;
    }
    pub(crate) fn mobile_ui(&mut self, root: &mut Ui, layout: &Layout, actions: &mut Vec<Action>) {
        if self.mobile_pages(root,actions) { return; }
        self.mobile_navigation(root,layout,actions);
        let streams: Vec<_> = self
            .participants
            .iter()
            .filter(|p| p.sharing && !p.local)
            .cloned()
            .collect();
        let active = self
            .focused
            .and_then(|id| streams.iter().find(|p| p.id == id))
            .cloned();
        if layout.landscape
            && let Some(p) = active.as_ref()
        {
            let side_width = (root.available_width() * 0.22).clamp(164.0, 240.0);
            egui::Panel::right("mobile_other_screens")
                .exact_size(side_width)
                .frame(egui::Frame::new().fill(PANEL).inner_margin(8))
                .show(root, |ui| {
                    let b = ui.add_sized(
                        [ui.available_width(), 44.0],
                        egui::Button::new(p2pshare_media::tr!("‹ Other screens")).frame(false),
                    );
                    self.snapshot.grid_button = Some(b.rect);
                    if b.clicked() {
                        self.focused = None;
                            self.mobile_controls_until = 0.0;
                    }
                    egui::ScrollArea::vertical().scroll_source(egui::scroll_area::ScrollSource::ALL)
                        .id_salt("landscape_others")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for other in streams.iter().filter(|o| o.id != p.id) {
                                self.mobile_card(ui, other, ui.available_width(), actions);
                                ui.add_space(6.0);
                            }
                        });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(8))
            .show(root, |ui| {
                self.snapshot.content_rect = Some(ui.max_rect());
                if let Some(p) = active {
                    if layout.landscape {
                        let content = ui.max_rect();
                        let rect=Rect::from_min_max(content.min,content.max-vec2(0.,42.));
                        self.mobile_stream_footer(ui,Rect::from_min_max(pos2(content.left(),rect.bottom()),content.max),&p);
                        self.snapshot.tiles.push((p.id, rect));
                        self.snapshot.viewer_rect = Some(rect);
                        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
                        paint_video(ui.painter(), rect, &p);
                        self.mobile_overlay(ui,rect,&p,actions);
                    } else {
                        let h = (ui.available_width() * p.size.height.max(1) as f32
                            / p.size.width.max(1) as f32)
                            .min((ui.available_height() - 205.0).max(90.0));
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
                        paint_video(ui.painter(), rect, &p);
                        self.snapshot.tiles.push((p.id, rect));
                        self.snapshot.viewer_rect = Some(rect);
                        self.mobile_overlay(ui,rect,&p,actions);
                        let (footer,_)=ui.allocate_exact_size(vec2(ui.available_width(),42.),Sense::hover());self.mobile_stream_footer(ui,footer,&p);
                        ui.label(RichText::new(p2pshare_media::tr!("Other screens")).size(14.0).strong());
                        let others: Vec<_> =
                            streams.iter().filter(|o| o.id != p.id).cloned().collect();
                        self.mobile_cards(ui, &others, 2, "portrait_others", actions);
                    }
                } else if streams.is_empty() {
                    ui.add_space(16.0);
                    ui.heading(p2pshare_media::tr!("Your screens, together"));
                    ui.label(
                        RichText::new(if self.in_room {
                            p2pshare_media::tr!("Waiting for someone to share a screen.")
                        } else {
                            p2pshare_media::tr!("Join the same room name as your friends.")
                        })
                        .color(MUTED),
                    );
                    if !self.in_room
                        && ui
                            .add_enabled(!self.busy, egui::Button::new(p2pshare_media::tr!("Join room")))
                            .clicked()
                    {
                        self.room_dialog = true;
                    }
                } else {
                    let columns = if layout.landscape {
                        3.min(streams.len())
                    } else {
                        1
                    };
                    self.mobile_cards(ui, &streams, columns, "mobile_gallery", actions);
                }
            });
        if let Some(id) = self.focused {
            self.snapshot.tiles.sort_by_key(|(peer, _)| *peer != id);
        }
    }
    fn mobile_cards(&mut self, ui: &mut Ui, streams: &[Participant], columns: usize, salt: &str, actions: &mut Vec<Action>) {
        egui::ScrollArea::vertical().scroll_source(egui::scroll_area::ScrollSource::ALL)
            .id_salt(salt)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = ((ui.available_width() - 10.0 * (columns - 1) as f32) / columns as f32)
                    .max(1.0);
                for row in streams.chunks(columns) {
                    ui.horizontal_top(|ui| {
                        for p in row {
                            self.mobile_card(ui, p, width, actions);
                        }
                    });
                    ui.add_space(2.0);
                }
            });
    }
    fn mobile_card(&mut self, ui: &mut Ui, p: &Participant, width: f32, actions: &mut Vec<Action>) {
        let height = width * 9.0 / 16.0 + 42.0;
        let playing = self.watching.contains(&p.id);
        // The video owns taps/drags when playing; the card must not also focus itself.
        let (rect, _) = ui.allocate_exact_size(vec2(width, height),Sense::hover());
        self.snapshot.tiles.push((p.id, rect));
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 8.0, TILE);
        let screen = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 42.0));
        painter.rect_filled(screen, 8.0, Color32::BLACK);
        paint_video(&painter, screen, p);
        // Playback continues when returning to the gallery; Play is for JPEG previews only.
        if !self.watching.contains(&p.id) {
            let c = screen.center();
            painter.circle_filled(c, 17.0, Color32::from_black_alpha(190));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + vec2(-4.0, -7.0),
                    c + vec2(7.0, 0.0),
                    c + vec2(-4.0, 7.0),
                ],
                Color32::WHITE,
                egui::Stroke::NONE,
            ));
        }
        self.mobile_stream_footer(ui,Rect::from_min_max(pos2(rect.left(),screen.bottom()),rect.max),p);
        if !self.watching.contains(&p.id) {
            self.snapshot.watch_buttons.push((p.id, screen));
        }
        if playing {
            self.snapshot.viewer_rect = Some(screen);
            self.mobile_overlay(ui, screen, p, actions);
        } else if ui.interact(screen,ui.id().with(("preview-watch",p.id)),Sense::click()).clicked() {
            self.mobile_select(p.id);
        }
    }
}
