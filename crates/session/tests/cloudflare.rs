#![cfg(windows)]
use openh264::{
    decoder::Decoder,
    encoder::Encoder,
    formats::{YUVBuffer, YUVSource},
};
use p2pshare_media::config::VideoFormat;
use p2pshare_session::{CloudflareInvite, Command, Session, VideoFrame};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn wait(label: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "Timed out: {label}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.build/cloudflare-native")
        .join(format!("{}-{}", std::process::id(), rand::random::<u32>()))
}
fn server() -> String {
    std::env::var("P2PSHARE_TEST_SIGNALING")
        .expect("Start the real local Worker and set P2PSHARE_TEST_SIGNALING")
}
fn create(session: &Session) -> String {
    session.command(Command::Create("Creator".into()));
    wait("created", || session.snapshot().in_room);
    session.snapshot().invite.unwrap()
}
fn join(session: &Session, invite: &str) {
    session.command(Command::Join {
        invite: invite.into(),
        name: "Viewer".into(),
    });
    wait("joined", || session.snapshot().in_room);
}
fn packet() -> Vec<u8> {
    let mut yuv = vec![90; 160 * 96];
    yuv.extend(vec![128; 160 * 96 / 2]);
    Encoder::new()
        .unwrap()
        .encode(&YUVBuffer::from_vec(yuv, 160, 96))
        .unwrap()
        .to_vec()
}
fn video(sender: &Session, receiver: &Session) {
    sender.command(Command::Publish {
        width: 160,
        height: 96,
    });
    wait("sharing announced", || {
        receiver
            .snapshot()
            .members
            .iter()
            .any(|m| m.id == sender.snapshot().local_id && m.sharing)
    });
    receiver.set_watching(&sender.snapshot().local_id, true);
    wait("watch subscription", || sender.snapshot().viewer_count > 0);
    sender.publish(VideoFrame {
        codec: VideoFormat::H264,
        data: packet().into(),
        micros: 0,
        keyframe: true,
    });
    let mut frames = vec![];
    wait("encrypted real video", || {
        frames.extend(receiver.take_video());
        !frames.is_empty()
    });
    let mut decoder = Decoder::new().unwrap();
    let pixels = decoder
        .decode(&frames[0].data)
        .unwrap()
        .expect("Actual video decodes");
    assert_eq!(pixels.dimensions(), (160, 96));
    assert!((85..=95).contains(&pixels.y()[0]));
    let mut encoder = p2pshare_media::audio::encoder(Default::default()).unwrap();
    let pcm = [0.15; p2pshare_media::audio::SAMPLES];
    let opus = encoder.encode_vec_float(&pcm, 1275).unwrap();
    sender.publish_audio(p2pshare_session::AudioPacket {
        data: opus.clone().into(),
        micros: 1000000,
    });
    let mut audio = Vec::new();
    wait("encrypted Opus alongside video", || {
        audio.extend(receiver.take_audio());
        !audio.is_empty()
    });
    assert_eq!(&*audio[0].data, &opus);
    assert_eq!(audio[0].peer, sender.snapshot().local_id);
    let mut mixer = p2pshare_media::audio::Mixer::new(10);
    mixer
        .push(
            &audio[0].peer,
            audio[0].micros,
            audio[0].data.clone(),
            Instant::now(),
        )
        .unwrap();
    let mut decoded = [0.0; p2pshare_media::audio::SAMPLES];
    mixer.render(Instant::now() + Duration::from_millis(20), &mut decoded);
    assert!(decoded.iter().any(|s| s.abs() > 0.01));
}
#[test]
fn invitation_roundtrip_and_security() {
    let invite = CloudflareInvite::new("wss://example.com".into()).unwrap();
    let parsed = CloudflareInvite::parse(&invite.encode().unwrap()).unwrap();
    assert_eq!(parsed.secret, invite.secret);
    for server in [
        "ws://public.example",
        "https://example.com",
        "wss://user:pass@example.com",
        "wss://example.com/query?q=1",
        "wss://example.com/path",
        "wss://example.com/#fragment",
    ] {
        assert!(CloudflareInvite::new(server.into()).is_err(), "{server}");
    }
    assert!(CloudflareInvite::new("ws://127.0.0.1:12345".into()).is_ok());
}
#[test]
#[ignore = "requires real local or public Worker"]
fn cloudflare_rooms_video_creator_exit_reuse_and_same_identity_restart() {
    let root = root();
    let server = server();
    let a = Session::start_cloudflare(root.join("a.key"), server.clone(), true).unwrap();
    let b = Session::start_cloudflare(root.join("b.key"), server.clone(), true).unwrap();
    let invite = create(&a);
    join(&b, &invite);
    wait("direct DTLS/SRTP", || {
        a.snapshot().connected_video.len() == 1 && b.snapshot().connected_video.len() == 1
    });
    video(&a, &b);
    let a_id = a.snapshot().local_id;
    drop(a);
    wait("creator leaves immediately", || {
        b.snapshot().members.len() == 1 && b.snapshot().connected_video.is_empty()
    });
    let c = Session::start_cloudflare(root.join("c.key"), server.clone(), true).unwrap();
    join(&c, &invite);
    wait("creator-independent media", || {
        b.snapshot().connected_video.len() == 1 && c.snapshot().connected_video.len() == 1
    });
    video(&c, &b);
    drop(c);
    let a = Session::start_cloudflare(root.join("a.key"), server.clone(), true).unwrap();
    wait("saved invitation", || a.snapshot().saved_invite.is_some());
    assert_eq!(a.snapshot().saved_invite.as_deref(), Some(invite.as_str()));
    join(&a, &invite);
    wait("creator rejoins", || {
        a.snapshot().connected_video.len() == 1 && b.snapshot().connected_video.len() == 1
    });
    assert_eq!(a.snapshot().local_id, a_id);
    video(&a, &b);
    // Reuse the identity before the old process exits: new socket supersedes old.
    let fresh = Session::start_cloudflare(root.join("a.key"), server, true).unwrap();
    join(&fresh, &invite);
    wait("old session replaced", || {
        !a.snapshot().in_room && a.snapshot().status.contains("another app instance")
    });
    wait("restarted direct media", || {
        fresh.snapshot().connected_video.len() == 1 && b.snapshot().connected_video.len() == 1
    });
    assert_eq!(b.snapshot().members.len(), 2);
    video(&fresh, &b);
    drop(a);
    std::thread::sleep(Duration::from_millis(200));
    assert!(fresh.snapshot().in_room);
    assert_eq!(b.snapshot().members.len(), 2);
    drop(fresh);
    drop(b);
    let reuse = Session::start_cloudflare(
        root.join("reuse.key"),
        CloudflareInvite::parse(&invite).unwrap().server,
        true,
    )
    .unwrap();
    join(&reuse, &invite);
    assert_eq!(reuse.snapshot().members.len(), 1);
}
#[test]
#[ignore = "requires real local or public Worker"]
fn cloudflare_admits_six_and_recovers_after_full_room() {
    let root = root();
    let server = server();
    let a = Session::start_cloudflare(root.join("a.key"), server.clone(), true).unwrap();
    let invite = create(&a);
    let mut sessions = vec![a];
    for n in 1..6 {
        let peer =
            Session::start_cloudflare(root.join(format!("{n}.key")), server.clone(), true).unwrap();
        join(&peer, &invite);
        sessions.push(peer);
    }
    wait("six members", || {
        sessions.iter().all(|s| s.snapshot().members.len() == 6)
    });
    let extra = Session::start_cloudflare(root.join("extra.key"), server, true).unwrap();
    extra.command(Command::Join {
        invite: invite.clone(),
        name: "Extra".into(),
    });
    wait("full room status", || {
        extra.snapshot().status == "Room is full. Waiting for a spot…"
    });
    assert!(!extra.snapshot().in_room);
    drop(sessions.pop());
    wait("waiting peer recovers", || extra.snapshot().in_room);
    wait("six again", || extra.snapshot().members.len() == 6);
}

