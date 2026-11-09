#[cfg(windows)]
use openh264::formats::YUVSource;
mod support;
#[cfg(windows)]
use p2pshare_session::VideoFrame;
use p2pshare_session::{Command, Session};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
fn wait(label: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "Timed out: {label}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.build/reusable-tests")
        .join(format!("{}-{}", std::process::id(), rand::random::<u32>()))
}
fn session(root: &Path, name: &str, relay: &support::Relay) -> Session {
    Session::start_with_relays(
        root.join(format!("{name}.key")),
        true,
        vec![relay.url.clone()],
    )
    .unwrap()
}
fn join(s: &Session, invite: &str, name: &str) {
    s.command(Command::Join {
        invite: invite.into(),
        name: name.into(),
    });
}
fn connected(sessions: &[&Session], count: usize) -> bool {
    sessions.iter().all(|s| {
        let st = s.snapshot();
        st.in_room && st.members.len() == count && st.connected_video.len() == count - 1
    })
}
#[cfg(windows)]
fn packet() -> Vec<u8> {
    use openh264::{
        encoder::{Encoder, EncoderConfig, Profile},
        formats::YUVBuffer,
    };
    let mut yuv = vec![90; 160 * 96];
    yuv.extend(vec![128; 160 * 96 / 2]);
    Encoder::with_api_config(
        openh264::OpenH264API::from_source(),
        EncoderConfig::new().profile(Profile::Baseline),
    )
    .unwrap()
    .encode(&YUVBuffer::from_vec(yuv, 160, 96))
    .unwrap()
    .to_vec()
}
#[test]
fn creator_exit_cold_join_all_offline_and_restart_same_link() {
    let relay = support::Relay::start();
    let root = root();
    let host = session(&root, "host", &relay);
    let a = session(&root, "a", &relay);
    let b = session(&root, "b", &relay);
    host.command(Command::Create("Creator".into()));
    wait("invitation", || host.snapshot().invite.is_some());
    let invite = host.snapshot().invite.unwrap();
    assert!(invite.starts_with("p2pshare://room/"));
    join(&a, &invite, "A");
    join(&b, &invite, "B");
    wait("three independent control/media peers", || {
        connected(&[&host, &a, &b], 3)
    });
    a.command(Command::Publish {
        width: 160,
        height: 96,
    });
    wait("publisher state", || {
        b.snapshot()
            .members
            .iter()
            .any(|m| m.name == "A" && m.sharing)
    });
    let creator_id = host.snapshot().local_id;
    drop(host);
    wait("creator absent; survivors stay connected", || {
        connected(&[&a, &b], 2) && !b.snapshot().members.iter().any(|m| m.id == creator_id)
    });
    #[cfg(windows)]
    {
        a.publish(VideoFrame {
            codec: p2pshare_media::config::VideoFormat::H264,
            data: packet().into(),
            micros: 0,
            keyframe: true,
        });
        let mut decoded = false;
        wait("H264 after creator exit", || {
            for f in b.take_video() {
                let mut d = openh264::decoder::Decoder::new().unwrap();
                if let Some(yuv) = d.decode(&f.data).unwrap() {
                    decoded = yuv.dimensions() == (160, 96);
                }
            }
            decoded
        });
    }
    let cold = session(&root, "cold", &relay);
    join(&cold, &invite, "Cold newcomer");
    wait("new device knows only old link", || {
        connected(&[&a, &b, &cold], 3)
    });
    assert_eq!(cold.snapshot().invite.as_deref(), Some(invite.as_str()));
    let old_id = a.snapshot().local_id;
    drop(a);
    drop(b);
    drop(cold);
    // Reuse the actual stored identity and saved capability, with every previous participant gone.
    let a = session(&root, "a", &relay);
    let fresh = session(&root, "fresh", &relay);
    wait("saved room loaded after restart", || {
        a.snapshot().saved_invite.as_deref() == Some(invite.as_str())
    });
    join(&a, &a.snapshot().saved_invite.unwrap(), "Restarted A");
    join(&fresh, &invite, "Fresh device");
    wait("empty room reopened without creator", || {
        connected(&[&a, &fresh], 2)
    });
    assert_eq!(old_id, a.snapshot().local_id);
    assert_eq!(a.snapshot().saved_invite.as_deref(), Some(invite.as_str()));
    assert!(relay.accepted.load(std::sync::atomic::Ordering::Relaxed) >= 5);
}
#[test]
fn different_capability_is_separate_and_relays_can_disappear() {
    let mut relay = support::Relay::start();
    let root = root();
    let a = session(&root, "a", &relay);
    let b = session(&root, "b", &relay);
    a.command(Command::Create("A".into()));
    wait("invitation", || a.snapshot().invite.is_some());
    let invite = a.snapshot().invite.unwrap();
    join(&b, &invite, "B");
    wait("two connected", || connected(&[&a, &b], 2));
    let mut wrong = p2pshare_discovery::RoomInvite::parse(&invite).unwrap();
    wrong.secret = "A".repeat(43);
    let isolated = session(&root, "wrong", &relay);
    join(&isolated, &wrong.encode().unwrap(), "Other room");
    wait("different room ready", || isolated.snapshot().in_room);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(isolated.snapshot().members.len(), 1);
    assert_eq!(a.snapshot().members.len(), 2);
    relay.shutdown();
    wait("discovery outage reported", || {
        a.snapshot().discovery_status.contains("reconnecting")
    });
    assert!(connected(&[&a, &b], 2));
    a.command(Command::Rename("Still connected".into()));
    wait("controls survive relay outage", || {
        b.snapshot()
            .members
            .iter()
            .any(|m| m.name == "Still connected")
    });
}

