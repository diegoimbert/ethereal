//! Which peer addresses a TURN allocation may relay to or from (docs/COLLAB.md §10: no
//! SSRF into the relay's network).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// `true` for addresses a public relay must never relay to: loopback, link-local, private
/// and shared (CGNAT) ranges, IPv6 unique-local, and other special-purpose blocks.
pub fn is_private_or_local(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => is_private_v6(v6),
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || a == 0 // "this network"
        || (a == 100 && (64..128).contains(&b)) // shared address space (CGNAT)
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 198 && (b == 18 || b == 19)) // benchmarking
        || a >= 240 // reserved, broadcast
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    ip.is_loopback()
        || (s[0] & 0xfe00) == 0xfc00 // unique local
        || (s[0] & 0xffc0) == 0xfe80 // link-local
        || (s[0] & 0xffc0) == 0xfec0 // site-local (deprecated)
        || (s[0] == 0x64 && s[1] == 0xff9b) // NAT64: may embed a private IPv4
        || (s[0] == 0x2002) // 6to4: may embed a private IPv4
}

/// The peer-address policy of the TURN server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerFilter {
    /// `--turn-allow-private`: allow private, local and own addresses (LAN tests).
    pub allow_private: bool,
    /// The relay's own addresses (listen address, public address).
    pub own: Vec<IpAddr>,
}

impl PeerFilter {
    /// `true` if traffic may be relayed to/from `ip`. Unspecified, multicast and broadcast
    /// addresses are always denied.
    pub fn allows(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        let never = match ip {
            IpAddr::V4(v4) => v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast(),
            IpAddr::V6(v6) => v6.is_unspecified() || v6.is_multicast(),
        };
        if never {
            return false;
        }
        if self.allow_private {
            return true;
        }
        !is_private_or_local(ip) && !self.own.iter().any(|o| o.to_canonical() == ip)
    }
}
