use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2, vec2};
use p2pshare_room_ui::{Action, Participant, RoomUi};
fn live_room(room: &mut RoomUi) {
    room.in_room = true;
    room.busy = false;
    for id in 1..=3 {
        room.participants.push(Participant {
            id,
            name: format!("Peer {id}"),
            local: false,
            sharing: true,
            texture: None,
            size: p2pshare_media::Dimensions::new(1280, 720),
        });
    }
}
fn frame(ctx: &egui::Context, room: &mut RoomUi, size: Vec2, events: Vec<Event>) -> Vec<Action> {
    let mut actions = vec![];
    let _ = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            actions = room.ui(ui);
        },
    );
    actions
}
fn click(ctx: &egui::Context, room: &mut RoomUi, size: Vec2, pos: Pos2) -> Vec<Action> {
    let mut actions = frame(
        ctx,
        room,
        size,
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    actions.extend(frame(
        ctx,
        room,
        size,
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ],
    ));
    actions
}
#[test]
fn screen_click_focuses_and_grid_returns_to_all_streams() {
    let ctx = egui::Context::default();
    RoomUi::configure(&ctx);
    let mut room = RoomUi::new(true);
    live_room(&mut room);
    let size = vec2(1280.0, 800.0);
    frame(&ctx, &mut room, size, vec![]);
    let (id, rect) = room.snapshot.tiles[1];
    click(&ctx, &mut room, size, rect.center());
    assert_eq!(room.focused, Some(id));
    frame(&ctx, &mut room, size, vec![]);
    assert_eq!(room.snapshot.tiles[0].0, id);
    let grid = room.snapshot.grid_button.unwrap();
    click(&ctx, &mut room, size, grid.center());
    assert_eq!(room.focused, None);
}
#[test]
fn phone_layout_stays_in_bounds_and_touch_focus_works() {
    let ctx = egui::Context::default();
    RoomUi::configure(&ctx);
    let mut room = RoomUi::new(false);
    live_room(&mut room);
    let size = vec2(393.0, 780.0);
    frame(&ctx, &mut room, size, vec![]);
    for (_, rect) in &room.snapshot.tiles {
        assert!(
            rect.left() >= 0.0 && rect.right() <= size.x + 1.0,
            "{rect:?}"
        );
    }
    let (id, rect) = room.snapshot.tiles[0];
    click(&ctx, &mut room, size, rect.center());
    assert_eq!(room.focused, Some(id));
    frame(&ctx, &mut room, size, vec![]);
    assert_eq!(room.snapshot.tiles[0].0, id);
}
#[test]
fn closing_a_focused_stream_returns_to_grid() {
    let ctx = egui::Context::default();
    RoomUi::configure(&ctx);
    let mut room = RoomUi::new(false);
    live_room(&mut room);
    room.focused = Some(2);
    room.participants
        .iter_mut()
        .find(|p| p.id == 2)
        .unwrap()
        .sharing = false;
    frame(&ctx, &mut room, vec2(1280.0, 800.0), vec![]);
    assert_eq!(room.focused, None);
    assert!(room.snapshot.tiles.iter().all(|(id, _)| *id != 2));
}
#[test]
fn sharing_requires_a_room_and_windows_requests_source_enumeration() {
    let ctx = egui::Context::default();
    RoomUi::configure(&ctx);
    let size = vec2(1280.0, 800.0);
    let mut room = RoomUi::new(true);
    room.in_room = true;
    room.busy = false;
    frame(&ctx, &mut room, size, vec![]);
    let pos = room.snapshot.share_button.unwrap().center();
    let actions = click(&ctx, &mut room, size, pos);
    assert!(actions.contains(&Action::RefreshSources));
    assert!(!actions.iter().any(|a| matches!(a, Action::StartCapture(_))));
    let mut mobile = RoomUi::new(false);
    let size = vec2(393.0, 780.0);
    frame(&ctx, &mut mobile, size, vec![]);
    assert!(mobile.snapshot.share_button.is_none());
    assert!(!mobile.participants[0].sharing);
    assert!(!mobile.capture_running);
    mobile.set_platform(p2pshare_room_ui::Platform::Android);
    live_room(&mut mobile);
    frame(&ctx, &mut mobile, vec2(740.0, 393.0), vec![]);
    assert!(mobile.snapshot.share_button.is_none());
    assert_eq!(mobile.snapshot.tiles.len(), 3);
}

