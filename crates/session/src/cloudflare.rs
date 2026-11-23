//! Cloudflare room membership/SDP. Media remains direct UDP/DTLS-SRTP.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use http::{HeaderName, HeaderValue, Uri};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_websockets::{ClientBuilder, Limits, Message};

const MAX_VIEWER_LIST_BYTES: usize = 49153;
const MAX_NAMED_ROOM_PEERS: usize = 64;
const OFFER_RATE_WINDOW: Duration = Duration::from_secs(2);
const MAX_OFFERS_PER_WINDOW: usize = 3;
const ROOM_FULL_STATUS: &str = "Room is full. Waiting for a spot…";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalingConfig {
    pub server: String,
}
impl SignalingConfig {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        let config = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<Self>(
                bytes.strip_prefix(&[239, 187, 191]).unwrap_or(&bytes),
            )
            .context("Invalid signaling.json")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = Self {
                    server: String::new(),
                };
                std::fs::write(path, serde_json::to_vec_pretty(&config)?)?;
                config
            }
            Err(error) => return Err(error.into()),
        };
        if !config.server.is_empty() {
            validate_server(&config.server)?;
        }
        Ok(config)
    }
}
pub(super) fn validate_server(server: &str) -> Result<()> {
    let uri: Uri = server.parse().context("Invalid signaling server URL")?;
    let host = uri.host().context("Signaling server needs a host")?;
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]");
    if server.len() > 2048
        || server.contains('@')
        || server.contains('#')
        || uri.path() != "/"
        || uri.query().is_some()
        || !(uri.scheme_str() == Some("wss") || (loopback && uri.scheme_str() == Some("ws")))
    {
        bail!("Use a wss:// signaling server URL (ws:// is allowed only on loopback)");
    }
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudflareInvite {
    pub version: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub secret: String,
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
}
impl CloudflareInvite {
    pub fn new(server: String) -> Result<Self> {
        validate_server(&server)?;
        Ok(Self {
            version: 5,
            room: None,
            secret: URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()),
            server,
        })
    }
    fn validate(&self) -> Result<()> {
        match self.version {
            5 if self.room.is_none()
                && self.secret.len() == 43
                && URL_SAFE_NO_PAD.decode(&self.secret)?.len() == 32 => {}
            6 if self.room.is_some()
                && self.secret.len() == 43
                && URL_SAFE_NO_PAD.decode(&self.secret)?.len() == 32 => {}
            7 if self.room.is_some() && self.secret.is_empty() => {}
            _ => bail!("Invalid room invitation"),
        }
        if let Some(room) = &self.room
            && canonical_room(room)? != *room
        {
            bail!("Invalid canonical room name");
        }
        validate_server(&self.server)
    }
    fn upgraded(self) -> Result<Self> {
        self.validate()?;
        if self.version == 6 {
            return Self::named(
                self.server,
                self.room.as_deref().context("Missing room name")?,
            );
        }
        Ok(self)
    }
    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        Ok(format!(
            "p2pshare://room/{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?)
        ))
    }
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > 4096 {
            bail!("Room invitation is too long");
        }
        let data = URL_SAFE_NO_PAD.decode(
            text.trim()
                .strip_prefix("p2pshare://room/")
                .context("Paste a new Cloudflare room invitation")?,
        )?;
        let invite: Self = serde_json::from_slice(&data).context(
            "This invitation is from an older room backend. Create a new room in version 0.8",
        )?;
        invite.upgraded()
    }
    pub fn named(server: String, room: &str) -> Result<Self> {
        validate_server(&server)?;
        Ok(Self {
            version: 7,
            secret: String::new(),
            server,
            room: Some(canonical_room(room)?),
        })
    }
    fn room_id(&self) -> Result<String> {
        Ok(if let Some(room) = &self.room {
            hex(&Sha256::digest(
                format!("P2PShare named room v1:{room}").as_bytes(),
            ))
        } else {
            hex(&Sha256::digest(URL_SAFE_NO_PAD.decode(&self.secret)?))
        })
    }
}
pub(super) fn canonical_room(room: &str) -> Result<String> {
    let room = room
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if !(3..=64).contains(&room.len())
        || !room
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b' ' | b'-' | b'_'))
    {
        bail!("Room name must be 3..64 ASCII letters, numbers, spaces, hyphens or underscores");
    }
    Ok(room)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Deserialize)]
struct RemoteMember {
    id: String,
    session: String,
    name: String,
    sharing: bool,
    width: u32,
    height: u32,
    codec: VideoFormat,
}
impl RemoteMember {
    fn member(&self) -> Member {
        Member {
            id: self.id.clone(),
            name: self.name.clone(),
            sharing: self.sharing,
            width: self.width,
            height: self.height,
        }
    }
}
struct RefreshOutput {
    ips: Vec<std::net::IpAddr>,
    sockets: stun::Sockets,
    gather: stun::GatherResult,
    rebound: bool,
}
enum Event {
    Text(u64, String),
    Closed(u64, bool),
    Failed(u64, Option<u16>),
    Rejected(u64, u16),
    Gathered(u64, Result<RefreshOutput, String>),
}
struct SocketTask {
    sender: mpsc::Sender<String>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for SocketTask {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A socket task owns reconnect authentication and I/O, never media frames.
async fn socket_loop(
    invite: CloudflareInvite,
    key: SigningKey,
    generation: u64,
    events: mpsc::Sender<Event>,
    mut outgoing: mpsc::Receiver<String>,
) -> Result<bool> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let id = hex(key.verifying_key().as_bytes());
    let session = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .to_string();
    let room = invite.room_id()?;
    let named = invite.room.is_some();
    let proof = if named {
        format!("P2PShare signaling v3\n{room}\n{id}\n{session}\n{timestamp}")
    } else {
        format!("P2PShare signaling v1\n{room}\n{id}\n{session}\n{timestamp}")
    };
    let signature = URL_SAFE_NO_PAD.encode(key.sign(proof.as_bytes()).to_bytes());
    let url = format!(
        "{}/v{}/rooms/{room}",
        invite.server.trim_end_matches('/'),
        if named { 3 } else { 1 }
    );
    let mut client = ClientBuilder::new()
        .uri(&url)?
        .limits(Limits::default().max_payload_len(Some(65536)));
    for (name, value) in [
        ("x-p2p-peer", id),
        ("x-p2p-session", session),
        ("x-p2p-timestamp", timestamp),
        ("x-p2p-signature", signature),
    ] {
        client = client.add_header(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(&value)?,
        )?;
    }
    if let Some(room) = &invite.room {
        client = client.add_header(
            HeaderName::from_static("x-p2p-room"),
            HeaderValue::from_str(room)?,
        )?;
    } else {
        client = client.add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", invite.secret))?,
        )?;
    }
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(8), client.connect())
        .await
        .context("Signaling connection timed out")??;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut received = Instant::now();
    loop {
        tokio::select! {
            incoming=socket.next() => match incoming {
                Some(Ok(message)) if message.is_close() => {
                    // Another copy of this device took over; don't fight it in a reconnect loop.
                    let replaced=message.as_close().is_some_and(|(code,_)| u16::from(code)==4001);
                    return Ok(replaced);
                }
                Some(Ok(message)) if message.is_text() => {
                    received=Instant::now();
                    let text=message.as_text().context("Invalid signaling text")?;
                    if text!="pong" { events.send(Event::Text(generation,text.into())).await?; }
                }
                Some(Ok(_)) => {},
                Some(Err(_)) | None => return Ok(false),
            },
            message=outgoing.recv() => {
                let Some(message)=message else {return Ok(false)};
                let leave=message=="{\"type\":\"leave\"}";
                tokio::time::timeout(Duration::from_secs(2),socket.send(Message::text(message))).await??;
                if leave {
                    let _=tokio::time::timeout(Duration::from_millis(300),socket.close()).await;
                    return Ok(false);
                }
            }
            _=heartbeat.tick() => {
                if received.elapsed()>Duration::from_secs(25) {return Ok(false)};
                tokio::time::timeout(Duration::from_secs(2),socket.send(Message::text("ping"))).await??;
            }
        }
    }
}