#[test]
fn seven_simultaneous_joiners_converge_to_six_without_creator_privileges() {
    let relay = support::Relay::start();
    let root = root();
    let mut sessions: Vec<_> = (0..7)
        .map(|n| session(&root, &format!("peer{n}"), &relay))
        .collect();
    wait("seven identities", || {
        sessions.iter().all(|s| !s.snapshot().local_id.is_empty())
    });
    sessions.sort_by_key(|s| s.snapshot().local_id);
    // Highest ID creates, demonstrating that creator has no privileged slot.
    sessions[6].command(Command::Create("Creator".into()));
    wait("invite", || sessions[6].snapshot().invite.is_some());
    let invite = sessions[6].snapshot().invite.unwrap();
    for (n, s) in sessions[..6].iter().enumerate() {
        join(s, &invite, &format!("Peer {n}"));
    }
    let started = Instant::now();
    let mut logged = false;
    wait("lowest six form complete mesh", || {
        if !logged && started.elapsed() > Duration::from_secs(8) {
            for (n, s) in sessions.iter().enumerate() {
                let snapshot = s.snapshot();
                eprintln!(
                    "device {n}: in_room={} members={} rtc={} status={}",
                    snapshot.in_room,
                    snapshot.members.len(),
                    snapshot.connected_video.len(),
                    snapshot.status
                );
            }
            logged = true;
        }
        connected(&sessions[..6].iter().collect::<Vec<_>>(), 6)
    });
    wait("seventh clearly waits outside", || {
        !sessions[6].snapshot().in_room && sessions[6].snapshot().status.contains("full")
    });
    for s in &sessions {
        assert!(s.snapshot().members.len() <= 6);
    }
}

#[test]
#[ignore = "opt-in public discovery and normal Iroh connectivity; no desktop capture"]
fn public_relays_creator_exit_and_fresh_newcomer() {
    let root = root();
    let server = std::env::var("P2PSHARE_TEST_SIGNALING").expect("Set a signaling server for the public test");
    let create = || {
        Session::start_cloudflare(root.join(format!("{}.key", rand::random::<u32>())), server.clone(), false).unwrap()
    };
    let host = create();
    let survivor = create();
    host.command(Command::Create("Public probe creator".into()));
    wait("public invitation", || host.snapshot().invite.is_some());
    let invite = host.snapshot().invite.unwrap();
    join(&survivor, &invite, "Survivor");
    wait("public discovery and real media path", || {
        connected(&[&host, &survivor], 2)
    });
    drop(host);
    wait("creator closed", || {
        survivor.snapshot().members.len() == 1 && survivor.snapshot().in_room
    });
    let fresh = create();
    join(&fresh, &invite, "Fresh newcomer");
    wait("fresh newcomer finds survivor", || {
        connected(&[&survivor, &fresh], 2)
    });
    assert!(fresh.snapshot().discovery_status.contains("Cloudflare"));
}