#[test]
fn android_landscape_uses_phone_chrome_and_a_large_focused_view() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);
    for size in [vec2(852.0,393.0),vec2(640.0,360.0),vec2(393.0,852.0)] {
        frame(&ctx,&mut room,size,vec![]);frame(&ctx,&mut room,size,vec![]);
        let video=room.snapshot.viewer_rect.unwrap();assert!(video.width()>size.x*0.60);assert!(video.height()>size.y*0.2);
    }

}
fn timed_frame(ctx: &egui::Context, room: &mut RoomUi, size: Vec2, time: f64) {
    let _ = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(time),
            focused: true,
            ..Default::default()
        },
        |ui| {
            room.ui(ui);
        },
    );
}
#[test]
fn fullscreen_tap_reveals_controls_and_back_preserves_the_room() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);
    room.enter_fullscreen(1);timed_frame(&ctx,&mut room,size,10.0);assert!(room.snapshot.exit_fullscreen_button.is_none());
    let video=room.snapshot.viewer_rect.unwrap();click(&ctx,&mut room,size,video.center());frame(&ctx,&mut room,size,vec![]);
    assert!(room.snapshot.exit_fullscreen_button.is_some());timed_frame(&ctx,&mut room,size,30.0);assert!(room.snapshot.exit_fullscreen_button.is_none());room.back();assert!(room.in_room);

}
#[test]
fn fullscreen_exit_button_rotation_and_stream_end_restore_navigation() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);
    room.enter_fullscreen(1);frame(&ctx,&mut room,size,vec![]);
    let video=room.snapshot.viewer_rect.unwrap();click(&ctx,&mut room,size,video.center());frame(&ctx,&mut room,size,vec![]);
    let b=room.snapshot.exit_fullscreen_button.unwrap();click(&ctx,&mut room,size,b.center());assert!(!room.is_fullscreen());assert!(room.in_room);

}
#[test]
fn mute_is_per_participant_and_does_not_stop_any_video() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);    let video=room.snapshot.viewer_rect.unwrap();click(&ctx,&mut room,size,video.center());frame(&ctx,&mut room,size,vec![]);
    let b=room.snapshot.audio_buttons[0].1;let actions=click(&ctx,&mut room,size,b.center());assert!(actions.iter().any(|a|matches!(a,Action::AudioMute{participant:1,muted:true})));assert!(!room.participant_audio_muted(2));assert!(room.watching.contains(&1));

}

#[test]
fn delayed_local_publish_snapshot_keeps_the_capture_owned_fullscreen_preview() {
    let mut room = RoomUi::new(true);
    room.capture_running = true;
    room.set_local_frame(
        egui::TextureId::User(1),
        p2pshare_media::Dimensions::new(1280, 720),
    );
    room.enter_fullscreen(0);
    room.apply_snapshot(&p2pshare_room_ui::RoomState {
        local_id: "local".into(),
        members: vec![p2pshare_room_ui::MemberState {
            id: "local".into(),
            name: "You".into(),
            sharing: false,
            width: 0,
            height: 0,
        }],
        in_room: true,
        busy: false,
        invite: None,
        saved_invite: None,
        status: "Room ready".into(),
    });
    assert!(room.is_fullscreen());
    assert_eq!(room.participants[0].texture, Some(egui::TextureId::User(1)));
    assert_eq!(
        room.participants[0].size,
        p2pshare_media::Dimensions::new(1280, 720)
    );
    room.clear_local_frame();
    assert!(!room.is_fullscreen());
}

#[test]
fn local_texture_dimensions_survive_metadata_only_redraws_and_follow_capture_resize() {
    use p2pshare_media::Dimensions;
    use p2pshare_room_ui::{MemberState, RoomState};
    let state = RoomState {
        local_id: "local".into(),
        members: vec![MemberState {
            id: "local".into(),
            name: "You".into(),
            sharing: true,
            width: 818,
            height: 720,
        }],
        in_room: true,
        busy: false,
        invite: None,
        saved_invite: None,
        status: String::new(),
    };
    let mut room = RoomUi::new(true);
    room.capture_running = true;
    room.apply_snapshot(&state);
    assert_eq!(room.participants[0].size, Dimensions::new(818, 720));
    room.set_local_frame(egui::TextureId::User(1), Dimensions::new(819, 721));
    for _ in 0..120 {
        room.apply_snapshot(&state);
        assert_eq!(room.participants[0].size, Dimensions::new(819, 721));
    }
    room.set_local_frame(egui::TextureId::User(2), Dimensions::new(1025, 769));
    room.apply_snapshot(&state);
    assert_eq!(room.participants[0].size, Dimensions::new(1025, 769));
    room.clear_local_frame();
    room.apply_snapshot(&state);
    assert_eq!(room.participants[0].size, Dimensions::new(818, 720));
}

