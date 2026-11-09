//! Compact client-drawn title bar. Platform adapters execute the window operations.
use crate::app::{App, Intent, rect};
use egui::{Align2, CursorIcon, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};
pub const HEIGHT: f32 = 32.;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    Minimize,
    ToggleMaximize,
    Close,
    Drag,
    Resize(ResizeEdge),
}
fn edge_rects(r: Rect) -> [(ResizeEdge, Rect); 8] {
    let e = 5.;
    let c = 9.;
    [
        (
            ResizeEdge::North,
            rect(r.left() + c, r.top(), r.width() - 2. * c, e),
        ),
        (ResizeEdge::NorthEast, rect(r.right() - c, r.top(), c, c)),
        (
            ResizeEdge::East,
            rect(r.right() - e, r.top() + c, e, r.height() - 2. * c),
        ),
        (
            ResizeEdge::SouthEast,
            rect(r.right() - c, r.bottom() - c, c, c),
        ),
        (
            ResizeEdge::South,
            rect(r.left() + c, r.bottom() - e, r.width() - 2. * c, e),
        ),
        (ResizeEdge::SouthWest, rect(r.left(), r.bottom() - c, c, c)),
        (
            ResizeEdge::West,
            rect(r.left(), r.top() + c, e, r.height() - 2. * c),
        ),
        (ResizeEdge::NorthWest, rect(r.left(), r.top(), c, c)),
    ]
}
impl App {
    pub(crate) fn window_chrome(&mut self, ctx: &egui::Context, viewport: Rect) {
        // The room selector is now at the bottom. Reveal across the whole
        // caption area, including above the sidebar, without a narrow gap target.
        let show_caption = ctx
            .pointer_hover_pos()
            .is_some_and(|pos| viewport.contains(pos) && pos.y < viewport.top() + HEIGHT);
        self.caption_visible = show_caption;
        if show_caption {
            let bar = rect(viewport.left(), viewport.top(), viewport.width(), HEIGHT);
            self.regions.push(("titlebar".into(), bar));
            let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("desktop-titlebar"));
            egui::Area::new(layer.id)
                .order(layer.order)
                .fixed_pos(bar.min)
                .movable(false)
                .constrain(false)
                .fade_in(false)
                .show(ctx, |ui| {
                    ui.set_min_size(bar.size());
                    ui.set_max_size(bar.size());
                    ui.set_clip_rect(bar);
                    let p = self.palette();
                    ui.painter().rect_filled(bar, 0., p.bg);
                    ui.painter().hline(
                        bar.left()..=bar.right(),
                        bar.bottom() - 0.5,
                        Stroke::new(1., p.border),
                    );
                    let drag = rect(
                        bar.left() + 5.,
                        bar.top() + 5.,
                        bar.width() - 148.,
                        HEIGHT - 5.,
                    );
                    self.hits.insert("window-drag".into(), drag);
                    let response =
                        ui.interact(drag, Id::new("window-drag"), Sense::click_and_drag());
                    if response.double_clicked() {
                        self.intents
                            .push(Intent::Window(WindowAction::ToggleMaximize));
                    } else if response.drag_started() {
                        self.intents.push(Intent::Window(WindowAction::Drag));
                    }
                    crate::icons::app_icon(ui, rect(bar.left() + 8., bar.top() + 4., 24., 24.));
                    let title = self.popped_out.map_or_else(
                        || {
                            if self.live {
                                "P2PShare".to_owned()
                            } else {
                                "P2PShare · UI prototype".to_owned()
                            }
                        },
                        |id| {
                            let name = if id == 0 {
                                self.config.username.as_str()
                            } else {
                                self.members
                                    .get(id)
                                    .map_or(p2pshare_media::tr!("Screen share"), |m| m.name.as_str())
                            };
                            format!(
                                "{name} · {}",
                                if self.live {
                                    "P2PShare"
                                } else {
                                    "UI prototype"
                                }
                            )
                        },
                    );
                    ui.painter().text(
                        bar.min + vec2(42., HEIGHT / 2.),
                        Align2::LEFT_CENTER,
                        title,
                        FontId::proportional(12.),
                        if self.window_focused { p.text } else { p.muted },
                    );
                    let controls = [
                        ("window-minimize", WindowAction::Minimize, p2pshare_media::tr!("Minimize")),
                        (
                            "window-maximize",
                            WindowAction::ToggleMaximize,
                            if self.maximized {
                                p2pshare_media::tr!("Restore")
                            } else {
                                p2pshare_media::tr!("Maximize")
                            },
                        ),
                        ("window-close", WindowAction::Close, p2pshare_media::tr!("Close")),
                    ];
                    for (i, (key, action, tooltip)) in controls.into_iter().enumerate() {
                        let r = rect(bar.right() - 138. + i as f32 * 46., bar.top(), 46., HEIGHT);
                        self.caption_button(ui, r, key, action, tooltip);
                    }
                });
            ctx.move_to_top(layer);
        }
        // Border resizing stays available even when the caption is hidden.
        if !self.maximized {
            for (edge, r) in edge_rects(viewport) {
                let id = Id::new(format!("window-resize-{edge:?}"));
                let layer = egui::LayerId::new(egui::Order::Foreground, id);
                egui::Area::new(id)
                    .order(layer.order)
                    .fixed_pos(r.min)
                    .movable(false)
                    .constrain(false)
                    .fade_in(false)
                    .show(ctx, |ui| {
                        ui.set_min_size(r.size());
                        ui.set_max_size(r.size());
                        ui.set_clip_rect(r);
                        let response = ui.interact(r, id.with("hit"), Sense::click_and_drag());
                        if response.hovered() {
                            ui.ctx().set_cursor_icon(match edge {
                                ResizeEdge::North | ResizeEdge::South => CursorIcon::ResizeVertical,
                                ResizeEdge::East | ResizeEdge::West => CursorIcon::ResizeHorizontal,
                                ResizeEdge::NorthWest | ResizeEdge::SouthEast => {
                                    CursorIcon::ResizeNwSe
                                }
                                ResizeEdge::NorthEast | ResizeEdge::SouthWest => {
                                    CursorIcon::ResizeNeSw
                                }
                            });
                        }
                        if response.is_pointer_button_down_on()
                            && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
                        {
                            self.intents
                                .push(Intent::Window(WindowAction::Resize(edge)));
                        }
                        self.hits.insert(format!("resize-{edge:?}"), r);
                    });
                // Keep edges above the caption even after restoring from maximized.
                ctx.set_sublayer(
                    egui::LayerId::new(egui::Order::Foreground, Id::new("desktop-titlebar")),
                    layer,
                );
            }
        }
    }
    fn caption_button(
        &mut self,
        ui: &mut Ui,
        r: Rect,
        key: &str,
        action: WindowAction,
        tooltip: &str,
    ) {
        let p = self.palette();
        let response = ui.interact(r, Id::new(key), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tooltip));
        let close = action == WindowAction::Close;
        if response.hovered() {
            ui.painter()
                .rect_filled(r, 0., if close { p.red } else { p.hover });
        }
        let color = if close && response.hovered() {
            p.bg
        } else if self.window_focused {
            p.text
        } else {
            p.muted
        };
        let stroke = Stroke::new(1.3, color);
        let center = r.center();
        match action {
            WindowAction::Minimize => {
                ui.painter()
                    .line_segment([center - vec2(5., 0.), center + vec2(5., 0.)], stroke);
            }
            WindowAction::ToggleMaximize => {
                let square = Rect::from_center_size(center, Vec2::splat(10.));
                if self.maximized {
                    ui.painter().rect_stroke(
                        square.translate(vec2(2., -2.)),
                        0.,
                        stroke,
                        StrokeKind::Inside,
                    );
                    ui.painter().rect_filled(
                        square.translate(vec2(-1., 1.)),
                        0.,
                        if response.hovered() { p.hover } else { p.bg },
                    );
                    ui.painter().rect_stroke(
                        square.translate(vec2(-1., 1.)),
                        0.,
                        stroke,
                        StrokeKind::Inside,
                    );
                } else {
                    ui.painter()
                        .rect_stroke(square, 0., stroke, StrokeKind::Inside);
                }
            }
            WindowAction::Close => {
                ui.painter()
                    .line_segment([center - vec2(5., 5.), center + vec2(5., 5.)], stroke);
                ui.painter()
                    .line_segment([center + vec2(-5., 5.), center + vec2(5., -5.)], stroke);
            }
            _ => {}
        }
        self.hits.insert(key.into(), response.rect);
        if response.on_hover_text(tooltip).clicked() {
            self.intents.push(Intent::Window(action));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_caption_area_reveals_without_shifting_the_video() {
        let ctx = egui::Context::default();
        let mut app = App::new(Default::default(), None);
        app.dialog = crate::app::Dialog::None;
        let mut frame = |point: egui::Pos2| {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect(0., 0., 1280., 832.)),
                    events: vec![egui::Event::PointerMoved(point)],
                    ..Default::default()
                },
                |ui| app.ui(ui),
            );
            let video = app
                .regions
                .iter()
                .find(|(name, _)| name == "tile-1")
                .map(|(_, r)| *r);
            (
                app.regions.iter().any(|(name, _)| name == "titlebar"),
                video,
            )
        };
        let hidden = frame(egui::pos2(400., 100.));
        assert!(!hidden.0);
        let sidebar = frame(egui::pos2(100., 24.));
        assert!(
            sidebar.0,
            "Hovering the caption over the sidebar should reveal it"
        );
        assert_eq!(hidden.1, sidebar.1);
        assert!(frame(egui::pos2(800., 28.)).0);
        assert!(!frame(egui::pos2(800., HEIGHT + 2.)).0);
    }
}