#[test]
fn invalid_capability_and_spoofed_identity_never_become_members() {
    let relay = support::Relay::start();
    let root = root();
    let host = session(&root, "host", &relay);
    host.command(Command::Create("Host".into()));
    wait("invitation", || host.snapshot().invite.is_some());
    let invite = p2pshare_discovery::RoomInvite::parse(&host.snapshot().invite.unwrap()).unwrap();
    wait("stored signed presence", || {
        !relay.records.lock().unwrap().is_empty()
    });
    let record = relay
        .records
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    let event = serde_json::from_value(record).unwrap();
    let address = p2pshare_discovery::verify_presence(&invite, &event)
        .unwrap()
        .address;
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        for spoof in [false,true] {
            let endpoint=p2pshare_control_iroh::bind(&root.join(format!("attacker{spoof}.key")),true).await.unwrap();
            let connection=p2pshare_control_iroh::connect_peer(&endpoint,address.clone()).await.unwrap();
            let (mut send,mut recv)=connection.open_bi().await.unwrap();
            let member_id=if spoof { host.snapshot().local_id } else { endpoint.id().to_string() };
            let capability=if spoof { invite.secret.clone() } else { "B".repeat(43) };
            let message=serde_json::json!({"MeshJoin":{"room":invite.topic().unwrap(),"capability":capability,"member":{"id":member_id,"name":"Intruder","sharing":false,"width":0,"height":0},"address":endpoint.addr()}});
            p2pshare_control_iroh::send(&mut send,&message).await.unwrap();
            let response:serde_json::Value=tokio::time::timeout(Duration::from_secs(5),p2pshare_control_iroh::receive(&mut recv)).await.unwrap().unwrap();
            assert!(response.get("Error").is_some());
            connection.close(0u32.into(),b"Test completed");
            endpoint.close().await;
        }
    });
    assert_eq!(host.snapshot().members.len(), 1);
}

#[test]
fn same_device_leaves_and_rejoins_repeatedly() {
    let relay = support::Relay::start();
    let root = root();
    let a = session(&root, "a", &relay);
    let b = session(&root, "b", &relay);
    a.command(Command::Create("A".into()));
    wait("invite", || a.snapshot().invite.is_some());
    let invite = a.snapshot().invite.unwrap();
    for _ in 0..3 {
        join(&b, &invite, "B");
        wait("repeated connection", || connected(&[&a, &b], 2));
        b.command(Command::Leave);
        wait("leave completes", || {
            !b.snapshot().in_room && a.snapshot().members.len() == 1
        });
    }
}