#[test]
fn android_explicit_watch_stop_and_volume_are_independent() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);    let video=room.snapshot.viewer_rect.unwrap();click(&ctx,&mut room,size,video.center());frame(&ctx,&mut room,size,vec![]);
    assert!(room.snapshot.volume_sliders.is_empty());
    let b=room.snapshot.stop_button.unwrap();let actions=click(&ctx,&mut room,size,b.center());assert!(actions.contains(&Action::Watch{participant:1,enabled:false}));assert!(room.in_room);

}

#[test]
fn android_controls_fit_small_phones_with_long_names() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);
    room.participants[1].name="A very long participant name that must not displace controls".into();
    for size in [vec2(320.0,640.0),vec2(640.0,360.0)] {
        frame(&ctx,&mut room,size,vec![]);let v=room.snapshot.viewer_rect.unwrap();
        if room.snapshot.pip_button.is_none(){click(&ctx,&mut room,size,v.center());}
        frame(&ctx,&mut room,size,vec![]);
        let bounds=Rect::from_min_size(Pos2::ZERO,size);
        for r in [room.snapshot.pip_button.unwrap(),room.snapshot.stop_button.unwrap(),room.snapshot.fullscreen_button.unwrap(),room.snapshot.audio_buttons[0].1] {assert!(bounds.contains_rect(r),"{r:?}");}
    }

}

#[test]
fn android_portrait_cards_never_overlap_after_focusing() {
    let ctx = egui::Context::default();
    RoomUi::configure(&ctx);
    let mut room = RoomUi::new(false);
    room.set_platform(p2pshare_room_ui::Platform::Android);
    live_room(&mut room);
    room.focused = Some(1);
    for size in [vec2(320.0, 640.0), vec2(393.0, 852.0)] {
        frame(&ctx, &mut room, size, vec![]);
        for (index,(_,a)) in room.snapshot.tiles.iter().enumerate() {
            for (_,b) in &room.snapshot.tiles[index+1..] {
                assert!(!a.shrink(0.5).intersects(b.shrink(0.5)), "{a:?} overlaps {b:?}");
            }
        }
    }
}

#[test]
fn android_pip_is_chrome_free_and_fits_resizing_without_losing_watch_state() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);
    room.pip_mode=true;
    for size in [vec2(240.0,135.0),vec2(360.0,202.0)] {frame(&ctx,&mut room,size,vec![]);assert!(room.snapshot.pip_button.is_none());assert!(room.snapshot.audio_buttons.is_empty());assert!(room.watching.contains(&1));}
    room.pip_mode=false;frame(&ctx,&mut room,size,vec![]);assert!(room.snapshot.pip_button.is_none());

}
#[test]
fn android_pip_button_emits_platform_intent_without_leaving_room() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);    let video=room.snapshot.viewer_rect.unwrap();click(&ctx,&mut room,size,video.center());frame(&ctx,&mut room,size,vec![]);
    let b=room.snapshot.pip_button.unwrap();let actions=click(&ctx,&mut room,size,b.center());assert!(actions.contains(&Action::PictureInPicture));assert!(room.in_room);

}
#[test]
fn android_swipe_adjusts_volume_without_a_slider() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);
    room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.focused=Some(1);
    let size=vec2(393.0,852.0);frame(&ctx,&mut room,size,vec![]);
    assert!(room.snapshot.audio_buttons.is_empty());
    let video=room.snapshot.viewer_rect.unwrap();click(&ctx,&mut room,size,video.center());frame(&ctx,&mut room,size,vec![]);
    let start=room.snapshot.audio_buttons[0].1.center();
    frame(&ctx,&mut room,size,vec![Event::PointerMoved(start),Event::PointerButton{pos:start,button:PointerButton::Primary,pressed:true,modifiers:Modifiers::NONE}]);
    frame(&ctx,&mut room,size,vec![Event::PointerMoved(start+vec2(0.0,60.0))]);
    frame(&ctx,&mut room,size,vec![Event::PointerMoved(start+vec2(0.0,90.0))]);
    frame(&ctx,&mut room,size,vec![Event::PointerButton{pos:start+vec2(0.0,90.0),button:PointerButton::Primary,pressed:false,modifiers:Modifiers::NONE}]);
    assert!(room.participant_volume(1)<100);assert_eq!(room.participant_volume(2),100);assert!(!room.snapshot.audio_buttons.is_empty());
    assert!(room.snapshot.volume_sliders.is_empty());assert!(room.in_room);
}
#[test]
fn android_settings_and_users_are_full_pages_and_back_preserves_room() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);
    room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
    let size=vec2(393.0,852.0);room.open_settings();frame(&ctx,&mut room,size,vec![]);
    assert!(room.snapshot.tiles.is_empty());assert!(room.back());assert!(room.in_room);
    room.open_users();frame(&ctx,&mut room,size,vec![]);assert!(room.snapshot.tiles.is_empty());assert!(room.back());assert!(room.in_room);
}

