//! Fullscreen-only presentation: bottom-edge reveal and spacious, stable controls.
use crate::app::{App, rect};
use egui::{Align2, Color32, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
impl App {
    pub(crate) fn fullscreen_controls(&mut self, ui: &mut Ui, viewport: Rect, member: usize) {
        // No movement/click timer: moving elsewhere in the video never reveals controls.
        let near_bottom = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|at| viewport.contains(at) && at.y >= viewport.bottom() - 96.);
        if !near_bottom {
            return;
        }
        let p = self.palette();
        let has_volume = member != 0;
        let area = Rect::from_center_size(
            pos2(viewport.center().x, viewport.bottom() - 44.),
            vec2(if has_volume { 452. } else { 204. }, 60.),
        );
        self.regions.push(("fullscreen-toolbar".into(), area));
        ui.painter().rect_filled(
            area.translate(vec2(0., 3.)).expand(2.),
            14.,
            Color32::from_black_alpha(75),
        );
        ui.painter().rect(
            area,
            12.,
            p.side,
            Stroke::new(1., p.border),
            StrokeKind::Inside,
        );
        let exit = rect(area.left() + 10., area.top() + 10., 184., 40.);
        if self.fullscreen_exit(ui, exit) {
            self.fullscreen = false;
        }
        if has_volume {
            self.volume_control(
                ui,
                rect(area.left() + 210., area.top() + 10., 232., 40.),
                member,
                "fullscreen",
            );
        }
    }
    fn fullscreen_exit(&mut self, ui: &mut Ui, r: Rect) -> bool {
        let p = self.palette();
        let response = ui.interact(
            r,
            Id::new(("fullscreen-control", "exit-full")),
            Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, p2pshare_media::tr!("Exit fullscreen"))
        });
        let color = p.text;
        let painter = ui.painter().with_clip_rect(r);
        painter.rect(
            r,
            8.,
            if response.hovered() {
                p.hover
            } else {
                p.surface
            },
            Stroke::new(1., p.border),
            StrokeKind::Inside,
        );
        let center = pos2(r.left() + 21., r.center().y);
        let stroke = Stroke::new(1.5, color);
        {
            for (sx, sy) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                let corner = center + vec2(sx * 3., sy * 3.);
                painter.line_segment([corner, corner + vec2(sx * 5., 0.)], stroke);
                painter.line_segment([corner, corner + vec2(0., sy * 5.)], stroke);
            }
            let badge = rect(r.right() - 37., r.center().y - 10., 28., 20.);
            painter.rect(
                badge,
                4.,
                p.side,
                Stroke::new(1., p.border),
                StrokeKind::Inside,
            );
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                "Esc",
                FontId::proportional(10.),
                p.muted,
            );
        }
        let label = painter.text(
            pos2(r.left() + 38., r.center().y),
            Align2::LEFT_CENTER,
            p2pshare_media::tr!("Exit fullscreen"),
            FontId::proportional(13.),
            color,
        );
        self.regions
            .push(("fullscreen-label-exit-full".into(), label));
        self.hits.insert("exit-full".into(), response.rect);
        response.on_hover_text(p2pshare_media::tr!("Exit fullscreen (Esc)")).clicked()
    }
}
