//! Shared UDP sockets with bounded STUN discovery. The receive task demultiplexes
//! discovery replies so refreshing candidates never consumes another peer's RTP.
use super::*;
use tokio::sync::oneshot;
const COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];
const SERVERS: [&str; 3] = [
    "stun.cloudflare.com:3478",
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
];
type Transaction = [u8; 12];
type Pending = Arc<Mutex<HashMap<Transaction, (SocketAddr, oneshot::Sender<SocketAddr>)>>>;
pub(super) type Sockets = HashMap<SocketAddr, Arc<MediaSocket>>;
pub(super) struct MediaSocket {
    udp: Arc<UdpSocket>,
    pending: Pending,
    receiver: tokio::task::JoinHandle<()>,
}
impl std::ops::Deref for MediaSocket {
    type Target = UdpSocket;
    fn deref(&self) -> &Self::Target {
        &self.udp
    }
}
impl Drop for MediaSocket {
    fn drop(&mut self) {
        self.receiver.abort();
    }
}
struct Registration {
    pending: Pending,
    transaction: Transaction,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.transaction);
    }
}
fn binding_response(bytes: &[u8], transaction: Transaction) -> Option<SocketAddr> {
    if bytes.len() < 20
        || bytes[..2] != [1, 1]
        || bytes[4..8] != COOKIE
        || bytes[8..20] != transaction
    {
        return None;
    }
    let size = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    if !size.is_multiple_of(4) || size + 20 != bytes.len() {
        return None;
    }
    let mut pos = 20;
    let mut mapped = None;
    while pos + 4 <= bytes.len() {
        let kind = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
        let len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        pos += 4;
        let value = bytes.get(pos..pos.checked_add(len)?)?;
        if kind == 0x20 && len == 8 && value[0] == 0 && value[1] == 1 {
            let port = u16::from_be_bytes([value[2], value[3]]) ^ 0x2112;
            let ip = std::net::Ipv4Addr::new(
                value[4] ^ COOKIE[0],
                value[5] ^ COOKIE[1],
                value[6] ^ COOKIE[2],
                value[7] ^ COOKIE[3],
            );
            if port != 0 && !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast() {
                mapped = Some(SocketAddr::new(ip.into(), port));
            }
        }
        pos += (len + 3) & !3;
    }
    (pos == bytes.len()).then_some(mapped).flatten()
}
fn dispatch(pending: &Pending, source: SocketAddr, bytes: &[u8]) -> bool {
    // Ordinary RTP/DTLS never takes the pending-transaction mutex.
    if bytes.len() < 20 || bytes[..2] != [1, 1] || bytes[4..8] != COOKIE {
        return false;
    }
    let Some(transaction) = bytes.get(8..20).and_then(|v| v.try_into().ok()) else {
        return false;
    };
    let mut entries = pending.lock().unwrap();
    let Some((expected, _)) = entries.get(&transaction) else {
        return false;
    };
    if *expected != source {
        return false;
    }
    let Some(mapped) = binding_response(bytes, transaction) else {
        return false;
    };
    if let Some((_, reply)) = entries.remove(&transaction) {
        let _ = reply.send(mapped);
    }
    true
}
pub(super) fn interfaces(isolated: bool) -> Result<Vec<std::net::IpAddr>> {
    Ok(network::select(
        if_addrs::get_if_addrs()?
            .into_iter()
            .map(|i| i.ip())
            .collect(),
        isolated,
    ))
}
async fn bind(
    ip: std::net::IpAddr,
    events: mpsc::Sender<Internal>,
) -> Result<(SocketAddr, Arc<MediaSocket>)> {
    let udp = Arc::new(UdpSocket::bind(SocketAddr::new(ip, 0)).await?);
    let options = socket2::SockRef::from(&*udp);
    if let Err(error) = options.set_recv_buffer_size(4 * 1024 * 1024) {
        tracing::warn!(%error, "Could not enlarge video receive buffer");
    }
    if let Err(error) = options.set_send_buffer_size(512 * 1024) {
        tracing::warn!(%error, "Could not enlarge video send buffer");
    }
    let addr = udp.local_addr()?;
    let pending: Pending = Default::default();
    let table = pending.clone();
    let socket = udp.clone();
    let receiver = tokio::spawn(async move {
        let mut buf = [0; 2048];
        loop {
            let (n, source) = match socket.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(error) => {
                    if events.is_closed() {
                        break;
                    }
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionRefused
                            | std::io::ErrorKind::Interrupted
                    ) {
                        tokio::task::yield_now().await;
                    } else {
                        tracing::debug!(%error,"UDP receive failed; retrying");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    continue;
                }
            };
            if dispatch(&table, source, &buf[..n]) {
                continue;
            }
            if events
                .send(Internal::Datagram {
                    source,
                    destination: addr,
                    data: buf[..n].to_vec(),
                })
                .await
                .is_err()
            {
                break;
            }
        }
    });
    Ok((
        addr,
        Arc::new(MediaSocket {
            udp,
            pending,
            receiver,
        }),
    ))
}
impl MediaSocket {
    async fn query(&self, server: SocketAddr) -> Result<SocketAddr> {
        let transaction: Transaction = rand::random();
        let (reply, mut incoming) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(transaction, (server, reply));
        let _registration = Registration {
            pending: self.pending.clone(),
            transaction,
        };
        let mut request = [0; 20];
        request[1] = 1;
        request[4..8].copy_from_slice(&COOKIE);
        request[8..20].copy_from_slice(&transaction);
        // Retransmit the SAME transaction on this media socket, bounded to 2.1s.
        for delay in [300, 600, 1200] {
            self.udp.send_to(&request, server).await?;
            if let Ok(result) =
                tokio::time::timeout(Duration::from_millis(delay), &mut incoming).await
            {
                return result.context("STUN receiver closed");
            }
        }
        bail!("STUN binding timed out after three transmissions")
    }
}
async fn servers() -> Vec<SocketAddr> {
    let replies = futures_util::future::join_all(SERVERS.map(|name| async move {
        match tokio::time::timeout(Duration::from_secs(2), tokio::net::lookup_host(name)).await {
            Ok(Ok(mut addresses)) => addresses.find(SocketAddr::is_ipv4),
            Ok(Err(error)) => {
                tracing::warn!(server=name,%error,"stun_dns_failed");
                None
            }
            Err(error) => {
                tracing::warn!(server=name,%error,"stun_dns_timeout");
                None
            }
        }
    }))
    .await;
    let mut result = Vec::new();
    for address in replies.into_iter().flatten() {
        if !result.contains(&address) {
            result.push(address);
        }
    }
    result
}
pub(super) async fn gather(sockets: &Sockets) -> Vec<IceCandidate> {
    let mut candidates = Vec::new();
    let mut addresses: Vec<_> = sockets.keys().copied().collect();
    addresses.sort_by_key(|a| (network::rank(&a.ip()), *a));
    for addr in &addresses {
        if let Ok(candidate) = p2pshare_rtc::host_candidate(*addr) {
            candidates.push(candidate);
        }
    }
    if !sockets.keys().any(|a| a.is_ipv4() && !a.ip().is_loopback()) {
        return candidates;
    }
    let servers = servers().await;
    let servers = &servers;
    let results = futures_util::future::join_all(sockets.iter().filter(|(a,_)|a.is_ipv4()&&!a.ip().is_loopback()).map(|(base,socket)|async move {
        let mut mapped = Vec::new();
        for (server,result) in futures_util::future::join_all(servers.iter().map(|server|async {(*server,socket.query(*server).await)})).await {
            match result {
                Ok(public) => {
                    tracing::info!(local=%base,%server,%public,"stun_binding_discovered");
                    if !mapped.contains(&public) {mapped.push(public);}
                }
                Err(error) => tracing::warn!(local=%base,%server,%error,"stun_binding_failed"),
            }
        }
        if mapped.len()>1 {tracing::warn!(local=%base,mappings=mapped.len(),"stun_mapping_varies_by_server_direct_connection_may_need_relay");}
        (*base,mapped)
    })).await;
    for (base, mapped) in results {
        for public in mapped {
            if let Ok(c) = p2pshare_rtc::reflexive_candidate(public, base) {
                candidates.push(c);
            }
        }
    }
    tracing::info!(
        host = sockets.len(),
        reflexive = candidates.len() - sockets.len(),
        "media_candidate_discovery_complete"
    );
    candidates
}
pub(super) async fn open(
    events: mpsc::Sender<Internal>,
    isolated: bool,
) -> Result<(Sockets, Vec<IceCandidate>)> {
    let ips = interfaces(isolated)?;
    tracing::info!(
        ipv6 = ips.iter().filter(|a| a.is_ipv6()).count(),
        ipv4 = ips.iter().filter(|a| a.is_ipv4()).count(),
        "media_interfaces_ipv6_preferred"
    );
    let mut sockets = HashMap::new();
    for ip in ips {
        match bind(ip, events.clone()).await {
            Ok((addr, socket)) => {
                sockets.insert(addr, socket);
            }
            Err(error) => tracing::warn!(%ip,%error,"media_interface_bind_failed"),
        }
    }
    if sockets.is_empty() {
        bail!("No usable network interface");
    }
    let candidates = gather(&sockets).await;
    Ok((sockets, candidates))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn response(request: &[u8], public: SocketAddr) -> Vec<u8> {
        let mut bytes = request[..20].to_vec();
        bytes[0] = 1;
        bytes[1] = 1;
        bytes[3] = 12;
        bytes.extend_from_slice(&[0, 0x20, 0, 8, 0, 1]);
        bytes.extend_from_slice(&(public.port() ^ 0x2112).to_be_bytes());
        let std::net::IpAddr::V4(ip) = public.ip() else {
            panic!()
        };
        bytes.extend(ip.octets().iter().zip(COOKIE).map(|(a, b)| a ^ b));
        bytes
    }
    #[test]
    fn rejects_spoofed_truncated_and_mismatched_bindings() {
        let transaction = [7; 12];
        let mut request = [0; 20];
        request[4..8].copy_from_slice(&COOKIE);
        request[8..].copy_from_slice(&transaction);
        let public = "203.0.113.1:1234".parse().unwrap();
        let valid = response(&request, public);
        assert_eq!(binding_response(&valid, transaction), Some(public));
        assert_eq!(binding_response(&valid, [8; 12]), None);
        for n in 0..valid.len() {
            assert_eq!(binding_response(&valid[..n], transaction), None);
        }
        let mut wrong = valid.clone();
        wrong[3] = 8;
        assert_eq!(binding_response(&wrong, transaction), None);
        let pending: Pending = Default::default();
        let (tx, mut rx) = oneshot::channel();
        pending
            .lock()
            .unwrap()
            .insert(transaction, ("127.0.0.1:1".parse().unwrap(), tx));
        assert!(!dispatch(&pending, "127.0.0.1:2".parse().unwrap(), &valid));
        assert!(rx.try_recv().is_err());
        assert!(dispatch(&pending, "127.0.0.1:1".parse().unwrap(), &valid));
        assert_eq!(rx.try_recv().unwrap(), public);
    }
    #[tokio::test]
    async fn lost_first_binding_retries_without_stealing_media_and_cleans_cancellation() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let (tx, mut rx) = mpsc::channel(16);
        let (local, socket) = bind("127.0.0.1".parse().unwrap(), tx).await.unwrap();
        let public = "203.0.113.2:5555".parse().unwrap();
        let fixture = tokio::spawn(async move {
            let mut buf = [0; 64];
            let (_, source) = server.recv_from(&mut buf).await.unwrap();
            server.send_to(b"synthetic RTP", source).await.unwrap();
            let (n, source) = server.recv_from(&mut buf).await.unwrap();
            server
                .send_to(&response(&buf[..n], public), source)
                .await
                .unwrap();
        });
        assert_eq!(socket.query(address).await.unwrap(), public);
        fixture.await.unwrap();
        let Some(Internal::Datagram {
            destination, data, ..
        }) = rx.recv().await
        else {
            panic!()
        };
        assert_eq!(destination, local);
        assert_eq!(data, b"synthetic RTP");
        assert!(socket.pending.lock().unwrap().is_empty());
        let unanswered = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                socket.query(unanswered.local_addr().unwrap())
            )
            .await
            .is_err()
        );
        assert!(socket.pending.lock().unwrap().is_empty());
    }
}