#[test]
fn android_return_to_gallery_keeps_playback_without_drawing_a_play_button() {
    let ctx = egui::Context::default();
    RoomUi::configure(&ctx);
    let mut room = RoomUi::new(false);
    room.set_platform(p2pshare_room_ui::Platform::Android);
    live_room(&mut room);
    let portrait = vec2(393.0, 852.0);
    let landscape = vec2(852.0, 393.0);
    room.focused = Some(1);
    frame(&ctx, &mut room, portrait, vec![]);
    frame(&ctx, &mut room, landscape, vec![]);
    for via_room_button in [false, true] {
        room.focused = Some(1);
        frame(&ctx, &mut room, landscape, vec![]);
        if via_room_button {
            let button = room.snapshot.grid_button.unwrap();
            click(&ctx, &mut room, landscape, button.center());
        } else {
            assert!(room.back());
        }
        assert_eq!(room.focused, None);
        let mut actions = Vec::new();
        let output = ctx.run_ui(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, landscape)),
            focused: true, ..Default::default()
        }, |ui| { actions = room.ui(ui); });
        assert!(room.watching.contains(&1));
        assert!(!actions.contains(&Action::Watch { participant: 1, enabled: false }));
        assert!(!room.snapshot.watch_buttons.iter().any(|(id, _)| *id == 1));
        assert!(room.snapshot.watch_buttons.iter().any(|(id, _)| *id == 2));
        let playing_tile = room.snapshot.tiles.iter().find(|(id, _)| *id == 1).unwrap().1;
        // Inspect real paint output, not only semantic button snapshots.
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Path(path) if path.points.len() == 3
                && path.fill == egui::Color32::WHITE
                && path.points.iter().all(|point| playing_tile.contains(*point)))));
        let actions = click(&ctx, &mut room, landscape, playing_tile.center());
        assert_eq!(room.focused, None);
        assert!(room.snapshot.audio_buttons.iter().any(|(id, _)| *id == 1));
        assert!(!actions.iter().any(|a| matches!(a, Action::Watch { participant: 1, .. })));
    }
}

#[test]
fn android_volume_swipe_is_monotonic_stationary_and_reversible() {
    for size in [vec2(393.0,852.0),vec2(852.0,393.0)] {
        for mode in 0..3 {
            let ctx=egui::Context::default();RoomUi::configure(&ctx);
            let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
            room.focused=Some(1);frame(&ctx,&mut room,size,vec![]);
            if mode==1 {room.back();frame(&ctx,&mut room,size,vec![]);}
            if mode==2 {room.enter_fullscreen(1);frame(&ctx,&mut room,size,vec![]);}
            let video=room.snapshot.viewer_rect.unwrap();
            let start=if mode==2 {video.center()} else {click(&ctx,&mut room,size,video.center());frame(&ctx,&mut room,size,vec![]);room.snapshot.audio_buttons[0].1.center()};
            let before=room.snapshot.tiles.clone();
            frame(&ctx,&mut room,size,vec![Event::PointerMoved(start),Event::PointerButton{pos:start,button:PointerButton::Primary,pressed:true,modifiers:Modifiers::NONE}]);
            let mut previous=room.participant_volume(1);
            for offset in [20.0,45.0,60.0,75.0] {
                frame(&ctx,&mut room,size,vec![Event::PointerMoved(start+vec2(0.0,offset))]);
                let current=room.participant_volume(1);
                assert!(current<=previous,"{size:?} mode={mode}: {previous} -> {current}");
                let expected=(100.0-offset/video.height().max(100.0)*100.0).round().clamp(0.0,100.0) as u8;
                assert_eq!(current,expected);
                frame(&ctx,&mut room,size,vec![]);
                assert_eq!(room.participant_volume(1),current,"A stationary finger changed volume");
                assert_eq!(room.snapshot.tiles,before,"Swipe moved the layout");
                if mode==2 {assert!(room.snapshot.audio_buttons.is_empty());}else{assert!(!room.snapshot.audio_buttons.is_empty());}
                previous=current;
            }
            frame(&ctx,&mut room,size,vec![Event::PointerMoved(start+vec2(0.0,35.0))]);
            assert!(room.participant_volume(1)>previous,"Reversing direction must increase volume");
            let level=room.participant_volume(1);
            frame(&ctx,&mut room,size,vec![Event::PointerButton{pos:start+vec2(0.0,35.0),button:PointerButton::Primary,pressed:false,modifiers:Modifiers::NONE}]);
            frame(&ctx,&mut room,size,vec![]);assert_eq!(room.participant_volume(1),level);
            assert_eq!(room.participant_volume(2),100);assert!(room.in_room);
        }
    }
}

