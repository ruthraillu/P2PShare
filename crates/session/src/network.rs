//! Bounded interface selection: prefer native IPv6 without starving IPv4 fallback.
use std::net::IpAddr;
const MAX_INTERFACES: usize = 8;
fn usable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !v.is_unspecified() && !v.is_multicast() && !v.is_loopback() && !v.is_broadcast()
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
pub(super) fn rank(ip: &IpAddr) -> u8 {
    match ip {
        IpAddr::V6(v) if v.segments()[0] & 0xe000 == 0x2000 => 0, // Global unicast.
        IpAddr::V6(_) => 1, // Includes ULA paths usable on a LAN.
        IpAddr::V4(v) if !v.is_link_local() => 2,
        IpAddr::V4(_) => 3,
    }
}
pub(super) fn select(mut ips: Vec<IpAddr>, isolated: bool) -> Vec<IpAddr> {
    if isolated {
        return vec![std::net::Ipv4Addr::LOCALHOST.into()];
    }
    ips.retain(usable);
    ips.sort_by_key(|ip| (rank(ip), *ip));
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
}
