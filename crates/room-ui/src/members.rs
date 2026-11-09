use crate::{Action, RoomUi, theme::*};
use egui::{Align2, Color32, FontId, RichText, Sense, Ui, vec2};
impl RoomUi {
    pub(crate) fn sidebar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.label(RichText::new(p2pshare_media::tr!("USERS")).size(11.0).color(MUTED).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("# {}", self.room_name))
                .color(ACCENT)
                .strong(),
        );
        ui.add_space(18.0);
        ui.label(
            RichText::new(p2pshare_media::tr!("IN THIS ROOM"))
                .size(11.0)
                .color(MUTED)
                .strong(),
        );
        for p in &mut self.participants {
            ui.horizontal(|ui| {
                let color = avatar_color(p.id);
                let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 15.0, color);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    initials(&p.name),
                    FontId::proportional(13.0),
                    Color32::WHITE,
                );
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!("{}{}", p.name, if p.local { p2pshare_media::tr!(" (you)") } else { "" }))
                            .strong(),
                    );
                    ui.label(
                        RichText::new(if p.sharing {
                            p2pshare_media::tr!("Sharing a screen")
                        } else {
                            p2pshare_media::tr!("In the lounge")
                        })
                        .size(11.0)
                        .color(if p.sharing { GREEN } else { MUTED }),
                    );
                });
            });
            ui.add_space(4.0);
        }
        ui.separator();
        ui.label(RichText::new(p2pshare_media::tr!("Display name")).small().color(MUTED));
        if cfg!(target_os = "android") {
            ui.label(&self.name);
            if ui.button(p2pshare_media::tr!("Change name")).clicked() {
                actions.push(Action::EditName(self.name.clone()));
            }
        } else {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.name)
                        .desired_width(ui.available_width())
                        .char_limit(32),
                )
                .changed()
                && let Some(p) = self.participants.iter_mut().find(|p| p.local)
            {
                p.name = if self.name.trim().is_empty() {
                    p2pshare_media::tr!("You").into()
                } else {
                    self.name.trim().into()
                };
            }
        }
    }
}
