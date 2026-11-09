//! Interactive approved compact toolbar; prototype presentation only.
use crate::app::{App, Intent, label, rect};
use egui::{Align2, Color32, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
#[derive(Clone, Copy)]
enum Icon {
    Grid,
    Gear,
    Stop,
    Monitor,
    Speaker(bool),
    Leave,
}
#[derive(Clone, Copy)]
enum Tone {
    Normal,
    Group,
    Selected,
    Stop,
    Primary,
    Leave,
}
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |x: u8, y: u8| (x as f32 * (1. - t) + y as f32 * t).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}
fn draw_icon(ui: &Ui, area: Rect, icon: Icon, color: Color32) {
    let c = area.center();
    let stroke = Stroke::new(1.4, color);
    match icon {
        Icon::Grid => {
            for dx in [-5., 2.] {
                for dy in [-5., 2.] {
                    ui.painter().rect_stroke(
                        rect(c.x + dx, c.y + dy, 4., 4.),
                        1.,
                        Stroke::new(1.2, color),
                        StrokeKind::Inside,
                    );
                }
            }
        }
        Icon::Gear => {
            let points = (0..64)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 64.;
                    let r = if (1..=4).contains(&(i % 8)) { 8. } else { 6.3 };
                    c + vec2(a.cos() * r, a.sin() * r)
                })
                .collect();
            ui.painter().add(egui::epaint::PathShape::closed_line(
                points,
                Stroke::new(1.3, color),
            ));
            ui.painter().circle_stroke(c, 2.6, Stroke::new(1.3, color));
        }
        Icon::Stop => {
            ui.painter()
                .rect_filled(Rect::from_center_size(c, vec2(8., 8.)), 1., color);
        }
        Icon::Monitor => {
            ui.painter().rect_stroke(
                Rect::from_center_size(c - vec2(0., 1.), vec2(16., 10.)),
                2.,
                stroke,
                StrokeKind::Inside,
            );
            ui.painter()
                .line_segment([c + vec2(0., 4.), c + vec2(0., 8.)], stroke);
            ui.painter()
                .line_segment([c + vec2(-4., 8.), c + vec2(4., 8.)], stroke);
        }
        Icon::Speaker(muted) => crate::icons::speaker(ui, area, color, muted),
        Icon::Leave => {
            ui.painter().add(egui::epaint::PathShape::line(
                [(-1., -7.), (-8., -7.), (-8., 7.), (-1., 7.)]
                    .into_iter()
                    .map(|(x, y)| c + vec2(x, y))
                    .collect(),
                stroke,
            ));
            for (a, b) in [
                ((-2., 0.), (9., 0.)),
                ((5., -4.), (9., 0.)),
                ((9., 0.), (5., 4.)),
            ] {
                ui.painter()
                    .line_segment([c + vec2(a.0, a.1), c + vec2(b.0, b.1)], stroke);
            }
        }
    }
}
impl App {
    pub fn footer_selection_color(&self) -> Color32 {
        let p = self.palette();
        mix(p.surface, p.accent, 0.13)
    }
    fn bar_button(
        &mut self,
        ui: &mut Ui,
        key: &str,
        r: Rect,
        title: &str,
        icon: Icon,
        tone: Tone,
    ) -> bool {
        let p = self.palette();
        let response = ui.interact(r, Id::new(("bottom-action", key)), Sense::click());
        let (fill, border, text) = match tone {
            Tone::Selected => (self.footer_selection_color(), Color32::TRANSPARENT, p.text),
            Tone::Group => (p.surface, Color32::TRANSPARENT, p.text),
            Tone::Stop => (mix(p.bg, p.red, 0.12), mix(p.bg, p.red, 0.28), p.red),
            Tone::Primary => (p.accent, p.accent, p.accent_text),
            Tone::Leave => (p.bg, p.border, p.red),
            Tone::Normal => (p.surface, p.border, p.text),
        };
        let grouped = matches!(tone, Tone::Group | Tone::Selected);
        let painted = if grouped { r.shrink(3.) } else { r };
        ui.painter().rect_filled(
            painted,
            6.,
            if response.hovered() {
                mix(fill, p.text, 0.07)
            } else {
                fill
            },
        );
        if !grouped {
            ui.painter()
                .rect_stroke(painted, 6., Stroke::new(1., border), StrokeKind::Inside);
        }
        let font = if key == "share" { 13. } else { 12. };
        let text_width = if title.is_empty() {
            0.
        } else {
            ui.painter()
                .layout_no_wrap(title.into(), FontId::proportional(font), text)
                .size()
                .x
        };
        let icon_width = if matches!(icon, Icon::Stop) { 8. } else { 16. };
        let gap = if title.is_empty() { 0. } else { 8. };
        let width = icon_width + gap + text_width;
        let content = Rect::from_center_size(r.center(), vec2(width, 20.));
        self.regions
            .push((format!("bottom-button-content-{key}"), content));
        let icon_area = Rect::from_center_size(
            pos2(content.left() + icon_width / 2., r.center().y),
            vec2(20., 20.),
        );
        draw_icon(
            ui,
            icon_area,
            icon,
            if matches!(tone, Tone::Selected) {
                p.accent
            } else if matches!(tone, Tone::Normal | Tone::Group) {
                p.muted
            } else {
                text
            },
        );
        if !title.is_empty() {
            ui.painter().text(
                pos2(content.left() + icon_width + gap, r.center().y),
                Align2::LEFT_CENTER,
                title,
                FontId::proportional(font),
                text,
            );
        }
        self.hits.insert(key.into(), r);
        let tooltip = match key {
            "appearance" => p2pshare_media::tr!("Appearance & profile"),
            "mute-all" if title.is_empty() => {
                if self.members.iter().skip(1).all(|m| m.muted) {
                    p2pshare_media::tr!("Unmute all")
                } else {
                    p2pshare_media::tr!("Mute all")
                }
            }
            _ => title,
        };
        response.on_hover_text(tooltip).clicked()
    }
    pub(crate) fn profile_footer(&mut self, ui: &mut Ui, r: Rect) {
        let p = self.palette();
        let c = pos2(r.left() + 28., r.center().y);
        ui.painter().circle_filled(c, 14., p.accent);
        let name = if self.config.username.trim().is_empty() {
            p2pshare_media::tr!("You")
        } else {
            &self.config.username
        };
        let initial = name
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .to_string();
        let glyph = ui
            .painter()
            .layout_no_wrap(initial, FontId::proportional(14.), p.accent_text);
        let ink = if glyph.mesh_bounds.is_positive() {
            glyph.mesh_bounds
        } else {
            glyph.rect
        };
        ui.painter()
            .galley(c - ink.center().to_vec2(), glyph, p.accent_text);
        if let Some(texture)=self.avatars.get(&0){crate::profile::circle(ui.painter(),c,14.,*texture,Rect::from_min_max(pos2(0.,0.),pos2(1.,1.)));}
        let gear = rect(r.right() - 42., r.top() + 6., 32., 32.);
        label(
            ui,
            rect(
                r.left() + 52.,
                r.center().y - 12.,
                (gear.left() - r.left() - 60.).max(1.),
                24.,
            ),
            name,
            14.,
            p.text,
        );
        if self.bar_button(ui, "appearance", gear, "", Icon::Gear, Tone::Normal) {
            self.open_appearance();
        }
    }
    pub(crate) fn room_footer(&mut self, ui: &mut Ui, r: Rect) {
        let p = self.palette();
        let y = r.top() + 6.;
        ui.painter()
            .hline(r.left()..=r.right(), r.top(), Stroke::new(1., p.border));
        let compact = r.width() < 780.;
        let group_width = if compact { 240. } else { 260. };
        let group = rect(r.left() + 12., y, group_width, 32.);
        ui.painter().rect_filled(group, 6., p.surface);
        ui.painter()
            .rect_stroke(group, 6., Stroke::new(1., p.border), StrokeKind::Inside);
        if self.bar_button(
            ui,
            "grid",
            rect(group.left(), y, 120., 32.),
            p2pshare_media::tr!("All screens"),
            Icon::Grid,
            if self.focus.is_none() {
                Tone::Selected
            } else {
                Tone::Group
            },
        ) {
            self.focus = None;
        }
        if self.bar_button(
            ui,
            "share-settings",
            rect(group.left() + 122., y, group_width - 122., 32.),
            p2pshare_media::tr!("Stream settings"),
            Icon::Gear,
            Tone::Group,
        ) {
            self.open_share();
            self.settings_only = self.live;
        }
        ui.painter().vline(
            group.left() + 121.,
            (y + 7.)..=(y + 25.),
            Stroke::new(1., p.border),
        );
        let active = self.members[0].scene.is_some();
        let share = rect(group.right() + 12., y, 128., 32.);
        if self.bar_button(
            ui,
            "share",
            share,
            if active {
                p2pshare_media::tr!("Stop sharing")
            } else {
                p2pshare_media::tr!("Share screen")
            },
            if active { Icon::Stop } else { Icon::Monitor },
            if active { Tone::Stop } else { Tone::Primary },
        ) {
            if active {
                self.members[0].scene = None;
                self.focus = None;
                if self.live {
                    self.intents.push(Intent::Stop);
                }
            } else if self.joined {
                self.open_share();
            } else {
                self.open_room();
            }
        }
        let has_remote = self.members.len() > 1;
        let all = has_remote && self.members.iter().skip(1).all(|m| m.muted);
        let mute = rect(
            share.right() + 12.,
            y,
            if compact { 32. } else { 104. },
            32.,
        );
        let title = if compact {
            ""
        } else if all {
            p2pshare_media::tr!("Unmute all")
        } else {
            p2pshare_media::tr!("Mute all")
        };
        if ui
            .add_enabled_ui(has_remote, |ui| {
                self.bar_button(
                    ui,
                    "mute-all",
                    mute,
                    title,
                    Icon::Speaker(all),
                    Tone::Normal,
                )
            })
            .inner
        {
            for member in self.members.iter_mut().skip(1) {
                member.muted = !all;
            }
        }
        if self.bar_button(
            ui,
            "leave",
            rect(r.right() - 124., y, 112., 32.),
            if self.joined {
                p2pshare_media::tr!("Leave room")
            } else {
                p2pshare_media::tr!("Join room")
            },
            Icon::Leave,
            if self.joined {
                Tone::Leave
            } else {
                Tone::Normal
            },
        ) {
            if self.joined {
                self.joined = false;
                self.focus = None;
                self.members[0].scene = None;
                if self.live {
                    self.intents.push(Intent::Leave);
                }
            } else {
                self.open_room();
            }
        }
    }
}