#[test]
fn android_playing_gallery_tap_reveals_controls_in_place_and_supports_pip() {
    for size in [vec2(393.0,852.0),vec2(852.0,393.0)] {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);
    let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
    room.focused=Some(1);frame(&ctx,&mut room,size,vec![]);
    room.back();frame(&ctx,&mut room,size,vec![]);
    let before=room.snapshot.tiles.clone();let video=room.snapshot.viewer_rect.unwrap();
    assert!(room.snapshot.audio_buttons.is_empty());
    let actions=click(&ctx,&mut room,size,video.center());frame(&ctx,&mut room,size,vec![]);
    assert_eq!(room.focused,None);assert_eq!(room.snapshot.tiles,before);
    assert!(room.snapshot.audio_buttons.iter().any(|(id,_)| *id==1));
    assert!(!actions.iter().any(|a|matches!(a,Action::Watch {..})));
    let pip=room.snapshot.pip_button.unwrap();
    let actions=click(&ctx,&mut room,size,pip.center());assert!(actions.contains(&Action::PictureInPicture));
    assert_eq!(room.playback_participant().map(|p|p.id),Some(1));
    room.pip_mode=true;frame(&ctx,&mut room,vec2(240.0,135.0),vec![]);
    assert_eq!(room.snapshot.viewer_rect,Some(Rect::from_min_size(Pos2::ZERO,vec2(240.0,135.0))));
    assert_eq!(room.playback_participant().map(|p|p.id),Some(1));
    }
}

#[test]
fn android_gallery_video_swipes_scroll_without_changing_volume() {
    for size in [vec2(393.0,852.0),vec2(852.0,393.0)] {
        let ctx=egui::Context::default();RoomUi::configure(&ctx);
        let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
        for id in 4..=5 {let mut p=room.participants[1].clone();p.id=id;room.participants.push(p);}
        room.watching.insert(1);frame(&ctx,&mut room,size,vec![]);frame(&ctx,&mut room,size,vec![]);
        let top=room.snapshot.tiles[0].1.top();let start=room.snapshot.viewer_rect.unwrap().center();
        let touch=|phase,pos|Event::Touch{device_id:egui::TouchDeviceId(1),id:egui::TouchId(1),phase,pos,force:None};
        frame(&ctx,&mut room,size,vec![touch(egui::TouchPhase::Start,start),Event::PointerMoved(start),Event::PointerButton{pos:start,button:PointerButton::Primary,pressed:true,modifiers:Modifiers::NONE}]);
        for offset in [-20.0,-60.0,-110.0] {let pos=start+vec2(0.0,offset);frame(&ctx,&mut room,size,vec![touch(egui::TouchPhase::Move,pos),Event::PointerMoved(pos)]);}
        assert_eq!(room.participant_volume(1),100);assert!(room.snapshot.audio_buttons.is_empty());
        assert!(room.snapshot.tiles[0].1.top()<top,"Gallery did not scroll at {size:?}");
    }
}

#[test]
fn android_portrait_navigation_icons_are_present_on_every_room_page() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);
    let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
    let size=vec2(393.0,852.0);
    for page in 0..3 {
        room.close_mobile_page();if page==1{room.open_users();}if page==2{room.open_settings();}
        let out=ctx.run_ui(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,size)),focused:true,..Default::default()},|ui|{room.ui(ui);});
        assert_eq!(room.snapshot.navigation_buttons.len(),3);
        for (index,rect) in &room.snapshot.navigation_buttons {
            assert!(out.shapes.iter().any(|s|match &s.shape {
                egui::Shape::Rect(r)=>*index==0 && (r.rect.size()-vec2(18.0,12.0)*1.12).length()<0.01 && rect.contains_rect(r.rect),
                egui::Shape::Circle(c)=>((*index==1 && (c.radius-3.0*1.12).abs()<0.01)||(*index==2 && (c.radius-8.0*1.12).abs()<0.01)) && rect.contains(c.center),
                _=>false
            }),"Missing nav icon {index} on page {page}");
        }
    }
    let screens=room.snapshot.navigation_buttons.iter().find(|(id,_)|*id==0).unwrap().1;
    click(&ctx,&mut room,size,screens.center());frame(&ctx,&mut room,size,vec![]);
    assert!(!room.snapshot.tiles.is_empty());assert!(room.in_room);
}

