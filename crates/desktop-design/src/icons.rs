//! Font-independent vector playback icons.
use egui::{Color32, Rect, Shape, Stroke, Ui};

pub(crate) fn speaker(ui: &Ui, area: Rect, color: Color32, muted: bool) {
    let scale = area.width().min(area.height()) / 24.;
    let origin = area.center() - egui::vec2(12., 12.) * scale;
    let point = |x: f32, y: f32| origin + egui::vec2(x, y) * scale;
    let stroke = Stroke::new(1.7 * scale, color);
    ui.painter().add(egui::epaint::PathShape {
        points: [
            (11., 5.),
            (6., 9.),
            (3., 9.),
            (3., 15.),
            (6., 15.),
            (11., 19.),
        ]
        .into_iter()
        .map(|(x, y)| point(x, y))
        .collect(),
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke: stroke.into(),
    });
    if muted {
        ui.painter()
            .line_segment([point(3., 3.), point(21., 21.)], stroke);
    } else {
        for points in [
            [(15., 8.), (18., 10.), (18., 14.), (15., 16.)],
            [(18., 5.), (23., 8.), (23., 16.), (18., 19.)],
        ] {
            ui.painter().add(Shape::CubicBezier(
                egui::epaint::CubicBezierShape::from_points_stroke(
                    points.map(|(x, y)| point(x, y)),
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ),
            ));
        }
    }
}

/// External-window arrow, distinct from the four-corner fullscreen icon.
pub(crate) fn pop_out(ui: &Ui, area: Rect, color: Color32) {
    let scale = area.width().min(area.height()) / 24.;
    let origin = area.center() - egui::vec2(12., 12.) * scale;
    let point = |x, y| origin + egui::vec2(x, y) * scale;
    let stroke = Stroke::new(1.7 * scale, color);
    ui.painter().add(egui::epaint::PathShape::line(
        [(19., 14.), (19., 20.), (4., 20.), (4., 5.), (10., 5.)]
            .into_iter()
            .map(|(x, y)| point(x, y))
            .collect(),
        stroke,
    ));
    for (a, b) in [
        ((11., 13.), (21., 3.)),
        ((14., 3.), (21., 3.)),
        ((21., 3.), (21., 10.)),
    ] {
        ui.painter()
            .line_segment([point(a.0, a.1), point(b.0, b.1)], stroke);
    }
}

pub(crate) fn pin(ui: &Ui, area: Rect, color: Color32) {
    let scale = area.width().min(area.height()) / 24.;
    let origin = area.center() - egui::vec2(12., 12.) * scale;
    let point = |x, y| origin + egui::vec2(x, y) * scale;
    let stroke = Stroke::new(1.7 * scale, color);
    ui.painter().add(egui::epaint::PathShape {
        points: [
            (8., 3.),
            (16., 3.),
            (16., 5.),
            (15., 5.),
            (15., 10.),
            (19., 14.),
            (19., 16.),
            (5., 16.),
            (5., 14.),
            (9., 10.),
            (9., 5.),
            (8., 5.),
        ]
        .into_iter()
        .map(|(x, y)| point(x, y))
        .collect(),
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke: stroke.into(),
    });
    ui.painter()
        .line_segment([point(12., 16.), point(12., 22.)], stroke);
}

/// The same embedded artwork used by the executable and native Windows icon.
pub(crate) fn app_icon(ui: &Ui, area: Rect) {
    let key = egui::Id::new("p2pshare-app-icon");
    let existing = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(key));
    let texture = existing.unwrap_or_else(|| {
        let texture = ui.ctx().load_texture(
            "p2pshare-app-icon",
            egui::ColorImage::from_rgba_unmultiplied(
                [64, 64],
                include_bytes!("../../../assets/app-icon-64.rgba"),
            ),
            egui::TextureOptions::LINEAR,
        );
        ui.ctx()
            .data_mut(|data| data.insert_temp(key, texture.clone()));
        texture
    });
    ui.painter().image(
        texture.id(),
        area,
        Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
        Color32::WHITE,
    );
}
