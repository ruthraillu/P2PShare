//! Compact phone navigation, with one source of truth for room/count icons.
use crate::{RoomUi,Action,layout::Layout,theme::*};
use egui::{Ui,Rect,vec2,pos2,Align2,FontId,Color32,Sense};
fn glyph(ui:&Ui,c:egui::Pos2,kind:usize,color:Color32){
    let p=ui.painter();let stroke=egui::Stroke::new(1.7,color);
    let q=1.12;
    let pt=|x:f32,y:f32| c+vec2(x*q,y*q);
    if kind==0 {
        p.circle_stroke(pt(-3.5,-4.0),3.0*q,stroke);p.circle_stroke(pt(4.5,-2.5),2.5*q,stroke);
        p.add(egui::Shape::line(vec![pt(-9.5,7.0),pt(-8.0,2.0),pt(-4.0,0.0),pt(0.5,2.0),pt(2.5,7.0)],stroke));
        p.add(egui::Shape::line(vec![pt(2.0,2.0),pt(6.0,1.0),pt(9.5,4.0),pt(10.0,7.0)],stroke));
    }else if kind==1{
        p.rect_stroke(Rect::from_min_max(pt(-9.0,-8.0),pt(9.0,4.0)),2.0,stroke,egui::StrokeKind::Inside);
        p.line_segment([pt(0.0,4.0),pt(0.0,8.0)],stroke);p.line_segment([pt(-6.0,8.0),pt(6.0,8.0)],stroke);
    }else{
        p.circle_stroke(c,8.0*q,stroke);p.circle_stroke(c,3.0*q,stroke);
        for i in 0..8 {let a=i as f32*std::f32::consts::FRAC_PI_4;p.line_segment([pt(a.cos()*8.0,a.sin()*8.0),pt(a.cos()*10.5,a.sin()*10.5)],stroke);}
    }
}

fn badge_text(count:usize)->String{if count>99{"99+".into()}else{count.to_string()}}
fn badge_size(ui:&Ui,count:usize)->egui::Vec2{
    let galley=ui.painter().layout_no_wrap(badge_text(count),FontId::proportional(10.0),Color32::WHITE);
    vec2((galley.size().x+7.0).max(16.0),16.0)
}
fn count_badge(ui:&Ui,center:egui::Pos2,count:usize,active:bool){
    let rect=Rect::from_center_size(center,badge_size(ui,count));
    ui.painter().rect_filled(rect,8.0,if active{Color32::from_rgb(82,77,121)}else{Color32::from_rgb(62,64,82)});
    ui.painter().text(center,Align2::CENTER_CENTER,badge_text(count),FontId::proportional(10.0),Color32::WHITE);
}