#[test]
fn android_landscape_counts_are_corner_badges() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);
    let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
    let size=vec2(852.0,393.0);
    for shares in [3,2] {
        room.participants[3].sharing=shares==3;
        let out=ctx.run_ui(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,size)),focused:true,..Default::default()},|ui|{room.ui(ui);});
        for (index,wanted,label) in [(0,shares.to_string(),"Screens"),(1,"4".to_string(),"Users")] {
            let rect=room.snapshot.navigation_buttons.iter().find(|(id,_)|*id==index).unwrap().1;
            let texts:Vec<_>=out.shapes.iter().filter_map(|s|if let egui::Shape::Text(t)=&s.shape{Some(t)}else{None}).collect();
            let value=texts.iter().find(|t|t.galley.text()==wanted && rect.contains(t.pos)).expect("Count missing from navigation button");
            let caption=texts.iter().find(|t|t.galley.text()==label && rect.contains(t.pos)).unwrap();
            assert!(value.pos.y<caption.pos.y && value.pos.x>rect.center().x-5.0);
        }
    }
}

#[test]
fn android_screens_navigation_returns_from_both_pages_in_both_orientations() {
    for size in [vec2(393.0,852.0),vec2(852.0,393.0)] {
        for settings in [false,true] {
            let ctx=egui::Context::default();RoomUi::configure(&ctx);
            let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
            room.watching.insert(1);
            if settings {room.open_settings();}else{room.open_users();}
            frame(&ctx,&mut room,size,vec![]);frame(&ctx,&mut room,size,vec![]);
            let screens=room.snapshot.navigation_buttons.iter().find(|(id,_)|*id==0).unwrap().1;
            click(&ctx,&mut room,size,screens.center());frame(&ctx,&mut room,size,vec![]);
            assert!(!room.snapshot.tiles.is_empty(),"Screens did not return from settings={settings} at {size:?}");
            assert!(room.in_room);assert!(room.watching.contains(&1));
        }
    }
}

#[test]
fn android_portrait_badges_follow_labels_and_header_has_no_counts() {
    for size in [vec2(320.0,640.0),vec2(393.0,852.0)] {
        for count in [1,10,100] {
            let ctx=egui::Context::default();RoomUi::configure(&ctx);
            let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
            let template=room.participants[1].clone();room.participants.clear();
            for id in 0..count{let mut p=template.clone();p.id=id as u64;p.sharing=true;room.participants.push(p);}
            for page in 0..3 {
                room.close_mobile_page();if page==1{room.open_users();}if page==2{room.open_settings();}
                let out=ctx.run_ui(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,size)),focused:true,..Default::default()},|ui|{room.ui(ui);});
                let texts:Vec<_>=out.shapes.iter().filter_map(|s|if let egui::Shape::Text(t)=&s.shape{Some(t)}else{None}).collect();
                for (index,label) in [(0,"Screens"),(1,"Users")] {
                    let rect=room.snapshot.navigation_buttons.iter().find(|(id,_)|*id==index).unwrap().1;
                    let caption=texts.iter().find(|t|t.galley.text()==label && rect.contains(t.pos)).unwrap();
                    let wanted=if count>99{"99+".into()}else{count.to_string()};
                    let badge=texts.iter().find(|t|t.galley.text()==wanted && rect.contains(t.pos)).expect("Missing count badge");
                    assert!(badge.pos.x>=caption.pos.x+caption.galley.size().x);
                    // Icon and caption stay centered independently of the count width.
                    let primary_left=caption.pos.x-28.0;
                    let primary_right=caption.pos.x+caption.galley.size().x;
                    assert!(((primary_left+primary_right)*0.5-rect.center().x).abs()<0.1);
                    assert!(rect.contains_rect(Rect::from_min_size(badge.pos,badge.galley.size())));
                }
                if page==0 {assert!(!texts.iter().any(|t|t.pos.y<40.0 && t.galley.text()==count.to_string()));}
            }
        }
    }
}

