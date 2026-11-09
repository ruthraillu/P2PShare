//! Bounded, encrypted room presence over Nostr. Records are hints, never membership.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use futures_util::{SinkExt, StreamExt};
use hkdf::Hkdf;
use nostr::prelude::*;
use p2pshare_control_iroh::PeerAddress;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};
use tokio_websockets::{ClientBuilder, Limits, Message};

const DOMAIN: &[u8] = b"p2pshare/rooms/v4";
const KIND: u16 = 30378;
const LEASE: u64 = 120;
pub const DEFAULT_RELAYS: [&str; 3] = [
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
];

/// Capability link, independent of any participant's identity or network address.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomInvite {
    pub version: u8,
    pub secret: String,
    pub relays: Vec<String>,
}
impl RoomInvite {
    pub fn new(relays: Vec<String>) -> Result<Self> {
        let invite = Self {
            version: 4,
            secret: URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()),
            relays,
        };
        invite.validate()?;
        Ok(invite)
    }
    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        Ok(format!(
            "p2pshare://room/{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?)
        ))
    }
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.trim().len() <= 4096, "Invitation is too long");
        let bytes = URL_SAFE_NO_PAD.decode(
            text.trim()
                .strip_prefix("p2pshare://room/")
                .context("Paste a reusable P2PShare invitation")?,
        )?;
        let invite: Self = serde_json::from_slice(&bytes)?;
        invite.validate()?;
        Ok(invite)
    }
    fn secret_bytes(&self) -> Result<[u8; 32]> {
        URL_SAFE_NO_PAD
            .decode(&self.secret)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid room capability"))
    }
    fn validate(&self) -> Result<()> {
        ensure!(self.version == 4, "Unsupported room invitation");
        ensure!(self.secret.len() == 43, "Invalid room capability");
        self.secret_bytes()?;
        ensure!(
            !self.relays.is_empty() && self.relays.len() <= 3,
            "Use one to three discovery relays"
        );
        ensure!(
            self.relays
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.relays.len(),
            "Duplicate discovery relay"
        );
        for relay in &self.relays {
            validate_relay(relay)?;
        }
        Ok(())
    }
    pub fn topic(&self) -> Result<String> {
        Ok(URL_SAFE_NO_PAD.encode(derive(&self.secret_bytes()?, b"lookup")))
    }
}
fn validate_relay(text: &str) -> Result<()> {
    ensure!(text.len() <= 256, "Relay URL too long");
    let uri: http::Uri = text.parse()?;
    let host = uri.host().context("Missing relay host")?;
    ensure!(
        !uri.authority().unwrap().as_str().contains('@')
            && uri.path_and_query().is_none_or(|p| p.query().is_none()),
        "Relay URL must not contain credentials or a query"
    );
    ensure!(
        uri.scheme_str() == Some("wss")
            || (uri.scheme_str() == Some("ws")
                && matches!(host, "127.0.0.1" | "localhost" | "[::1]")),
        "Discovery requires WSS (WS allowed only on loopback)"
    );
    Ok(())
}
fn derive(secret: &[u8; 32], label: &[u8]) -> [u8; 32] {
    let mut bytes = [0; 32];
    Hkdf::<Sha256>::new(Some(DOMAIN), secret)
        .expand(label, &mut bytes)
        .expect("fixed HKDF length");
    bytes
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub version: u8,
    pub address: PeerAddress,
    pub expires: u64,
}
impl Presence {
    pub fn new(address: PeerAddress) -> Self {
        Self {
            version: 4,
            address,
            expires: now() + LEASE,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let n = now();
        ensure!(
            self.version == 4 && self.expires > n && self.expires <= n + LEASE + 30,
            "Invalid presence lease"
        );
        ensure!(
            self.address.addrs.len() <= 12 && serde_json::to_vec(&self.address)?.len() <= 4096,
            "Invalid endpoint address"
        );
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub enum Notice {
    Peer(Presence),
    Relay { index: usize, available: bool },
}
/// Dropping this handle cancels every subscription/retry on leave or room change.
pub struct Discovery {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Discovery {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Discovery {
    pub fn start(
        invite: RoomInvite,
        device_seed: [u8; 32],
        address: watch::Receiver<PeerAddress>,
        notices: mpsc::Sender<Notice>,
        clock_file: &Path,
    ) -> Result<Self> {
        invite.validate()?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        // A different signing identity for each device AND each room. Never derive it from the shared capability alone.
        let topic = invite.topic()?;
        let writer = derive(&device_seed, topic.as_bytes());
        let keys = Keys::new(SecretKey::from_slice(&writer)?);
        // NIP-01 replaces by second-resolution timestamp. Persist a shared clock so
        // rapid app restarts cannot lose a new endpoint to an old record's ID tie-break.
        let previous = std::fs::read(clock_file)
            .ok()
            .and_then(|b| b.try_into().ok())
            .map(u64::from_be_bytes)
            .filter(|t| *t <= now() + 10)
            .unwrap_or(0);
        let clock = Arc::new(tokio::sync::Mutex::new((
            previous,
            clock_file.to_path_buf(),
        )));
        let mut tasks = Vec::new();
        for (index, relay) in invite.relays.iter().cloned().enumerate() {
            let invite = invite.clone();
            let clock = clock.clone();
            let keys = keys.clone();
            let address = address.clone();
            let notices = notices.clone();
            tasks.push(tokio::spawn(async move {
                let mut delay = 2u64;
                loop {
                    let connected = relay_session(
                        index,
                        &relay,
                        &invite,
                        &keys,
                        address.clone(),
                        &notices,
                        &clock,
                    )
                    .await;
                    let _ = notices.try_send(Notice::Relay {
                        index,
                        available: false,
                    });
                    if notices.is_closed() {
                        break;
                    }
                    if connected.is_ok() {
                        delay = 2;
                    }
                    tokio::time::sleep(
                        Duration::from_secs(delay)
                            + Duration::from_millis(rand::random::<u16>() as u64 % 1000),
                    )
                    .await;
                    delay = (delay * 2).min(60);
                }
            }));
        }
        Ok(Self { tasks })
    }
}
fn seal(invite: &RoomInvite, presence: &Presence) -> Result<String> {
    let nonce: [u8; 12] = rand::random();
    let key = derive(&invite.secret_bytes()?, b"encryption");
    let cipher = ChaCha20Poly1305::new((&key).into());
    let topic = invite.topic()?;
    let data = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &serde_json::to_vec(presence)?,
                aad: topic.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("Presence encryption failed"))?;
    let mut bytes = nonce.to_vec();
    bytes.extend(data);
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
pub fn signed_presence(invite: &RoomInvite, keys: &Keys, presence: &Presence) -> Result<Event> {
    signed_presence_at(invite, keys, presence, now())
}
fn signed_presence_at(
    invite: &RoomInvite,
    keys: &Keys,
    presence: &Presence,
    created: u64,
) -> Result<Event> {
    presence.validate()?;
    Ok(EventBuilder::new(Kind::from(KIND), seal(invite, presence)?)
        .custom_created_at(Timestamp::from(created))
        .tags([
            Tag::identifier(invite.topic()?),
            Tag::parse(["t", &invite.topic()?])?,
            Tag::expiration(Timestamp::from(presence.expires)),
        ])
        .finalize(keys)?)
}
pub fn verify_presence(invite: &RoomInvite, event: &Event) -> Result<Presence> {
    ensure!(
        event.kind == Kind::from(KIND) && event.content.len() <= 8192 && event.tags.len() <= 8,
        "Invalid presence event"
    );
    let topic = invite.topic()?;
    ensure!(
        event.tags.iter().any(|t| t.as_slice() == ["t", &topic])
            && event.tags.iter().any(|t| t.as_slice() == ["d", &topic]),
        "Wrong room topic"
    );
    let n = now();
    ensure!(
        event.created_at.as_secs() <= n + 30 && event.created_at.as_secs() + LEASE + 30 > n,
        "Stale or future presence event"
    );
    event.verify()?;
    let bytes = URL_SAFE_NO_PAD.decode(&event.content)?;
    ensure!(bytes.len() >= 28, "Truncated presence");
    let key = derive(&invite.secret_bytes()?, b"encryption");
    let cipher = ChaCha20Poly1305::new((&key).into());
    let plain = cipher
        .decrypt(
            Nonce::from_slice(&bytes[..12]),
            Payload {
                msg: &bytes[12..],
                aad: topic.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("Presence authentication failed"))?;
    let presence: Presence = serde_json::from_slice(&plain)?;
    presence.validate()?;
    ensure!(
        event
            .tags
            .iter()
            .any(|t| t.as_slice() == ["expiration", &presence.expires.to_string()]),
        "Lease mismatch"
    );
    Ok(presence)
}
type PublishClock = Arc<tokio::sync::Mutex<(u64, PathBuf)>>;
async fn next_presence(
    invite: &RoomInvite,
    keys: &Keys,
    address: PeerAddress,
    clock: &PublishClock,
) -> Result<Event> {
    let mut state = clock.lock().await;
    let current = now();
    if state.0 >= current {
        tokio::time::sleep(Duration::from_secs(state.0 - current + 1)).await;
    }
    let created = now();
    let event = signed_presence_at(invite, keys, &Presence::new(address), created)?;
    std::fs::write(&state.1, created.to_be_bytes())?;
    state.0 = created;
    Ok(event)
}
#[allow(clippy::too_many_arguments)]
async fn relay_session(
    index: usize,
    relay: &str,
    invite: &RoomInvite,
    keys: &Keys,
    mut address: watch::Receiver<PeerAddress>,
    notices: &mpsc::Sender<Notice>,
    clock: &PublishClock,
) -> Result<()> {
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(12),
        ClientBuilder::new()
            .uri(relay)?
            .limits(Limits::default().max_payload_len(Some(64 * 1024)))
            .connect(),
    )
    .await??;
    let sub = "p2pshare-v4";
    socket.send(Message::text(json!(["REQ",sub,{"kinds":[KIND],"#t":[invite.topic()?],"since":now().saturating_sub(LEASE+30),"limit":64}]).to_string())).await?;
    let mut heartbeat =
        tokio::time::interval(Duration::from_secs(20 + rand::random::<u8>() as u64 % 5));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut recent: HashMap<String, (u64, std::cmp::Reverse<String>)> = HashMap::new();
    let mut pending_id = String::new();
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut accepted = false;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                // A socket that silently stops processing is unusable even if TCP remains open.
                anyhow::bail!("Discovery relay acknowledgement timed out");
            }
            _ = heartbeat.tick() => {
                let current = address.borrow().clone();
                let event = next_presence(invite, keys, current, clock).await?;
                pending_id = event.id.to_hex(); accepted = false;
                deadline = tokio::time::Instant::now()+Duration::from_secs(15);
                socket.send(Message::text(json!(["EVENT",event]).to_string())).await?;
            }
            changed = address.changed() => {
                if changed.is_err() { return Ok(()); }
                // Beat immediately so changed Iroh addresses reach new joiners.
                heartbeat.reset_immediately();
            }
            message = socket.next() => {
                let message = message.context("Discovery disconnected")??;
                if message.is_close() { return Ok(()); }
                let Some(text) = message.as_text() else { continue; };
                let Ok(value) = serde_json::from_str::<Value>(text) else { continue; };
                match value[0].as_str() {
                    Some("OK") if value[1].as_str() == Some(&pending_id) => {
                        ensure!(value[2] == true, "Relay rejected presence");
                        accepted = true;
                        deadline = tokio::time::Instant::now()+Duration::from_secs(45);
                        let _ = notices.try_send(Notice::Relay { index, available: true });
                    }
                    Some("EOSE") if value[1].as_str() == Some(sub) && accepted => { let _ = notices.try_send(Notice::Relay { index, available: true }); }
                    Some("AUTH" | "CLOSED") => anyhow::bail!("Relay requires authentication or closed subscription"),
                    Some("EVENT") if value[1].as_str() == Some(sub) => {
                        let Ok(event) = serde_json::from_value::<Event>(value[2].clone()) else { continue; };
                        let Ok(presence) = verify_presence(invite, &event) else { continue; };
                        if presence.address.id == address.borrow().id { continue; }
                        let order = (event.created_at.as_secs(), std::cmp::Reverse(event.id.to_hex()));
                        recent.retain(|_,(t,_)| *t+LEASE+30 > now());
                        if recent.get(&event.pubkey.to_hex()).is_some_and(|old| old >= &order) { continue; }
                        if recent.len() >= 64 && !recent.contains_key(&event.pubkey.to_hex()) { continue; }
                        recent.insert(event.pubkey.to_hex(), order);
                        let _ = notices.try_send(Notice::Peer(presence));
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_signatures_do_not_allow_stale_future_or_wrong_cipher_records() {
        let invite =
            RoomInvite::new(DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()).unwrap();
        let keys = Keys::generate();
        let mut p = Presence::new(PeerAddress::new(p2pshare_control_iroh::new_peer_id()));
        let future = signed_presence_at(&invite, &keys, &p, now() + 31).unwrap();
        assert!(verify_presence(&invite, &future).is_err());
        p.expires = now() - 1;
        let expired = EventBuilder::new(Kind::from(KIND), seal(&invite, &p).unwrap())
            .tags([
                Tag::identifier(invite.topic().unwrap()),
                Tag::parse(["t", &invite.topic().unwrap()]).unwrap(),
                Tag::expiration(Timestamp::from(p.expires)),
            ])
            .finalize(&keys)
            .unwrap();
        expired.verify().unwrap();
        assert!(verify_presence(&invite, &expired).is_err());
        p.expires = now() + 120;
        let wrong = RoomInvite::new(invite.relays.clone()).unwrap();
        let wrong_cipher = EventBuilder::new(Kind::from(KIND), seal(&wrong, &p).unwrap())
            .tags([
                Tag::identifier(invite.topic().unwrap()),
                Tag::parse(["t", &invite.topic().unwrap()]).unwrap(),
                Tag::expiration(Timestamp::from(p.expires)),
            ])
            .finalize(&keys)
            .unwrap();
        wrong_cipher.verify().unwrap();
        assert!(verify_presence(&invite, &wrong_cipher).is_err());
    }
    #[test]
    fn restart_replaces_a_cached_endpoint_even_within_the_same_second() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/discovery-tests")
            .join(format!("{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("presence-clock");
        let invite =
            RoomInvite::new(DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()).unwrap();
        let keys = Keys::generate();
        let id = p2pshare_control_iroh::new_peer_id();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let clock = Arc::new(tokio::sync::Mutex::new((0, path.clone())));
                let a = next_presence(
                    &invite,
                    &keys,
                    PeerAddress::new(id).with_ip_addr("127.0.0.1:10101".parse().unwrap()),
                    &clock,
                )
                .await
                .unwrap();
                drop(clock);
                let saved = u64::from_be_bytes(std::fs::read(&path).unwrap().try_into().unwrap());
                let clock = Arc::new(tokio::sync::Mutex::new((saved, path.clone())));
                let b = next_presence(
                    &invite,
                    &keys,
                    PeerAddress::new(id).with_ip_addr("127.0.0.1:20202".parse().unwrap()),
                    &clock,
                )
                .await
                .unwrap();
                assert!(b.created_at > a.created_at);
                assert!(b.created_at.as_secs() <= now());
                let a = verify_presence(&invite, &a).unwrap();
                let b = verify_presence(&invite, &b).unwrap();
                assert_eq!(a.address.id, b.address.id);
                assert_ne!(a.address.addrs, b.address.addrs);
            });
    }
    #[test]
    fn reusable_invite_and_authenticated_presence() {
        let invite =
            RoomInvite::new(DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()).unwrap();
        assert_eq!(
            RoomInvite::parse(&invite.encode().unwrap())
                .unwrap()
                .topic()
                .unwrap(),
            invite.topic().unwrap()
        );
        let peer = PeerAddress::new(p2pshare_control_iroh::new_peer_id());
        let presence = Presence::new(peer);
        let event = signed_presence(&invite, &Keys::generate(), &presence).unwrap();
        assert_eq!(
            verify_presence(&invite, &event).unwrap().address.id,
            presence.address.id
        );
        let wrong = RoomInvite::new(invite.relays.clone()).unwrap();
        assert!(verify_presence(&wrong, &event).is_err());
        let mut damaged = serde_json::to_value(&event).unwrap();
        damaged["content"] = json!("altered");
        assert!(verify_presence(&invite, &serde_json::from_value(damaged).unwrap()).is_err());
        let mut stale = presence;
        stale.expires = now() - 1;
        assert!(signed_presence(&invite, &Keys::generate(), &stale).is_err());
        let mut unsafe_invite = invite;
        unsafe_invite.relays = vec!["ws://example.org".into()];
        assert!(unsafe_invite.encode().is_err());
        unsafe_invite.relays = vec!["wss://user:pass@example.org".into()];
        assert!(unsafe_invite.encode().is_err());
    }
}
