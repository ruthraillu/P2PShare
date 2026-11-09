//! Encrypted, authenticated control transport. Video is carried separately by WebRTC.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::{
    Endpoint, EndpointAddr, SecretKey,
    endpoint::{Connection, presets},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{path::Path, time::Duration};
pub const ALPN: &[u8] = b"p2pshare/room/3";
pub const MESH_ALPN: &[u8] = b"p2pshare/room/4";
pub use iroh::EndpointAddr as PeerAddress;
pub fn new_peer_id() -> iroh::EndpointId {
    SecretKey::generate().public()
}
pub const MAX_MESSAGE: usize = 64 * 1024;
pub use iroh::endpoint::{RecvStream, SendStream};
#[derive(Clone, Serialize, Deserialize)]
pub struct Invite {
    pub version: u8,
    pub token: String,
    pub coordinator: EndpointAddr,
}
impl Invite {
    pub fn encode(&self) -> Result<String> {
        Ok(format!(
            "p2pshare://join/{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?)
        ))
    }
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() > 16_384 {
            bail!("Invitation is too long");
        }
        let data = URL_SAFE_NO_PAD.decode(
            text.strip_prefix("p2pshare://join/")
                .context("Paste a P2PShare room invitation")?,
        )?;
        let invite: Self = serde_json::from_slice(&data)?;
        if invite.version != 3 || invite.token.len() != 43 {
            bail!("Unsupported or invalid room invitation");
        }
        Ok(invite)
    }
    pub fn new(endpoint: &Endpoint) -> Self {
        Self {
            version: 3,
            token: URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()),
            coordinator: endpoint.addr(),
        }
    }
}
pub async fn bind(identity: &Path, isolated: bool) -> Result<Endpoint> {
    let key = match std::fs::read(identity) {
        Ok(bytes) => SecretKey::from_bytes(
            &bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid identity file"))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            if let Some(parent) = identity.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(identity, key.to_bytes())?;
            key
        }
        Err(e) => return Err(e.into()),
    };
    let builder = if isolated {
        Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .bind_addr("127.0.0.1:0")?
    } else {
        Endpoint::builder(presets::N0)
    };
    let ep = builder
        .secret_key(key)
        .alpns(vec![ALPN.to_vec(), MESH_ALPN.to_vec()])
        .bind()
        .await?;
    // LAN works without a relay; give the public control relay a bounded opportunity to come online.
    if !isolated {
        let _ = tokio::time::timeout(Duration::from_secs(8), ep.online()).await;
    }
    Ok(ep)
}
pub async fn connect(endpoint: &Endpoint, invite: &Invite) -> Result<Connection> {
    Ok(tokio::time::timeout(
        Duration::from_secs(20),
        endpoint.connect(invite.coordinator.clone(), ALPN),
    )
    .await
    .context("Room connection timed out")??)
}
pub async fn send<T: Serialize>(stream: &mut SendStream, message: &T) -> Result<()> {
    let bytes = serde_json::to_vec(message)?;
    if bytes.len() > MAX_MESSAGE {
        bail!("Control message exceeds limit");
    }
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await?;
    stream.write_all(&bytes).await?;
    Ok(())
}
pub async fn receive<T: DeserializeOwned>(stream: &mut RecvStream) -> Result<T> {
    let mut size = [0; 4];
    stream.read_exact(&mut size).await?;
    let size = u32::from_be_bytes(size) as usize;
    if size > MAX_MESSAGE || size == 0 {
        bail!("Invalid control message length");
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn peer_id(connection: &Connection) -> String {
    connection.remote_id().to_string()
}
pub use iroh::Endpoint as ControlEndpoint;
pub use iroh::endpoint::Connection as ControlConnection;

pub async fn connect_peer(endpoint: &Endpoint, address: EndpointAddr) -> Result<Connection> {
    Ok(tokio::time::timeout(
        Duration::from_secs(12),
        endpoint.connect(address, MESH_ALPN),
    )
    .await
    .context("Peer connection timed out")??)
}

pub const ROOM_FULL_CODE: u32 = 4;
pub fn room_full_close(connection: &Connection) -> bool {
    matches!(connection.close_reason(), Some(iroh::endpoint::ConnectionError::ApplicationClosed(closed)) if closed.error_code == ROOM_FULL_CODE.into())
}