#[test]
fn android_landscape_icons_share_the_settings_center_line() {
    let ctx=egui::Context::default();RoomUi::configure(&ctx);
    let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
    for page in 0..3 {
        room.close_mobile_page();if page==1{room.open_users();}if page==2{room.open_settings();}
        let size=vec2(852.0,393.0);
        let out=ctx.run_ui(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,size)),focused:true,..Default::default()},|ui|{room.ui(ui);});
        for (index,rect) in &room.snapshot.navigation_buttons {
            let center=rect.center()-vec2(0.0,10.0);
            let aligned=out.shapes.iter().any(|s|match &s.shape {
                egui::Shape::Rect(r)=>*index==0 && (r.rect.center().x-center.x).abs()<0.01 && (r.rect.size()-vec2(18.0,12.0)*1.12).length()<0.01,
                egui::Shape::Circle(c)=>{
                    let (offset,radius)=if *index==1{(vec2(-3.5,-4.0)*1.12,3.0*1.12)}else{(vec2(0.0,0.0),8.0*1.12)};
                    *index!=0 && c.center.distance(center+offset)<0.01 && (c.radius-radius).abs()<0.01
                },
                _=>false,
            });
            assert!(aligned,"Icon {index} is displaced on page {page}");
        }
    }
}

#[test]
fn android_landscape_display_name_is_aligned_to_the_right_edge() {
    for size in [vec2(852.0,393.0),vec2(640.0,360.0)] {
        let ctx=egui::Context::default();RoomUi::configure(&ctx);
        let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
        room.name="Right edge name".into();room.open_settings();
        frame(&ctx,&mut room,size,vec![]);
        let out=ctx.run_ui(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,size)),focused:true,..Default::default()},|ui|{room.ui(ui);});
        let name=out.shapes.iter().filter_map(|s|if let egui::Shape::Text(t)=&s.shape{(t.galley.text()=="Right edge name").then_some(t)}else{None}).max_by(|a,b|(a.pos.x+a.galley.rect.right()).total_cmp(&(b.pos.x+b.galley.rect.right()))).expect("Display name missing");
        let right=name.pos.x+name.galley.rect.right();
        assert!(right>size.x-50.0 && right<size.x,"Display name is not right aligned: {right}");
    }
}

#[test]
fn android_share_controls_request_consent_and_stop_without_leaving_room() {
    for size in [vec2(393.0,852.0),vec2(852.0,393.0)] {
        let ctx=egui::Context::default();RoomUi::configure(&ctx);
        let mut room=RoomUi::new(true);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
        frame(&ctx,&mut room,size,vec![]);frame(&ctx,&mut room,size,vec![]);
        let button=room.snapshot.share_button.expect("Share control missing");
        let actions=click(&ctx,&mut room,size,button.center());
        assert!(actions.contains(&Action::StartCapture(0)));assert!(!room.capture_running,"Consent must precede capture");
        room.capture_running=true;frame(&ctx,&mut room,size,vec![]);
        let button=room.snapshot.share_button.unwrap();let actions=click(&ctx,&mut room,size,button.center());
        assert!(actions.contains(&Action::StopCapture));assert!(room.in_room);
    }
}

#[test]
fn source_metadata_switch_keeps_the_viewer_watching_and_focused() {
    let mut room=RoomUi::new(false);live_room(&mut room);room.watching.insert(1);room.focused=Some(1);
    let member=p2pshare_room_ui::MemberState{id:"remote-source".into(),name:"Publisher".into(),sharing:true,width:1280,height:720};
    let id=p2pshare_room_ui::peer_ui_id(&member.id);room.watching.insert(id);room.focused=Some(id);
    let mut state=p2pshare_room_ui::RoomState{local_id:"local".into(),members:vec![member],in_room:true,busy:false,invite:None,saved_invite:None,status:"Connected".into()};
    room.apply_snapshot(&state);state.members[0].width=720;state.members[0].height=1280;room.apply_snapshot(&state);
    assert!(room.watching.contains(&id));assert_eq!(room.focused,Some(id));
}