#[test]
#[ignore = "requires real local or public Worker"]
fn continuous_motion_preserves_interframe_dependencies_and_frame_rate() {
    use openh264::encoder::{
        BitRate, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, Profile,
    };
    let root = root();
    let host = Session::start_cloudflare(root.join("host.key"), server(), true).unwrap();
    let viewer = Session::start_cloudflare(root.join("viewer.key"), server(), true).unwrap();
    host.command(Command::Create("Motion publisher".into()));
    wait("invite", || host.snapshot().invite.is_some());
    viewer.command(Command::Join {
        invite: host.snapshot().invite.unwrap(),
        name: "Viewer".into(),
    });
    wait("DTLS/SRTP", || viewer.snapshot().connected_video.len() == 1);
    host.command(Command::Publish {
        width: 640,
        height: 360,
    });
    wait("share announced", || {
        viewer.snapshot().members.iter().any(|m| m.sharing)
    });
    viewer.set_watching(&host.snapshot().local_id, true);
    wait("motion watch subscription", || host.snapshot().viewer_count == 1);
    let mut encoder = Encoder::with_api_config(
        openh264::OpenH264API::from_source(),
        EncoderConfig::new()
            .profile(Profile::Baseline)
            .num_threads(1)
            .bitrate(BitRate::from_bps(4_000_000))
            .max_frame_rate(FrameRate::from_hz(30.0))
            .intra_frame_period(IntraFramePeriod::from_num_frames(120)),
    )
    .unwrap();
    let mut decoder = Decoder::new().unwrap();
    let mut decoded = 0;
    let mut count_keyframes = 0;
    for batch in 0..20 {
        // Three encoded frames can complete before the GUI/network thread gets scheduled.
        for n in batch * 3..batch * 3 + 3 {
            let mut pixels = vec![128; 640 * 360 * 3 / 2];
            for y in 0..360 {
                for x in 0..640 {
                    pixels[y * 640 + x] = (32 + ((x + n * 8) / 16 + y / 16) % 180) as u8;
                }
            }
            let encoded = encoder
                .encode(&YUVBuffer::from_vec(pixels, 640, 360))
                .unwrap();
            let keyframe = encoded.frame_type() == FrameType::IDR;
            count_keyframes += keyframe as usize;
            host.publish(VideoFrame {
                codec: p2pshare_media::config::VideoFormat::H264,
                data: encoded.to_vec().into(),
                micros: n as u64 * 33_333,
                keyframe,
            });
        }
        let until = Instant::now() + Duration::from_millis(100);
        while Instant::now() < until {
            for frame in viewer.take_video() {
                if let Some(yuv) = decoder.decode(&frame.data).unwrap() {
                    assert_eq!(yuv.dimensions(), (640, 360));
                    decoded += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until && decoded < 60 {
        for frame in viewer.take_video() {
            if decoder.decode(&frame.data).unwrap().is_some() {
                decoded += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        count_keyframes <= 2,
        "Requires an interframe chain, not an all-IDR sample"
    );
    assert!(decoded >= 57, "Decoded {decoded}/60 continuous frames");
    println!("Motion stream: {decoded}/60 frames decoded, {count_keyframes} IDR frames");
}
#[test]
#[ignore = "requires the owned local Worker restart harness"]
fn cloudflare_reconnects_after_signaling_runtime_restart() {
    let server = server();
    assert!(server.starts_with("ws://127.0.0.1:"));
    let root = root();
    let a = Session::start_cloudflare(root.join("a.key"), server.clone(), true).unwrap();
    let b = Session::start_cloudflare(root.join("b.key"), server, true).unwrap();
    let invite = create(&a);
    join(&b, &invite);
    wait("media before disconnect", || {
        a.snapshot().connected_video.len() == 1 && b.snapshot().connected_video.len() == 1
    });
    video(&a, &b);
    std::fs::write(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.build/cloudflare-native/restart"),
        b"restart",
    )
    .unwrap();
    wait("disconnect detected", || {
        a.snapshot().status.contains("retrying") || b.snapshot().status.contains("retrying")
    });
    wait("fresh signaling welcome", || {
        !a.snapshot().busy
            && !b.snapshot().busy
            && a.snapshot().connected_video.len() == 1
            && b.snapshot().connected_video.len() == 1
            && a.snapshot().members.len() == 2
            && b.snapshot().members.len() == 2
            && !a.snapshot().status.contains("retrying")
            && !b.snapshot().status.contains("retrying")
    });
    video(&a, &b);
    assert_eq!(a.snapshot().invite.as_deref(), Some(invite.as_str()));
}

#[test]
#[ignore = "requires real local Worker"]
fn named_room_accepts_more_than_six_native_clients() {
    let root = root();
    let server = server();
    assert!(server.starts_with("ws://127.0.0.1:"));
    let room = format!("capacity-{}", rand::random::<u64>());
    let mut occupants = Vec::new();
    for index in 0..7 {
        let session =
            Session::start_cloudflare(root.join(format!("{index}.key")), server.clone(), true)
                .unwrap();
        session.command(Command::NamedRoom {
            room: room.clone(),
            name: format!("User {index}"),
        });
        wait("room join", || session.snapshot().in_room);
        occupants.push(session);
    }
    wait("all participants visible to first and latest users", || {
        occupants[0].snapshot().members.len() == 7
            && occupants[6].snapshot().members.len() == 7
    });
}

#[test]
#[ignore = "requires real local or public Worker"]
fn named_rooms_join_only_reuse_isolation_and_background_heartbeats() {
    let root = root();
    let server = server();
    let name = format!("native-{}", rand::random::<u64>());
    let a = Session::start_cloudflare(root.join("a.key"), server.clone(), true).unwrap();
    let b = Session::start_cloudflare(root.join("b.key"), server.clone(), true).unwrap();
    // Both use the same operation: the first join needs no registration/create request.
    a.command(Command::NamedRoom {
        room: name.to_ascii_uppercase(),
        name: "First user".into(),
    });
    b.command(Command::NamedRoom {
        room: format!("  {name}  "),
        name: "Viewer".into(),
    });
    wait("both room-name-only joins", || {
        a.snapshot().in_room
            && b.snapshot().in_room
            && a.snapshot().members.len() == 2
            && b.snapshot().members.len() == 2
    });
    wait("named direct media", || {
        a.snapshot().connected_video.len() == 1 && b.snapshot().connected_video.len() == 1
    });
    video(&a, &b);
    let other = Session::start_cloudflare(root.join("other.key"), server.clone(), true).unwrap();
    other.command(Command::NamedRoom {
        room: format!("{name}-other"),
        name: "Other room".into(),
    });
    wait("different room isolated", || other.snapshot().in_room);
    assert_eq!(other.snapshot().members.len(), 1);
    assert_eq!(a.snapshot().members.len(), 2);
    // Longer than the Worker heartbeat expiry, without any UI/session polling.
    std::thread::sleep(Duration::from_secs(35));
    assert_eq!(a.snapshot().members.len(), 2);
    assert!(b.snapshot().in_room);
    video(&a, &b);
    let saved = CloudflareInvite::parse(&a.snapshot().invite.unwrap()).unwrap();
    assert_eq!(saved.version, 7);
    assert!(saved.secret.is_empty());
    drop(a);
    wait("first user can leave", || b.snapshot().members.len() == 1);
    drop(b);
    let reuse = Session::start_cloudflare(root.join("reuse.key"), server.clone(), true).unwrap();
    reuse.command(Command::NamedRoom {
        room: name.clone(),
        name: "Reuse".into(),
    });
    wait("empty group rejoins by name", || reuse.snapshot().in_room);
    assert_eq!(reuse.snapshot().members.len(), 1);
    if server.starts_with("ws://127.0.0.1:") {
        std::fs::write(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.build/cloudflare-native/restart"),
            b"restart",
        )
        .unwrap();
        wait("named restart detected", || {
            reuse.snapshot().status.contains("retrying")
        });
        wait("named group survives runtime restart", || {
            !reuse.snapshot().busy
                && !reuse.snapshot().status.contains("retrying")
                && reuse.snapshot().in_room
        });
        let fresh = Session::start_cloudflare(root.join("fresh.key"), server, true).unwrap();
        fresh.command(Command::NamedRoom {
            room: name,
            name: "Fresh".into(),
        });
        wait("fresh peer joins after restart", || {
            fresh.snapshot().in_room && reuse.snapshot().members.len() == 2
        });
    }
}
#[test]
fn old_named_invites_migrate_to_room_name_only() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let old = serde_json::json!({"version":6,"room":"old-room","server":"wss://example.com","secret":URL_SAFE_NO_PAD.encode([42u8;32])});
    let link = format!(
        "p2pshare://room/{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&old).unwrap())
    );
    let upgraded = CloudflareInvite::parse(&link).unwrap();
    assert_eq!(upgraded.version, 7);
    assert_eq!(upgraded.room.as_deref(), Some("old-room"));
    assert!(upgraded.secret.is_empty());
}

#[test]
#[ignore = "requires the owned local Worker"]
fn continuous_opus_video_loss_and_mute() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("p2pshare_session=warn,p2pshare_rtc=warn")
        .try_init();
    p2pshare_media::diagnostics::set_enabled(true);
    use p2pshare_media::audio::*;
    let root = root();
    let url = server();
    let room = format!("audio-{}", rand::random::<u64>());
    let a = Session::start_cloudflare(root.join("a.key"), url.clone(), true).unwrap();
    let b = Session::start_cloudflare(root.join("b.key"), url, true).unwrap();
    for (session, name) in [(&a, "Sender"), (&b, "Viewer")] {
        session.command(Command::NamedRoom {
            room: room.clone(),
            name: name.into(),
        });
    }
    wait("audio test room", || {
        a.snapshot().connected_video.len() == 1 && b.snapshot().connected_video.len() == 1
    });
    a.command(Command::Publish {
        width: 160,
        height: 96,
    });
    wait("audio share announced", || {
        b.snapshot()
            .members
            .iter()
            .any(|m| m.id == a.snapshot().local_id && m.sharing)
    });
    b.set_watching(&a.snapshot().local_id, true);
    wait("audio watch subscription", || a.snapshot().viewer_count == 1);
    let frame = packet();
    let mut encoder = encoder(Default::default()).unwrap();
    let mut mixer = Mixer::new(30);
    let peer = a.snapshot().local_id;
    let mut input = [0.0; SAMPLES];
    let mut output = [0.0; SAMPLES];
    let start = Instant::now();
    let mut video_frames = 0;
    let mut video_index = 0u64;
    let mut audible = 0;
    let mut timing = String::from("render_index,receive_us,media_us,network_age_us\n");
    for n in 0..300u64 {
        let deadline = start + Duration::from_millis(n * 10);
        if let Some(delay) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(delay);
        }
        for i in 0..FRAMES {
            input[i * 2] = ((n * FRAMES as u64 + i as u64) as f32 * 0.0576).sin() * 0.25;
            input[i * 2 + 1] = input[i * 2];
        }
        let data = encoder.encode_vec_float(&input, 1275).unwrap();
        // Deliberate network-equivalent loss, preserving the media clock.
        if ![90, 190].contains(&n) {
            a.publish_audio(p2pshare_session::AudioPacket {
                data: data.into(),
                micros: 1000000 + n * PACKET_US,
            });
        }
        while video_index * 1_000_000 / 120 <= n * PACKET_US {
            a.publish(VideoFrame {
                codec: VideoFormat::H264,
                data: frame.clone().into(),
                micros: 1000000 + video_index * 1_000_000 / 120,
                keyframe: true,
            });
            video_index += 1;
        }
        for packet in b.take_audio() {
            timing.push_str(&format!(
                "{n},{},{},{}\n",
                start.elapsed().as_micros(),
                packet.micros,
                packet.received_at.elapsed().as_micros()
            ));
            mixer
                .push(&packet.peer, packet.micros, packet.data, packet.received_at)
                .unwrap();
        }
        video_frames += b.take_video().len();
        mixer.set_muted(&peer, (110..160).contains(&n));
        mixer.render(Instant::now(), &mut output);
        if (110..160).contains(&n) {
            assert!(output.iter().all(|s| *s == 0.0));
        } else if n > 20 && output.iter().any(|s| s.abs() > 0.01) {
            audible += 1;
        }
    }
    std::fs::write(root.join("audio-timing.csv"), &timing).unwrap();
    println!("audio timing: {} / {:?}", root.display(), mixer.stats);
    p2pshare_media::diagnostics::set_enabled(false);
    assert!(
        mixer.stats.received >= 280,
        "audio receive {:?}",
        mixer.stats
    );
    assert!(
        video_frames >= 345,
        "video must continue alongside audio: {video_frames}"
    );
    assert!(
        audible >= 210,
        "continuous audio should be audible outside mute: {audible}, {:?}",
        mixer.stats
    );
    assert!(mixer.stats.concealed >= 2, "missing packets must use PLC");
    assert!(
        mixer.stats.overflows == 0,
        "bounded queues must remain stable"
    );
}

#[test]
#[ignore = "requires owned local Worker runtime"]
fn synchronized_playback_buffer_releases_audio_video_after_target_delay() {
    let root = root();
    std::fs::create_dir_all(&root).unwrap();
    let url = server();
    let a = Session::start_cloudflare(root.join("buffer-sender.key"), url.clone(), true).unwrap();
    let b = Session::start_cloudflare(root.join("buffer-viewer.key"), url, true).unwrap();
    let room = format!("buffer-{}", rand::random::<u32>());
    for (session, name) in [(&a, "Sender"), (&b, "Viewer")] {
        session.command(Command::NamedRoom {
            room: room.clone(),
            name: name.into(),
        });
    }
    wait("buffer room", || {
        a.snapshot().connected_video.len() == 1 && b.snapshot().connected_video.len() == 1
    });
    a.command(Command::Publish {
        width: 160,
        height: 96,
    });
    wait("buffer share", || {
        b.snapshot()
            .members
            .iter()
            .any(|m| m.id == a.snapshot().local_id && m.sharing)
    });
    b.set_watching(&a.snapshot().local_id, true);
    wait("buffer watch subscription", || a.snapshot().viewer_count == 1);
    let mut opus = p2pshare_media::audio::encoder(Default::default()).unwrap();
    let audio = opus
        .encode_vec_float(&[0.15; p2pshare_media::audio::SAMPLES], 1275)
        .unwrap();
    let video = packet();
    for delay in [200u32, 1000] {
        b.discard_playback();
        b.set_playback_config(p2pshare_media::playback::PlaybackConfig { buffer_ms: delay })
            .unwrap();
        let micros = 1_000_000 + u64::from(delay) * 1000;
        let sent = Instant::now();
        a.publish(VideoFrame {
            codec: VideoFormat::H264,
            data: video.clone().into(),
            micros,
            keyframe: true,
        });
        a.publish_audio(p2pshare_session::AudioPacket {
            data: audio.clone().into(),
            micros,
        });
        std::thread::sleep(Duration::from_millis(u64::from(delay / 2)));
        assert!(
            b.take_video().is_empty() && b.take_audio().is_empty(),
            "Media released before target delay"
        );
        let mut videos = Vec::new();
        let mut audios = Vec::new();
        wait("buffer release", || {
            videos.extend(b.take_video());
            audios.extend(b.take_audio());
            !videos.is_empty() && !audios.is_empty()
        });
        assert!(sent.elapsed() >= Duration::from_millis(u64::from(delay)));
        assert!(sent.elapsed() < Duration::from_millis(u64::from(delay) + 500));
        assert_eq!(
            videos[0].micros, audios[0].micros,
            "Audio/video RTP clocks must preserve shared sender timestamps"
        );
        assert!(
            videos[0].received_at.elapsed()
                >= Duration::from_millis(u64::from(delay).saturating_sub(30))
        );
        println!(
            "buffer={delay}ms: video/audio released after {}ms, shared timestamp={}",
            sent.elapsed().as_millis(),
            videos[0].micros
        );
    }
}

#[test]
#[ignore = "requires the owned local Worker runtime"]
fn jpeg_previews_are_default_and_full_media_requires_watch_subscription() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("p2pshare_session=info,p2pshare_rtc=debug")
        .try_init();
    let root = root();
    let a = Session::start_cloudflare(root.join("preview-host.key"), server(), true).unwrap();
    let b = Session::start_cloudflare(root.join("preview-viewer.key"), server(), true).unwrap();
    let invite = create(&a);
    join(&b, &invite);
    wait("direct connection", || {
        a.snapshot().connected_video.len() == 1 && b.snapshot().connected_video.len() == 1
    });
    let host = a.snapshot().local_id;
    a.command(Command::Publish {
        width: 160,
        height: 96,
    });
    a.set_stream_info(p2pshare_session::StreamInfo {
        codec: None,
        bitrate_kbps: 20_000,
    });
    wait("sender stream details", || {
        b.stream_info(&host)
            .is_some_and(|info| info.bitrate_kbps == 20_000)
    });
    wait("share announced", || {
        b.snapshot()
            .members
            .iter()
            .any(|p| p.id == host && p.sharing)
    });
    let jpeg =
        p2pshare_media::preview::encode(512, 288, &[40, 100, 180, 255].repeat(512 * 288)).unwrap();
    a.publish_preview(jpeg.clone());
    let mut previews = Vec::new();
    wait("encrypted JPEG", || {
        previews.extend(b.take_previews());
        !previews.is_empty()
    });
    assert_eq!(&*previews[0].1, &jpeg);
    let image = p2pshare_media::preview::decode(&previews[0].1).unwrap();
    assert_eq!((image.width, image.height), (512, 288));
    assert!(!b.is_watching(&host));
    assert_eq!(a.snapshot().viewer_count, 0);
    assert!(!a.needs_live_video());
    let mut opus = p2pshare_media::audio::encoder(Default::default()).unwrap();
    let audio: std::sync::Arc<[u8]> = opus
        .encode_vec_float(&[0.2; p2pshare_media::audio::SAMPLES], 1275)
        .unwrap()
        .into();
    let send = || {
        a.publish(VideoFrame {
            codec: VideoFormat::H264,
            data: packet().into(),
            micros: a.media_micros(),
            keyframe: true,
        });
        a.publish_audio(p2pshare_session::AudioPacket {
            data: audio.clone(),
            micros: a.media_micros(),
        });
    };
    send();
    std::thread::sleep(Duration::from_millis(250));
    assert!(b.take_video().is_empty());
    assert!(b.take_audio().is_empty());
    b.set_watching(&host, true);
    wait("watch enabled", || a.snapshot().viewer_count == 1);
    wait("actual AVC details", || {
        send();
        b.stream_info(&host).is_some_and(|info| {
            info.codec == Some(VideoFormat::H264) && info.bitrate_kbps == 20_000
        })
    });
    assert!(a.needs_live_video());
    send();
    let mut video = Vec::new();
    wait("watch video", || {
        video.extend(b.take_video());
        !video.is_empty()
    });
    let mut sound = Vec::new();
    wait("watch audio", || {
        sound.extend(b.take_audio());
        !sound.is_empty()
    });
    assert!(
        Decoder::new()
            .unwrap()
            .decode(&video[0].data)
            .unwrap()
            .is_some()
    );
    b.set_watching(&host, false);
    wait("watch disabled", || a.snapshot().viewer_count == 0);
    let mut cached = Vec::new();
    wait("cached preview after stop", || {
        cached.extend(b.take_previews());
        !cached.is_empty()
    });
    send();
    std::thread::sleep(Duration::from_millis(250));
    assert!(b.take_video().is_empty());
    assert!(b.take_audio().is_empty());
    b.set_watching(&host, true);
    wait("watch restarted", || a.snapshot().viewer_count == 1);
    send();
    wait("video after restart", || !b.take_video().is_empty());
    a.command(Command::Unpublish);
    wait("share stopped", || {
        b.snapshot()
            .members
            .iter()
            .any(|m| m.id == host && !m.sharing)
    });
    assert!(!b.is_watching(&host));
    assert_eq!(a.snapshot().viewer_count, 0);
    a.command(Command::Publish {
        width: 160,
        height: 96,
    });
    wait("new share announced", || {
        b.snapshot()
            .members
            .iter()
            .any(|m| m.id == host && m.sharing)
    });
    send();
    std::thread::sleep(Duration::from_millis(250));
    assert!(b.take_video().is_empty());
    assert!(b.take_audio().is_empty());
    // Details are refreshed and cached for viewers joining an existing share.
    let details = p2pshare_session::StreamInfo {
        codec: Some(VideoFormat::H264),
        bitrate_kbps: 12_000,
    };
    a.set_stream_info(details);
    wait("changed bitrate delivered", || {
        b.stream_info(&host) == Some(details)
    });
    a.set_stream_info(p2pshare_session::StreamInfo {
        codec: None,
        bitrate_kbps: 0,
    });
    assert_eq!(a.stream_info(&host), Some(details));
    let late =
        Session::start_cloudflare(root.join("preview-late-viewer.key"), server(), true).unwrap();
    join(&late, &invite);
    wait("late viewer stream details", || {
        late.stream_info(&host) == Some(details)
    });
    assert!(!late.is_watching(&host));
}