// Internal fixture entry point. Ordinary test runs return before starting a session.
#[test]
fn crash_creator_child() {
    let Some(root) = std::env::var_os("P2PSHARE_CRASH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.build")
        .canonicalize()
        .unwrap();
    assert!(root.canonicalize().unwrap().starts_with(allowed));
    let relay = std::env::var("P2PSHARE_CRASH_RELAY").unwrap();
    assert!(relay.starts_with("ws://127.0.0.1:"));
    let host = Session::start_with_relays(root.join("crash-host.key"), true, vec![relay]).unwrap();
    host.command(Command::Create("Crash fixture creator".into()));
    wait("child invitation", || host.snapshot().invite.is_some());
    std::fs::write(root.join("crash.invite"), host.snapshot().invite.unwrap()).unwrap();
    loop {
        let state = host.snapshot();
        if state.members.len() == 3 && state.connected_video.len() == 2 {
            std::fs::write(root.join("crash.ready"), b"ready").unwrap();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
struct CrashChild(std::process::Child);
impl Drop for CrashChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn abrupt_creator_process_death_does_not_stop_video_or_cold_join() {
    let relay = support::Relay::start();
    let root = root();
    std::fs::create_dir_all(&root).unwrap();
    let mut child = CrashChild(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_creator_child", "--test-threads=1"])
            .env("P2PSHARE_CRASH_ROOT", &root)
            .env("P2PSHARE_CRASH_RELAY", &relay.url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait("child invite file", || root.join("crash.invite").exists());
    let invite = std::fs::read_to_string(root.join("crash.invite")).unwrap();
    let a = session(&root, "a", &relay);
    let b = session(&root, "b", &relay);
    join(&a, &invite, "A");
    join(&b, &invite, "B");
    wait("child full mesh", || root.join("crash.ready").exists());
    a.command(Command::Publish {
        width: 160,
        height: 96,
    });
    wait("share state", || {
        b.snapshot()
            .members
            .iter()
            .any(|m| m.name == "A" && m.sharing)
    });
    child.0.kill().unwrap();
    child.0.wait().unwrap(); // No Session drop or graceful QUIC close.
    #[cfg(windows)]
    {
        a.publish(VideoFrame {
            codec: p2pshare_media::config::VideoFormat::H264,
            data: packet().into(),
            micros: 0,
            keyframe: true,
        });
        let mut decoded = false;
        wait("video after process crash", || {
            for f in b.take_video() {
                let mut d = openh264::decoder::Decoder::new().unwrap();
                if let Some(y) = d.decode(&f.data).unwrap() {
                    decoded = y.dimensions() == (160, 96);
                }
            }
            decoded
        });
    }
    let fresh = session(&root, "fresh", &relay);
    join(&fresh, &invite, "Fresh");
    let started = Instant::now();
    let mut logged = false;
    wait(
        "cold newcomer joins surviving peers after process crash",
        || {
            if !logged && started.elapsed() > Duration::from_secs(5) {
                for (name, session) in [("a", &a), ("b", &b), ("fresh", &fresh)] {
                    let s = session.snapshot();
                    eprintln!(
                        "{name}: in_room={} members={:?} rtc={} status={} discovery={}",
                        s.in_room,
                        s.members.iter().map(|m| &m.name).collect::<Vec<_>>(),
                        s.connected_video.len(),
                        s.status,
                        s.discovery_status
                    );
                }
                logged = true;
            }
            connected(&[&fresh], 3)
        },
    );
    assert!(
        fresh
            .snapshot()
            .members
            .iter()
            .all(|m| m.name != "Crash fixture creator")
    );
    assert!(a.snapshot().in_room && b.snapshot().in_room);
}

#[test]
fn normal_quit_and_same_identity_restart_resume_without_long_backoff() {
    fn within(label: &str, limit: Duration, mut ready: impl FnMut() -> bool) -> Duration {
        let started = Instant::now();
        while !ready() {
            assert!(started.elapsed() < limit, "Timed out: {label}");
            std::thread::sleep(Duration::from_millis(10));
        }
        started.elapsed()
    }
    // Cover both deterministic mesh-dial directions with the same saved identity.
    for restart_lower_id in [true, false] {
        let relay = support::Relay::start();
        let root = root();
        let first = session(&root, "first", &relay);
        let second = session(&root, "second", &relay);
        first.command(Command::Create("First".into()));
        wait("invite", || first.snapshot().invite.is_some());
        let invite = first.snapshot().invite.unwrap();
        join(&second, &invite, "Second");
        wait("initial pair", || connected(&[&first, &second], 2));
        let first_is_lower = first.snapshot().local_id < second.snapshot().local_id;
        let (publisher, viewer, key_name) = if first_is_lower == restart_lower_id {
            (first, second, "first")
        } else {
            (second, first, "second")
        };
        let id = publisher.snapshot().local_id;
        publisher.command(Command::Publish {
            width: 160,
            height: 96,
        });
        wait("sharing", || {
            viewer
                .snapshot()
                .members
                .iter()
                .any(|m| m.id == id && m.sharing)
        });
        let quit_at = Instant::now();
        drop(publisher);
        let removal = within("quit removes old publisher", Duration::from_secs(2), || {
            !viewer.snapshot().members.iter().any(|m| m.id == id)
        });
        println!(
            "lower_id={restart_lower_id}: quit/drop={}ms, remaining removal={}ms",
            quit_at.elapsed().as_millis(),
            removal.as_millis()
        );
        let restarted = session(&root, key_name, &relay);
        join(&restarted, &invite, "Restarted");
        let reconnect = within(
            "same identity reconnects promptly",
            Duration::from_secs(4),
            || connected(&[&restarted, &viewer], 2),
        );
        assert_eq!(restarted.snapshot().local_id, id);
        restarted.command(Command::Publish {
            width: 160,
            height: 96,
        });
        within("fresh publisher", Duration::from_secs(1), || {
            viewer
                .snapshot()
                .members
                .iter()
                .any(|m| m.id == id && m.sharing)
        });
        #[cfg(windows)]
        {
            let bytes = packet();
            let started = Instant::now();
            let mut received = false;
            while started.elapsed() < Duration::from_secs(1) && !received {
                restarted.publish(VideoFrame {
                    codec: p2pshare_media::config::VideoFormat::H264,
                    data: bytes.clone().into(),
                    micros: started.elapsed().as_micros() as u64,
                    keyframe: true,
                });
                std::thread::sleep(Duration::from_millis(20));
                received = !viewer.take_video().is_empty();
            }
            assert!(received, "Fresh stream never arrived");
        }
        println!(
            "lower_id={restart_lower_id}: reconnect={}ms; fresh video delivered",
            reconnect.as_millis()
        );
        drop(restarted);
        drop(viewer);
    }
}
