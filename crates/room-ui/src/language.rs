use crate::{RoomUi,Action,theme::*};
use egui::Color32;
impl RoomUi {
 pub(crate) fn language_dialog(&mut self,root:&mut egui::Ui)->Vec<Action>{
  self.snapshot=Default::default();self.profiles.hits.clear();root.painter().rect_filled(root.max_rect(),0.,BG);let mut actions=Vec::new();
  let safe=crate::layout::safe_rect(root.max_rect(),self.safe_insets);
  egui::Modal::new(egui::Id::new("first-language")).frame(egui::Frame::new().fill(PANEL).corner_radius(14).inner_margin(20)).show(root.ctx(),|ui|{
   ui.set_width((safe.width()-56.).min(340.));ui.label(egui::RichText::new("Choose your language").size(21.).color(Color32::WHITE));ui.label(egui::RichText::new("Escolha seu idioma").size(18.).color(MUTED));ui.add_space(10.);
   for (language,label,key) in [(p2pshare_media::i18n::Language::English,"English","language-en"),(p2pshare_media::i18n::Language::PtBr,"Português (Brasil)","language-pt-BR")]{let b=ui.add_sized([ui.available_width(),44.],egui::Button::new(label));self.profiles.hits.insert(key.into(),b.rect);if b.clicked(){self.language=language;self.language_prompt=false;actions.push(Action::SetLanguage(language));ui.ctx().request_repaint();}}
   if self.status.starts_with("Could not save language:"){ui.label(egui::RichText::new(p2pshare_media::i18n::status(&self.status)).color(Color32::LIGHT_RED));}
  });actions
 }
}
