//! Windows/Android sessions: Cloudflare signaling and direct encrypted codec-aware video.
//! Legacy Iroh/Nostr actors remain available for transport regression fixtures.
mod cloudflare;
mod config;
mod network;
mod recovery;
mod stun;
use anyhow::{Context, Result, bail};
pub use cloudflare::{CloudflareInvite, SignalingConfig};
pub use config::AppConfig;
#[cfg(not(target_os = "android"))]
use p2pshare_control_iroh::{
    self as control, ControlConnection, ControlEndpoint, Invite, RecvStream, SendStream,
};
#[cfg(not(target_os = "android"))]
use p2pshare_discovery::{Discovery, Notice, Presence, RoomInvite};
use p2pshare_media::config::VideoFormat;
use p2pshare_rtc::{IceCandidate, PeerRtc, RtcEvent};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[cfg(not(target_os = "android"))]
use tokio::sync::watch;
use tokio::{net::UdpSocket, sync::mpsc};
pub const MAX_PEERS: usize = 6;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub sharing: bool,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub local_id: String,
    pub members: Vec<Member>,
    pub invite: Option<String>,
    pub saved_invite: Option<String>,
    pub in_room: bool,
    pub busy: bool,
    pub status: String,
    pub connected_video: Vec<String>,
    pub viewer_count: usize,
    pub discovery_status: String,
}
#[derive(Clone)]
pub struct VideoFrame {
    pub codec: VideoFormat,
    pub data: Arc<[u8]>,
    pub micros: u64,
    pub keyframe: bool,
}
pub struct RemoteFrame {
    pub codec: VideoFormat,
    pub received_at: Instant,
    pub micros: u64,
    pub peer: String,
    pub data: Arc<[u8]>,
    pub keyframe: bool,
}
#[derive(Clone)]
pub struct AudioPacket {
    pub data: Arc<[u8]>,
    pub micros: u64,
}
pub struct RemoteAudio {
    pub peer: String,
    pub data: Arc<[u8]>,
    pub micros: u64,
    pub received_at: Instant,
}
/// Sender's configured video bitrate and actual encoded format (None while Auto is idle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamInfo {
    pub codec: Option<VideoFormat>,
    pub bitrate_kbps: u32,
}
impl StreamInfo {
    fn valid(self) -> bool {
        (100..=50_000).contains(&self.bitrate_kbps)
    }
}
pub enum Command {
    SetSignalingServer(String),
    Create(String),
    NamedRoom { room: String, name: String },
    Join { invite: String, name: String },
    Leave,
    Rename(String),
    Publish { width: u32, height: u32 },
    Unpublish,
    RequestKeyframe(String),
    Watch { peer: String, enabled: bool },
    Preview(Arc<[u8]>),
    StreamInfo(StreamInfo),
    Avatar(Option<Vec<u8>>),
    Shutdown,
}
#[cfg(not(target_os = "android"))]
#[derive(Serialize, Deserialize, Clone)]
enum Wire {
    Join {
        token: String,
        name: String,
    },
    Welcome {
        members: Vec<Member>,
    },
    Members(Vec<Member>),
    Signal {
        from: String,
        to: String,
        answer: bool,
        sdp: String,
    },
    Share {
        width: u32,
        height: u32,
    },
    Name(String),
    Error(String),
    MeshJoin {
        room: String,
        capability: String,
        member: Member,
        address: control::PeerAddress,
    },
    MeshWelcome {
        room: String,
        member: Member,
        address: control::PeerAddress,
        contacts: Vec<Presence>,
    },
    Contacts(Vec<Presence>),
}
const RECEIVE_MAX_FRAMES: usize = 24;
const RECEIVE_MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Default)]
struct VideoInbox {
    queues: HashMap<String, VecDeque<RemoteFrame>>,
    audio: HashMap<String, VecDeque<RemoteAudio>>,
    waiting: HashMap<String, bool>,
    bytes: HashMap<String, usize>,
    gaps: u64,
    overflows: u64,
    received: u64,
    codecs: HashMap<String, VideoFormat>,
    playback_ms: u32,
    timelines: HashMap<String, p2pshare_media::playback::Timeline>,
    previews: HashMap<String, Arc<[u8]>>,
    stream_info: HashMap<String, StreamInfo>,
    own_stream_info: Option<StreamInfo>,
    stream_viewers: HashMap<String,Vec<String>>,
    avatar_hashes:HashMap<String,String>,
    avatar_cache:HashMap<String,Arc<[u8]>>,
    watching: std::collections::HashSet<String>,
}
impl VideoInbox {
    fn observe(&mut self, peer: &str, micros: u64, received: Instant) {
        if self.playback_ms == 0 {
            return;
        }
        let reset = self
            .timelines
            .entry(peer.into())
            .or_insert_with(|| p2pshare_media::playback::Timeline::new(micros, received))
            .observe(micros, received);
        if reset {
            self.queues.remove(peer);
            self.audio.remove(peer);
            self.bytes.remove(peer);
            self.waiting.insert(peer.into(), true);
            tracing::info!(peer, "playback_timeline_reset");
        }
    }
    fn push_audio(&mut self, packet: RemoteAudio) {
        self.observe(&packet.peer, packet.micros, packet.received_at);
        let queue = self.audio.entry(packet.peer.clone()).or_default();
        if queue.len() >= if self.playback_ms == 0 { 12 } else { 120 } {
            queue.pop_front();
            tracing::warn!("audio_receive_queue_overflow");
        }
        queue.push_back(packet);
    }
    #[cfg(test)]
    fn push(&mut self, peer: &str, data: Arc<[u8]>, keyframe: bool, contiguous: bool) -> bool {
        self.push_timed(
            RemoteFrame {
                peer: peer.into(),
                codec: VideoFormat::H264,
                data,
                keyframe,
                received_at: Instant::now(),
                micros: 0,
            },
            contiguous,
        )
    }
    fn push_timed(&mut self, frame: RemoteFrame, contiguous: bool) -> bool {
        let RemoteFrame {
            peer,
            codec,
            data,
            keyframe,
            received_at,
            micros,
        } = frame;
        self.observe(&peer, micros, received_at);
        let peer = peer.as_str();
        let changed = self
            .codecs
            .insert(peer.into(), codec)
            .is_some_and(|previous| previous != codec);
        let contiguous = contiguous && !changed;
        self.received += 1;
        if !contiguous {
            self.gaps += 1;
        }
        let queue = self.queues.entry(peer.into()).or_default();
        let bytes = self.bytes.entry(peer.into()).or_default();
        let full = queue.len()
            >= if self.playback_ms == 0 {
                RECEIVE_MAX_FRAMES
            } else {
                144
            }
            || *bytes + data.len() > RECEIVE_MAX_BYTES;
        if full {
            self.overflows += 1;
        }
        let waiting = self.waiting.entry(peer.into()).or_insert(true);
        let mut request = !contiguous;
        if (full || !contiguous) && p2pshare_media::diagnostics::enabled() {
            tracing::warn!(
                queue_frames = queue.len(),
                queue_bytes = *bytes,
                full,
                contiguous,
                keyframe,
                "receive_drop_and_resync"
            );
        }
        if full || !contiguous {
            queue.clear();
            *bytes = 0;
            *waiting = true;
            request = true;
        }
        if keyframe {
            *waiting = false;
        }
        if !*waiting {
            *bytes += data.len();
            queue.push_back(RemoteFrame {
                codec,
                received_at,
                micros,
                peer: peer.into(),
                data,
                keyframe,
            });
        }
        request || *waiting
    }
}
#[derive(Clone)]
struct PublishedFrame {
    frame: VideoFrame,
    sequence: u64,
}
pub struct Session {
    on_demand_video: bool,
    commands: mpsc::Sender<Command>,
    frames: mpsc::Sender<PublishedFrame>,
    audio: mpsc::Sender<AudioPacket>,
    sequence: AtomicU64,
    publish_drops: AtomicU64,
    media_epoch: Instant,
    audio_muted: Mutex<std::collections::HashSet<String>>,
    audio_volume: Mutex<std::collections::HashMap<String, u8>>,
    state: Arc<Mutex<Snapshot>>,
    inbox: Arc<Mutex<VideoInbox>>,
    keyframe: Arc<AtomicBool>,
    stopped: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl Session {
    pub fn start(identity: PathBuf) -> Result<Self> {
        Self::start_cloudflare(identity, String::new(), false)
    }
    /// Central signaling with direct encrypted video; isolated sockets are for tests.
    pub fn start_cloudflare(identity: PathBuf, server: String, isolated: bool) -> Result<Self> {
        if !server.is_empty() {
            cloudflare::validate_server(&server)?;
        }
        Self::start_worker(identity, isolated, None, Some(server))
    }
    /// Deterministic loopback integration tests without public discovery or STUN traffic.
    #[cfg(not(target_os = "android"))]
    pub fn start_isolated(identity: PathBuf) -> Result<Self> {
        Self::start_inner(identity, true, None)
    }
    /// Custom discovery providers; loopback mode is for real socket integration tests.
    #[cfg(not(target_os = "android"))]
    pub fn start_with_relays(
        identity: PathBuf,
        isolated: bool,
        relays: Vec<String>,
    ) -> Result<Self> {
        RoomInvite::new(relays.clone())?;
        Self::start_inner(identity, isolated, Some(relays))
    }
    #[cfg(not(target_os = "android"))]
    fn start_inner(identity: PathBuf, isolated: bool, relays: Option<Vec<String>>) -> Result<Self> {
        Self::start_worker(identity, isolated, relays, None)
    }
    fn start_worker(
        identity: PathBuf,
        isolated: bool,
        _relays: Option<Vec<String>>,
        server: Option<String>,
    ) -> Result<Self> {
        let on_demand_video = server.is_some();
        let (commands, rx) = mpsc::channel(64);
        let (frames, video) = mpsc::channel(8);
        let (audio, sound) = mpsc::channel(8);
        let state = Arc::new(Mutex::new(Snapshot {
            busy: true,
            status: "Preparing encrypted connections…".into(),
            ..Default::default()
        }));
        let inbox = Arc::new(Mutex::new(VideoInbox::default()));
        let keyframe = Arc::new(AtomicBool::new(true));
        let s = state.clone();
        let i = inbox.clone();
        let k = keyframe.clone();
        let (finished, stopped) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("p2pshare-network".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(async {
                        if let Some(server) = server {
                            cloudflare::CloudflareActor::new(
                                identity,
                                server,
                                isolated,
                                rx,
                                video,
                                sound,
                                s.clone(),
                                i,
                                k,
                            )
                            .await?
                            .run()
                            .await
                        } else {
                            #[cfg(target_os = "android")]
                            {
                                bail!("Android requires named-room signaling");
                            }
                            #[cfg(not(target_os = "android"))]
                            {
                                Actor::new(
                                    identity,
                                    isolated,
                                    _relays,
                                    rx,
                                    video,
                                    sound,
                                    s.clone(),
                                    i,
                                    k,
                                )
                                .await?
                                .run()
                                .await
                            }
                        }
                    })
                })();
                if let Err(error) = result {
                    tracing::error!(%error, "Room worker stopped");
                    let mut state = s.lock().unwrap();
                    state.busy = false;
                    state.in_room = false;
                    state.status = format!("Connection error: {error:#}");
                }
                let _ = finished.send(());
            })?;
        Ok(Self {
            on_demand_video,
            commands,
            frames,
            audio,
            sequence: AtomicU64::new(0),
            publish_drops: AtomicU64::new(0),
            media_epoch: Instant::now(),
            audio_muted: Mutex::new(Default::default()),
            audio_volume: Mutex::new(Default::default()),
            state,
            inbox,
            keyframe,
            stopped: Mutex::new(stopped),
        })
    }
    pub fn command(&self, command: Command) {
        if matches!(
            &command,
            Command::Create(_) | Command::Join { .. } | Command::NamedRoom { .. }
        ) {
            self.state.lock().unwrap().busy = true;
        }
        if self.commands.try_send(command).is_err() {
            self.state.lock().unwrap().status = "Connection worker is busy; try again".into();
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().unwrap().clone();
        self.audio_muted
            .lock()
            .unwrap()
            .retain(|peer| state.members.iter().any(|m| &m.id == peer));
        self.audio_volume
            .lock()
            .unwrap()
            .retain(|peer, _| state.members.iter().any(|m| &m.id == peer));
        state
    }
    pub fn set_watching(&self, peer: &str, enabled: bool) {
        let mut inbox = self.inbox.lock().unwrap();
        let changed = if enabled {
            inbox.watching.insert(peer.into())
        } else {
            inbox.watching.remove(peer)
        };
        if !changed {
            return;
        }
        inbox.queues.remove(peer);
        inbox.audio.remove(peer);
        inbox.bytes.remove(peer);
        inbox.timelines.remove(peer);
        inbox.waiting.insert(peer.into(), true);
        drop(inbox);
        self.command(Command::Watch {
            peer: peer.into(),
            enabled,
        });
    }
    pub fn needs_live_video(&self) -> bool {
        !self.on_demand_video || self.state.lock().unwrap().viewer_count > 0
    }
    pub fn is_watching(&self, peer: &str) -> bool {
        self.inbox.lock().unwrap().watching.contains(peer)
    }
    pub fn publish_preview(&self, jpeg: Vec<u8>) {
        if jpeg.len() <= p2pshare_media::preview::MAX_BYTES {
            self.command(Command::Preview(jpeg.into()));
        }
    }
    pub fn take_previews(&self) -> Vec<(String, Arc<[u8]>)> {
        self.inbox.lock().unwrap().previews.drain().collect()
    }
    pub fn video_codec(&self, peer: &str) -> VideoFormat {
        self.inbox
            .lock()
            .unwrap()
            .codecs
            .get(peer)
            .copied()
            .unwrap_or_default()
    }
    pub fn set_stream_info(&self, info: StreamInfo) {
        if info.valid() {
            self.inbox.lock().unwrap().own_stream_info = Some(info);
            self.command(Command::StreamInfo(info));
        }
    }
    pub fn stream_info(&self, peer: &str) -> Option<StreamInfo> {
        let own = self.state.lock().unwrap().local_id == peer;
        let inbox = self.inbox.lock().unwrap();
        if own {
            inbox.own_stream_info
        } else {
            inbox.stream_info.get(peer).copied()
        }
    }
    /// None means the sharer has not supplied viewer presence (older client).
    pub fn avatar(&self,peer:&str)->Option<Arc<[u8]>>{let inbox=self.inbox.lock().unwrap();inbox.avatar_hashes.get(peer).and_then(|h|inbox.avatar_cache.get(h)).cloned()}
    pub fn stream_viewers(&self,source:&str)->Option<Vec<String>>{
        let state=self.snapshot();let inbox=self.inbox.lock().unwrap();
        inbox.stream_viewers.get(source).map(|ids|ids.iter().filter(|id|state.members.iter().any(|m|&m.id==*id)).cloned().collect())
    }
    pub fn publish(&self, frame: VideoFrame) {
        // Follow the actual encoder, including Auto and hardware/software fallback.
        let changed = {
            let mut inbox = self.inbox.lock().unwrap();
            inbox.own_stream_info.as_mut().and_then(|info| {
                if info.codec == Some(frame.codec) {
                    None
                } else {
                    info.codec = Some(frame.codec);
                    Some(*info)
                }
            })
        };
        if let Some(info) = changed {
            self.command(Command::StreamInfo(info));
        }
        if frame.data.len() <= 2 * 1024 * 1024 {
            let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
            if self
                .frames
                .try_send(PublishedFrame { frame, sequence })
                .is_err()
            {
                tracing::warn!("publisher_queue_frame_dropped");
                self.publish_drops.fetch_add(1, Ordering::Relaxed);
                self.keyframe.store(true, Ordering::Relaxed);
            }
        }
    }
    pub fn media_micros(&self) -> u64 {
        self.media_epoch.elapsed().as_micros() as u64
    }
    pub fn set_audio_muted(&self, peer: &str, muted: bool) {
        let mut prefs = self.audio_muted.lock().unwrap();
        if muted {
            prefs.insert(peer.into());
        } else {
            prefs.remove(peer);
        }
    }
    pub fn audio_muted(&self, peer: &str) -> bool {
        self.audio_muted.lock().unwrap().contains(peer)
    }
    /// Local playback level only; never changes the outgoing stream.
    pub fn set_audio_volume(&self, peer: &str, percent: u8) {
        let mut levels = self.audio_volume.lock().unwrap();
        let percent = percent.min(100);
        if percent == 100 {
            levels.remove(peer);
        } else {
            levels.insert(peer.into(), percent);
        }
    }
    pub fn audio_volume(&self, peer: &str) -> u8 {
        self.audio_volume
            .lock()
            .unwrap()
            .get(peer)
            .copied()
            .unwrap_or(100)
    }
    pub fn publish_audio(&self, packet: AudioPacket) {
        if !packet.data.is_empty()
            && packet.data.len() <= 1275
            && self.audio.try_send(packet).is_err()
        {
            tracing::warn!("audio_publish_queue_overflow");
        }
    }
    pub fn set_playback_config(
        &self,
        config: p2pshare_media::playback::PlaybackConfig,
    ) -> Result<()> {
        config.validate()?;
        self.inbox.lock().unwrap().playback_ms = config.buffer_ms;
        tracing::info!(buffer_ms = config.buffer_ms, "playback_configuration");
        Ok(())
    }
    pub fn playback_delay_ms(&self) -> u32 {
        self.inbox.lock().unwrap().playback_ms
    }
    /// Background viewers discard the entire reservoir, then resume from a fresh keyframe.
    pub fn discard_playback(&self) {
        let mut inbox = self.inbox.lock().unwrap();
        inbox.queues.clear();
        inbox.audio.clear();
        inbox.bytes.clear();
        inbox.timelines.clear();
        for wait in inbox.waiting.values_mut() {
            *wait = true;
        }
    }
    pub fn take_audio(&self) -> Vec<RemoteAudio> {
        let mut inbox = self.inbox.lock().unwrap();
        let now = Instant::now();
        let delay = inbox.playback_ms;
        let mut frames = Vec::new();
        let VideoInbox {
            audio, timelines, ..
        } = &mut *inbox;
        for (peer, queue) in audio {
            while queue.front().is_some_and(|f| {
                delay == 0
                    || timelines
                        .get(peer)
                        .is_none_or(|t| t.deadline(f.micros, delay) <= now)
            }) {
                frames.push(queue.pop_front().unwrap());
            }
        }
        frames
    }
    pub fn take_video(&self) -> Vec<RemoteFrame> {
        let mut inbox = self.inbox.lock().unwrap();
        let now = Instant::now();
        let delay = inbox.playback_ms;
        let mut frames = Vec::new();
        let VideoInbox {
            queues,
            timelines,
            bytes,
            ..
        } = &mut *inbox;
        for (peer, queue) in queues {
            while queue.front().is_some_and(|f| {
                delay == 0
                    || timelines
                        .get(peer)
                        .is_none_or(|t| t.deadline(f.micros, delay) <= now)
            }) {
                let frame = queue.pop_front().unwrap();
                if let Some(n) = bytes.get_mut(peer) {
                    *n = n.saturating_sub(frame.data.len());
                }
                frames.push(frame);
            }
        }
        frames
    }
    pub fn video_diagnostics(&self) -> String {
        let inbox = self.inbox.lock().unwrap();
        format!(
            "Video frames {} • network gaps {} • queue overflows {} • waiting {} • publish drops {} • playback buffer {} ms / {} packets / {} bytes",
            inbox.received,
            inbox.gaps,
            inbox.overflows,
            inbox.waiting.values().filter(|waiting| **waiting).count(),
            self.publish_drops.load(Ordering::Relaxed),
            inbox.playback_ms,
            inbox.queues.values().map(|q| q.len()).sum::<usize>(),
            inbox.bytes.values().sum::<usize>()
        )
    }
    pub fn needs_keyframe(&self) -> bool {
        self.keyframe.swap(false, Ordering::Relaxed)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let started = Instant::now();
        let (closed_sender, _) = mpsc::channel(1);
        let commands = std::mem::replace(&mut self.commands, closed_sender);
        let _ = commands.try_send(Command::Shutdown);
        drop(commands); // Also wakes the actor on channel closure if the queue was full.
        // The process must not exit before the actor sends QUIC close packets.
        // Bound the wait so a congested/starting network worker cannot hang Quit.
        match self
            .stopped
            .get_mut()
            .unwrap()
            .recv_timeout(Duration::from_millis(1500))
        {
            Ok(()) => tracing::info!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                "session_shutdown_complete"
            ),
            Err(error) => {
                tracing::warn!(%error,elapsed_ms=started.elapsed().as_millis() as u64,"session_shutdown_wait_ended")
            }
        }
    }
}
#[cfg(not(target_os = "android"))]
struct Link {
    ticket: u64,
    connection: ControlConnection,
    sender: mpsc::Sender<Wire>,
}
enum Internal {
    #[cfg(not(target_os = "android"))]
    Incoming {
        connection: ControlConnection,
        send: SendStream,
        recv: RecvStream,
        token: String,
        name: String,
    },
    #[cfg(not(target_os = "android"))]
    Joined {
        generation: u64,
        connection: ControlConnection,
        send: SendStream,
        recv: RecvStream,
        members: Vec<Member>,
    },
    #[cfg(not(target_os = "android"))]
    MeshIncoming {
        generation: u64,
        connection: ControlConnection,
        send: SendStream,
        recv: RecvStream,
        room: String,
        capability: String,
        member: Member,
        address: control::PeerAddress,
    },
    #[cfg(not(target_os = "android"))]
    MeshJoined {
        generation: u64,
        connection: ControlConnection,
        send: SendStream,
        recv: RecvStream,
        member: Member,
        address: control::PeerAddress,
        contacts: Vec<Presence>,
    },
    #[cfg(not(target_os = "android"))]
    Discovery(u64, Notice),
    #[cfg(not(target_os = "android"))]
    MeshFailed(u64, String, control::PeerAddress, String),
    #[cfg(not(target_os = "android"))]
    Message(String, u64, Wire),
    #[cfg(not(target_os = "android"))]
    Closed(String, u64, bool),
    #[cfg(not(target_os = "android"))]
    Error(u64, String),
    Datagram {
        source: SocketAddr,
        destination: SocketAddr,
        data: Vec<u8>,
    },
}
#[cfg(not(target_os = "android"))]
struct Actor {
    endpoint: ControlEndpoint,
    local: String,
    commands: mpsc::Receiver<Command>,
    video: mpsc::Receiver<PublishedFrame>,
    audio: mpsc::Receiver<AudioPacket>,
    state: Arc<Mutex<Snapshot>>,
    inbox: Arc<Mutex<VideoInbox>>,
    keyframe: Arc<AtomicBool>,
    internal: mpsc::Receiver<Internal>,
    events: mpsc::Sender<Internal>,
    links: HashMap<String, Link>,
    peers: HashMap<String, PeerRtc>,
    sockets: stun::Sockets,
    candidates: Vec<IceCandidate>,
    members: Vec<Member>,
    host: bool,
    coordinator: Option<String>,
    invite: Option<Invite>,
    generation: u64,
    in_room: bool,
    name: String,
    requests: HashMap<String, Instant>,
    last_video_sequence: u64,
    waiting_video: bool,
    mesh: Option<RoomInvite>,
    discovery: Option<Discovery>,
    discovery_bridge: Option<tokio::task::JoinHandle<()>>,
    address_updates: Option<watch::Sender<control::PeerAddress>>,
    relay_health: Vec<bool>,
    default_relays: Option<Vec<String>>,
    device_seed: [u8; 32],
    room_file: PathBuf,
    accept_epoch: Arc<AtomicU64>,
    contacts: HashMap<String, Presence>,
    dialing: HashMap<String, tokio::task::JoinHandle<()>>,
    retry_after: HashMap<String, Instant>,
    retry_failures: HashMap<String, u32>,
    mesh_tick: Instant,
    gossip_tick: Instant,
    admission_blocked: Option<Instant>,
    rejections: Arc<tokio::sync::Semaphore>,
}
#[cfg(not(target_os = "android"))]
impl Actor {
    #[allow(clippy::too_many_arguments)]
    async fn new(
        identity: PathBuf,
        isolated: bool,
        default_relays: Option<Vec<String>>,
        commands: mpsc::Receiver<Command>,
        video: mpsc::Receiver<PublishedFrame>,
        audio: mpsc::Receiver<AudioPacket>,
        state: Arc<Mutex<Snapshot>>,
        inbox: Arc<Mutex<VideoInbox>>,
        keyframe: Arc<AtomicBool>,
    ) -> Result<Self> {
        let (events, internal) = mpsc::channel(1024);
        let endpoint = control::bind(&identity, isolated).await?;
        let local = endpoint.id().to_string();
        let device_seed: [u8; 32] = std::fs::read(&identity)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid device key"))?;
        let room_file = identity.with_extension("room.json");
        let accept_epoch = Arc::new(AtomicU64::new(0));
        let epoch = accept_epoch.clone();
        let ep = endpoint.clone();
        let tx = events.clone();
        tokio::spawn(async move {
            let handshakes = Arc::new(tokio::sync::Semaphore::new(16));
            while let Some(incoming) = ep.accept().await {
                let Ok(permit) = handshakes.clone().try_acquire_owned() else {
                    incoming.refuse();
                    continue;
                };
                let tx = tx.clone();
                let generation = epoch.load(Ordering::Relaxed);
                tokio::spawn(async move {
                    let _permit = permit;
                    let result = tokio::time::timeout(Duration::from_secs(10), async {
                        let connection = incoming.await?;
                        let (send, mut recv) = connection.accept_bi().await?;
                        match control::receive(&mut recv).await? {
                            Wire::Join { token, name } => {
                                let _ = tx
                                    .send(Internal::Incoming {
                                        connection,
                                        send,
                                        recv,
                                        token,
                                        name,
                                    })
                                    .await;
                            }
                            Wire::MeshJoin {
                                room,
                                capability,
                                member,
                                address,
                            } => {
                                let _ = tx
                                    .send(Internal::MeshIncoming {
                                        generation,
                                        connection,
                                        send,
                                        recv,
                                        room,
                                        capability,
                                        member,
                                        address,
                                    })
                                    .await;
                            }
                            _ => bail!("Expected room handshake"),
                        }
                        Ok::<_, anyhow::Error>(())
                    })
                    .await;
                    if let Ok(Err(e)) = result {
                        tracing::debug!(%e, "Rejected control handshake");
                    }
                });
            }
        });
        let (sockets, candidates) = media_sockets(events.clone(), isolated).await?;
        {
            let mut s = state.lock().unwrap();
            s.saved_invite = std::fs::read(&room_file)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<RoomInvite>(&bytes).ok())
                .and_then(|invite| invite.encode().ok());
            s.local_id = local.clone();
            s.busy = false;
            s.status = "Create a room or paste an invitation to join".into();
        }
        Ok(Self {
            endpoint,
            local,
            commands,
            video,
            audio,
            state,
            inbox,
            keyframe,
            internal,
            events,
            links: HashMap::new(),
            peers: HashMap::new(),
            sockets,
            candidates,
            members: vec![],
            host: false,
            coordinator: None,
            invite: None,
            generation: 0,
            in_room: false,
            name: "You".into(),
            requests: HashMap::new(),
            last_video_sequence: 0,
            waiting_video: true,
            mesh: None,
            discovery: None,
            discovery_bridge: None,
            address_updates: None,
            relay_health: vec![],
            default_relays,
            device_seed,
            room_file,
            accept_epoch,
            contacts: HashMap::new(),
            dialing: HashMap::new(),
            retry_after: HashMap::new(),
            retry_failures: HashMap::new(),
            mesh_tick: Instant::now(),
            gossip_tick: Instant::now(),
            admission_blocked: None,
            rejections: Arc::new(tokio::sync::Semaphore::new(16)),
        })
    }
    async fn run(mut self) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let result = tokio::select! {
                c = self.commands.recv() => match c { Some(Command::Shutdown) | None => break, Some(c) => self.command(c).await },
                e = self.internal.recv() => match e { Some(e) => self.internal(e).await, None => break },
                sound = self.audio.recv() => match sound { Some(sound) => self.sound(sound).await, None => break },
                frame = self.video.recv() => match frame { Some(frame) => self.frame(frame).await, None => break },
                _ = tick.tick() => self.tick().await,
            };
            if let Err(error) = result {
                tracing::warn!(%error, "Room operation failed");
                self.status(format!("{error:#}"));
            }
        }
        self.leave();
        self.endpoint.close().await;
        Ok(())
    }
    fn status(&self, text: String) {
        self.state.lock().unwrap().status = text;
    }
    fn local_member(&self) -> Member {
        Member {
            id: self.local.clone(),
            name: self.name.clone(),
            sharing: false,
            width: 0,
            height: 0,
        }
    }
    fn leave(&mut self) {
        while self.video.try_recv().is_ok() {}
        while self.audio.try_recv().is_ok() {}
        self.waiting_video = true;
        self.generation += 1;
        self.accept_epoch.store(self.generation, Ordering::Relaxed);
        self.discovery.take();
        if let Some(bridge) = self.discovery_bridge.take() {
            bridge.abort();
        }
        self.address_updates = None;
        for (_, task) in self.dialing.drain() {
            task.abort();
        }
        self.contacts.clear();
        self.retry_after.clear();
        self.retry_failures.clear();
        self.relay_health.clear();
        self.mesh = None;
        self.admission_blocked = None;
        for (_, link) in self.links.drain() {
            link.connection.close(0u32.into(), b"Left room");
        }
        self.peers.clear();
        self.members.clear();
        self.requests.clear();
        self.in_room = false;
        self.host = false;
        self.invite = None;
        self.coordinator = None;
        let mut inbox = self.inbox.lock().unwrap();
        inbox.queues.clear();
        inbox.audio.clear();
        inbox.waiting.clear();
        inbox.codecs.clear();
        inbox.bytes.clear();
        inbox.timelines.clear();
        drop(inbox);
        let mut state = self.state.lock().unwrap();
        state.in_room = false;
        state.busy = false;
        state.invite = None;
        state.members.clear();
        state.connected_video.clear();
        state.status = "Create a room or paste an invitation to join".into();
    }
    async fn command(&mut self, c: Command) -> Result<()> {
        match c {
            Command::Create(name) => {
                if let Some(relays) = self.default_relays.clone() {
                    return self.open_mesh(RoomInvite::new(relays)?, name).await;
                }
                self.leave();
                self.name = clean_name(&name);
                self.host = true;
                self.in_room = true;
                self.members.push(self.local_member());
                let invite = Invite::new(&self.endpoint);
                {
                    let mut s = self.state.lock().unwrap();
                    s.invite = Some(invite.encode()?);
                    s.status = "Room open • copy the invitation for your other devices".into();
                }
                self.invite = Some(invite);
                self.roster().await?;
            }
            Command::Join { invite, name } => {
                if invite.trim().starts_with("p2pshare://room/") {
                    return self.open_mesh(RoomInvite::parse(&invite)?, name).await;
                }
                let invite = Invite::parse(&invite)?;
                if invite.coordinator.id.to_string() == self.local {
                    bail!("This invitation belongs to this device; share it with another device");
                }
                self.leave();
                self.name = clean_name(&name);
                self.coordinator = Some(invite.coordinator.id.to_string());
                self.state.lock().unwrap().busy = true;
                self.status("Joining room…".into());
                let ep = self.endpoint.clone();
                let tx = self.events.clone();
                let generation = self.generation;
                let name = self.name.clone();
                tokio::spawn(async move {
                    let result = async {
                        let connection = control::connect(&ep, &invite).await?;
                        let (mut send, mut recv) = connection.open_bi().await?;
                        control::send(
                            &mut send,
                            &Wire::Join {
                                token: invite.token,
                                name,
                            },
                        )
                        .await?;
                        let welcome: Wire = tokio::time::timeout(
                            Duration::from_secs(15),
                            control::receive(&mut recv),
                        )
                        .await??;
                        match welcome {
                            Wire::Welcome { members } => {
                                let _ = tx
                                    .send(Internal::Joined {
                                        generation,
                                        connection,
                                        send,
                                        recv,
                                        members,
                                    })
                                    .await;
                                Ok(())
                            }
                            Wire::Error(e) => bail!("{e}"),
                            _ => bail!("Invalid room handshake"),
                        }
                    }
                    .await;
                    if let Err(e) = result {
                        let _ = tx
                            .send(Internal::Error(
                                generation,
                                format!("Could not join: {e:#}"),
                            ))
                            .await;
                    }
                });
            }
            Command::Leave => self.leave(),
            Command::Rename(name) => {
                self.name = clean_name(&name);
                self.local_message(Wire::Name(self.name.clone())).await?;
            }
            Command::Publish { width, height } => {
                if !self.in_room {
                    bail!("Create or join a room before sharing");
                }
                self.keyframe.store(true, Ordering::Relaxed);
                self.local_message(Wire::Share { width, height }).await?;
            }
            Command::Unpublish => {
                self.local_message(Wire::Share {
                    width: 0,
                    height: 0,
                })
                .await?;
            }
            Command::RequestKeyframe(peer) => {
                if let Some(rtc) = self.peers.get_mut(&peer) {
                    rtc.request_keyframe()?;
                    self.drain(&peer).await?;
                }
            }
            Command::Watch { .. } | Command::Preview(_) | Command::StreamInfo(_) | Command::Avatar(_) => {}
            Command::NamedRoom { .. } => bail!("Named rooms require Cloudflare signaling"),
            Command::SetSignalingServer(_) | Command::Shutdown => {}
        }
        Ok(())
    }
    async fn open_mesh(&mut self, invite: RoomInvite, name: String) -> Result<()> {
        // Persist the exact capability so the same room can be reopened after every device exits.
        let encoded = invite.encode()?;
        std::fs::write(&self.room_file, serde_json::to_vec(&invite)?)?;
        self.leave();
        self.name = clean_name(&name);
        self.relay_health = vec![false; invite.relays.len()];
        let (notices, mut rx) = mpsc::channel(64);
        let (updates, address) = watch::channel(self.endpoint.addr());
        self.discovery = Some(Discovery::start(
            invite.clone(),
            self.device_seed,
            address,
            notices,
            &self.room_file.with_extension("presence-clock"),
        )?);
        self.address_updates = Some(updates);
        self.in_room = true;
        self.members.push(self.local_member());
        self.mesh = Some(invite);
        let generation = self.generation;
        let events = self.events.clone();
        self.discovery_bridge = Some(tokio::spawn(async move {
            while let Some(notice) = rx.recv().await {
                if events
                    .send(Internal::Discovery(generation, notice))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }));
        {
            let mut state = self.state.lock().unwrap();
            state.saved_invite = Some(encoded.clone());
            state.invite = Some(encoded);
            state.busy = false;
            state.discovery_status = "Connecting to discovery relays…".into();
        }
        self.status("Room ready • reusable invitation works without its creator".into());
        self.roster().await
    }
    fn current_member(&self) -> Member {
        self.members
            .iter()
            .find(|m| m.id == self.local)
            .cloned()
            .unwrap_or_else(|| self.local_member())
    }
    fn broadcast(&self, wire: Wire) {
        for link in self.links.values() {
            if link.sender.try_send(wire.clone()).is_err() {
                link.connection.close(0u32.into(), b"Control congested");
            }
        }
    }
    fn remember(&mut self, presence: Presence) {
        let id = presence.address.id.to_string();
        if id == self.local || presence.validate().is_err() {
            return;
        }
        if self.contacts.len() >= 64 && !self.contacts.contains_key(&id) {
            return;
        }
        let changed = self
            .contacts
            .get(&id)
            .is_some_and(|previous| previous.address != presence.address);
        if changed && !self.links.contains_key(&id) {
            self.retry_after.remove(&id);
            self.retry_failures.remove(&id);
            if let Some(task) = self.dialing.remove(&id) {
                task.abort();
            }
            tracing::info!(peer=%id,"peer_endpoint_changed_retry_now");
        }
        self.contacts.insert(id, presence);
    }
    fn live_contacts(&self) -> Vec<Presence> {
        let mut contacts = vec![Presence::new(self.endpoint.addr())];
        for peer in self.links.keys() {
            if let Some(presence) = self.contacts.get(peer) {
                contacts.push(Presence::new(presence.address.clone()));
            }
        }
        contacts
    }
    fn dial_mesh(&mut self) {
        let Some(invite) = &self.mesh else {
            return;
        };
        if self
            .admission_blocked
            .is_some_and(|until| until > Instant::now())
        {
            return;
        }
        let mut candidates: Vec<_> = self
            .contacts
            .iter()
            .filter(|(id, p)| {
                self.local < **id
                    && p.expires > p2pshare_discovery::now()
                    && !self.links.contains_key(*id)
                    && !self.dialing.contains_key(*id)
                    && self
                        .retry_after
                        .get(*id)
                        .is_none_or(|t| *t <= Instant::now())
            })
            .map(|(id, p)| (id.clone(), p.address.clone()))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        // One deterministic dialer per pair, at most five concurrent attempts.
        for (peer, address) in candidates
            .into_iter()
            .take(MAX_PEERS - 1 - self.dialing.len().min(MAX_PEERS - 1))
        {
            if self.members.len() >= MAX_PEERS && self.members.iter().all(|m| m.id < peer) {
                continue;
            }
            let ep = self.endpoint.clone();
            let tx = self.events.clone();
            let generation = self.generation;
            let invite = invite.clone();
            let member = self.current_member();
            let failed_peer = peer.clone();
            let attempt_address = address.clone();
            let attempt_started = Instant::now();
            let task = tokio::spawn(async move {
                let result = tokio::time::timeout(Duration::from_secs(20), async {
                    let connection = control::connect_peer(&ep, address).await?;
                    let (mut send, mut recv) = connection.open_bi().await?;
                    let room = invite.topic()?;
                    control::send(
                        &mut send,
                        &Wire::MeshJoin {
                            room: room.clone(),
                            capability: invite.secret,
                            member,
                            address: ep.addr(),
                        },
                    )
                    .await?;
                    match control::receive(&mut recv).await? {
                        Wire::MeshWelcome {
                            room: got,
                            member,
                            address,
                            contacts,
                        } if got == room && contacts.len() <= MAX_PEERS => {
                            tracing::info!(peer=%failed_peer,elapsed_ms=attempt_started.elapsed().as_millis() as u64,"peer_handshake_complete");
                            tx.send(Internal::MeshJoined {
                                generation,
                                connection,
                                send,
                                recv,
                                member,
                                address,
                                contacts,
                            })
                            .await
                            .context("Room closed")?;
                            Ok(())
                        }
                        Wire::Error(error) => bail!("{error}"),
                        _ => bail!("Invalid reusable-room handshake"),
                    }
                })
                .await;
                let error = match result {
                    Ok(Ok(())) => return,
                    Ok(Err(e)) => format!("{e:#}"),
                    Err(_) => "Peer handshake timed out".into(),
                };
                let _ = tx
                    .send(Internal::MeshFailed(
                        generation,
                        failed_peer,
                        attempt_address,
                        error,
                    ))
                    .await;
            });
            self.dialing.insert(peer, task);
        }
    }
    /// Lowest endpoint IDs win concurrent admission, identically on every device.
    /// No special privileges for the creator; every roster is still bounded to six.
    async fn admit_mesh(&mut self, peer: &str) -> Result<bool> {
        if self
            .admission_blocked
            .is_some_and(|until| until > Instant::now())
        {
            return Ok(false);
        }
        if self.members.len() < MAX_PEERS {
            return Ok(true);
        }
        let highest = self
            .members
            .iter()
            .map(|m| m.id.as_str())
            .max()
            .unwrap_or(&self.local)
            .to_string();
        if peer >= highest.as_str() {
            return Ok(false);
        }
        if highest == self.local {
            self.block_mesh().await?;
            return Ok(false);
        }
        if let Some(link) = self.links.remove(&highest) {
            link.connection
                .close(control::ROOM_FULL_CODE.into(), b"Room admission limit");
        }
        self.members.retain(|m| m.id != highest);
        self.retry_after
            .insert(highest, Instant::now() + Duration::from_secs(30));
        self.roster().await?;
        Ok(true)
    }
    async fn block_mesh(&mut self) -> Result<()> {
        for (_, link) in self.links.drain() {
            link.connection.close(0u32.into(), b"Yielded room place");
        }
        for (_, task) in self.dialing.drain() {
            task.abort();
        }
        self.members = vec![self.local_member()];
        self.admission_blocked = Some(Instant::now() + Duration::from_secs(30));
        self.in_room = false;
        self.roster().await?;
        self.status("Room is full (6 devices) • waiting for a place".into());
        Ok(())
    }
    async fn mesh_connected(&mut self) -> Result<()> {
        self.in_room = true;
        self.admission_blocked = None;
        self.status("Connected • creator can leave; screens stay peer-to-peer".into());
        self.roster().await?;
        self.broadcast(Wire::Contacts(self.live_contacts()));
        self.dial_mesh();
        Ok(())
    }
    fn reject(
        &self,
        connection: ControlConnection,
        mut send: SendStream,
        recv: RecvStream,
        error: &str,
    ) {
        let Ok(permit) = self.rejections.clone().try_acquire_owned() else {
            connection.close(0u32.into(), b"Too many handshakes");
            return;
        };
        let error = error.to_string();
        tokio::spawn(async move {
            let _permit = permit;
            let _recv = recv;
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                control::send(&mut send, &Wire::Error(error)),
            )
            .await;
            let _ = send.finish();
            // Keep the connection alive until rejection bytes are acknowledged.
            let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
            connection.close(0u32.into(), b"Handshake rejected");
        });
    }
    fn add_link(
        &mut self,
        connection: ControlConnection,
        mut send: SendStream,
        mut recv: RecvStream,
    ) {
        let id = control::peer_id(&connection);
        self.retry_after.remove(&id);
        self.retry_failures.remove(&id);
        tracing::info!(peer=%id,"peer_control_connected");
        let ticket = rand::random::<u64>();
        let (sender, mut messages) = mpsc::channel(64);
        let tx = self.events.clone();
        let peer = id.clone();
        let conn = connection.clone();
        tokio::spawn(async move {
            let reader = async {
                loop {
                    let message = control::receive(&mut recv).await?;
                    tx.send(Internal::Message(peer.clone(), ticket, message))
                        .await
                        .context("Room closed")?;
                }
                #[allow(unreachable_code)]
                Ok::<(), anyhow::Error>(())
            };
            let writer = async {
                while let Some(message) = messages.recv().await {
                    control::send(&mut send, &message).await?;
                }
                Ok::<(), anyhow::Error>(())
            };
            tokio::select! { r = reader => { if let Err(e) = r { tracing::debug!(%e, "Control reader closed"); } }, r = writer => { if let Err(e) = r { tracing::debug!(%e, "Control writer closed"); } }, _ = conn.closed() => {} }
            let capacity = control::room_full_close(&conn);
            conn.close(0u32.into(), b"Control closed");
            let _ = tx.send(Internal::Closed(peer, ticket, capacity)).await;
        });
        self.links.insert(
            id,
            Link {
                ticket,
                connection,
                sender,
            },
        );
    }
    fn send(&self, target: &str, wire: Wire) -> Result<()> {
        self.links
            .get(target)
            .context("Peer left the room")?
            .sender
            .try_send(wire)
            .context("Peer control connection is congested")?;
        Ok(())
    }
    async fn local_message(&mut self, wire: Wire) -> Result<()> {
        if !self.in_room {
            return Ok(());
        }
        if self.mesh.is_some() {
            self.message(self.local.clone(), wire.clone()).await?;
            self.broadcast(wire);
            Ok(())
        } else if self.host {
            self.message(self.local.clone(), wire).await
        } else {
            self.send(self.coordinator.as_deref().context("No coordinator")?, wire)
        }
    }
    async fn internal(&mut self, e: Internal) -> Result<()> {
        match e {
            Internal::Discovery(generation, notice)
                if generation == self.generation && self.mesh.is_some() =>
            {
                match notice {
                    Notice::Peer(presence) => self.remember(presence),
                    Notice::Relay { index, available } => {
                        if let Some(health) = self.relay_health.get_mut(index) {
                            *health = available;
                        }
                        let online = self.relay_health.iter().filter(|v| **v).count();
                        self.state.lock().unwrap().discovery_status = if online == 0 {
                            "Discovery reconnecting • established peers continue".into()
                        } else {
                            format!("Discovery: {online}/{} relays", self.relay_health.len())
                        };
                    }
                }
                self.dial_mesh();
            }
            Internal::Discovery(_, _) => {}
            Internal::MeshIncoming {
                generation,
                connection,
                mut send,
                recv,
                room,
                capability,
                member,
                address,
            } => {
                let peer = control::peer_id(&connection);
                let valid = generation == self.generation
                    && self.mesh.as_ref().is_some_and(|i| {
                        i.secret == capability && i.topic().ok().as_deref() == Some(&room)
                    });
                if !valid
                    || !valid_member(&member, &peer)
                    || address.id.to_string() != peer
                    || self.links.contains_key(&peer)
                {
                    self.reject(
                        connection,
                        send,
                        recv,
                        "Room capability is invalid or device already connected",
                    );
                    return Ok(());
                }
                if !self.admit_mesh(&peer).await? {
                    self.reject(connection, send, recv, "Room is full (6 devices)");
                    return Ok(());
                }
                control::send(
                    &mut send,
                    &Wire::MeshWelcome {
                        room,
                        member: self.current_member(),
                        address: self.endpoint.addr(),
                        contacts: self.live_contacts(),
                    },
                )
                .await?;
                self.remember(Presence::new(address));
                self.members.push(member);
                self.add_link(connection, send, recv);
                self.mesh_connected().await?;
            }
            Internal::MeshJoined {
                generation,
                connection,
                send,
                recv,
                member,
                address,
                contacts,
            } => {
                let peer = control::peer_id(&connection);
                if generation != self.generation
                    || self.mesh.is_none()
                    || self.links.contains_key(&peer)
                {
                    connection.close(0u32.into(), b"Cancelled");
                    return Ok(());
                }
                self.dialing.remove(&peer);
                if !valid_member(&member, &peer)
                    || address.id.to_string() != peer
                    || !self.admit_mesh(&peer).await?
                {
                    connection.close(0u32.into(), b"Invalid or full room");
                    return Ok(());
                }
                self.remember(Presence::new(address));
                for presence in contacts.into_iter().take(MAX_PEERS) {
                    self.remember(presence);
                }
                self.members.push(member);
                self.add_link(connection, send, recv);
                self.mesh_connected().await?;
            }
            Internal::MeshFailed(generation, peer, attempted_address, error) => {
                if generation == self.generation
                    && self.mesh.is_some()
                    && !self.links.contains_key(&peer)
                    && self
                        .contacts
                        .get(&peer)
                        .is_some_and(|p| p.address == attempted_address)
                {
                    self.dialing.remove(&peer);
                    let failures = self.retry_failures.entry(peer.clone()).or_default();
                    let delay = 1u64 << (*failures).min(3);
                    *failures = failures.saturating_add(1);
                    tracing::info!(peer=%peer,retry_ms=delay*1000,%error,"peer_connect_retry");
                    self.retry_after
                        .insert(peer, Instant::now() + Duration::from_secs(delay));
                    if self.links.is_empty() {
                        self.status(if error.contains("Room is full") {
                            "Room is full (6 devices) • waiting for a place".into()
                        } else {
                            "Room ready • waiting for peers to connect".into()
                        });
                    }
                }
            }
            Internal::Incoming {
                connection,
                mut send,
                recv,
                token,
                name,
            } => {
                let peer = control::peer_id(&connection);
                let rejection =
                    if !self.host || self.invite.as_ref().is_none_or(|i| i.token != token) {
                        Some("This invitation is no longer valid")
                    } else if self.members.len() >= MAX_PEERS {
                        Some("Room is full (6 devices)")
                    } else if self.members.iter().any(|m| m.id == peer) {
                        Some("Device is already in this room")
                    } else {
                        None
                    };
                if let Some(error) = rejection {
                    self.reject(connection, send, recv, error);
                    return Ok(());
                }
                self.members.push(Member {
                    id: peer,
                    name: clean_name(&name),
                    sharing: false,
                    width: 0,
                    height: 0,
                });
                control::send(
                    &mut send,
                    &Wire::Welcome {
                        members: self.members.clone(),
                    },
                )
                .await?;
                self.add_link(connection, send, recv);
                self.roster().await?;
            }
            Internal::Joined {
                generation,
                connection,
                send,
                recv,
                members,
            } => {
                if generation != self.generation {
                    connection.close(0u32.into(), b"Cancelled");
                    return Ok(());
                }
                validate_members(&members, &self.local, Some(MAX_PEERS))?;
                self.in_room = true;
                self.members = members;
                self.add_link(connection, send, recv);
                self.state.lock().unwrap().busy = false;
                self.status("Connected • share your screen or watch another device".into());
                self.roster().await?;
            }
            Internal::Message(peer, ticket, wire) => {
                if self
                    .links
                    .get(&peer)
                    .is_some_and(|link| link.ticket == ticket)
                {
                    self.message(peer, wire).await?;
                }
            }
            Internal::Closed(peer, ticket, capacity) => {
                if self
                    .links
                    .get(&peer)
                    .is_some_and(|link| link.ticket == ticket)
                    && self.links.remove(&peer).is_some()
                {
                    if self.mesh.is_some() && capacity && peer < self.local {
                        self.block_mesh().await?;
                    } else if self.mesh.is_some() {
                        tracing::info!(peer=%peer,"peer_control_disconnected");
                        self.members.retain(|m| m.id != peer);
                        self.retry_failures.remove(&peer);
                        self.retry_after
                            .insert(peer.clone(), Instant::now() + Duration::from_secs(1));
                        self.roster().await?;
                        if self.links.is_empty() {
                            self.status(
                                "Room ready • share this reusable invitation or wait for peers"
                                    .into(),
                            );
                        }
                    } else if self.host {
                        self.members.retain(|m| m.id != peer);
                        self.roster().await?;
                    } else {
                        self.leave();
                        self.status(
                            "Room coordinator disconnected; create or join a room again".into(),
                        );
                    }
                }
            }
            Internal::Error(generation, text) => {
                if generation == self.generation {
                    self.leave();
                    self.status(text);
                }
            }
            Internal::Datagram {
                source,
                destination,
                data,
            } => {
                let peer = self
                    .peers
                    .iter()
                    .find(|(_, r)| r.accepts(source, destination, &data))
                    .map(|(p, _)| p.clone());
                if let Some(peer) = peer {
                    self.peers
                        .get_mut(&peer)
                        .unwrap()
                        .receive(source, destination, &data)?;
                    self.drain(&peer).await?;
                }
            }
        }
        Ok(())
    }
    async fn message(&mut self, peer: String, wire: Wire) -> Result<()> {
        match wire {
            Wire::Members(members) if !self.host && self.coordinator.as_ref() == Some(&peer) => {
                validate_members(&members, &self.local, Some(MAX_PEERS))?;
                self.members = members;
                self.roster().await?;
            }
            Wire::Share { width, height } if self.host || self.mesh.is_some() => {
                if width > 4096 || height > 4096 || (width == 0) != (height == 0) {
                    bail!("Invalid stream dimensions");
                }
                if let Some(m) = self.members.iter_mut().find(|m| m.id == peer) {
                    m.sharing = width != 0;
                    m.width = width;
                    m.height = height;
                    self.roster().await?;
                }
            }
            Wire::Name(name) if self.host || self.mesh.is_some() => {
                if let Some(m) = self.members.iter_mut().find(|m| m.id == peer) {
                    m.name = clean_name(&name);
                    self.roster().await?;
                }
            }
            Wire::Signal {
                from,
                to,
                answer,
                sdp,
            } => {
                if sdp.len() > 48_000 {
                    bail!("SDP exceeds limit");
                }
                let from = if self.mesh.is_some() {
                    if to != self.local || from != peer {
                        bail!("Unauthenticated direct signal");
                    }
                    peer
                } else if self.host {
                    peer
                } else {
                    if self.coordinator.as_ref() != Some(&peer) {
                        bail!("Unauthenticated signal");
                    }
                    from
                };
                if !self.members.iter().any(|m| m.id == from)
                    || !self.members.iter().any(|m| m.id == to)
                    || from == to
                {
                    bail!("Signal does not belong to this room");
                }
                if to != self.local {
                    if self.host {
                        self.send(
                            &to,
                            Wire::Signal {
                                from,
                                to: to.clone(),
                                answer,
                                sdp,
                            },
                        )?;
                    }
                } else {
                    let rtc = self.peers.get_mut(&from).context("Unknown media peer")?;
                    if answer {
                        rtc.accept_answer(&sdp)?;
                        self.drain(&from).await?;
                    } else {
                        let response = rtc.accept_offer(&sdp)?;
                        self.drain(&from).await?;
                        self.signal(&from, true, response)?;
                    }
                }
            }
            Wire::Contacts(contacts) if self.mesh.is_some() => {
                if contacts.len() > MAX_PEERS {
                    bail!("Too many peer contacts");
                }
                for presence in contacts.into_iter().take(MAX_PEERS) {
                    self.remember(presence);
                }
                self.dial_mesh();
            }
            Wire::Error(e) => self.status(e),
            _ => bail!("Unexpected control message"),
        }
        Ok(())
    }
    fn signal(&self, peer: &str, answer: bool, sdp: String) -> Result<()> {
        let message = Wire::Signal {
            from: self.local.clone(),
            to: peer.into(),
            answer,
            sdp,
        };
        self.send(
            if self.host || self.mesh.is_some() {
                peer
            } else {
                self.coordinator.as_deref().context("No coordinator")?
            },
            message,
        )
    }
    async fn roster(&mut self) -> Result<()> {
        if self.host {
            for link in self.links.values() {
                if link
                    .sender
                    .try_send(Wire::Members(self.members.clone()))
                    .is_err()
                {
                    link.connection.close(0u32.into(), b"Control congested");
                }
            }
        }
        self.peers
            .retain(|p, _| self.members.iter().any(|m| &m.id == p));
        self.inbox
            .lock()
            .unwrap()
            .queues
            .retain(|p, _| self.members.iter().any(|m| &m.id == p && m.sharing));
        self.inbox
            .lock()
            .unwrap()
            .waiting
            .retain(|p, _| self.members.iter().any(|m| &m.id == p && m.sharing));
        self.inbox
            .lock()
            .unwrap()
            .bytes
            .retain(|p, _| self.members.iter().any(|m| &m.id == p && m.sharing));
        self.requests
            .retain(|p, _| self.members.iter().any(|m| &m.id == p));
        for member in self.members.clone() {
            if member.id == self.local || self.peers.contains_key(&member.id) {
                continue;
            }
            let rtc = PeerRtc::new(&self.candidates)?;
            self.peers.insert(member.id.clone(), rtc);
            self.drain(&member.id).await?;
            if self.local < member.id {
                let offer = self.peers.get_mut(&member.id).unwrap().offer()?;
                self.drain(&member.id).await?;
                self.signal(&member.id, false, offer)?;
            }
        }
        let mut s = self.state.lock().unwrap();
        s.members = self.members.clone();
        s.in_room = self.in_room;
        s.connected_video = self
            .peers
            .iter()
            .filter(|(_, r)| r.connected)
            .map(|(p, _)| p.clone())
            .collect();
        Ok(())
    }
    async fn send_packets(&self, packets: Vec<(SocketAddr, SocketAddr, Vec<u8>)>) {
        for (n, (source, destination, bytes)) in packets.into_iter().enumerate() {
            if let Some(socket) = self.sockets.get(&source) {
                // send_to waits for writable readiness instead of silently losing
                // fragments of a large IDR when the kernel send buffer is busy.
                match tokio::time::timeout(
                    Duration::from_millis(20),
                    socket.send_to(&bytes, destination),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => tracing::warn!(%error, "Video UDP send failed"),
                    Err(_) => tracing::warn!("Video UDP send stalled for 20 ms"),
                }
            }
            // This runtime also receives RTP/RTX. Large IDRs must not starve it.
            if n % 64 == 63 {
                // Socket/AP queues can drop a giant IDR even when send_to
                // succeeds. Break it into short bursts, allowing UDP receive
                // and retransmission tasks to run between them.
                tokio::time::sleep(Duration::from_millis(1)).await;
            } else if n % 16 == 15 {
                tokio::task::yield_now().await;
            }
        }
    }
    async fn drain(&mut self, peer: &str) -> Result<()> {
        let out = self
            .peers
            .get_mut(peer)
            .context("Missing media peer")?
            .drain()?;
        self.send_packets(out.packets).await;

        for event in out.events {
            match event {
                RtcEvent::ControlReady | RtcEvent::Control(_) => {}
                RtcEvent::Connected => {
                    self.keyframe.store(true, Ordering::Relaxed);
                    self.status("Media connected • encrypted peer-to-peer screens".into());
                    self.state.lock().unwrap().connected_video = self
                        .peers
                        .iter()
                        .filter(|(_, r)| r.connected)
                        .map(|(p, _)| p.clone())
                        .collect();
                }
                RtcEvent::Disconnected => {
                    self.status("A video path disconnected • checking connectivity".into());
                    self.state
                        .lock()
                        .unwrap()
                        .connected_video
                        .retain(|p| p != peer);
                }
                RtcEvent::Keyframe => {
                    self.keyframe.store(true, Ordering::Relaxed);
                }
                RtcEvent::Audio {
                    data,
                    micros,
                    received_at,
                } => {
                    if self.members.iter().any(|m| m.id == peer && m.sharing) {
                        self.inbox.lock().unwrap().push_audio(RemoteAudio {
                            peer: peer.into(),
                            data,
                            micros,
                            received_at,
                        });
                    }
                }
                RtcEvent::Frame {
                    codec,
                    data,
                    keyframe,
                    contiguous,
                    received_at,
                    micros,
                } => {
                    if !self.members.iter().any(|m| m.id == peer && m.sharing) {
                        continue;
                    }
                    let request = self.inbox.lock().unwrap().push_timed(
                        RemoteFrame {
                            peer: peer.into(),
                            codec,
                            data,
                            keyframe,
                            received_at,
                            micros,
                        },
                        contiguous,
                    );
                    if p2pshare_media::diagnostics::enabled() && (keyframe || !contiguous) {
                        tracing::info!(
                            assembly_ms = received_at.elapsed().as_secs_f64() * 1000.,
                            keyframe,
                            contiguous,
                            "video_received"
                        );
                    }
                    if request
                        && self
                            .requests
                            .get(peer)
                            .is_none_or(|t| t.elapsed() > Duration::from_millis(500))
                    {
                        self.requests.insert(peer.into(), Instant::now());
                        self.peers.get_mut(peer).unwrap().request_keyframe()?;
                        // Drain the feedback mutation before the next actor operation.
                        let feedback = self.peers.get_mut(peer).unwrap().drain()?;
                        self.send_packets(feedback.packets).await;
                    }
                }
            }
        }
        Ok(())
    }
    async fn sound(&mut self, packet: AudioPacket) -> Result<()> {
        if !self.in_room || !self.members.iter().any(|m| m.id == self.local && m.sharing) {
            return Ok(());
        }
        for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
            self.peers
                .get_mut(&peer)
                .unwrap()
                .write_audio(packet.data.clone(), packet.micros)?;
            self.drain(&peer).await?;
        }
        Ok(())
    }
    async fn frame(&mut self, published: PublishedFrame) -> Result<()> {
        let frame = published.frame;
        if published.sequence != self.last_video_sequence + 1 {
            self.waiting_video = true;
            self.keyframe.store(true, Ordering::Relaxed);
        }
        self.last_video_sequence = published.sequence;
        if frame.keyframe {
            self.waiting_video = false;
        }
        if self.waiting_video {
            self.keyframe.store(true, Ordering::Relaxed);
            return Ok(());
        }
        if !self.in_room || !self.members.iter().any(|m| m.id == self.local && m.sharing) {
            return Ok(());
        }
        for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
            self.peers.get_mut(&peer).unwrap().write_codec(
                frame.data.clone(),
                frame.micros,
                frame.codec,
            )?;
            self.drain(&peer).await?;
        }
        Ok(())
    }
    async fn tick(&mut self) -> Result<()> {
        let now = Instant::now();
        if self.mesh.is_some() && self.mesh_tick.elapsed() > Duration::from_secs(1) {
            self.mesh_tick = now;
            if let Some(updates) = &self.address_updates {
                let address = self.endpoint.addr();
                if *updates.borrow() != address {
                    updates.send_replace(address);
                }
            }
            self.contacts
                .retain(|_, presence| presence.expires > p2pshare_discovery::now());
            self.dial_mesh();
            // Gossip live authenticated endpoints; no relay is needed between established peers.
            if self.generation > 0
                && self.mesh_tick.duration_since(self.gossip_tick) > Duration::from_secs(20)
            {
                self.gossip_tick = now;
                self.broadcast(Wire::Contacts(self.live_contacts()));
            }
        }
        for peer in self
            .peers
            .iter()
            .filter(|(_, r)| r.deadline <= now)
            .map(|(p, _)| p.clone())
            .collect::<Vec<_>>()
        {
            self.peers.get_mut(&peer).unwrap().tick()?;
            self.drain(&peer).await?;
        }
        Ok(())
    }
}
#[cfg(not(target_os = "android"))]
fn valid_member(member: &Member, peer: &str) -> bool {
    member.id == peer
        && member.id.len() == 64
        && member.name.chars().count() <= 32
        && !member.name.chars().any(char::is_control)
        && member.width <= 4096
        && member.height <= 4096
        && ((member.width == 0 && member.height == 0 && !member.sharing)
            || (member.width > 0 && member.height > 0 && member.sharing))
}
fn clean_name(name: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(32)
        .collect();
    if name.is_empty() {
        "Guest".into()
    } else {
        name
    }
}
fn validate_members(members: &[Member], local: &str, limit: Option<usize>) -> Result<()> {
    if limit.is_some_and(|limit| members.len() > limit) || !members.iter().any(|m| m.id == local) {
        bail!("Invalid room membership");
    }
    let mut ids = std::collections::HashSet::new();
    for m in members {
        if m.id.len() != 64
            || !ids.insert(&m.id)
            || m.name.chars().count() > 32
            || m.width > 4096
            || m.height > 4096
        {
            bail!("Invalid peer announcement");
        }
    }
    Ok(())
}
async fn media_sockets(
    events: mpsc::Sender<Internal>,
    isolated: bool,
) -> Result<(stun::Sockets, Vec<IceCandidate>)> {
    stun::open(events, isolated).await
}

