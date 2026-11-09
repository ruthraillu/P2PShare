#[cfg(windows)]
mod windows {
    use openh264::{
        decoder::Decoder,
        encoder::{BitRate, Encoder, EncoderConfig, Profile},
        formats::{YUVBuffer, YUVSource},
    };
    use p2pshare_session::{Command, Session, VideoFrame};
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };
    fn wait(label: &str, mut condition: impl FnMut() -> bool) {
        let start = Instant::now();
        while !condition() {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "Timed out: {label}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/room-tests")
            .join(format!("{}-{}", std::process::id(), rand::random::<u32>()))
    }
    fn packet(color: u8) -> Vec<u8> {
        let mut yuv = vec![color; 160 * 96];
        yuv.extend(vec![128; 160 * 96 / 2]);
        let mut encoder = Encoder::with_api_config(
            openh264::OpenH264API::from_source(),
            EncoderConfig::new()
                .profile(Profile::Baseline)
                .bitrate(BitRate::from_bps(500_000)),
        )
        .unwrap();
        encoder
            .encode(&YUVBuffer::from_vec(yuv, 160, 96))
            .unwrap()
            .to_vec()
    }
    #[test]
    fn real_rooms_three_publishers_send_encrypted_h264_and_leave() {
        let root = root();
        let host = Session::start_isolated(root.join("host.key")).unwrap();
        let a = Session::start_isolated(root.join("a.key")).unwrap();
        let b = Session::start_isolated(root.join("b.key")).unwrap();
        host.command(Command::Create("Host".into()));
        wait("create invitation", || host.snapshot().invite.is_some());
        let invite = host.snapshot().invite.unwrap();
        a.command(Command::Join {
            invite: invite.clone(),
            name: "Windows".into(),
        });
        b.command(Command::Join {
            invite: invite.clone(),
            name: "Android".into(),
        });
        wait("three actual members", || {
            host.snapshot().members.len() == 3
                && a.snapshot().members.len() == 3
                && b.snapshot().members.len() == 3
        });
        wait("full-mesh DTLS/SRTP", || {
            [host.snapshot(), a.snapshot(), b.snapshot()]
                .iter()
                .all(|s| s.connected_video.len() == 2)
        });
        for session in [&host, &a, &b] {
            session.command(Command::Publish {
                width: 160,
                height: 96,
            });
        }
        wait("three publishers", || {
            a.snapshot().members.iter().filter(|m| m.sharing).count() == 3
                && b.snapshot().members.iter().filter(|m| m.sharing).count() == 3
        });
        let colors = [40, 100, 190];
        for (i, session) in [&host, &a, &b].iter().enumerate() {
            session.publish(VideoFrame {
                codec: p2pshare_media::config::VideoFormat::H264,
                data: packet(colors[i]).into(),
                micros: 0,
                keyframe: true,
            });
        }
        let mut received = [
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ];
        wait("bidirectional H264 media", || {
            for (i, session) in [&host, &a, &b].iter().enumerate() {
                for f in session.take_video() {
                    received[i].insert(f.peer, f.data);
                }
            }
            received.iter().all(|r| r.len() == 2)
        });
        for packets in received {
            for bytes in packets.values() {
                let mut decoder = Decoder::new().unwrap();
                let yuv = decoder.decode(bytes).unwrap().expect("Actual H264 decodes");
                assert_eq!(yuv.dimensions(), (160, 96));
                let mean = yuv.y().iter().map(|v| *v as u64).sum::<u64>() / yuv.y().len() as u64;
                assert!(
                    colors.iter().any(|c| mean.abs_diff(*c as u64) < 5),
                    "Received pixels {mean}"
                );
            }
        }
        b.command(Command::Unpublish);
        wait("unpublish reaches room", || {
            a.snapshot().members.iter().filter(|m| m.sharing).count() == 2
        });
        b.command(Command::Leave);
        wait("leave removes peer", || {
            host.snapshot().members.len() == 2
                && a.snapshot().members.len() == 2
                && a.snapshot().connected_video.len() == 1
        });
        let bad = Session::start_isolated(root.join("bad.key")).unwrap();
        let mut wrong = p2pshare_control_iroh::Invite::parse(&invite).unwrap();
        wrong.token = "A".repeat(43);
        let wrong = wrong.encode().unwrap();
        bad.command(Command::Join {
            invite: wrong,
            name: "Unauthorized".into(),
        });
        wait("reject altered invitation", || {
            !bad.snapshot().busy && bad.snapshot().status.contains("Could not join")
        });
        assert_eq!(host.snapshot().members.len(), 2);
    }
    #[test]
    fn continuous_motion_preserves_interframe_dependencies_and_frame_rate() {
        use openh264::encoder::{FrameRate, FrameType, IntraFramePeriod};
        let root = root();
        let host = Session::start_isolated(root.join("host.key")).unwrap();
        let viewer = Session::start_isolated(root.join("viewer.key")).unwrap();
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
    fn hd_large_frames_survive_udp_bursts() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("warn,p2pshare_rtc=debug,str0m=debug")
            .try_init();
        use openh264::encoder::{FrameRate, FrameType, IntraFramePeriod};
        let root = root();
        let host = Session::start_isolated(root.join("host.key")).unwrap();
        let viewer = Session::start_isolated(root.join("viewer.key")).unwrap();
        host.command(Command::Create("HD burst publisher".into()));
        wait("invite", || host.snapshot().invite.is_some());
        viewer.command(Command::Join {
            invite: host.snapshot().invite.unwrap(),
            name: "Viewer".into(),
        });
        wait("DTLS/SRTP", || viewer.snapshot().connected_video.len() == 1);
        host.command(Command::Publish {
            width: 1280,
            height: 720,
        });
        wait("share announced", || {
            viewer.snapshot().members.iter().any(|m| m.sharing)
        });
        let mut encoder = Encoder::with_api_config(
            openh264::OpenH264API::from_source(),
            EncoderConfig::new()
                .profile(Profile::Baseline)
                .skip_frames(false)
                .rate_control_mode(openh264::encoder::RateControlMode::Bitrate)
                .scene_change_detect(false)
                .num_threads(1)
                .bitrate(BitRate::from_bps(16_000_000))
                .max_frame_rate(FrameRate::from_hz(30.0))
                .intra_frame_period(IntraFramePeriod::from_num_frames(120)),
        )
        .unwrap();
        let mut decoder = Decoder::new().unwrap();
        let mut decoded = 0;
        let mut largest = 0;
        let mut total = 0;
        let mut frames = Vec::new();
        let mut noise = 0x12345678u32;
        let mut texture = vec![0; 1280 * 720];
        for value in &mut texture {
            noise ^= noise << 13;
            noise ^= noise >> 17;
            noise ^= noise << 5;
            *value = 32 + (noise % 192) as u8;
        }
        for n in 0..45 {
            let mut pixels = vec![128; 1280 * 720 * 3 / 2];
            // A detailed moving surface, rather than unrelated full-frame noise
            // that exceeds the bitrate budget even with maximum quantization.
            for y in 0..720 {
                for x in 0..1280 {
                    pixels[y * 1280 + x] = texture[y * 1280 + (x + n as usize * 4) % 1280];
                }
            }
            let encoded = encoder
                .encode_at(
                    &YUVBuffer::from_vec(pixels, 1280, 720),
                    openh264::Timestamp::from_millis(n * 33),
                )
                .unwrap();
            let bytes = encoded.to_vec();
            largest = largest.max(bytes.len());
            total += bytes.len();
            frames.push(VideoFrame {
                codec: p2pshare_media::config::VideoFormat::H264,
                data: bytes.into(),
                micros: n * 33_333,
                keyframe: encoded.frame_type() == FrameType::IDR,
            });
        }
        // Validate the encoder chain separately. CPU decoding must not turn
        // the timed publisher into an unintended burst when this PC is busy.
        let mut source_decoder = Decoder::new().unwrap();
        for frame in &frames {
            assert!(source_decoder.decode(&frame.data).unwrap().is_some());
        }
        let mut received = Vec::new();
        for frame in frames {
            let until = Instant::now() + Duration::from_micros(33_333);
            host.publish(frame);
            while Instant::now() < until {
                received.extend(viewer.take_video());
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until && received.len() < 45 {
            received.extend(viewer.take_video());
            std::thread::sleep(Duration::from_millis(2));
        }
        println!("HD receive diagnostics: {}", viewer.video_diagnostics());
        println!("HD sender diagnostics: {}", host.video_diagnostics());
        for frame in received {
            if let Some(yuv) = decoder.decode(&frame.data).unwrap() {
                assert_eq!(yuv.dimensions(), (1280, 720));
                decoded += 1;
            }
        }
        println!(
            "HD burst stream: {decoded}/45 decoded; largest frame {largest} bytes; total {total} bytes"
        );
        assert!(
            largest > 64 * 1024,
            "Must exceed the small default Windows UDP receive buffer"
        );
        assert_eq!(
            decoded, 45,
            "HD interframe chain must survive large packet bursts"
        );
    }
}
