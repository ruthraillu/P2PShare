//! Sans-I/O WebRTC H.264 adapter. Every mutation is followed by drain().
use anyhow::{Context, Result};
use p2pshare_media::config::VideoFormat;
use std::{
    net::SocketAddr,
    sync::{Arc, Once},
    time::{Duration, Instant},
};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc, RtcConfig};
pub enum RtcEvent {
    ControlReady,
    Control(Vec<u8>),
    Audio {
        data: Arc<[u8]>,
        micros: u64,
        received_at: Instant,
    },
    Connected,
    Disconnected,
    Frame {
        codec: VideoFormat,
        data: Arc<[u8]>,
        keyframe: bool,
        contiguous: bool,
        received_at: Instant,
        micros: u64,
    },
    Keyframe,
}
pub struct Drain {
    pub packets: Vec<(SocketAddr, SocketAddr, Vec<u8>)>,
    pub events: Vec<RtcEvent>,
}
pub struct PeerRtc {
    rtc: Rtc,
    pending: Option<SdpPendingOffer>,
    mid: Option<Mid>,
    audio_mid: Option<Mid>,
    pub deadline: Instant,
    pub connected: bool,
    first_rtp: Option<Instant>,
    rtp_packets: u64,
    first_frame_reported: bool,
    selected_path: Option<(SocketAddr, SocketAddr)>,
    control_channel: Option<str0m::channel::ChannelId>,
}
impl PeerRtc {
    pub fn new(candidates: &[Candidate]) -> Result<Self> {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            str0m::crypto::from_feature_flags().install_process_default();
        });
        let now = Instant::now();
        // A missing fragment must not accumulate two seconds of compressed
        // frames and then overrun the bounded UI/decoder queues on recovery.
        let mut config = RtcConfig::new()
            .clear_codecs()
            .set_reordering_timeout_audio(Some(Duration::from_millis(15)))
            .set_reordering_size_audio(3)
            .set_reordering_timeout_video(Some(Duration::from_millis(500)))
            .set_send_buffer_video(4096)
            .set_stats_interval(Some(Duration::from_secs(1)));
        config
            .codec_config()
            .add_h264(127.into(), Some(121.into()), true, 0x420033);
        config
            .codec_config()
            .add_h264(108.into(), Some(109.into()), true, 0x42e033);
        config.codec_config().enable_h265(true);
        config.codec_config().add_config(
            111.into(),
            None,
            str0m::format::Codec::Opus,
            str0m::media::Frequency::FORTY_EIGHT_KHZ,
            Some(2),
            str0m::format::FormatParams {
                min_p_time: Some(10),
                stereo: Some(true),
                sprop_stereo: Some(true),
                use_inband_fec: Some(false),
                use_dtx: Some(false),
                ..Default::default()
            },
        );
        let mut rtc = config.build(now);
        for candidate in candidates {
            rtc.add_local_candidate(candidate.clone())
                .context("Invalid local candidate")?;
        }
        Ok(Self {
            rtc,
            pending: None,
            mid: None,
            audio_mid: None,
            deadline: now,
            connected: false,
            first_rtp: None,
            rtp_packets: 0,
            first_frame_reported: false,
            selected_path: None,
            control_channel: None,
        })
    }
    /// Trickle candidates into the current ICE session; the caller immediately drains.
    pub fn add_local_candidate(&mut self, candidate: &Candidate) {
        self.rtc.add_local_candidate(candidate.clone());
    }
    pub fn add_remote_candidate(&mut self, candidate: &Candidate) {
        self.rtc.add_remote_candidate(candidate.clone());
    }
    pub fn path_version(&self) -> Option<&'static str> {
        self.selected_path
            .map(|p| if p.1.is_ipv6() { "IPv6" } else { "IPv4" })
    }
    pub fn send_control(&mut self, bytes: &[u8]) -> Result<bool> {
        anyhow::ensure!(bytes.len() <= 49153, "Control message too large");
        let Some(id) = self.control_channel else {
            return Ok(false);
        };
        let Some(mut channel) = self.rtc.channel(id) else {
            return Ok(false);
        };
        if channel.buffered_amount() > 65536 {
            return Ok(false);
        }
        Ok(channel.write(true, bytes)?)
    }
    pub fn offer(&mut self) -> Result<String> {
        let mut change = self.rtc.sdp_api();
        self.mid = Some(change.add_media(MediaKind::Video, Direction::SendRecv, None, None, None));
        self.audio_mid =
            Some(change.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None));
        change.add_channel("p2pshare-preview-v1".into());
        let (offer, pending) = change.apply().context("No SDP offer")?;
        self.pending = Some(pending);
        Ok(serde_json::to_string(&offer)?)
    }
    pub fn accept_offer(&mut self, json: &str) -> Result<String> {
        let offer: SdpOffer = serde_json::from_str(json)?;
        Ok(serde_json::to_string(
            &self.rtc.sdp_api().accept_offer(offer)?,
        )?)
    }
    pub fn accept_answer(&mut self, json: &str) -> Result<()> {
        let answer: SdpAnswer = serde_json::from_str(json)?;
        let pending = self.pending.take().context("Unexpected SDP answer")?;
        self.rtc.sdp_api().accept_answer(pending, answer)?;
        Ok(())
    }
    pub fn accepts(&self, source: SocketAddr, destination: SocketAddr, bytes: &[u8]) -> bool {
        bytes.try_into().ok().is_some_and(|contents| {
            self.rtc.accepts(&Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination,
                    contents,
                },
            ))
        })
    }
    pub fn receive(
        &mut self,
        source: SocketAddr,
        destination: SocketAddr,
        bytes: &[u8],
    ) -> Result<()> {
        if bytes.len() >= 12
            && bytes[0] & 0xc0 == 0x80
            && matches!(bytes[1] & 0x7f, 127 | 108 | 121 | 109 | 102 | 103 | 45 | 46)
        {
            self.first_rtp.get_or_insert_with(Instant::now);
            self.rtp_packets += 1;
        }
        self.rtc.handle_input(Input::Receive(
            Instant::now(),
            Receive {
                proto: Protocol::Udp,
                source,
                destination,
                contents: bytes.try_into()?,
            },
        ))?;
        Ok(())
    }
    pub fn tick(&mut self) -> Result<()> {
        self.rtc.handle_input(Input::Timeout(Instant::now()))?;
        Ok(())
    }
    pub fn write(&mut self, bytes: Arc<[u8]>, micros: u64) -> Result<()> {
        self.write_codec(bytes, micros, VideoFormat::H264)
    }
    pub fn write_codec(&mut self, bytes: Arc<[u8]>, micros: u64, codec: VideoFormat) -> Result<()> {
        if !self.connected {
            return Ok(());
        }
        if let Some(writer) = self.mid.and_then(|mid| self.rtc.writer(mid)) {
            let expected = match codec {
                VideoFormat::H264 => str0m::format::Codec::H264,
                VideoFormat::Hevc => str0m::format::Codec::H265,
            };
            let pt = writer
                .payload_params()
                .find(|p| p.spec().codec == expected)
                .map(|p| p.pt());
            anyhow::ensure!(
                pt.is_some(),
                "Peer did not negotiate {} decoding; choose AVC or update the viewer",
                codec.label()
            );
            if let Some(pt) = pt {
                writer.write(pt, Instant::now(), MediaTime::from_micros(micros), bytes)?;
            }
        }
        // Frame-mode packetization runs when the RTC clock advances. The actor's
        // 10-ms housekeeping tick cannot pace 100 audio or 120 video frames/sec.
        self.tick()
    }
    pub fn write_audio(&mut self, bytes: Arc<[u8]>, micros: u64) -> Result<()> {
        if !self.connected {
            return Ok(());
        }
        if let Some(writer) = self.audio_mid.and_then(|mid| self.rtc.writer(mid)) {
            let pt = writer
                .payload_params()
                .find(|p| p.spec().codec == str0m::format::Codec::Opus)
                .map(|p| p.pt());
            if let Some(pt) = pt {
                writer.write(pt, Instant::now(), MediaTime::from_micros(micros), bytes)?;
            }
        }
        self.tick()
    }
    pub fn request_keyframe(&mut self) -> Result<()> {
        if let Some(mut writer) = self.mid.and_then(|mid| self.rtc.writer(mid)) {
            writer.request_keyframe(None, KeyframeRequestKind::Pli)?;
        }
        Ok(())
    }
    pub fn drain(&mut self) -> Result<Drain> {
        let mut out = Drain {
            packets: vec![],
            events: vec![],
        };
        loop {
            match self.rtc.poll_output()? {
                Output::Timeout(t) => {
                    self.deadline = t.max(Instant::now() + Duration::from_millis(1));
                    break;
                }
                Output::Transmit(t) => {
                    out.packets
                        .push((t.source, t.destination, t.contents.to_vec()))
                }
                Output::Event(e) => match e {
                    Event::ChannelOpen(id, label) if label == "p2pshare-preview-v1" => {
                        self.control_channel = Some(id);
                        out.events.push(RtcEvent::ControlReady);
                    }
                    Event::ChannelClose(id) if self.control_channel == Some(id) => {
                        self.control_channel = None
                    }
                    Event::ChannelData(data)
                        if self.control_channel == Some(data.id)
                            && data.binary
                            && data.data.len() <= 49153 =>
                    {
                        out.events.push(RtcEvent::Control(data.data))
                    }
                    Event::Connected => {
                        self.connected = true;
                        out.events.push(RtcEvent::Connected);
                    }
                    Event::MediaAdded(m) => match m.kind {
                        MediaKind::Video => self.mid = Some(m.mid),
                        MediaKind::Audio => self.audio_mid = Some(m.mid),
                    },
                    Event::MediaData(m) if m.params.spec().codec == str0m::format::Codec::Opus => {
                        if !m.data.is_empty() && m.data.len() <= 1275 {
                            out.events.push(RtcEvent::Audio {
                                data: m.data,
                                micros: m.time.as_micros(),
                                received_at: m.network_time,
                            });
                        }
                    }
                    Event::MediaData(m) if m.data.len() <= 2 * 1024 * 1024 => {
                        if !self.first_frame_reported || !m.contiguous {
                            tracing::debug!(
                                bytes = m.data.len(),
                                keyframe = m.is_keyframe(),
                                contiguous = m.contiguous,
                                packets = self.rtp_packets,
                                startup_ms = self.first_rtp.map(|t| t.elapsed().as_millis() as u64),
                                "Video startup or gap"
                            );
                            self.first_frame_reported = true;
                        }
                        let codec = match m.params.spec().codec {
                            str0m::format::Codec::H264 => VideoFormat::H264,
                            str0m::format::Codec::H265 => VideoFormat::Hevc,
                            _ => continue,
                        };
                        out.events.push(RtcEvent::Frame {
                            codec,
                            keyframe: m.is_keyframe(),
                            contiguous: m.contiguous,
                            received_at: m.network_time,
                            micros: m.time.as_micros(),
                            data: m.data,
                        })
                    }
                    Event::PeerStats(s) => {
                        if let Some(pair) = &s.selected_candidate_pair {
                            let path = (pair.local.addr, pair.remote.addr);
                            if self.selected_path != Some(path) {
                                self.selected_path = Some(path);
                                tracing::info!(local=%path.0, remote=%path.1, ip_version=if path.1.is_ipv6() { "IPv6" } else { "IPv4" }, "media_path_selected");
                            }
                        }
                        if p2pshare_media::diagnostics::enabled() {
                            tracing::info!(
                                ip_version = self
                                    .selected_path
                                    .map(|p| if p.1.is_ipv6() { "IPv6" } else { "IPv4" })
                                    .unwrap_or("unknown"),
                                ice_rtt_ms = s
                                    .selected_candidate_pair
                                    .as_ref()
                                    .and_then(|p| p.current_round_trip_time)
                                    .map(|r| r.as_secs_f64() * 1000.)
                                    .unwrap_or(-1.),
                                rx_bytes = s.bytes_rx,
                                tx_bytes = s.bytes_tx,
                                rtt_ms = s.rtt.map(|d| d.as_secs_f64() * 1000.).unwrap_or(-1.),
                                rx_loss_fraction = s.ingress_loss_fraction.unwrap_or(-1.),
                                tx_loss_fraction = s.egress_loss_fraction.unwrap_or(-1.),
                                "network_peer"
                            );
                        }
                    }
                    Event::MediaIngressStats(s) => {
                        if p2pshare_media::diagnostics::enabled() {
                            tracing::info!(
                                bytes = s.bytes,
                                packets = s.packets,
                                nacks = s.nacks,
                                plis = s.plis,
                                media = if Some(s.mid) == self.audio_mid {
                                    "audio"
                                } else {
                                    "video"
                                },
                                jitter_ms = s.jitter as f64
                                    / if Some(s.mid) == self.audio_mid {
                                        48.
                                    } else {
                                        90.
                                    },
                                rtt_ms = s.rtt.map(|d| d.as_secs_f64() * 1000.).unwrap_or(-1.),
                                loss_fraction = s.loss.unwrap_or(-1.),
                                "network_receive"
                            );
                        }
                    }
                    Event::MediaEgressStats(s) => {
                        if p2pshare_media::diagnostics::enabled() {
                            tracing::info!(
                                bytes = s.bytes,
                                packets = s.packets,
                                nacks = s.nacks,
                                plis = s.plis,
                                rtt_ms = s.rtt.map(|d| d.as_secs_f64() * 1000.).unwrap_or(-1.),
                                media = if Some(s.mid) == self.audio_mid {
                                    "audio"
                                } else {
                                    "video"
                                },
                                remote_packets_lost =
                                    s.remote.as_ref().map(|r| r.packets_lost).unwrap_or(0),
                                remote_jitter_ms = s
                                    .remote
                                    .as_ref()
                                    .map(|r| r.jitter as f64
                                        / if Some(s.mid) == self.audio_mid {
                                            48.
                                        } else {
                                            90.
                                        })
                                    .unwrap_or(-1.),
                                "network_send"
                            );
                        }
                    }
                    Event::KeyframeRequest(_) => {
                        tracing::info!("keyframe_requested_by_receiver");
                        out.events.push(RtcEvent::Keyframe);
                    }
                    Event::IceConnectionStateChange(state) => {
                        tracing::info!(?state, "media_ice_state");
                        match state {
                            str0m::IceConnectionState::Connected
                            | str0m::IceConnectionState::Completed
                                if self.rtc.is_connected() =>
                            {
                                if !self.connected {
                                    self.connected = true;
                                    out.events.push(RtcEvent::Connected);
                                }
                            }
                            str0m::IceConnectionState::Disconnected => {
                                self.connected = false;
                                out.events.push(RtcEvent::Disconnected);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                },
            }
        }
        Ok(out)
    }
}
pub fn host_candidate(addr: SocketAddr) -> Result<Candidate> {
    Ok(Candidate::host(addr, "udp")?)
}
pub fn reflexive_candidate(public: SocketAddr, base: SocketAddr) -> Result<Candidate> {
    Ok(Candidate::server_reflexive(public, base, "udp")?)
}
pub use str0m::Candidate as IceCandidate;
pub use str0m::net::Protocol as IceProtocol;

#[cfg(test)]
mod codec_tests {
    use super::*;
    fn exchange(a: &mut PeerRtc, b: &mut PeerRtc) -> Result<Vec<RtcEvent>> {
        let mut events = Vec::new();
        for _ in 0..8 {
            let x = a.drain()?;
            let y = b.drain()?;
            let idle = x.packets.is_empty() && y.packets.is_empty();
            events.extend(y.events);
            for (source, dest, bytes) in x.packets {
                b.receive(source, dest, &bytes)?;
            }
            for (source, dest, bytes) in y.packets {
                a.receive(source, dest, &bytes)?;
            }
            if idle {
                break;
            }
        }
        Ok(events)
    }
    #[test]
    fn dual_stack_prefers_ipv6_and_falls_back_when_ipv6_is_unreachable() -> Result<()> {
        for blocked_ipv6 in [false, true] {
            let mut a = PeerRtc::new(&[
                host_candidate("[::1]:21001".parse()?)?,
                host_candidate("127.0.0.1:21001".parse()?)?,
            ])?;
            let mut b = PeerRtc::new(&[
                host_candidate("[::1]:21002".parse()?)?,
                host_candidate("127.0.0.1:21002".parse()?)?,
            ])?;
            let offer = a.offer()?;
            let answer = b.accept_offer(&offer)?;
            a.accept_answer(&answer)?;
            let until = Instant::now() + Duration::from_secs(5);
            while Instant::now() < until && (a.selected_path.is_none() || b.selected_path.is_none())
            {
                a.tick()?;
                b.tick()?;
                for (source, dest, bytes) in a.drain()?.packets {
                    if !blocked_ipv6 || source.is_ipv4() {
                        b.receive(source, dest, &bytes)?;
                    }
                }
                for (source, dest, bytes) in b.drain()?.packets {
                    if !blocked_ipv6 || source.is_ipv4() {
                        a.receive(source, dest, &bytes)?;
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            anyhow::ensure!(a.connected && b.connected, "Dual-stack handshake failed");
            for peer in [&a, &b] {
                let path = peer.selected_path.context("No selected path statistics")?;
                assert_eq!(path.0.is_ipv6(), !blocked_ipv6);
                assert_eq!(path.1.is_ipv6(), !blocked_ipv6);
            }
        }
        Ok(())
    }
    #[test]
    fn ipv4_hole_punches_through_two_simulated_cgnats_with_ipv6_blocked() -> Result<()> {
        let a_host = "192.168.10.2:21001".parse()?;
        let b_host = "192.168.20.2:21002".parse()?;
        let a_public = "203.0.113.10:31001".parse()?;
        let b_public = "203.0.113.20:31002".parse()?;
        let mut a = PeerRtc::new(&[
            host_candidate("[2001:db8::1]:21001".parse()?)?,
            host_candidate(a_host)?,
            reflexive_candidate(a_public, a_host)?,
        ])?;
        let mut b = PeerRtc::new(&[
            host_candidate("[2001:db8::2]:21002".parse()?)?,
            host_candidate(b_host)?,
            reflexive_candidate(b_public, b_host)?,
        ])?;
        let offer = a.offer()?;
        let answer = b.accept_offer(&offer)?;
        a.accept_answer(&answer)?;
        let mut a_open = false;
        let mut b_open = false;
        let mut filtered = 0;
        let until = Instant::now() + Duration::from_secs(6);
        while Instant::now() < until && (a.selected_path.is_none() || b.selected_path.is_none()) {
            a.tick()?;
            b.tick()?;
            for (_, dest, data) in a.drain()?.packets {
                if dest == b_public {
                    a_open = true;
                    // Endpoint-independent public mappings, address/port-restricted inbound filtering.
                    if b_open {
                        b.receive(a_public, b_host, &data)?;
                    } else {
                        filtered += 1;
                    }
                }
            }
            for (_, dest, data) in b.drain()?.packets {
                if dest == a_public {
                    b_open = true;
                    if a_open {
                        a.receive(b_public, a_host, &data)?;
                    } else {
                        filtered += 1;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            a_open && b_open && filtered > 0,
            "Both NATs must require outbound hole punching"
        );
        assert!(
            a.connected && b.connected,
            "Both CGNAT peers should establish DTLS"
        );
        assert_eq!(a.path_version(), Some("IPv4"));
        assert_eq!(b.path_version(), Some("IPv4"));
        a.send_control(b"synthetic CGNAT preview")?;
        let mut received = false;
        for _ in 0..1000 {
            a.tick()?;
            b.tick()?;
            for (_, dest, data) in a.drain()?.packets {
                if dest == b_public {
                    b.receive(a_public, b_host, &data)?;
                }
            }
            let output = b.drain()?;
            for event in output.events {
                if let RtcEvent::Control(bytes) = event {
                    received = bytes == b"synthetic CGNAT preview";
                }
            }
            for (_, dest, data) in output.packets {
                if dest == a_public {
                    a.receive(b_public, a_host, &data)?;
                }
            }
            if received {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            received,
            "Encrypted data must pass across the simulated NATs"
        );
        Ok(())
    }
    #[test]
    fn trickle_recovers_both_changed_nat_ports_without_replacing_dtls_or_sdp() -> Result<()> {
        let a_host = "192.168.10.2:21001".parse()?;
        let b_host = "192.168.20.2:21002".parse()?;
        let a_old = "203.0.113.10:31001".parse()?;
        let b_old = "203.0.113.20:31002".parse()?;
        let a_new = "203.0.113.10:41001".parse()?;
        let b_new = "203.0.113.20:41002".parse()?;
        let mut a = PeerRtc::new(&[host_candidate(a_host)?, reflexive_candidate(a_old, a_host)?])?;
        let mut b = PeerRtc::new(&[host_candidate(b_host)?, reflexive_candidate(b_old, b_host)?])?;
        let offer = a.offer()?;
        let answer = b.accept_offer(&offer)?;
        a.accept_answer(&answer)?;
        // Both ISP mappings have expired since discovery. Checks to their old ports fail.
        for _ in 0..30 {
            a.tick()?;
            b.tick()?;
            a.drain()?;
            b.drain()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!a.connected && !b.connected);
        let a_candidate = reflexive_candidate(a_new, a_host)?;
        let b_candidate = reflexive_candidate(b_new, b_host)?;
        a.add_local_candidate(&a_candidate);
        a.drain()?;
        b.add_local_candidate(&b_candidate);
        b.drain()?;
        // Exchange candidates only, using the existing session's ICE credentials.
        a.add_remote_candidate(&b_candidate);
        let mut a_out = a.drain()?;
        b.add_remote_candidate(&a_candidate);
        let mut b_out = b.drain()?;
        let until = Instant::now() + Duration::from_secs(6);
        let mut ready_a = false;
        let mut ready_b = false;
        while Instant::now() < until && !(ready_a && ready_b) {
            for event in a_out.events {
                if matches!(event, RtcEvent::ControlReady) {
                    ready_a = true;
                }
            }
            for event in b_out.events {
                if matches!(event, RtcEvent::ControlReady) {
                    ready_b = true;
                }
            }
            for (_, dest, data) in a_out.packets {
                if dest == b_new {
                    b.receive(a_new, b_host, &data)?;
                }
            }
            for (_, dest, data) in b_out.packets {
                if dest == a_new {
                    a.receive(b_new, a_host, &data)?;
                }
            }
            a.tick()?;
            b.tick()?;
            a_out = a.drain()?;
            b_out = b.drain()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            a.connected && b.connected && ready_a && ready_b,
            "Trickled ports must restore the original session"
        );
        Ok(())
    }
    #[test]
    fn encrypted_rtp_preserves_codec_and_random_access() -> Result<()> {
        let mut a = PeerRtc::new(&[host_candidate("127.0.0.1:21001".parse()?)?])?;
        let mut b = PeerRtc::new(&[host_candidate("127.0.0.1:21002".parse()?)?])?;
        let offer = a.offer()?;
        let answer = b.accept_offer(&offer)?;
        a.accept_answer(&answer)?;
        for _ in 0..2000 {
            a.tick()?;
            b.tick()?;
            exchange(&mut a, &mut b)?;
            if a.connected && b.connected {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        anyhow::ensure!(a.connected && b.connected, "Handshake did not finish");
        let mut encoder = p2pshare_media::audio::encoder(Default::default())?;
        let pcm = [0.2; p2pshare_media::audio::SAMPLES];
        let opus = encoder.encode_vec_float(&pcm, 1275)?;
        b.write_audio(opus.clone().into(), 1230000)?;
        let mut found_audio = false;
        for _ in 0..100 {
            a.tick()?;
            b.tick()?;
            let x = b.drain()?;
            for (source, dest, bytes) in x.packets {
                a.receive(source, dest, &bytes)?;
            }
            let x = a.drain()?;
            for event in x.events {
                if let RtcEvent::Audio { data, .. } = event {
                    assert_eq!(&*data, &opus);
                    found_audio = true;
                }
            }
            for (source, dest, bytes) in x.packets {
                b.receive(source, dest, &bytes)?;
            }
            if found_audio {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        anyhow::ensure!(found_audio, "Missing answerer stereo Opus RTP packet");
        for (format, data) in [
            (VideoFormat::H264, vec![0, 0, 0, 1, 0x65, 0x88, 0x80]),
            (VideoFormat::Hevc, vec![0, 0, 0, 1, 38, 1, 0x80, 0x80]),
        ] {
            a.write_codec(data.into(), 1000000, format)?;
            let mut found = false;
            for _ in 0..100 {
                a.tick()?;
                b.tick()?;
                for event in exchange(&mut a, &mut b)? {
                    if let RtcEvent::Frame {
                        codec,
                        keyframe,
                        contiguous,
                        data,
                        ..
                    } = event
                    {
                        assert_eq!(codec, format);
                        assert!(keyframe);
                        assert!(contiguous);
                        assert!(!data.is_empty());
                        found = true;
                    }
                }
                if found {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            anyhow::ensure!(found, "Missing {} RTP frame", format.label());
        }
        Ok(())
    }
}