#[test]
fn android_profile_and_viewer_taps_do_not_start_playback_and_back_dismisses_only_the_sheet(){
 for size in [vec2(393.,852.),vec2(852.,393.)] {
  let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);
  room.profiles.viewers.insert(1,vec![2,3]);room.profiles.names.insert(1,(0..24).map(|i|format!("Old name {i}")).collect());
  frame(&ctx,&mut room,size,vec![]);frame(&ctx,&mut room,size,vec![]);
  let at=room.profiles.hits["stream-profile-1"].center();let actions=click(&ctx,&mut room,size,at);
  assert_eq!(room.profiles.selected,Some(1));assert!(!actions.iter().any(|a|matches!(a,Action::Watch{enabled:true,..})));
  frame(&ctx,&mut room,size,vec![]);assert!(room.profiles.hits.contains_key("all-names"));assert!(room.back());assert!(room.in_room);assert!(room.watching.is_empty());
  frame(&ctx,&mut room,size,vec![]);frame(&ctx,&mut room,size,vec![]);
  let at=room.profiles.hits["stream-viewers-1"].center();click(&ctx,&mut room,size,at);assert_eq!(room.profiles.viewers_for,Some(1));assert!(room.back());assert!(room.in_room);
 }
}

#[test]
fn android_inline_volume_popup_controls_audio_without_resizing_or_stopping_video(){
 for size in [vec2(320.,640.),vec2(393.,852.),vec2(852.,393.)] {
  let ctx=egui::Context::default();RoomUi::configure(&ctx);let mut room=RoomUi::new(false);room.set_platform(p2pshare_room_ui::Platform::Android);live_room(&mut room);room.watching.insert(1);room.focused=Some(1);
  frame(&ctx,&mut room,size,vec![]);frame(&ctx,&mut room,size,vec![]);
  let video=room.snapshot.viewer_rect.unwrap();let at=room.profiles.hits["stream-volume-1"].center();assert_eq!(room.profiles.hits["stream-volume-1"].height(),42.);
  click(&ctx,&mut room,size,at);frame(&ctx,&mut room,size,vec![]);frame(&ctx,&mut room,size,vec![]);
  let popup=room.snapshot.volume_popup.expect("Volume popup missing");assert!(Rect::from_min_size(Pos2::ZERO,size).contains_rect(popup));assert_eq!(room.snapshot.viewer_rect,Some(video));
  let slider=room.snapshot.volume_sliders.iter().find(|(id,_)|*id==1).unwrap().1;
  let actions=click(&ctx,&mut room,size,egui::pos2(slider.left()+7.+(slider.width()-14.)*0.75,slider.center().y));
  assert!((i16::from(room.participant_volume(1))-75).abs()<=1);assert_eq!(room.participant_volume(2),100);assert!(actions.iter().any(|a|matches!(a,Action::AudioVolume{participant:1,..})));assert!(!actions.iter().any(|a|matches!(a,Action::Watch{..})));
  frame(&ctx,&mut room,size,vec![]);let mute=room.snapshot.audio_buttons.iter().find(|(id,_)|*id==1).unwrap().1.center();click(&ctx,&mut room,size,mute);assert!(room.participant_audio_muted(1));click(&ctx,&mut room,size,mute);assert!(!room.participant_audio_muted(1));assert!((i16::from(room.participant_volume(1))-75).abs()<=1);
  assert!(room.back());frame(&ctx,&mut room,size,vec![]);assert!(room.snapshot.volume_popup.is_none());assert!(room.watching.contains(&1));assert_eq!(room.focused,Some(1));
  click(&ctx,&mut room,size,at);frame(&ctx,&mut room,size,vec![]);click(&ctx,&mut room,size,egui::pos2(size.x-2.,2.));frame(&ctx,&mut room,size,vec![]);assert!(room.snapshot.volume_popup.is_none());assert!(room.watching.contains(&1));
 }
}

#[test]
fn language_prompt_blocks_room_and_emits_choice_once() {
 let ctx=egui::Context::default();let mut room=RoomUi::new(true);room.set_platform(p2pshare_room_ui::Platform::Android);room.language_prompt=true;room.name="Settings".into();
 let size=vec2(393.,852.);for _ in 0..3{assert!(frame(&ctx,&mut room,size,vec![]).is_empty());}
 let at=room.profiles.hits["language-pt-BR"].center();let actions=click(&ctx,&mut room,size,at);
 assert!(actions.iter().any(|a|matches!(a,Action::SetLanguage(p2pshare_media::i18n::Language::PtBr))));assert!(!room.language_prompt);assert_eq!(room.name,"Settings");assert_eq!(room.language,p2pshare_media::i18n::Language::PtBr);
 assert!(!frame(&ctx,&mut room,size,vec![]).iter().any(|a|matches!(a,Action::SetLanguage(_))));
}
