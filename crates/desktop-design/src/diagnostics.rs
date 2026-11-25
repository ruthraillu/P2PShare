#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Profile,
    Appearance,
    ShareGroups,
    Diagnostics,
}

pub struct Run {
    pub id: String,
    pub time: String,
    pub status: &'static str,
}

pub struct Receipt {
    pub id: String,
    pub timestamp: Option<String>,
}

#[derive(Default)]
pub struct Diagnostics {
    pub runs: Vec<Run>,
    pub configured: bool,
    pub sending: bool,
    pub result: Option<Result<Receipt, String>>,
    pub(crate) selected: Option<String>,
}

impl crate::app::App {
    pub(crate) fn diagnostics_panel(&mut self, ui: &mut egui::Ui) {
        use crate::app::Intent;
        use p2pshare_media::tr;

        let state = &mut self.diagnostics;
        ui.heading(tr!("Recent runs"));
        if !state.configured {
            ui.label(tr!("Set a secure signaling server to send diagnostics."));
        }
        if state.runs.is_empty() {
            ui.label(tr!("No diagnostic runs yet."));
        }
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 150.).max(65.))
            .show(ui, |ui| {
                for run in &state.runs {
                    ui.radio_value(
                        &mut state.selected,
                        Some(run.id.clone()),
                        format!(
                            "{}  ·  {}",
                            run.time,
                            p2pshare_media::i18n::status(run.status)
                        ),
                    );
                }
            });
        ui.add_space(12.);
        if ui
            .add_enabled(
                state.configured
                    && !state.sending
                    && state
                        .runs
                        .iter()
                        .any(|run| Some(&run.id) == state.selected.as_ref()),
                egui::Button::new(tr!("Send diagnostics")),
            )
            .clicked()
            && let Some(id) = state.selected.clone()
        {
            state.sending = true;
            state.result = None;
            self.intents.push(Intent::SendDiagnostics { id });
        }
        if state.sending {
            ui.label(tr!("Sending diagnostics…"));
        }
        match &state.result {
            Some(Ok(receipt)) => {
                ui.label(tr!("Diagnostics sent"));
                if let Some(timestamp) = &receipt.timestamp {
                    ui.label(timestamp);
                }
                ui.label(format!("{} {}", tr!("Report ID"), receipt.id));
            }
            Some(Err(error)) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    p2pshare_media::i18n::status(error),
                );
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::App, config::Config};

    #[test]
    fn settings_tabs_render_in_both_languages() {
        use p2pshare_media::i18n::{Language, scope};
        let ctx = egui::Context::default();
        let mut app = App::new(Config::default(), None);
        app.open_appearance();
        app.diagnostics.runs.push(Run {
            id: "fixture".into(),
            time: "2026-11-13 01:48:00 UTC".into(),
            status: "Ended unexpectedly",
        });
        for language in [Language::English, Language::PtBr] {
            let _language = scope(language);
            for tab in [Tab::Profile, Tab::Appearance, Tab::ShareGroups, Tab::Diagnostics] {
                app.settings_tab = tab;
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    ..Default::default()
                };
                let output = ctx.run_ui(input, |ui| app.draw_dialog(ui.ctx()));
                assert!(!output.shapes.is_empty());
            }
        }
    }

    #[test]
    fn diagnostics_receipt_fits_inside_settings_modal_with_ten_runs() {
        fn find_text(shape: &egui::Shape, needle: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                    Some(text.visual_bounding_rect())
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find_text(s, needle)),
                _ => None,
            }
        }
        let ctx = egui::Context::default();
        let mut app = App::new(Config::default(), None);
        app.open_appearance();
        app.settings_tab = Tab::Diagnostics;
        app.diagnostics.runs = (0..10)
            .map(|i| Run {
                id: format!("fixture-{i}"),
                time: format!("2026-11-25 18:2{i}:00 UTC"),
                status: "Completed",
            })
            .collect();
        app.diagnostics.result = Some(Ok(Receipt {
            id: "87b926cf".into(),
            timestamp: Some("2026-11-25 18:22:06 UTC".into()),
        }));
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 832.),
                    )),
                    ..Default::default()
                },
                |ui| app.draw_dialog(ui.ctx()),
            ));
        }
        let output = output.unwrap();
        let modal = app
            .regions
            .iter()
            .find(|(name, _)| name == "modal")
            .map(|(_, rect)| *rect)
            .expect("Settings modal bounds missing");
        let (clip, receipt_rect) = output
            .shapes
            .iter()
            .find_map(|clipped| {
                find_text(&clipped.shape, "87b926cf").map(|rect| (clipped.clip_rect, rect))
            })
            .expect("Report ID receipt text missing");
        assert!(
            receipt_rect.bottom() <= modal.bottom() - 20. && receipt_rect.bottom() <= clip.bottom(),
            "Receipt text clipped: receipt={receipt_rect:?}, clip={clip:?}, modal={modal:?}"
        );
    }
}
