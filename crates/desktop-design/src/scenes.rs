//! Synthetic preview art. Never enumerates or captures the user's windows.
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, vec2};
fn rgb(n: u32) -> Color32 {
    Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)
}
struct Canvas {
    p: Painter,
    r: Rect,
    s: f32,
}
impl Canvas {
    fn new(p: &Painter, r: Rect) -> Self {
        Self {
            p: p.with_clip_rect(r),
            r,
            s: r.width() / 960.,
        }
    }
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.r.min + vec2(x, y) * self.s
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: u32, round: f32) {
        self.p.rect_filled(
            Rect::from_min_size(self.at(x, y), vec2(w, h) * self.s),
            round * self.s,
            rgb(color),
        );
    }
    fn text(&self, x: f32, y: f32, t: &str, size: f32, color: u32) {
        self.p.text(
            self.at(x, y),
            Align2::LEFT_TOP,
            t,
            FontId::proportional((size * self.s).max(5.)),
            rgb(color),
        );
    }
    fn line(&self, a: (f32, f32), b: (f32, f32), c: u32, width: f32) {
        self.p.line_segment(
            [self.at(a.0, a.1), self.at(b.0, b.1)],
            Stroke::new(width * self.s, rgb(c)),
        );
    }
    fn poly(&self, points: &[(f32, f32)], color: u32) {
        self.p.add(Shape::convex_polygon(
            points.iter().map(|&(x, y)| self.at(x, y)).collect(),
            rgb(color),
            Stroke::NONE,
        ));
    }
}
pub fn paint(p: &Painter, rect: Rect, kind: usize) {
    let c = Canvas::new(p, rect);
    c.rect(0., 0., 960., 540., 0x131723, 0.);
    match kind % 4 {
        0 => {
            // A calm cinematic feed, with a custom vector night landscape.
            for i in 0..80 {
                let f = i as f32 / 80.;
                let col = Color32::from_rgb(
                    (18. + 37. * f) as u8,
                    (27. + 36. * f) as u8,
                    (48. + 30. * f) as u8,
                );
                c.p.rect_filled(
                    Rect::from_min_size(c.at(0., i as f32 * 6.75), vec2(960., 7.) * c.s),
                    0.,
                    col,
                );
            }
            c.p.circle_filled(c.at(735., 128.), 43. * c.s, rgb(0xDCE3CA));
            for i in 0..28 {
                let x = ((i * 173 + 67) % 950) as f32;
                let y = ((i * 43 + 19) % 210) as f32;
                c.p.circle_filled(c.at(x, y), c.s * 1.4, rgb(0xBAC3D1));
            }
            c.poly(
                &[
                    (0., 370.),
                    (145., 220.),
                    (256., 348.),
                    (390., 165.),
                    (575., 367.),
                    (720., 257.),
                    (960., 390.),
                    (960., 540.),
                    (0., 540.),
                ],
                0x344F60,
            );
            c.poly(
                &[
                    (0., 440.),
                    (240., 326.),
                    (415., 440.),
                    (620., 300.),
                    (960., 470.),
                    (960., 540.),
                    (0., 540.),
                ],
                0x203D49,
            );
            c.poly(
                &[
                    (0., 503.),
                    (270., 414.),
                    (510., 510.),
                    (775., 390.),
                    (960., 480.),
                    (960., 540.),
                    (0., 540.),
                ],
                0x152C35,
            );
            c.rect(39., 38., 112., 26., 0x17252E, 5.);
            c.text(50., 42., "NIGHT WALK", 14., 0xDEE7DF);
            c.text(52., 429., "Somewhere quieter.", 32., 0xE2ECE8);
            c.text(54., 471., "A little room to wander.", 17., 0xB9CDCF);
        }
        1 => {
            c.rect(0., 0., 960., 42., 0x2A2B37, 0.);
            c.text(20., 12., "Studio / Component library", 17., 0xE8E9F4);
            c.text(790., 13., "100%   Share", 14., 0xB7B7D0);
            c.rect(0., 42., 172., 498., 0x20212B, 0.);
            c.text(18., 64., "LAYERS", 13., 0x9296AD);
            for (i, t) in [
                "Page 01",
                "  Components",
                "  Color tokens",
                "  Buttons",
                "  Navigation",
                "  Cards",
            ]
            .iter()
            .enumerate()
            {
                c.text(
                    20.,
                    103. + i as f32 * 32.,
                    t,
                    15.,
                    if i == 3 { 0xB2ADFF } else { 0xAEB0C4 },
                );
            }
            c.rect(172., 42., 788., 498., 0xDCDDE8, 0.);
            c.rect(213., 87., 694., 396., 0xF8F7FC, 13.);
            c.text(249., 119., "Make room for good ideas.", 29., 0x26243C);
            c.text(
                250.,
                164.,
                "A small collection of things that work together.",
                17.,
                0x747087,
            );
            for i in 0..5 {
                c.rect(
                    250. + i as f32 * 83.,
                    210.,
                    65.,
                    65.,
                    [0x7774C4, 0xADA2DB, 0xD9C0D9, 0xA8C6BF, 0xE8CE9E][i],
                    13.,
                );
            }
            c.rect(250., 311., 170., 48., 0x7774C4, 10.);
            c.text(282., 324., "Primary button", 17., 0xFFFFFF);
            c.rect(442., 311., 170., 48., 0xECEAF5, 10.);
            c.text(467., 324., "Secondary button", 16., 0x665E85);
            c.rect(250., 394., 600., 47., 0xEEEAF6, 8.);
            c.text(
                267.,
                408.,
                "Consistency starts with the little things.",
                16.,
                0x79708F,
            );
        }
        2 => {
            c.rect(0., 0., 960., 40., 0x252B36, 0.);
            c.text(18., 11., "workspace / src / player.rs", 15., 0xBEC5D4);
            c.rect(0., 40., 180., 500., 0x1D222D, 0.);
            c.text(20., 62., "EXPLORER", 13., 0x818FA6);
            for (i, t) in [
                "workspace",
                "  src",
                "    audio.rs",
                "    config.rs",
                "    player.rs",
                "    renderer.rs",
                "  Cargo.toml",
            ]
            .iter()
            .enumerate()
            {
                c.text(
                    20.,
                    100. + i as f32 * 31.,
                    t,
                    15.,
                    if i == 4 { 0x9DC8FF } else { 0x9AA9BD },
                );
            }
            c.rect(180., 40., 780., 500., 0x151B25, 0.);
            let lines = [
                ("use std::time::Duration;", 0x8CB7E9),
                ("", 0),
                ("pub struct PlaybackClock {", 0xD9BD83),
                ("    buffered_ms: u32,", 0xB7C9DC),
                ("    timestamp: Duration,", 0xB7C9DC),
                ("}", 0xB7C9DC),
                ("", 0),
                ("impl PlaybackClock {", 0xD9BD83),
                ("    pub fn new() -> Self {", 0x92C9BA),
                ("        Self {", 0xB7C9DC),
                ("            buffered_ms: 200,", 0xB7C9DC),
                ("            timestamp: Duration::ZERO,", 0xB7C9DC),
                ("        }", 0xB7C9DC),
                ("    }", 0xB7C9DC),
                ("}", 0xB7C9DC),
            ];
            for (i, (t, col)) in lines.iter().enumerate() {
                c.text(
                    200.,
                    65. + i as f32 * 27.,
                    &format!("{:2}", i + 1),
                    13.,
                    0x4E5D74,
                );
                c.text(244., 65. + i as f32 * 27., t, 17., *col);
            }
            c.rect(180., 512., 780., 28., 0x263749, 0.);
            c.text(
                200.,
                517.,
                "Rust    UTF-8    Spaces: 4                                      Ready",
                12.,
                0xB1C4D9,
            );
        }
        _ => {
            c.rect(0., 0., 960., 40., 0x242938, 0.);
            c.text(18., 11., "Overview / Weekly activity", 15., 0xDBE2F1);
            c.rect(0., 40., 960., 500., 0x1B2030, 0.);
            c.text(39., 75., "A good week, in numbers.", 30., 0xE4E9F5);
            c.text(40., 120., "Your workspace at a glance", 16., 0x98A6BC);
            for (i, (label, value, col)) in [
                ("Sessions", "124", 0xA3AEF1),
                ("Time together", "32.5h", 0x7CC5B7),
                ("This week", "+18%", 0xD6B482),
            ]
            .iter()
            .enumerate()
            {
                let x = 40. + i as f32 * 300.;
                c.rect(x, 170., 280., 117., 0x272F44, 10.);
                c.text(x + 20., 188., label, 16., 0xAEBAD0);
                c.text(x + 20., 222., value, 35., *col);
            }
            c.rect(40., 310., 880., 190., 0x252C3F, 10.);
            c.text(60., 326., "Activity", 17., 0xC7D2E6);
            for i in 0..14 {
                let height = 35. + ((i * 47 + 29) % 95) as f32;
                c.rect(
                    73. + i as f32 * 58.,
                    476. - height,
                    29.,
                    height,
                    if i > 9 { 0xA4B1ED } else { 0x6376B6 },
                    4.,
                );
            }
            c.line((60., 477.), (897., 477.), 0x3A455D, 1.);
        }
    }
}
pub fn fitted(rect: Rect) -> Rect {
    let size = vec2(
        rect.width().min(rect.height() * 16. / 9.),
        rect.height().min(rect.width() * 9. / 16.),
    );
    Rect::from_center_size(rect.center(), size)
}
