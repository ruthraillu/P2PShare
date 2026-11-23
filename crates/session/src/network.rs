//! Bounded interface selection: prefer native IPv6 without starving IPv4 fallback.
use std::net::IpAddr;
const MAX_INTERFACES: usize = 8;
fn usable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !v.is_unspecified()
                && !v.is_multicast()
                && !v.is_loopback()
                && !v.is_broadcast()
                && !v.is_link_local()
        }
        IpAddr::V6(v) => {
            !v.is_unspecified()
                && !v.is_multicast()
                && !v.is_loopback()
                && !v.is_unicast_link_local()
                && v.to_ipv4_mapped().is_none()
        }
    }
}
fn is_teredo_or_6to4(v: &std::net::Ipv6Addr) -> bool {
    let segs = v.segments();
    (segs[0] == 0x2001 && segs[1] == 0x0000) || segs[0] == 0x2002
}
pub(super) fn rank(ip: &IpAddr) -> u8 {
    match ip {
        IpAddr::V6(v) if v.segments()[0] & 0xe000 == 0x2000 && !is_teredo_or_6to4(v) => 0, // Normal global unicast.
        IpAddr::V6(v) if !is_teredo_or_6to4(v) => 1, // Includes ULA paths usable on a LAN.
        IpAddr::V6(_) => 2, // Teredo (2001:0000::/32) and 6to4 (2002::/16) tunneled prefixes.
        IpAddr::V4(v) if !v.is_link_local() => 3,
        IpAddr::V4(_) => 4,
    }
}
fn probe_default_route(bind: &str, target: &str) -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind(bind).ok()?;
    let _ = socket.set_nonblocking(true);
    socket.connect(target).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    usable(&ip).then_some(ip)
}
pub(super) fn default_route_ips() -> Vec<IpAddr> {
    let mut defaults = Vec::new();
    if let Some(v6) = probe_default_route("[::]:0", "[2606:4700:4700::1111]:80") {
        defaults.push(v6);
    }
    if let Some(v4) = probe_default_route("0.0.0.0:0", "1.1.1.1:80") {
        defaults.push(v4);
    }
    defaults
}
#[cfg(test)]
pub(super) fn select(ips: Vec<IpAddr>, isolated: bool) -> Vec<IpAddr> {
    select_with_defaults(ips, &[], isolated)
}
pub(super) fn select_with_defaults(
    mut ips: Vec<IpAddr>,
    defaults: &[IpAddr],
    isolated: bool,
) -> Vec<IpAddr> {
    if isolated {
        return vec![std::net::Ipv4Addr::LOCALHOST.into()];
    }
    for &ip in defaults {
        if usable(&ip) {
            ips.push(ip);
        }
    }
    ips.retain(usable);
    ips.sort_by_key(|ip| {
        let family_order = if ip.is_ipv6() { 0u8 } else { 1u8 };
        let is_default = defaults.contains(ip);
        (family_order, !is_default, rank(ip), *ip)
    });
    ips.dedup();
    // Leave space for two IPv4 interfaces even with many temporary IPv6 addresses.
    let reserved_v4 = ips.iter().filter(|ip| ip.is_ipv4()).count().min(2);
    let mut selected: Vec<_> = ips
        .iter()
        .copied()
        .filter(|ip| ip.is_ipv6())
        .take(MAX_INTERFACES - reserved_v4)
        .collect();
    let remaining = MAX_INTERFACES - selected.len();
    selected.extend(ips.into_iter().filter(|ip| ip.is_ipv4()).take(remaining));
    selected
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }
    #[test]
    fn ipv6_survives_many_ipv4_adapters_and_is_first() {
        let mut ips: Vec<_> = (1..=12).map(|n| ip(&format!("10.0.{n}.1"))).collect();
        ips.extend([ip("2001:db8::1"), ip("fd00::1"), ip("2001:db8::1")]);
        let selected = select(ips, false);
        assert_eq!(selected.len(), 8);
        assert_eq!(&selected[..2], &[ip("2001:db8::1"), ip("fd00::1")]);
        assert!(selected.iter().any(IpAddr::is_ipv4));
    }
    #[test]
    fn temporary_ipv6_addresses_do_not_remove_ipv4_fallback() {
        let mut ips: Vec<_> = (1..=12).map(|n| ip(&format!("2001:db8::{n}"))).collect();
        ips.extend([ip("192.168.1.2"), ip("10.0.0.2")]);
        let selected = select(ips, false);
        assert_eq!(selected.len(), 8);
        assert_eq!(selected.iter().filter(|ip| ip.is_ipv6()).count(), 6);
        assert_eq!(selected.iter().filter(|ip| ip.is_ipv4()).count(), 2);
    }
    #[test]
    fn unusable_addresses_are_excluded_and_single_stack_still_works() {
        let invalid = [
            "::",
            "::1",
            "fe80::1",
            "ff02::1",
            "::ffff:192.168.1.2",
            "0.0.0.0",
            "127.0.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "169.254.1.2",
        ]
        .map(ip)
        .to_vec();
        assert!(select(invalid.clone(), false).is_empty());
        assert_eq!(select(invalid, true), vec![ip("127.0.0.1")]);
        assert_eq!(
            select(vec![ip("192.168.1.2")], false),
            vec![ip("192.168.1.2")]
        );
        assert_eq!(
            select(vec![ip("2001:db8::1")], false),
            vec![ip("2001:db8::1")]
        );
    }
    #[test]
    fn default_routes_are_pinned_ahead_of_virtual_adapters_and_tunneled_ipv6_deprioritized() {
        let mut ips: Vec<_> = (1..=8).map(|n| ip(&format!("10.0.{n}.1"))).collect();
        ips.extend([
            ip("192.168.1.50"),
            ip("2001::1"),          // Teredo
            ip("2002:c000:201::1"), // 6to4
            ip("2001:db8::5"),      // Normal global unicast
            ip("2001:db8::99"),     // Default IPv6
        ]);
        let defaults = [ip("2001:db8::99"), ip("192.168.1.50")];
        let selected = select_with_defaults(ips, &defaults, false);
        assert_eq!(selected.len(), 8);
        assert_eq!(selected[0], ip("2001:db8::99"));
        assert_eq!(selected[1], ip("2001:db8::5"));
        // Normal global unicast ranks ahead of Teredo and 6to4.
        assert!(rank(&ip("2001:db8::5")) < rank(&ip("2001::1")));
        assert!(rank(&ip("2001:db8::5")) < rank(&ip("2002:c000:201::1")));
        // Default IPv4 survives despite 8 lower-sorted 10.0.x.x virtual adapters.
        let v4: Vec<_> = selected.iter().copied().filter(IpAddr::is_ipv4).collect();
        assert_eq!(v4[0], ip("192.168.1.50"));
    }
}
