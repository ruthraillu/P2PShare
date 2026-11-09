use crate::app::{App, Intent};
use egui::{Button, Color32, Frame, Id, Modal, RichText, ScrollArea, Stroke, Ui, vec2};
use p2pshare_media::tr;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Available,
    Downloading(u8),
    Ready,
}

pub struct Notice {
    pub current: String,
    pub version: String,
    pub notes: String,
    pub phase: Phase,
    pub error: Option<String>,
}

impl Phase {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Available => tr!("Update available"),
            Self::Downloading(_) => tr!("Downloading update…"),
            Self::Ready => tr!("Update ready"),
        }
    }
}

impl App {
    pub(crate) fn draw_update_dialog(&mut self, ui: &mut Ui) {
        if !self.update_open {
            return;
        }
        let Some(update) = &self.update else { return };
        let p = self.palette();
        let mut close = false;
        let modal = Modal::new(Id::new("p2pshare-update"))
            .frame(
                Frame::new()
                    .fill(p.side)
                    .stroke(Stroke::new(1., p.border))
                    .inner_margin(24)
                    .corner_radius(14),
            )
            .show(ui.ctx(), |ui| {
                ui.set_width(480.);
                ui.horizontal(|ui| {
                    ui.heading(if update.phase == Phase::Ready {
                        tr!("Ready to install")
                    } else {
                        tr!("P2PShare update")
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= ui.button("×").clicked();
                    });
                });
                ui.label(
                    RichText::new(format!("{}  →  {}", update.current, update.version))
                        .color(p.muted),
                );
                ui.add_space(8.);
                ui.separator();
                ui.add_space(8.);
                match update.phase {
                    Phase::Available => {
                        ui.label(RichText::new(tr!("Release notes")).strong());
                        ScrollArea::vertical().max_height(140.).show(ui, |ui| {
                            ui.add(
                                egui::Label::new(if update.notes.trim().is_empty() {
                                    tr!("No release notes provided.")
                                } else {
                                    &update.notes
                                })
                                .wrap(),
                            );
                        });
                        ui.add_space(8.);
                        ui.label(
                            RichText::new(tr!("You can keep using the app while it downloads."))
                                .color(p.muted),
                        );
                    }
                    Phase::Downloading(percent) => {
                        ui.label(tr!("Downloading update…"));
                        ui.add(egui::ProgressBar::new(f32::from(percent) / 100.).show_percentage());
                        ui.label(
                            RichText::new(tr!("You can keep using the app while it downloads."))
                                .color(p.muted),
                        );
                    }
                    Phase::Ready => {
                        ui.label(
                            RichText::new(tr!("Version {} is downloaded and verified.").replacen(
                                "{}",
                                &update.version,
                                1,
                            ))
                            .color(p.green),
                        );
                        ui.add(egui::ProgressBar::new(1.).fill(p.green).show_percentage());
                        ui.add_space(8.);
                        if self.joined {
                            ui.label(tr!("Restarting will leave your current room."));
                        }
                        ui.label(
                            RichText::new(tr!("Your settings and profile are kept."))
                                .color(p.muted),
                        );
                    }
                }
                if let Some(error) = &update.error {
                    ui.add_space(6.);
                    ui.label(
                        RichText::new(tr!("Update failed: {}").replacen("{}", error, 1))
                            .color(Color32::LIGHT_RED),
                    );
                }
                ui.add_space(12.);
                ui.separator();
                ui.add_space(6.);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let primary = match update.phase {
                        Phase::Available => Some((tr!("Download update"), Intent::DownloadUpdate)),
                        Phase::Ready => Some((tr!("Restart and install"), Intent::InstallUpdate)),
                        Phase::Downloading(_) => None,
                    };
                    if let Some((text, intent)) = primary {
                        let button = Button::new(RichText::new(text).color(p.accent_text))
                            .min_size(vec2(146., 32.))
                            .fill(p.accent);
                        let response = ui.add(button);
                        self.hits.insert("update-primary".into(), response.rect);
                        if response.clicked() {
                            self.intents.push(intent);
                        }
                    }
                    close |= ui
                        .add(Button::new(tr!("Later")).min_size(vec2(82., 32.)))
                        .clicked();
                });
            });
        if close || modal.should_close() {
            self.update_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    #[test]
    fn opening_the_update_dialog_does_not_request_a_download() {
        let context = egui::Context::default();
        let mut app = App::live(Default::default(), String::new(), false);
        app.update = Some(Notice {
            current: "0.12.0".into(),
            version: "0.13.0".into(),
            notes: "Release notes".into(),
            phase: Phase::Available,
            error: None,
        });
        let draw = |app: &mut App, events| {
            let _ = context.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200., 690.))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| app.ui(ui),
            );
        };
        let click = |app: &mut App, id: &str| {
            let pos = app.hits[id].center();
            for pressed in [true, false] {
                draw(
                    app,
                    vec![
                        Event::PointerMoved(pos),
                        Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                );
            }
        };
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        assert!(app.intents.is_empty());
        click(&mut app, "update");
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        assert!(app.update_open);
        assert!(app.intents.is_empty());
        click(&mut app, "update-primary");
        assert!(matches!(app.intents.pop(), Some(Intent::DownloadUpdate)));
        app.update.as_mut().unwrap().phase = Phase::Ready;
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        click(&mut app, "update-primary");
        assert!(matches!(app.intents.pop(), Some(Intent::InstallUpdate)));
    }
}
