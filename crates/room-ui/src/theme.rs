//! Visual tokens; change the palette here without touching media or rooms.
use egui::{Color32, vec2};
pub(crate) const BG: Color32 = Color32::from_rgb(24, 25, 30);
pub(crate) const PANEL: Color32 = Color32::from_rgb(31, 33, 40);
pub(crate) const TILE: Color32 = Color32::from_rgb(39, 42, 51);
pub(crate) const MUTED: Color32 = Color32::from_rgb(158, 164, 181);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(107, 108, 134);
pub(crate) const GREEN: Color32 = Color32::from_rgb(81, 207, 153);

impl crate::RoomUi {
    pub fn configure(ctx: &egui::Context) {
        let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = PANEL;
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.widgets.inactive.bg_fill = TILE;
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(62, 65, 83);
        style.spacing.button_padding = vec2(14.0, 10.0);
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.interact_size = vec2(44.0, 44.0);
        ctx.set_style_of(egui::Theme::Dark, style);
        ctx.set_theme(egui::ThemePreference::Dark);
    }
}
pub(crate) fn initials(name: &str) -> String {
    name.chars().next().unwrap_or('?').to_uppercase().collect()
}
pub(crate) fn avatar_color(id: u64) -> Color32 {
    [
        Color32::from_rgb(111, 119, 235),
        Color32::from_rgb(185, 107, 185),
        Color32::from_rgb(66, 155, 158),
        Color32::from_rgb(203, 143, 79),
    ][id as usize % 4]
}
