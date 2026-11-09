//! Portable transport contracts. No live networking is started in Phase 0/1.
use p2pshare_protocol::{Envelope, PeerId};
use std::{net::SocketAddr, time::Duration};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectivityKind {
    Native,
    Iroh,
    PeerRelay,
    Turn,
    Tailcat,
    Yggdrasil,
}
#[derive(Clone, Debug)]
pub struct PeerAddress {
    pub peer: PeerId,
    pub direct_candidates: Vec<SocketAddr>,
    pub bootstrap: Vec<u8>,
}
#[derive(Clone, Debug, Default)]
pub struct NetworkStats {
    pub rtt: Duration,
    pub jitter: Duration,
    pub packet_loss: f32,
    pub estimated_bandwidth_bps: u64,
    pub queue_delay: Duration,
    pub retransmissions: u64,
}
#[derive(Clone, Debug)]
pub struct NetworkPath {
    pub id: u64,
    pub kind: ConnectivityKind,
    pub local_peer: PeerId,
    pub remote_peer: PeerId,
    pub stats: NetworkStats,
    pub direct: bool,
    pub ipv6: bool,
    pub stable_since: Duration,
}
#[derive(Clone, Debug)]
pub struct RelayConfiguration {
    pub automatic: bool,
    pub custom_relays: Vec<String>,
    pub disable_relays: bool,
}
impl Default for RelayConfiguration {
    fn default() -> Self {
        Self {
            automatic: true,
            custom_relays: vec![],
            disable_relays: false,
        }
    }
}
#[derive(Default, Debug)]
pub struct PathManager {
    pub paths: Vec<NetworkPath>,
    pub active: Option<u64>,
}
pub trait ConnectivityBackend {
    type Error: std::error::Error;
    fn discover(&mut self, peer: &PeerAddress) -> Result<Vec<NetworkPath>, Self::Error>;
    fn stats(&self) -> NetworkStats;
}
pub trait ControlTransport {
    type Error: std::error::Error;
    fn listen(&mut self) -> Result<PeerAddress, Self::Error>;
    fn connect(&mut self, address: &PeerAddress) -> Result<(), Self::Error>;
    fn send(&mut self, peer: PeerId, message: &Envelope) -> Result<(), Self::Error>;
    fn disconnect(&mut self, peer: PeerId) -> Result<(), Self::Error>;
    fn stats(&self, peer: PeerId) -> Option<NetworkStats>;
}
pub trait MediaTransport {
    type Error: std::error::Error;
    fn send_datagram(&mut self, peer: PeerId, payload: &[u8]) -> Result<(), Self::Error>;
    fn receive_datagram(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<Option<(PeerId, usize)>, Self::Error>;
    fn stats(&self) -> NetworkStats;
}
