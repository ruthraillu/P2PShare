use crate::app::{App,Intent};
use egui::Color32;
impl App {
    pub(crate) fn signaling_dialog(&mut self, root: &mut egui::Ui) {
        let p = self.palette();
        let ctx = root.ctx().clone();
        root.painter().rect_filled(root.max_rect(), 0., p.bg);
        egui::Modal::new(egui::Id::new("first-signaling-server"))
            .frame(egui::Frame::new().fill(p.side).corner_radius(14).inner_margin(24))
            .show(&ctx, |ui| {
                ui.set_width(380.);
                ui.heading(p2pshare_media::tr!("Signaling server"));
                ui.label(p2pshare_media::tr!("Enter the WebSocket URL your group uses."));
                let server = self.signaling_setup.as_mut().unwrap();
                ui.add_sized([ui.available_width(), 38.], egui::TextEdit::singleline(server)
                    .hint_text("wss://").vertical_align(egui::Align::Center).char_limit(2048));
                if ui.add_enabled(!server.trim().is_empty(), egui::Button::new(p2pshare_media::tr!("Continue"))).clicked() {
                    self.intents.push(Intent::SetSignalingServer(server.clone()));
                }
                if !self.notice.is_empty() {
                    ui.label(egui::RichText::new(&self.notice).color(Color32::LIGHT_RED));
                }
            });
        self.window_chrome(&ctx, root.max_rect());
    }

 pub(crate) fn language_dialog(&mut self,root:&mut egui::Ui){
  root.painter().rect_filled(root.max_rect(),0.,self.palette().bg);
  let p=self.palette();let ctx=root.ctx().clone();
  egui::Modal::new(egui::Id::new("first-language")).frame(egui::Frame::new().fill(p.side).corner_radius(14).inner_margin(24)).show(&ctx,|ui|{
   ui.set_width(340.);ui.label(egui::RichText::new("Choose your language").size(22.).color(p.text));ui.label(egui::RichText::new("Escolha seu idioma").size(18.).color(p.muted));ui.add_space(14.);
   for (language,label,key) in [(p2pshare_media::i18n::Language::English,"English","language-en"),(p2pshare_media::i18n::Language::PtBr,"Português (Brasil)","language-pt-BR")]{
    let b=ui.add_sized([ui.available_width(),42.],egui::Button::new(egui::RichText::new(label).size(16.).color(p.text)));self.hits.insert(key.into(),b.rect);
    if b.clicked(){self.config.language=Some(language);self.draft.language=Some(language);self.language_prompt=false;self.intents.push(Intent::Language(language));ui.ctx().request_repaint();}
   }
   if !self.notice.is_empty(){ui.label(egui::RichText::new(p2pshare_media::i18n::status(&self.notice)).color(Color32::LIGHT_RED));}
  });self.window_chrome(&ctx,root.max_rect());
 }
}

#[cfg(test)]mod tests {
 #[test]fn first_language_choice_preserves_pending_name_setup(){
  use egui::{RawInput,Rect,Pos2,vec2,Event,PointerButton,Modifiers};
  let ctx=egui::Context::default();let mut app=crate::app::App::live(Default::default(),"grupo".into(),true);app.language_prompt=true;
  let frame=|app:&mut crate::app::App,events|{let _=ctx.run_ui(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,vec2(900.,600.))),events,focused:true,..Default::default()},|ui|app.ui(ui));};
  for _ in 0..3{frame(&mut app,vec![]);}assert!(app.intents.is_empty());let at=app.hits["language-pt-BR"].center();
  for pressed in [true,false]{frame(&mut app,vec![Event::PointerMoved(at),Event::PointerButton{pos:at,button:PointerButton::Primary,pressed,modifiers:Modifiers::NONE}]);}
  assert!(!app.language_prompt);assert_eq!(app.config.language,Some(p2pshare_media::i18n::Language::PtBr));assert!(matches!(app.dialog,crate::app::Dialog::Name));assert_eq!(app.intents.iter().filter(|i|matches!(i,crate::app::Intent::Language(_))).count(),1);
 }
}