impl RoomUi{
    fn mobile_share_button(&mut self,ui:&mut Ui,actions:&mut Vec<Action>){
        if self.capture_supported && self.in_room {
            let narrow=ui.available_width()<80.0;
            let label=if self.capture_running{if narrow{p2pshare_media::tr!("Stop")}else{p2pshare_media::tr!("Stop sharing")}}else if narrow{p2pshare_media::tr!("Share")}else{p2pshare_media::tr!("Share screen")};
            let (rect,response)=ui.allocate_exact_size(vec2(ui.available_width(),36.0),Sense::click());
            ui.painter().rect_filled(rect,6.0,if self.capture_running{Color32::from_rgb(73,40,48)}else{Color32::from_rgb(54,53,75)});
            let font=if narrow{11.0}else{13.0};let width=ui.painter().layout_no_wrap(label.into(),FontId::proportional(font),Color32::WHITE).size().x;let font=(font*((rect.width()-6.0)/width.max(1.0)).min(1.0)).max(9.0);
            ui.painter().text(rect.center(),Align2::CENTER_CENTER,label,FontId::proportional(font),Color32::WHITE);
            response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,true,label));
            self.snapshot.share_button=Some(rect);
            if response.clicked(){actions.push(if self.capture_running{Action::StopCapture}else{Action::StartCapture(0)});}

        }
    }
    pub(crate) fn mobile_navigation(&mut self,root:&mut Ui,layout:&Layout,actions:&mut Vec<Action>){
        if layout.landscape {
            egui::Panel::left("mobile_rail").exact_size(56.0).frame(egui::Frame::new().fill(PANEL).inner_margin(4)).show(root,|ui|{
                let (title,response)=ui.allocate_exact_size(vec2(ui.available_width(),48.0),Sense::click());
                let mut job=egui::text::LayoutJob::simple(self.room_name.clone(),FontId::proportional(11.0),Color32::WHITE,title.width());
                job.wrap.max_rows=2;let galley=ui.painter().layout_job(job);
                ui.painter_at(title).galley(title.center()-galley.size()*0.5,galley,Color32::WHITE);
                if response.clicked(){if self.focused.is_some(){self.focused=None;self.mobile_controls_until=0.0;}else{actions.push(Action::EditRoom(self.room_name.clone()));}}
                self.mobile_share_button(ui,actions);
                ui.add_space(8.0);
                for (index,label) in [(1,p2pshare_media::tr!("Screens")),(0,p2pshare_media::tr!("Users")),(2,p2pshare_media::tr!("Settings"))]{
                    if index==2 {ui.add_space((ui.available_height()-64.0).max(0.0));}
                    let (r,b)=ui.allocate_exact_size(vec2(ui.available_width(),if index==2 {60.0}else{76.0}),Sense::click());
                    self.snapshot.navigation_buttons.push((if index==1 {0}else if index==0 {1}else{2},r));
                    let active=match index {1=>!self.members_open&&!self.room_dialog,0=>self.members_open,_=>self.room_dialog};
                    let color=if active{Color32::from_rgb(184,178,250)}else{MUTED};
                    // Keep all glyphs on one center line; badges must not shift the glyph.
                    if active{ui.painter().rect_filled(r.shrink2(vec2(0.0,8.0)),10.0,Color32::from_rgb(54,53,75));}
                    if index==1{self.snapshot.grid_button=Some(r);}
                    let with_count=index!=2;
                    let center=r.center()-vec2(0.0,10.0);
                    glyph(ui,center,index,color);
                    if with_count {
                        let count=if index==1 {self.participants.iter().filter(|p|p.sharing).count()}else{self.participants.len()};
                        count_badge(ui,center+vec2(16.0,-9.0),count,active);
                    }
                    ui.painter().text(r.center()+vec2(0.0,17.0),Align2::CENTER_CENTER,label,FontId::proportional(9.0),color);
                    if b.clicked(){match index{1=>{self.close_mobile_page();self.focused=None;self.mobile_controls_until=0.0;},0=>self.open_users(),_=>self.open_settings()}}
                }
            });
        }else{
            self.mobile_bottom_navigation(root);
            egui::Panel::top("phone_room_header").frame(egui::Frame::new().fill(BG).inner_margin(4)).show(root,|ui|{
                let (row,_)=ui.allocate_exact_size(vec2(ui.available_width(),32.0),Sense::hover());
                let mut title=row;
                if self.capture_supported && self.in_room {
                    let share=Rect::from_min_max(pos2((row.right()-116.0).max(row.left()),row.top()),row.max);
                    let mut child=ui.new_child(egui::UiBuilder::new().max_rect(share));self.mobile_share_button(&mut child,actions);
                    title.max.x=share.left()-6.0;
                }
                ui.painter_at(title).text(title.left_center()+vec2(4.0,0.0),Align2::LEFT_CENTER,&self.room_name,FontId::proportional(15.0),Color32::WHITE);
                if ui.interact(title,ui.id().with("room_title"),Sense::click()).clicked(){actions.push(Action::EditRoom(self.room_name.clone()));}
            });
        }
    }
    /// Shared by the gallery, player, Users and Settings pages.
    pub(crate) fn mobile_bottom_navigation(&mut self,root:&mut Ui){
        egui::Panel::bottom("phone_nav").frame(egui::Frame::new().fill(PANEL).inner_margin(4)).show(root,|ui|{
            ui.horizontal(|ui|{
                let width=((ui.available_width()-20.0)/3.0).max(1.0);
                for (index,label,kind) in [(0,p2pshare_media::tr!("Screens"),1),(1,p2pshare_media::tr!("Users"),0),(2,p2pshare_media::tr!("Settings"),2)] {
                    let (r,b)=ui.allocate_exact_size(vec2(width,44.0),Sense::click());
                    self.snapshot.navigation_buttons.push((index,r));
                    if index==0 {self.snapshot.grid_button=Some(r);}
                    let active=match index {0=>!self.members_open&&!self.room_dialog,1=>self.members_open,_=>self.room_dialog};
                    let color=if active{Color32::from_rgb(184,178,250)}else{MUTED};
                    if active {ui.painter().rect_filled(r.shrink2(vec2(2.0,4.0)),16.0,Color32::from_rgb(54,53,75));}
                    let galley=ui.painter().layout_no_wrap(label.into(),FontId::proportional(if r.width()<110.0{11.0}else{12.0}),color);
                    let count=match index{0=>Some(self.participants.iter().filter(|p|p.sharing).count()),1=>Some(self.participants.len()),_=>None};

                    let total=24.0+4.0+galley.size().x;
                    let start=r.center().x-total*0.5;
                    glyph(ui,pos2(start+12.0,r.center().y),kind,color);
                    let text_y=r.center().y-galley.size().y*0.5;
                    let text_width=galley.size().x;
                    ui.painter().galley(pos2(start+28.0,text_y),galley,color);
                    if let Some(count)=count{
                        let left=start+28.0+text_width+2.0;
                        let width=badge_size(ui,count).x.min((r.right()-1.0-left).max(8.0));
                        let badge=Rect::from_center_size(pos2(left+width*0.5,r.center().y),vec2(width,16.0));
                        let text=badge_text(count);
                        let measured=ui.painter().layout_no_wrap(text.clone(),FontId::proportional(10.0),Color32::WHITE).size().x;
                        let font_size=10.0*((width-3.0)/measured.max(1.0)).min(1.0);
                        ui.painter().rect_filled(badge,8.0,if active{Color32::from_rgb(82,77,121)}else{Color32::from_rgb(62,64,82)});
                        ui.painter().text(badge.center(),Align2::CENTER_CENTER,text,FontId::proportional(font_size),Color32::WHITE);
                    }
                    if b.clicked(){match index {
                        0=>{self.close_mobile_page();self.focused=None;self.mobile_controls_until=0.0;},
                        1=>self.open_users(),_=>self.open_settings()
                    }}
                }
            });
        });
    }
}