#[cfg(test)]
mod video_inbox_tests {
    use super::*;
    #[test]
    fn one_second_buffer_retains_120fps_and_audio_with_bounded_bytes() {
        let mut inbox = VideoInbox {
            playback_ms: 1000,
            ..Default::default()
        };
        let now = Instant::now();
        for n in 0..120 {
            assert!(!inbox.push_timed(
                RemoteFrame {
                    peer: "peer".into(),
                    codec: VideoFormat::H264,
                    received_at: now + Duration::from_micros(n * 8333),
                    micros: 1_000_000 + n * 8333,
                    data: Arc::from([1]),
                    keyframe: n == 0
                },
                true
            ));
            if n < 100 {
                inbox.push_audio(RemoteAudio {
                    peer: "peer".into(),
                    received_at: now + Duration::from_micros(n * 10000),
                    micros: 1_000_000 + n * 10000,
                    data: Arc::from([2]),
                });
            }
        }
        assert_eq!(inbox.queues["peer"].len(), 120);
        assert_eq!(inbox.audio["peer"].len(), 100);
        assert_eq!(inbox.overflows, 0);
        let clock = inbox.timelines["peer"];
        assert_eq!(
            clock.deadline(1_000_000, 1000),
            now + Duration::from_secs(1)
        );
        assert_eq!(
            clock.deadline(1_080_000, 200),
            now + Duration::from_millis(280)
        );
    }
    #[test]
    fn recovery_burst_preserves_reference_frames() {
        let mut inbox = VideoInbox::default();
        for n in 0..20 {
            assert!(!inbox.push("peer", Arc::from([n]), n == 0, true));
        }
        let values: Vec<_> = inbox.queues["peer"]
            .iter()
            .map(|frame| frame.data[0])
            .collect();
        assert_eq!(values, (0..20).collect::<Vec<_>>());
        assert_eq!(inbox.overflows, 0);
    }
    #[test]
    fn compressed_byte_limit_recovers_with_new_idr() {
        let mut inbox = VideoInbox::default();
        let big: Arc<[u8]> = vec![0; 2 * 1024 * 1024].into();
        for n in 0..4 {
            assert!(!inbox.push("peer", big.clone(), n == 0, true));
        }
        assert!(inbox.push("peer", Arc::from([1]), false, true));
        assert!(inbox.queues["peer"].is_empty());
        assert_eq!(inbox.bytes["peer"], 0);
        assert!(!inbox.push("peer", Arc::from([2]), true, true));
        assert_eq!(inbox.queues["peer"].len(), 1);
        assert_eq!(inbox.overflows, 1);
    }
}
