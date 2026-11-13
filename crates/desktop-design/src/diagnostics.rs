#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Profile,
    Appearance,
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
            .max_height((ui.available_height() - 115.).max(65.))
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
            for tab in [Tab::Profile, Tab::Appearance, Tab::Diagnostics] {
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
}