pub(super) struct CloudflareActor {
    local: String,
    key: SigningKey,
    server: String,
    room_file: PathBuf,
    commands: mpsc::Receiver<Command>,
    video: mpsc::Receiver<PublishedFrame>,
    audio: mpsc::Receiver<AudioPacket>,
    state: Arc<Mutex<Snapshot>>,
    inbox: Arc<Mutex<VideoInbox>>,
    keyframe: Arc<AtomicBool>,
    internal: mpsc::Receiver<Internal>,
    events: mpsc::Sender<Event>,
    incoming: mpsc::Receiver<Event>,
    peers: HashMap<String, PeerRtc>,
    sockets: stun::Sockets,
    observed_interfaces: Vec<std::net::IpAddr>,
    stun_mappings: std::collections::HashSet<(SocketAddr, SocketAddr)>,
    symmetric_nat: bool,
    candidates: Vec<IceCandidate>,
    udp_routes: HashMap<(SocketAddr, SocketAddr), String>,
    offer_times: HashMap<String, VecDeque<Instant>>,
    members: Vec<Member>,
    sessions: HashMap<String, String>,
    room: Option<CloudflareInvite>,
    socket: Option<SocketTask>,
    generation: u64,
    gather_generation: u64,
    own_session: String,
    in_room: bool,
    name: String,
    sharing: Option<(u32, u32)>,
    codec: VideoFormat,
    requests: HashMap<String, Instant>,
    last_video_sequence: u64,
    waiting_video: bool,
    viewers: std::collections::HashSet<String>,
    viewers_waiting: std::collections::HashSet<String>,
    preview: Option<Arc<[u8]>>,
    reconnect: Option<Instant>,
    failures: u32,
    recovery: HashMap<String, recovery::Recovery>,
    datagrams: mpsc::Sender<Internal>,
    isolated: bool,
    gather_task: Option<GatherTask>,
    gathering: bool,
    refreshed: Instant,
    interface_check: Instant,
    unmatched_datagrams: u64,
}
struct GatherTask(tokio::task::JoinHandle<()>);
impl Drop for GatherTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl CloudflareActor {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn new(
        identity: PathBuf,
        server: String,
        isolated: bool,
        commands: mpsc::Receiver<Command>,
        video: mpsc::Receiver<PublishedFrame>,
        audio: mpsc::Receiver<AudioPacket>,
        state: Arc<Mutex<Snapshot>>,
        inbox: Arc<Mutex<VideoInbox>>,
        keyframe: Arc<AtomicBool>,
    ) -> Result<Self> {
        let seed: [u8; 32] = match std::fs::read(&identity) {
            Ok(bytes) => bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid device key"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let seed = rand::random();
                if let Some(parent) = identity.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&identity, seed)?;
                seed
            }
            Err(error) => return Err(error.into()),
        };
        let key = SigningKey::from_bytes(&seed);
        let local = hex(key.verifying_key().as_bytes());
        let room_file = identity.with_extension("cloudflare-room.json");
        let (datagrams, internal) = mpsc::channel(1024);
        let opened = media_sockets(datagrams.clone(), isolated).await?;
        let (events, incoming) = mpsc::channel(128);
        {
            let mut s = state.lock().unwrap();
            s.local_id = local.clone();
            s.busy = false;
            s.saved_invite = std::fs::read(&room_file)
                .ok()
                .and_then(|b| serde_json::from_slice::<CloudflareInvite>(&b).ok())
                .and_then(|i| i.upgraded().ok())
                .and_then(|i| i.encode().ok());
            s.status = "Enter a room name to join".into();
            s.discovery_status = if opened.gather.symmetric_nat {
                "Cloudflare signaling • direct encrypted video • Symmetric NAT/VPN detected".into()
            } else {
                "Cloudflare signaling • direct encrypted video".into()
            };
        }
        Ok(Self {
            local,
            key,
            server,
            room_file,
            commands,
            video,
            audio,
            state,
            inbox,
            keyframe,
            internal,
            events,
            incoming,
            peers: HashMap::new(),
            sockets: opened.sockets,
            observed_interfaces: opened.ips,
            stun_mappings: opened.gather.mappings,
            symmetric_nat: opened.gather.symmetric_nat,
            candidates: opened.gather.candidates,
            udp_routes: HashMap::new(),
            offer_times: HashMap::new(),
            members: vec![],
            sessions: HashMap::new(),
            room: None,
            socket: None,
            generation: 0,
            gather_generation: 0,
            own_session: String::new(),
            in_room: false,
            name: "Guest".into(),
            sharing: None,
            codec: VideoFormat::H264,
            requests: HashMap::new(),
            last_video_sequence: 0,
            waiting_video: true,
            viewers: Default::default(),
            viewers_waiting: Default::default(),
            preview: None,
            reconnect: None,
            failures: 0,
            recovery: HashMap::new(),
            datagrams,
            isolated,
            gather_task: None,
            gathering: false,
            refreshed: Instant::now(),
            interface_check: Instant::now(),
            unmatched_datagrams: 0,
        })
    }
    fn status(&self, text: String) {
        self.state.lock().unwrap().status = text;
    }
    fn send(&self, message: Value) -> Result<()> {
        self.socket
            .as_ref()
            .context("Signaling is reconnecting")?
            .sender
            .try_send(message.to_string())
            .context("Signaling queue is full")
    }
    fn connect(&mut self) {
        self.socket = None;
        self.generation += 1;
        self.reconnect = None;
        let Some(invite) = self.room.clone() else {
            return;
        };
        let (sender, outgoing) = mpsc::channel(256);
        let events = self.events.clone();
        let generation = self.generation;
        let key = self.key.clone();
        let task = tokio::spawn(async move {
            match socket_loop(invite, key, generation, events.clone(), outgoing).await {
                Ok(replaced) => {
                    let _ = events.send(Event::Closed(generation, replaced)).await;
                }
                Err(error) => {
                    let status = match error.downcast_ref::<tokio_websockets::Error>() {
                        Some(tokio_websockets::Error::Upgrade(
                            tokio_websockets::upgrade::Error::DidNotSwitchProtocols(status),
                        )) => Some(*status),
                        _ => None,
                    };
                    if let Some(status @ (401 | 403 | 404 | 410)) = status {
                        let _ = events.send(Event::Rejected(generation, status)).await;
                        return;
                    }
                    tracing::warn!(?status, error = %format_args!("{error:#}"), "signaling_connection_failed");
                    let _ = events.send(Event::Failed(generation, status)).await;
                }
            }
        });
        self.socket = Some(SocketTask { sender, task });
        let mut s = self.state.lock().unwrap();
        s.busy = true;
        if s.status != ROOM_FULL_STATUS {
            s.status = "Connecting to Cloudflare signaling…".into();
        }
    }
    fn reset_media(&mut self) {
        self.peers.clear();
        self.udp_routes.clear();
        self.offer_times.clear();
        self.recovery.clear();
        self.gather_generation += 1;
        self.gather_task = None;
        self.gathering = false;
        self.state.lock().unwrap().viewer_count = 0;
        self.viewers.clear();
        self.viewers_waiting.clear();
        self.preview = None;
        while self.audio.try_recv().is_ok() {}
        self.members.clear();
        self.sessions.clear();
        self.requests.clear();
        self.own_session.clear();
        let mut i = self.inbox.lock().unwrap();
        i.previews.clear();
        i.stream_info.clear();
        if self.sharing.is_none() {
            i.own_stream_info = None;
        }
        i.watching.clear();i.stream_viewers.clear();
        i.queues.clear();
        i.audio.clear();
        i.bytes.clear();
        i.waiting.clear();
        i.codecs.clear();
        i.timelines.clear();
        let mut s = self.state.lock().unwrap();
        s.members.clear();
        s.connected_video.clear();
        self.keyframe.store(true, Ordering::Relaxed);
        self.waiting_video = true;
    }
    async fn leave(&mut self) {
        // Write the leave message and close before aborting the owned socket task.
        if let Some(mut socket) = self.socket.take() {
            let _ = socket.sender.try_send("{\"type\":\"leave\"}".into());
            let _ = tokio::time::timeout(Duration::from_millis(400), &mut socket.task).await;
        }
        self.generation += 1;
        self.reconnect = None;
        self.room = None;
        self.in_room = false;
        self.sharing = None;
        self.reset_media();
        let mut s = self.state.lock().unwrap();
        s.busy = false;
        s.in_room = false;
        s.invite = None;
        s.status = "Left room • enter a room name to join".into();
    }
    async fn open(&mut self, invite: CloudflareInvite, name: String) -> Result<()> {
        self.leave().await;
        invite.validate()?;
        {
            let mut s = self.state.lock().unwrap();
            let link = invite.encode()?;
            s.invite = Some(link);
        }
        self.name = clean_name(&name);
        self.room = Some(invite);
        self.failures = 0;
        self.connect();
        Ok(())
    }
    fn send_share(&self) -> Result<()> {
        if let Some((width, height)) = self.sharing {
            self.send(json!({"type":"share","sharing":true,"width":width,"height":height,"codec":self.codec}))
        } else {
            self.send(json!({"type":"share","sharing":false}))
        }
    }
    async fn command(&mut self, c: Command) -> Result<()> {
        match c {
            Command::SetSignalingServer(server) => {
                validate_server(&server)?;
                self.server = server;
            }
            Command::Create(name) => {
                self.open(CloudflareInvite::new(self.server.clone())?, name)
                    .await?
            }
            Command::Join { invite, name } => {
                let invite = CloudflareInvite::parse(&invite)?;
                self.open(invite, name).await?;
            }
            Command::Leave => self.leave().await,
            Command::NamedRoom { room, name } => {
                let invite = CloudflareInvite::named(self.server.clone(), &room)?;
                self.open(invite, name).await?;
            }
            Command::Rename(name) => {
                self.name = clean_name(&name);
                if self.in_room {
                    self.send(json!({"type":"name","name":self.name}))?;
                }
            }
            Command::Publish { width, height } => {
                if !self.in_room {
                    bail!("Join a room before sharing");
                }
                self.sharing = Some((width, height));
                self.keyframe.store(true, Ordering::Relaxed);
                self.send_share()?;
                for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
                    self.send_preview(&peer).await;
                    self.send_stream_info(&peer).await;
                }
            }
            Command::Unpublish => {
                self.viewers.clear();
                self.viewers_waiting.clear();
                self.state.lock().unwrap().viewer_count = 0;
                self.sharing = None;
                self.preview = None;
                self.inbox.lock().unwrap().own_stream_info = None;
                if self.in_room {
                    self.send_share()?;
                }
            }
            Command::RequestKeyframe(peer) => {
                if let Some(rtc) = self.peers.get_mut(&peer) {
                    match rtc.request_keyframe() {
                        Ok(()) => self.drain(&peer).await,
                        Err(error) => {
                            tracing::warn!(peer, %error, "request_keyframe_failed");
                            self.failed_peer(&peer, &format!("keyframe request: {error:#}"));
                        }
                    }
                }
            }
            Command::Watch { peer, enabled } => {
                self.send_priority_control(&peer, &[1, enabled as u8]).await;
            }
            Command::Avatar(bytes) => {
                use sha2::{Digest, Sha256};
                let hash = if let Some(bytes) = bytes {
                    p2pshare_media::avatar::validate(&bytes)?;
                    let hash = format!("{:x}", Sha256::digest(&bytes));
                    let mut inbox = self.inbox.lock().unwrap();
                    inbox.avatar_cache.insert(hash.clone(), bytes.into());
                    inbox.avatar_hashes.insert(self.local.clone(), hash.clone());
                    hash
                } else {
                    self.inbox.lock().unwrap().avatar_hashes.remove(&self.local);
                    String::new()
                };
                let _ = hash;
                for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
                    self.send_avatar_hash(&peer).await;
                }
            }
            Command::Preview(bytes) => {
                if bytes.len() <= p2pshare_media::preview::MAX_BYTES && self.in_room {
                    self.preview = Some(bytes);
                    for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
                        if !self.viewers.contains(&peer) {
                            self.send_preview(&peer).await;
                        }
                    }
                }
            }
            Command::StreamInfo(info) => {
                if info.valid() {
                    self.inbox.lock().unwrap().own_stream_info = Some(info);
                    for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
                        self.send_stream_info(&peer).await;
                    }
                }
            }
            Command::Shutdown => {}
        }
        Ok(())
    }
    fn disconnected(&mut self, replaced: bool) {
        self.socket = None;
        self.state.lock().unwrap().busy = false;
        if replaced {
            self.reconnect = None;
            self.room = None;
            self.in_room = false;
            self.reset_media();
            let mut s = self.state.lock().unwrap();
            s.in_room = false;
            s.status =
                "This device connected from another app instance. Rejoin to take over.".into();
        } else if self.room.is_some() {
            self.failures = (self.failures + 1).min(4);
            self.reconnect = Some(Instant::now() + Duration::from_secs(1 << (self.failures - 1)));
            let status = if self.in_room {
                "Signaling disconnected • retrying; existing video can continue"
            } else {
                "Could not connect to the signaling server. Retrying…"
            };
            self.status(status.into());
            tracing::info!(
                retry_seconds = 1 << (self.failures - 1),
                "signaling_reconnect_scheduled"
            );
        }
    }
    async fn text(&mut self, text: String) -> Result<()> {
        let message: Value = serde_json::from_str(&text)?;
        match message.get("type").and_then(Value::as_str) {
            Some("welcome") => {
                if message["protocol"] != 1 || message["id"] != self.local {
                    bail!("Invalid signaling welcome");
                }
                if let Some(invite) = &self.room {
                    std::fs::write(&self.room_file, serde_json::to_vec(invite)?)?;
                    self.state.lock().unwrap().saved_invite = Some(invite.encode()?);
                }
                self.reset_media();
                self.own_session = message["session"]
                    .as_str()
                    .context("Missing signaling session")?
                    .into();
                self.in_room = true;
                self.failures = 0;
                {
                    let mut s = self.state.lock().unwrap();
                    s.busy = false;
                    s.in_room = true;
                    s.status = "Room connected • Cloudflare signaling".into();
                }
                self.roster(serde_json::from_value(message["members"].clone())?)
                    .await?;
                self.send(json!({"type":"name","name":self.name}))?;
                self.send_share()?;
                tracing::info!("signaling_room_connected");
            }
            Some("members") if self.in_room => {
                self.roster(serde_json::from_value(message["members"].clone())?)
                    .await?
            }
            Some("signal") if self.in_room => {
                let peer = message["from"].as_str().context("Missing signaling peer")?;
                if message["to_session"] != self.own_session
                    || self
                        .sessions
                        .get(peer)
                        .is_none_or(|s| message["from_session"] != *s)
                {
                    return Ok(());
                }
                let payload = message["payload"].as_str().context("Missing SDP")?;
                let value: Value = serde_json::from_str(payload)?;
                let token = value["p2pshare_attempt"].as_str().map(str::to_owned);
                if token.as_ref().is_some_and(|s| s.len() > 32) {
                    bail!("Invalid media attempt");
                }
                match message["kind"].as_str() {
                    Some("offer") if self.local.as_str() > peer => {
                        if token.is_some()
                            && self
                                .recovery
                                .get(peer)
                                .is_some_and(|r| r.negotiated && r.token == token)
                        {
                            tracing::info!(peer, "media_duplicate_offer_ignored");
                            return Ok(());
                        }
                        let now = Instant::now();
                        let times = self.offer_times.entry(peer.into()).or_default();
                        while times
                            .front()
                            .is_some_and(|t| now.duration_since(*t) > OFFER_RATE_WINDOW)
                        {
                            times.pop_front();
                        }
                        if times.len() >= MAX_OFFERS_PER_WINDOW {
                            tracing::warn!(peer, "media_offer_rate_limited");
                            return Ok(());
                        }
                        // Validate incoming offer into a fresh PeerRtc before replacing a healthy peer.
                        let mut candidate_rtc = PeerRtc::new(&self.candidates)?;
                        candidate_rtc.set_peer_label(peer.into());
                        let answer = candidate_rtc.accept_offer(payload)?;
                        let tagged_answer = tagged_sdp(answer, token.as_deref())?;
                        self.offer_times
                            .entry(peer.into())
                            .or_default()
                            .push_back(now);
                        self.commit_peer(peer, candidate_rtc);
                        self.recovery
                            .get_mut(peer)
                            .unwrap()
                            .attempting(now, token.clone());
                        self.recovery.get_mut(peer).unwrap().negotiated = true;
                        self.signal(peer, true, tagged_answer)?;
                        tracing::info!(
                            peer,
                            local_candidates = %candidate_summary(&self.candidates),
                            remote_candidates = %sdp_candidate_summary(payload),
                            "media_offer_accepted"
                        );
                    }
                    Some("answer") if self.local.as_str() < peer => {
                        let Some(attempt) = self.recovery.get(peer) else {
                            return Ok(());
                        };
                        if attempt.negotiated {
                            tracing::info!(peer, "media_duplicate_answer_ignored");
                            return Ok(());
                        }
                        // Old clients omit the token; permit that on the first attempt only.
                        if token != attempt.token && !(token.is_none() && attempt.failures == 0) {
                            tracing::info!(peer, "media_stale_answer_ignored");
                            return Ok(());
                        }
                        self.peers
                            .get_mut(peer)
                            .context("Missing media peer")?
                            .accept_answer(payload)?;
                        tracing::info!(
                            peer,
                            local_candidates = %candidate_summary(&self.candidates),
                            remote_candidates = %sdp_candidate_summary(payload),
                            "media_answer_accepted"
                        );
                    }
                    Some("ice") if value["candidates"].is_array() => {
                        self.remote_candidates(peer, token.as_deref(), &value["candidates"])
                            .await?;
                        return Ok(());
                    }
                    Some("ice") if self.local.as_str() < peer && value["restart"] == true => {
                        if self
                            .recovery
                            .get(peer)
                            .is_some_and(|r| r.started.elapsed() >= Duration::from_secs(5))
                        {
                            self.failed_peer(peer, "remote requested connectivity retry");
                        }
                        return Ok(());
                    }
                    _ => return Ok(()),
                }
                self.recovery.get_mut(peer).unwrap().negotiated = true;
                self.drain(peer).await;
                self.sync_candidates(peer).await?;
            }
            Some("error") => {
                let error = message["error"].as_str().unwrap_or("unknown");
                tracing::warn!(error, "signaling_server_error");
                if error != "peer_session_changed" {
                    self.status(format!("Signaling: {error}"));
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn signal(&self, peer: &str, answer: bool, sdp: String) -> Result<()> {
        self.send(json!({"type":"signal","to":peer,"to_session":self.sessions.get(peer).context("Peer left")?,
            "kind":if answer {"answer"}else{"offer"},"payload":sdp}))
    }
    async fn init_peer_session(&mut self, peer: &str) -> Result<()> {
        self.replace_peer(peer)?;
        if self.local.as_str() < peer {
            let sdp = self.peers.get_mut(peer).unwrap().offer()?;
            self.drain(peer).await;
            let token = format!("{:016x}", rand::random::<u64>());
            self.recovery.get_mut(peer).unwrap().token = Some(token.clone());
            self.signal(peer, false, tagged_sdp(sdp, Some(&token))?)?;
            tracing::info!(
                peer,
                local_candidates = %candidate_summary(&self.candidates),
                "media_offer_sent"
            );
        }
        Ok(())
    }
    async fn roster(&mut self, members: Vec<RemoteMember>) -> Result<()> {
        let public: Vec<_> = members.iter().map(RemoteMember::member).collect();
        let limit = if self.room.as_ref().is_some_and(|invite| invite.version == 5) {
            Some(MAX_PEERS)
        } else {
            Some(MAX_NAMED_ROOM_PEERS)
        };
        validate_members(&public, &self.local, limit)?;
        let mut sessions = HashMap::new();
        for m in &members {
            if m.session.len() != 22 || URL_SAFE_NO_PAD.decode(&m.session)?.len() != 16 {
                bail!("Invalid member session");
            }
            if m.id == self.local && m.session != self.own_session {
                bail!("Device session changed");
            }
            sessions.insert(m.id.clone(), m.session.clone());
        }
        self.peers
            .retain(|id, _| self.sessions.get(id) == sessions.get(id) && sessions.contains_key(id));
        self.udp_routes.retain(|_, id| self.peers.contains_key(id));
        self.offer_times.retain(|id, _| self.peers.contains_key(id));
        {
            let mut inbox = self.inbox.lock().unwrap();
            let live = |id: &String| {
                members.iter().any(|m| &m.id == id && m.sharing)
                    && self.sessions.get(id) == sessions.get(id)
            };
            inbox.stream_viewers.retain(|id,_|live(id));
            inbox.avatar_hashes.retain(|id,_|id==&self.local||members.iter().any(|m|&m.id==id));
            inbox.previews.retain(|id, _| live(id));
            inbox.stream_info.retain(|id, _| live(id));
            inbox.watching.retain(live);
            inbox.queues.retain(|id, _| live(id));
            inbox.audio.retain(|id, _| live(id));
            inbox.waiting.retain(|id, _| live(id));
            inbox.bytes.retain(|id, _| live(id));
            inbox.codecs.retain(|id, _| live(id));
            inbox.timelines.retain(|id, _| live(id));
            for m in &members {
                if m.sharing {
                    // SDP/roster may arrive after RTP. Preserve the format learned from actual frames.
                    inbox.codecs.entry(m.id.clone()).or_insert(m.codec);
                }
            }
        }
        self.viewers.retain(|id| self.peers.contains_key(id));
        self.viewers_waiting
            .retain(|id| self.peers.contains_key(id));
        self.state.lock().unwrap().viewer_count = self.viewers.len();
        self.recovery.retain(|id, _| self.peers.contains_key(id));
        self.sessions = sessions;
        self.members = public;
        self.requests.retain(|id, _| self.sessions.contains_key(id));
        let mut added = false;
        for m in members {
            if m.id == self.local || self.peers.contains_key(&m.id) {
                continue;
            }
            added = true;
            tracing::info!(peer = %m.id, "signaling_peer_media_created");
            if let Err(error) = self.init_peer_session(&m.id).await {
                tracing::warn!(peer = %m.id, %error, "signaling_peer_init_failed");
                self.failed_peer(&m.id, &format!("init: {error:#}"));
            }
        }
        if added && self.refreshed.elapsed() > Duration::from_secs(2) {
            self.refresh_candidates();
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
    pub(super) async fn run(mut self) -> Result<()> {
        let mut timer = tokio::time::interval(Duration::from_millis(10));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let result = tokio::select! {
                command=self.commands.recv()=>match command {
                    Some(Command::Shutdown)|None=>{self.leave().await;break;}
                    Some(c)=>self.command(c).await,
                },
                sound=self.audio.recv()=>match sound {Some(sound)=>self.sound(sound).await,None=>Ok(())},
                frame=self.video.recv()=>match frame {Some(frame)=>self.publish_frame(frame).await,None=>Ok(())},
                event=self.incoming.recv()=>match event {
                    Some(Event::Text(g,text)) if g==self.generation=>self.text(text).await,
                    Some(Event::Gathered(g,result)) if g==self.gather_generation=>{
                        self.gather_task=None;self.gathering=false;self.refreshed=Instant::now();
                        match result {
                            Ok(output)=>self.apply_candidates(output.ips,output.sockets,output.gather,output.rebound).await,
                            Err(error)=>tracing::warn!(%error,"media_candidate_refresh_failed"),
                        }
                        Ok(())
                    },
                    Some(Event::Closed(g,replaced)) if g==self.generation=>{self.disconnected(replaced);Ok(())},
                    Some(Event::Rejected(g,status)) if g==self.generation=>{
                        self.socket=None;self.reconnect=None;self.in_room=false;self.room=None;self.reset_media();
                        let mut state=self.state.lock().unwrap();state.busy=false;state.in_room=false;state.invite=None;
                        state.status=match status {403=>"Room access rejected",404|410=>"Update the signaling Worker to support joining rooms by name",_=>"Room authentication failed; check the device clock"}.into();
                        tracing::warn!(status,"signaling_join_rejected");Ok(())
                    }
                    Some(Event::Failed(g, status)) if g == self.generation => {
                        self.disconnected(false);
                        if status == Some(409) && !self.in_room {
                            self.status(ROOM_FULL_STATUS.into());
                        }
                        Ok(())
                    },
                    _=>Ok(()),
                },
                event=self.internal.recv()=>match event {
                    Some(Internal::Datagram{source,destination,data})=>async {
                        let cached = self.udp_routes.get(&(source, destination)).cloned();
                        let peer = if let Some(peer) = cached
                            && self
                                .peers
                                .get(&peer)
                                .is_some_and(|rtc| rtc.accepts(source, destination, &data))
                        {
                            Some(peer)
                        } else {
                            let found = self
                                .peers
                                .iter()
                                .find(|(_, rtc)| rtc.accepts(source, destination, &data))
                                .map(|(id, _)| id.clone());
                            if let Some(peer) = &found {
                                if self.udp_routes.len() >= 1024 {
                                    self.udp_routes.clear();
                                }
                                self.udp_routes.insert((source, destination), peer.clone());
                            } else {
                                self.udp_routes.remove(&(source, destination));
                            }
                            found
                        };
                        if let Some(peer)=peer {
                            if let Some(r)=self.recovery.get_mut(&peer) {r.rx_packets+=1;}
                            match self.peers.get_mut(&peer).unwrap().receive(source,destination,&data) {
                                Ok(())=>{self.drain(&peer).await;Ok(())},
                                Err(error)=>{self.failed_peer(&peer,&format!("receive: {error:#}"));Ok(())}
                            }
                        }else{
                            self.unmatched_datagrams+=1;
                            if self.unmatched_datagrams<=4 || self.unmatched_datagrams.is_power_of_two() {
                                tracing::info!(%source,%destination,bytes=data.len(),total=self.unmatched_datagrams,"media_unmatched_udp");
                            }
                            Ok(())
                        }
                    }.await,
                    _=>Ok(()),
                },
                _=timer.tick()=>self.tick().await,
            };
            if let Err(error) = result {
                tracing::warn!(%error,"cloudflare_session_operation_failed");
                self.state.lock().unwrap().busy = false;
                self.status(format!("Room error: {error:#}"));
            }
        }
        Ok(())
    }
    fn update_discovery_status(&self) {
        let ipv6 = self
            .peers
            .values()
            .filter(|p| p.connected && p.path_version() == Some("IPv6"))
            .count();
        let ipv4 = self
            .peers
            .values()
            .filter(|p| p.connected && p.path_version() == Some("IPv4"))
            .count();
        let nat_note = if self.symmetric_nat {
            " • Symmetric NAT/VPN detected"
        } else {
            ""
        };
        self.state.lock().unwrap().discovery_status = format!(
            "Direct IPv6: {ipv6} • Direct IPv4: {ipv4} • Cloudflare signaling{nat_note}"
        );
    }
    async fn apply_candidates(
        &mut self,
        ips: Vec<std::net::IpAddr>,
        sockets: stun::Sockets,
        gather: stun::GatherResult,
        rebound: bool,
    ) {
        let changed = rebound || ips != self.observed_interfaces;
        let previous: std::collections::HashSet<_> = self
            .candidates
            .iter()
            .map(IceCandidate::to_sdp_string)
            .collect();
        let added = gather
            .candidates
            .iter()
            .filter(|c| !previous.contains(&c.to_sdp_string()))
            .count();
        self.observed_interfaces = ips;
        self.symmetric_nat = gather.symmetric_nat;
        self.stun_mappings = gather.mappings;
        self.sockets = sockets;
        self.candidates = gather.candidates;
        if changed {
            self.udp_routes.clear();
        }
        if self.in_room {
            self.update_discovery_status();
        }
        if added > 0 || changed {
            tracing::info!(
                added,
                interfaces_changed = changed,
                symmetric_nat = self.symmetric_nat,
                "media_candidates_changed"
            );
        }
        for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
            if changed {
                self.failed_peer(&peer, "network interfaces changed");
            } else if let Err(error) = self.sync_candidates(&peer).await {
                tracing::warn!(peer, %error, "sync_candidates_failed");
                self.failed_peer(&peer, &format!("candidate sync: {error:#}"));
            }
        }
    }
    async fn sync_candidates(&mut self, peer: &str) -> Result<()> {
        let Some(recovery) = self.recovery.get(peer) else {
            return Ok(());
        };
        if !recovery.negotiated
            || recovery.retry_at.is_some()
            || self.socket.is_none()
            || self.reconnect.is_some()
        {
            return Ok(());
        }
        let pending: Vec<_> = self
            .candidates
            .iter()
            .filter(|c| !recovery.local_candidates.contains(&c.to_sdp_string()))
            .cloned()
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        // Bound ICE's retained candidate/pair state across repeated ISP mapping changes.
        if recovery.local_candidates.len() + pending.len() > 64 {
            if !recovery.ready {
                self.failed_peer(
                    peer,
                    "candidate refresh limit; rebuild ICE with fresh mappings",
                );
            }
            return Ok(());
        }
        let token = recovery.token.clone();
        let descriptions: Vec<_> = pending.iter().map(IceCandidate::to_sdp_string).collect();
        for candidate in &pending {
            self.peers
                .get_mut(peer)
                .context("Missing media peer")?
                .add_local_candidate(candidate);
            self.drain(peer).await;
        }
        self.send(
            json!({"type":"signal","to":peer,"to_session":self.sessions.get(peer),"kind":"ice",
            "payload":json!({"candidates":descriptions,"p2pshare_attempt":token}).to_string()}),
        )?;
        self.recovery
            .get_mut(peer)
            .unwrap()
            .local_candidates
            .extend(descriptions);
        tracing::info!(
            peer,
            added = pending.len(),
            candidates = %candidate_summary(&pending),
            "media_local_candidates_trickled"
        );
        Ok(())
    }
    async fn remote_candidates(
        &mut self,
        peer: &str,
        token: Option<&str>,
        value: &Value,
    ) -> Result<()> {
        let Some(recovery) = self.recovery.get(peer) else {
            return Ok(());
        };
        if !recovery.negotiated || recovery.retry_at.is_some() || recovery.token.as_deref() != token
        {
            tracing::info!(peer, "media_stale_candidates_ignored");
            return Ok(());
        }
        let descriptions = value.as_array().context("Expected ICE candidate list")?;
        if descriptions.len() > 32 {
            bail!("Too many ICE candidates in one update");
        }
        let mut pending = Vec::new();
        let mut unique = std::collections::HashSet::new();
        for description in descriptions {
            let text = description
                .as_str()
                .context("Expected ICE candidate string")?;
            if text.len() > 1024 {
                bail!("ICE candidate too long");
            }
            if recovery.remote_candidates.contains(text) || !unique.insert(text.to_owned()) {
                continue;
            }
            let candidate = IceCandidate::from_sdp_string(text)?;
            anyhow::ensure!(
                candidate.proto() == p2pshare_rtc::IceProtocol::Udp
                    && candidate.addr().port() != 0
                    && !candidate.addr().ip().is_unspecified()
                    && !candidate.addr().ip().is_multicast(),
                "Invalid UDP ICE candidate"
            );
            pending.push((text.to_owned(), candidate));
        }
        anyhow::ensure!(
            recovery.remote_candidates.len() + pending.len() <= 64,
            "Remote ICE candidate update limit exceeded"
        );
        let count = pending.len();
        let mut added_candidates = Vec::with_capacity(count);
        for (text, candidate) in pending {
            self.peers
                .get_mut(peer)
                .context("Missing media peer")?
                .add_remote_candidate(&candidate);
            self.recovery
                .get_mut(peer)
                .unwrap()
                .remote_candidates
                .insert(text);
            added_candidates.push(candidate);
            self.drain(peer).await;
        }
        if count > 0 {
            tracing::info!(
                peer,
                added = count,
                candidates = %candidate_summary(&added_candidates),
                "media_remote_candidates_added"
            );
        }
        Ok(())
    }
    async fn publish_frame(&mut self, frame: PublishedFrame) -> Result<()> {
        if self.codec != frame.frame.codec {
            self.codec = frame.frame.codec;
            if self.in_room && self.sharing.is_some() {
                self.send_share()?;
            }
        }
        self.frame(frame).await
    }
    fn commit_peer(&mut self, peer: &str, mut rtc: PeerRtc) {
        rtc.set_peer_label(peer.into());
        self.peers.insert(peer.into(), rtc);
        self.udp_routes.retain(|_, id| id != peer);
        self.recovery
            .entry(peer.into())
            .or_insert_with(|| recovery::Recovery::new(Instant::now()));
        self.recovery.get_mut(peer).unwrap().local_candidates = self
            .candidates
            .iter()
            .map(IceCandidate::to_sdp_string)
            .collect();
        self.viewers.remove(peer);
        self.viewers_waiting.remove(peer);
        self.requests.remove(peer);
        let mut state = self.state.lock().unwrap();
        state.viewer_count = self.viewers.len();
        state.connected_video.retain(|id| id != peer);
        let mut inbox = self.inbox.lock().unwrap();
        inbox.stream_viewers.remove(peer);
        // Keep user watch intent and cached JPEG. Drop compressed dependencies from the old session.
        inbox.queues.remove(peer);
        inbox.audio.remove(peer);
        inbox.bytes.remove(peer);
        inbox.waiting.remove(peer);
        inbox.timelines.remove(peer);
    }
    fn replace_peer(&mut self, peer: &str) -> Result<()> {
        let rtc = PeerRtc::new(&self.candidates)?;
        self.commit_peer(peer, rtc);
        Ok(())
    }
    fn failed_peer(&mut self, peer: &str, reason: &str) {
        if let Some(r) = self.recovery.get_mut(peer) {
            if r.retry_at.is_some() {
                return;
            }
            let delay = r.failed(Instant::now());
            tracing::warn!(
                peer,
                reason,
                retry_seconds = delay.as_secs(),
                rx_datagrams = r.rx_packets,
                tx_datagrams = r.tx_packets,
                attempt = r.failures,
                "media_connection_retry_scheduled"
            );
        }
        if let Some(rtc) = self.peers.get_mut(peer) {
            rtc.connected = false;
            rtc.deadline = Instant::now() + Duration::from_secs(3600);
        }
        self.viewers.remove(peer);
        self.viewers_waiting.remove(peer);
        let mut state = self.state.lock().unwrap();
        state.viewer_count = self.viewers.len();
        state.connected_video.retain(|id| id != peer);
        state.status = if self.symmetric_nat {
            "Peer connection failed • retrying; symmetric NAT or VPN detected".into()
        } else {
            "Peer connection failed • retrying direct IPv4/IPv6; firewall or NAT may block UDP"
                .into()
        };
    }
    fn refresh_candidates(&mut self) {
        if self.gathering {
            return;
        }
        self.gathering = true;
        self.gather_generation += 1;
        let sockets = self.sockets.clone();
        let observed = self.observed_interfaces.clone();
        let known = self.stun_mappings.clone();
        let events = self.events.clone();
        let gather_generation = self.gather_generation;
        let datagrams = self.datagrams.clone();
        let isolated = self.isolated;
        self.gather_task = Some(GatherTask(tokio::spawn(async move {
            let result = async {
                let ips = stun::interfaces(isolated)?;
                if ips != observed {
                    let opened = stun::open_with_ips(ips, datagrams).await?;
                    Ok(RefreshOutput {
                        ips: opened.ips,
                        sockets: opened.sockets,
                        gather: opened.gather,
                        rebound: true,
                    })
                } else {
                    let gather = stun::gather(&sockets, &known).await;
                    Ok(RefreshOutput {
                        ips,
                        sockets,
                        gather,
                        rebound: false,
                    })
                }
            }
            .await
            .map_err(|e: anyhow::Error| format!("{e:#}"));
            let _ = events.send(Event::Gathered(gather_generation, result)).await;
        })));
    }
    async fn tick(&mut self) -> Result<()> {
        let mut viewers: Vec<_> = self.viewers.iter().cloned().collect();
        viewers.sort();
        let changed = {
            let mut inbox = self.inbox.lock().unwrap();
            if inbox.stream_viewers.get(&self.local) != Some(&viewers) {
                inbox.stream_viewers.insert(self.local.clone(), viewers);
                true
            } else {
                false
            }
        };
        if changed {
            for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
                self.send_watchers(&peer).await;
            }
        }

        if self.reconnect.is_some_and(|t| Instant::now() >= t) {
            self.connect();
        }
        let now = Instant::now();
        // Keep discovery mappings fresh without spamming DNS/STUN while all peers are healthy.
        let any_unready = self.recovery.values().any(|r| !r.ready);
        let refresh_interval = if any_unready {
            Duration::from_secs(8)
        } else {
            Duration::from_secs(60)
        };
        if self.in_room && self.refreshed.elapsed() >= refresh_interval {
            self.refresh_candidates();
        }
        if self.in_room && self.interface_check.elapsed() >= Duration::from_secs(5) {
            self.interface_check = now;
            self.update_discovery_status();
            if let Ok(ips) = stun::interfaces(self.isolated)
                && ips != self.observed_interfaces
            {
                self.refresh_candidates();
            }
        }
        let expired: Vec<_> = self
            .recovery
            .iter()
            .filter(|(_, r)| r.expired(now))
            .map(|(id, _)| id.clone())
            .collect();
        for peer in expired {
            self.failed_peer(
                &peer,
                "ICE/DTLS/control handshake timed out; no direct path established",
            );
        }
        let due: Vec<_> = self
            .recovery
            .iter()
            .filter(|(_, r)| r.due(now))
            .map(|(id, _)| id.clone())
            .collect();
        if !due.is_empty() && self.in_room && self.socket.is_some() && self.reconnect.is_none() {
            if self.refreshed.elapsed() >= Duration::from_secs(15) {
                self.refresh_candidates();
            }
            if !self.gathering {
                for peer in due {
                    if self.local < peer {
                        let attempt = (|| -> Result<()> {
                            self.replace_peer(&peer)?;
                            let token = format!("{:016x}", rand::random::<u64>());
                            self.recovery
                                .get_mut(&peer)
                                .unwrap()
                                .attempting(now, Some(token.clone()));
                            let sdp = self.peers.get_mut(&peer).unwrap().offer()?;
                            self.signal(&peer, false, tagged_sdp(sdp, Some(&token))?)?;
                            tracing::info!(
                                %peer,
                                local_candidates = %candidate_summary(&self.candidates),
                                "media_direct_connection_retry_started"
                            );
                            Ok(())
                        })();
                        match attempt {
                            Ok(()) => self.drain(&peer).await,
                            Err(error) => {
                                tracing::warn!(%peer, %error, "media_retry_offer_failed");
                                self.failed_peer(&peer, &format!("retry offer: {error:#}"));
                            }
                        }
                    } else {
                        // Ask the designated offerer; never send competing offers.
                        if let Err(error) = self.send(json!({
                            "type": "signal",
                            "to": peer,
                            "to_session": self.sessions.get(&peer),
                            "kind": "ice",
                            "payload": "{\"restart\":true}"
                        })) {
                            tracing::warn!(%peer, %error, "media_retry_restart_send_failed");
                            continue;
                        }
                        self.recovery.get_mut(&peer).unwrap().attempting(now, None);
                    }
                }
            }
        }
        for peer in self
            .peers
            .iter()
            .filter(|(_, r)| r.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
        {
            match self.peers.get_mut(&peer).unwrap().tick() {
                Ok(()) => self.drain(&peer).await,
                Err(error) => self.failed_peer(&peer, &format!("RTC tick: {error:#}")),
            }
        }
        Ok(())
    }
    async fn send_packets(&self, peer: &str, packets: Vec<(SocketAddr, SocketAddr, Vec<u8>)>) {
        for (n, (source, destination, bytes)) in packets.into_iter().enumerate() {
            if let Some(socket) = self.sockets.get(&source) {
                match tokio::time::timeout(
                    Duration::from_millis(5),
                    socket.send_to(&bytes, destination),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(peer, %source, %destination, %error, "Video UDP send failed");
                    }
                    Err(_) => {
                        tracing::warn!(peer, %source, %destination, "Video UDP send stalled for 5 ms");
                    }
                }
            } else {
                tracing::warn!(peer, %source, %destination, "Video UDP source socket missing");
            }
            if n % 32 == 31 {
                tokio::task::yield_now().await;
            }
        }
    }
    async fn drain(&mut self, peer: &str) {
        let mut queue = std::collections::VecDeque::new();
        let mut passes = 0u8;
        loop {
            if passes < 8 {
                let out = match self.peers.get_mut(peer) {
                    Some(rtc) => match rtc.drain() {
                        Ok(out) => out,
                        Err(error) => {
                            self.failed_peer(peer, &format!("RTC output: {error:#}"));
                            return;
                        }
                    },
                    None => return,
                };
                if !out.packets.is_empty() {
                    if let Some(r) = self.recovery.get_mut(peer) {
                        r.tx_packets += out.packets.len() as u64;
                    }
                    self.send_packets(peer, out.packets).await;
                }
                queue.extend(out.events);
                passes += 1;
            }
            let Some(event) = queue.pop_front() else {
                break;
            };
            match event {
                RtcEvent::ControlReady => {
                    // A late successful SCTP handshake can finish during retry backoff.
                    // Its usable encrypted channel proves this transport recovered.
                    if let Some(rtc) = self.peers.get_mut(peer) {
                        rtc.connected = true;
                        rtc.deadline = Instant::now();
                    }
                    {
                        let mut state = self.state.lock().unwrap();
                        if !state.connected_video.iter().any(|id| id == peer) {
                            state.connected_video.push(peer.into());
                        }
                        state.status = "Media connected • encrypted peer-to-peer screens".into();
                    }
                    if let Some(r) = self.recovery.get_mut(peer) {
                        tracing::info!(
                            peer,
                            elapsed_ms = r.started.elapsed().as_millis() as u64,
                            rx_datagrams = r.rx_packets,
                            tx_datagrams = r.tx_packets,
                            "media_connection_ready"
                        );
                        r.connected();
                    }
                    let watching = self.inbox.lock().unwrap().watching.contains(peer);
                    self.write_control_only(peer, &[1, watching as u8], true);
                    self.write_stream_info_only(peer);
                    self.write_watchers_only(peer);
                    self.write_avatar_hash_only(peer);
                    self.write_preview_only(peer);
                }
                RtcEvent::Control(bytes) => {
                    if bytes.len() == 2 && bytes[0] == 1 && bytes[1] <= 1 {
                        if bytes[1] == 1 {
                            if self.viewers.insert(peer.into()) {
                                self.viewers_waiting.insert(peer.into());
                                self.keyframe.store(true, Ordering::Relaxed);
                            }
                        } else {
                            self.viewers.remove(peer);
                            self.viewers_waiting.remove(peer);
                            self.write_preview_only(peer);
                        }
                        self.state.lock().unwrap().viewer_count = self.viewers.len();
                        tracing::info!(peer, watching = bytes[1] == 1, "remote_watch_subscription");
                    } else if bytes.first() == Some(&6) && bytes.len() <= 65 {
                        let hash = String::from_utf8_lossy(&bytes[1..]).to_string();
                        if hash.is_empty() {
                            self.inbox.lock().unwrap().avatar_hashes.remove(peer);
                        } else if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                            let missing = {
                                let mut inbox = self.inbox.lock().unwrap();
                                inbox.avatar_hashes.insert(peer.into(), hash.clone());
                                !inbox.avatar_cache.contains_key(&hash)
                            };
                            if missing {
                                let mut request = vec![7];
                                request.extend(hash.bytes());
                                self.write_control_only(peer, &request, true);
                            }
                        }
                    } else if bytes.first() == Some(&7) && bytes.len() == 65 {
                        let hash = String::from_utf8_lossy(&bytes[1..]).to_string();
                        let avatar = {
                            let inbox = self.inbox.lock().unwrap();
                            if inbox.avatar_hashes.get(&self.local) == Some(&hash) {
                                inbox.avatar_cache.get(&hash).cloned()
                            } else {
                                None
                            }
                        };
                        if let Some(avatar) = avatar {
                            let mut out = vec![8];
                            out.extend_from_slice(&avatar);
                            self.write_control_only(peer, &out, false);
                        }
                    } else if bytes.first() == Some(&8)
                        && bytes.len() <= p2pshare_media::avatar::MAX_BYTES + 1
                    {
                        use sha2::{Digest, Sha256};
                        let hash = format!("{:x}", Sha256::digest(&bytes[1..]));
                        let mut inbox = self.inbox.lock().unwrap();
                        if inbox.avatar_hashes.get(peer) == Some(&hash)
                            && p2pshare_media::avatar::validate(&bytes[1..]).is_ok()
                        {
                            if inbox.avatar_cache.len() > 32 {
                                let keep: std::collections::HashSet<_> =
                                    inbox.avatar_hashes.values().cloned().collect();
                                inbox.avatar_cache.retain(|h, _| keep.contains(h));
                            }
                            inbox.avatar_cache.insert(hash, Arc::from(&bytes[1..]));
                        }
                    } else if bytes.first() == Some(&5) && bytes.len() <= MAX_VIEWER_LIST_BYTES {
                        if let Ok(mut viewers) = serde_json::from_slice::<Vec<String>>(&bytes[1..])
                            && viewers.iter().all(|id| id.len() <= 128)
                        {
                            viewers.sort();
                            viewers.dedup();
                            self.inbox
                                .lock()
                                .unwrap()
                                .stream_viewers
                                .insert(peer.into(), viewers);
                        }
                    } else if bytes.first() == Some(&3)
                        && bytes.len() <= 512
                        && self.members.iter().any(|m| m.id == peer)
                    {
                        if let Ok(info) = serde_json::from_slice::<super::StreamInfo>(&bytes[1..])
                            && info.valid()
                        {
                            self.inbox
                                .lock()
                                .unwrap()
                                .stream_info
                                .insert(peer.into(), info);
                        }
                    } else if bytes.first() == Some(&2)
                        && bytes.len() > 1
                        && bytes.len() <= p2pshare_media::preview::MAX_BYTES + 1
                        && self.members.iter().any(|m| m.id == peer)
                    {
                        self.inbox
                            .lock()
                            .unwrap()
                            .previews
                            .insert(peer.into(), Arc::from(&bytes[1..]));
                    }
                }
                RtcEvent::Connected => {
                    let has_control = if let Some(rtc) = self.peers.get_mut(peer) {
                        rtc.connected = true;
                        rtc.deadline = Instant::now();
                        rtc.has_control_channel()
                    } else {
                        false
                    };
                    if has_control {
                        if let Some(r) = self.recovery.get_mut(peer) {
                            r.connected();
                        }
                        let watching = self.inbox.lock().unwrap().watching.contains(peer);
                        self.write_control_only(peer, &[1, watching as u8], true);
                    }
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
                    self.failed_peer(peer, "ICE connectivity lost");
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
                    if self.inbox.lock().unwrap().watching.contains(peer)
                        && self.members.iter().any(|m| m.id == peer && m.sharing)
                    {
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
                    if !self.inbox.lock().unwrap().watching.contains(peer) {
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
                            peer,
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
                        if let Some(rtc) = self.peers.get_mut(peer)
                            && let Err(error) = rtc.request_keyframe()
                        {
                            tracing::warn!(peer, %error, "request_keyframe_failed");
                        }
                    }
                }
            }
            if passes < 8 && queue.is_empty() {
                // Drain any packets/events produced by control writes or PLI feedback above.
                let out = match self.peers.get_mut(peer) {
                    Some(rtc) => match rtc.drain() {
                        Ok(out) => out,
                        Err(error) => {
                            self.failed_peer(peer, &format!("RTC output: {error:#}"));
                            return;
                        }
                    },
                    None => return,
                };
                if !out.packets.is_empty() {
                    if let Some(r) = self.recovery.get_mut(peer) {
                        r.tx_packets += out.packets.len() as u64;
                    }
                    self.send_packets(peer, out.packets).await;
                }
                queue.extend(out.events);
                passes += 1;
            }
        }
    }
    fn write_control_only(&mut self, peer: &str, bytes: &[u8], priority: bool) {
        if let Some(rtc) = self.peers.get_mut(peer) {
            let res = if priority {
                rtc.send_control_priority(bytes)
            } else {
                rtc.send_control(bytes)
            };
            if let Err(error) = res {
                tracing::warn!(peer, %error, priority, "control_write_failed");
                self.failed_peer(peer, &format!("control write: {error:#}"));
            }
        }
    }
    fn write_avatar_hash_only(&mut self, peer: &str) {
        let hash = self
            .inbox
            .lock()
            .unwrap()
            .avatar_hashes
            .get(&self.local)
            .cloned()
            .unwrap_or_default();
        let mut out = vec![6];
        out.extend(hash.bytes());
        self.write_control_only(peer, &out, true);
    }
    fn write_watchers_only(&mut self, peer: &str) {
        let mut viewers: Vec<_> = self.viewers.iter().collect();
        viewers.sort();
        let mut bytes = vec![5];
        if serde_json::to_writer(&mut bytes, &viewers).is_ok()
            && bytes.len() <= MAX_VIEWER_LIST_BYTES
        {
            self.write_control_only(peer, &bytes, true);
        }
    }
    fn write_stream_info_only(&mut self, peer: &str) {
        let info = self.inbox.lock().unwrap().own_stream_info;
        if let Some(info) = info
            && let Ok(encoded) = serde_json::to_vec(&info)
        {
            let mut bytes = vec![3];
            bytes.extend_from_slice(&encoded);
            self.write_control_only(peer, &bytes, true);
        }
    }
    fn write_preview_only(&mut self, peer: &str) {
        if let Some(jpeg) = self.preview.clone() {
            let mut bytes = Vec::with_capacity(jpeg.len() + 1);
            bytes.push(2);
            bytes.extend_from_slice(&jpeg);
            self.write_control_only(peer, &bytes, false);
        }
    }
    async fn send_priority_control(&mut self, peer: &str, bytes: &[u8]) {
        self.write_control_only(peer, bytes, true);
        self.drain(peer).await;
    }
    async fn send_avatar_hash(&mut self, peer: &str) {
        self.write_avatar_hash_only(peer);
        self.drain(peer).await;
    }
    async fn send_watchers(&mut self, peer: &str) {
        self.write_watchers_only(peer);
        self.drain(peer).await;
    }
    async fn send_stream_info(&mut self, peer: &str) {
        self.write_stream_info_only(peer);
        self.drain(peer).await;
    }
    async fn send_preview(&mut self, peer: &str) {
        self.write_preview_only(peer);
        self.drain(peer).await;
    }
    async fn sound(&mut self, packet: AudioPacket) -> Result<()> {
        if !self.in_room || !self.members.iter().any(|m| m.id == self.local && m.sharing) {
            return Ok(());
        }
        for peer in self.peers.keys().cloned().collect::<Vec<_>>() {
            if !self.viewers.contains(&peer) {
                continue;
            }
            let write = self
                .peers
                .get_mut(&peer)
                .unwrap()
                .write_audio(packet.data.clone(), packet.micros);
            match write {
                Ok(()) => self.drain(&peer).await,
                Err(error) => {
                    tracing::warn!(%peer, %error, "write_audio_failed");
                    self.failed_peer(&peer, &format!("write audio: {error:#}"));
                }
            }
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
            if !self.viewers.contains(&peer) {
                continue;
            }
            if frame.keyframe {
                self.viewers_waiting.remove(&peer);
            }
            if self.viewers_waiting.contains(&peer) {
                continue;
            }
            let rtc = self.peers.get_mut(&peer).unwrap();
            if !rtc.supports_codec(frame.codec) {
                tracing::debug!(%peer, codec = ?frame.codec, "peer_video_codec_unsupported_skipping");
                continue;
            }
            match rtc.write_codec(frame.data.clone(), frame.micros, frame.codec) {
                Ok(()) => self.drain(&peer).await,
                Err(error) => {
                    tracing::warn!(%peer, %error, "write_video_failed");
                    self.failed_peer(&peer, &format!("write video: {error:#}"));
                }
            }
        }
        Ok(())
    }
}

fn candidate_summary(candidates: &[IceCandidate]) -> String {
    candidates
        .iter()
        .map(|c| format!("{:?}:{}", c.kind(), c.addr()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sdp_candidate_summary(payload: &str) -> String {
    let sdp = serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|v| v["sdp"].as_str().map(str::to_owned))
        .unwrap_or_default();
    sdp.lines()
        .filter_map(|line| line.strip_prefix("a=candidate:"))
        .filter_map(|raw| {
            let parts: Vec<_> = raw.split_whitespace().collect();
            if parts.len() >= 8 {
                Some(format!("{}:{}:{}", parts[7], parts[4], parts[5]))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// Extra SDP metadata is ignored by legacy peers and passed through the existing Worker.
fn tagged_sdp(sdp: String, token: Option<&str>) -> Result<String> {
    let mut value: Value = serde_json::from_str(&sdp)?;
    if let Some(token) = token {
        value["p2pshare_attempt"] = json!(token);
    }
    Ok(serde_json::to_string(&value)?)
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    async fn actor(id: char) -> (CloudflareActor, mpsc::Receiver<String>) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/ipv4-fixtures")
            .join(format!("{}-{}", std::process::id(), rand::random::<u64>()));
        let (_, commands) = mpsc::channel(4);
        let (_, video) = mpsc::channel(4);
        let (_, audio) = mpsc::channel(4);
        let mut actor = CloudflareActor::new(
            root.join("identity.key"),
            "ws://127.0.0.1:12345".into(),
            true,
            commands,
            video,
            audio,
            Arc::new(Mutex::new(Snapshot::default())),
            Arc::new(Mutex::new(VideoInbox::default())),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        actor.local = id.to_string().repeat(64);
        actor.own_session = "AAAAAAAAAAAAAAAAAAAAAA".into();
        actor.in_room = true;
        let (sender, receiver) = mpsc::channel(64);
        actor.socket = Some(SocketTask {
            sender,
            task: tokio::spawn(std::future::pending()),
        });
        (actor, receiver)
    }
    fn roster(a: &CloudflareActor, b: &CloudflareActor) -> Vec<RemoteMember> {
        [a, b]
            .into_iter()
            .map(|a| RemoteMember {
                id: a.local.clone(),
                session: a.own_session.clone(),
                name: "Fixture".into(),
                sharing: true,
                width: 160,
                height: 96,
                codec: VideoFormat::H264,
            })
            .collect()
    }
    async fn deliver(
        from: &CloudflareActor,
        to: &mut CloudflareActor,
        rx: &mut mpsc::Receiver<String>,
    ) {
        while let Ok(text) = rx.try_recv() {
            let mut message: Value = serde_json::from_str(&text).unwrap();
            if message["type"] != "signal" {
                continue;
            }
            message["from"] = json!(from.local);
            message["from_session"] = json!(from.own_session);
            to.text(message.to_string()).await.unwrap();
        }
    }
    async fn packets(actor: &mut CloudflareActor) {
        while let Ok(Internal::Datagram {
            source,
            destination,
            data,
        }) = actor.internal.try_recv()
        {
            if let Some(peer) = actor
                .peers
                .iter()
                .find(|(_, r)| r.accepts(source, destination, &data))
                .map(|(id, _)| id.clone())
            {
                actor
                    .peers
                    .get_mut(&peer)
                    .unwrap()
                    .receive(source, destination, &data)
                    .unwrap();
                actor.drain(&peer).await;
            }
        }
    }
    #[tokio::test]
    async fn dropped_offer_recovers_ipv4_preview_and_watch_without_leaving_room() {
        let (mut a, mut a_signals) = actor('a').await;
        let (mut b, mut b_signals) = actor('b').await;
        let peer = b.local.clone();
        let sender = a.local.clone();
        a.roster(roster(&a, &b)).await.unwrap();
        b.roster(roster(&a, &b)).await.unwrap();
        let lost = a_signals.try_recv().unwrap();
        assert!(lost.contains("p2pshare_attempt"));
        a.recovery.get_mut(&peer).unwrap().started = Instant::now() - Duration::from_secs(26);
        a.tick().await.unwrap();
        assert!(a.recovery[&peer].retry_at.is_some());
        a.recovery.get_mut(&peer).unwrap().retry_at = Some(Instant::now());
        a.refreshed = Instant::now();
        a.preview = Some(Arc::from([0xff, 0xd8, 0xff, 0xd9]));
        b.inbox.lock().unwrap().watching.insert(sender.clone());
        a.tick().await.unwrap();
        let until = Instant::now() + Duration::from_secs(8);
        while Instant::now() < until {
            deliver(&a, &mut b, &mut a_signals).await;
            deliver(&b, &mut a, &mut b_signals).await;
            a.tick().await.unwrap();
            b.tick().await.unwrap();
            packets(&mut a).await;
            packets(&mut b).await;
            if a.viewers.contains(&peer) && b.inbox.lock().unwrap().previews.contains_key(&sender) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(a.in_room && b.in_room);
        assert!(a.recovery[&peer].ready && b.recovery[&sender].ready);
        assert!(
            a.viewers.contains(&peer),
            "Watch subscription must be restored"
        );
        assert_eq!(
            &*b.inbox.lock().unwrap().previews[&sender],
            &[0xff, 0xd8, 0xff, 0xd9]
        );
        // A late answer from another attempt cannot consume the current pending offer.
        let token = a.recovery[&peer].token.clone();
        let message = json!({"type":"signal","from":peer,"from_session":b.own_session,"to_session":a.own_session,
            "kind":"answer","payload":json!({"p2pshare_attempt":"stale","type":"answer","sdp":""}).to_string()});
        a.text(message.to_string()).await.unwrap();
        assert_eq!(a.recovery[&peer].token, token);
        // A peer's request causes a bounded retry, not a competing offer from both sides.
        a.recovery.get_mut(&peer).unwrap().started = Instant::now() - Duration::from_secs(6);
        a.text(json!({"type":"signal","from":peer,"from_session":b.own_session,"to_session":a.own_session,"kind":"ice","payload":"{\"restart\":true}"}).to_string()).await.unwrap();
        assert!(a.recovery[&peer].retry_at.is_some());
        assert!(b.inbox.lock().unwrap().watching.contains(&sender));
        a.recovery.get_mut(&peer).unwrap().retry_at = Some(Instant::now());
        a.tick().await.unwrap();
        let until = Instant::now() + Duration::from_secs(8);
        while Instant::now() < until {
            deliver(&a, &mut b, &mut a_signals).await;
            deliver(&b, &mut a, &mut b_signals).await;
            a.tick().await.unwrap();
            b.tick().await.unwrap();
            packets(&mut a).await;
            packets(&mut b).await;
            if a.viewers.contains(&peer) && a.recovery[&peer].ready && b.recovery[&sender].ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(a.viewers.contains(&peer));
        assert!(a.recovery[&peer].ready && b.recovery[&sender].ready);
        assert_ne!(a.recovery[&peer].token, token);
        let until=Instant::now()+Duration::from_secs(3);
        while Instant::now()<until {
            a.tick().await.unwrap();b.tick().await.unwrap();packets(&mut a).await;packets(&mut b).await;
            if b.inbox.lock().unwrap().stream_viewers.get(&sender)==Some(&vec![peer.clone()]){break;}
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(b.inbox.lock().unwrap().stream_viewers.get(&sender),Some(&vec![peer.clone()]));
        b.inbox.lock().unwrap().watching.remove(&sender);
        b.command(Command::Watch{peer:sender.clone(),enabled:false}).await.unwrap();
        let until=Instant::now()+Duration::from_secs(3);
        while Instant::now()<until {
            a.tick().await.unwrap();b.tick().await.unwrap();packets(&mut a).await;packets(&mut b).await;
            if b.inbox.lock().unwrap().stream_viewers.get(&sender)==Some(&Vec::new()){break;}
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(a.viewers.is_empty());assert_eq!(b.inbox.lock().unwrap().stream_viewers.get(&sender),Some(&Vec::new()));
        let image=p2pshare_media::avatar::Source::from_raw(256,256,[40u8,80,120,255].repeat(256*256)).unwrap();
        let jpeg=p2pshare_media::avatar::crop(&image,1.,0.5,0.5).unwrap();
        a.command(Command::Avatar(Some(jpeg.clone()))).await.unwrap();
        let until=Instant::now()+Duration::from_secs(3);
        while Instant::now()<until {
            a.tick().await.unwrap();b.tick().await.unwrap();packets(&mut a).await;packets(&mut b).await;
            let ready={let inbox=b.inbox.lock().unwrap();inbox.avatar_hashes.get(&sender).and_then(|h|inbox.avatar_cache.get(h)).is_some_and(|bytes|bytes.as_ref()==jpeg.as_slice())};
            if ready{break;}tokio::time::sleep(Duration::from_millis(2)).await;
        }
        {let inbox=b.inbox.lock().unwrap();let hash=inbox.avatar_hashes.get(&sender).expect("Avatar announced");assert_eq!(inbox.avatar_cache[hash].as_ref(),jpeg.as_slice());}
        a.command(Command::Avatar(None)).await.unwrap();
        let until=Instant::now()+Duration::from_secs(3);
        while Instant::now()<until{packets(&mut a).await;packets(&mut b).await;if !b.inbox.lock().unwrap().avatar_hashes.contains_key(&sender){break;}tokio::time::sleep(Duration::from_millis(2)).await;}
        assert!(!b.inbox.lock().unwrap().avatar_hashes.contains_key(&sender));

    }
    #[tokio::test]
    async fn refreshed_mapping_is_sent_to_existing_peer_and_stale_updates_are_ignored() {
        let (mut a, mut signals) = actor('a').await;
        let (mut b, mut b_signals) = actor('b').await;
        a.roster(roster(&a, &b)).await.unwrap();
        b.roster(roster(&a, &b)).await.unwrap();
        let original = signals.try_recv().unwrap();
        let offer: Value = serde_json::from_str(&original).unwrap();
        let offer_payload = offer["payload"].as_str().unwrap();
        let token: Value = serde_json::from_str(offer_payload).unwrap();
        // Use the real signal handler to negotiate the retained session.
        b.text(json!({"type":"signal","from":a.local,"from_session":a.own_session,"to_session":b.own_session,"kind":"offer","payload":offer_payload}).to_string()).await.unwrap();
        let a_peer = b.local.clone();
        let b_peer = a.local.clone();
        deliver(&b, &mut a, &mut b_signals).await;
        assert!(a.recovery[&a_peer].negotiated);
        let base = *a.sockets.keys().next().unwrap();
        let new =
            p2pshare_rtc::reflexive_candidate("203.0.113.10:41001".parse().unwrap(), base).unwrap();
        let description = new.to_sdp_string();
        let mut candidates = a.candidates.clone();
        candidates.push(new);
        a.apply_candidates(
            a.observed_interfaces.clone(),
            a.sockets.clone(),
            stun::GatherResult {
                candidates: candidates.clone(),
                symmetric_nat: false,
                mappings: Default::default(),
            },
            false,
        )
        .await;
        let message: Value = serde_json::from_str(&signals.try_recv().unwrap()).unwrap();
        assert_eq!(message["kind"], "ice");
        let payload: Value = serde_json::from_str(message["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["p2pshare_attempt"], token["p2pshare_attempt"]);
        assert_eq!(payload["candidates"], json!([description]));
        b.remote_candidates(&b_peer, Some("stale"), &payload["candidates"])
            .await
            .unwrap();
        assert!(b.recovery[&b_peer].remote_candidates.is_empty());
        b.remote_candidates(
            &b_peer,
            payload["p2pshare_attempt"].as_str(),
            &payload["candidates"],
        )
        .await
        .unwrap();
        assert_eq!(b.recovery[&b_peer].remote_candidates.len(), 1);
        b.remote_candidates(
            &b_peer,
            payload["p2pshare_attempt"].as_str(),
            &payload["candidates"],
        )
        .await
        .unwrap();
        assert_eq!(b.recovery[&b_peer].remote_candidates.len(), 1);
        a.apply_candidates(
            a.observed_interfaces.clone(),
            a.sockets.clone(),
            stun::GatherResult {
                candidates,
                symmetric_nat: false,
                mappings: Default::default(),
            },
            false,
        )
        .await;
        assert!(
            signals.try_recv().is_err(),
            "Unchanged mapping must not generate signaling traffic"
        );
        let too_many = json!(vec![description; 33]);
        assert!(
            b.remote_candidates(&b_peer, payload["p2pshare_attempt"].as_str(), &too_many)
                .await
                .is_err()
        );
    }
}
