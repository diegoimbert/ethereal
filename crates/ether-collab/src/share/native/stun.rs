//! A tiny STUN Binding codec (RFC 8489) for the share endpoint's server-reflexive candidate
//! (docs/SHARING.md §6.1: one Binding request per advertised STUN URL, on the endpoint's
//! socket). The same codec as the listen-on-peer sender (`ether-native/src/stream/stun.rs`;
//! `ether-collab` cannot depend on `ether-native`). Only what that needs: encode the 20-byte attribute-less request, recognise a
//! Binding success response and read its (XOR-)MAPPED-ADDRESS. Everything else is
//! str0m's (connectivity checks carry credentials and never match our transaction ids).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

const MAGIC_COOKIE: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// Default STUN port (RFC 8489 §18.1).
pub const DEFAULT_PORT: u16 = 3478;

pub type TransactionId = [u8; 12];

/// The 20-byte Binding request with transaction id `tid` (no attributes).
pub fn binding_request(tid: &TransactionId) -> [u8; 20] {
    let mut m = [0u8; 20];
    m[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    // length 0
    m[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
    m[8..20].copy_from_slice(tid);
    m
}

/// The transaction id of a STUN Binding success response (cheap check: first bits, type,
/// cookie, length), or `None` for anything else.
pub fn binding_response_tid(buf: &[u8]) -> Option<TransactionId> {
    if buf.len() < 20 || buf[0] & 0xC0 != 0 {
        return None;
    }
    let ty = u16::from_be_bytes([buf[0], buf[1]]);
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    let cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if ty != BINDING_SUCCESS
        || cookie != MAGIC_COOKIE
        || !len.is_multiple_of(4)
        || 20 + len > buf.len()
    {
        return None;
    }
    buf[8..20].try_into().ok()
}

/// The mapped address of a Binding success response: XOR-MAPPED-ADDRESS, else
/// MAPPED-ADDRESS (RFC 3489 servers).
pub fn parse_binding_response(buf: &[u8]) -> Option<(TransactionId, SocketAddr)> {
    let tid = binding_response_tid(buf)?;
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    let mut attrs = &buf[20..20 + len];
    let mut mapped = None;
    while attrs.len() >= 4 {
        let ty = u16::from_be_bytes([attrs[0], attrs[1]]);
        let alen = u16::from_be_bytes([attrs[2], attrs[3]]) as usize;
        let padded = alen.div_ceil(4) * 4;
        if 4 + alen > attrs.len() {
            return None;
        }
        let value = &attrs[4..4 + alen];
        match ty {
            ATTR_XOR_MAPPED_ADDRESS => {
                if let Some(a) = parse_address(value, Some(&tid)) {
                    return Some((tid, a));
                }
            }
            ATTR_MAPPED_ADDRESS => mapped = mapped.or_else(|| parse_address(value, None)),
            _ => {}
        }
        attrs = &attrs[(4 + padded).min(attrs.len())..];
    }
    mapped.map(|a| (tid, a))
}

/// A (XOR-)MAPPED-ADDRESS value; `xor` = the transaction id for XOR-MAPPED-ADDRESS.
fn parse_address(v: &[u8], xor: Option<&TransactionId>) -> Option<SocketAddr> {
    if v.len() < 4 {
        return None;
    }
    let cookie = MAGIC_COOKIE.to_be_bytes();
    let mut port = u16::from_be_bytes([v[2], v[3]]);
    if xor.is_some() {
        port ^= (MAGIC_COOKIE >> 16) as u16;
    }
    let ip = match v[1] {
        0x01 if v.len() >= 8 => {
            let mut o = [v[4], v[5], v[6], v[7]];
            if xor.is_some() {
                for (b, c) in o.iter_mut().zip(cookie) {
                    *b ^= c;
                }
            }
            IpAddr::V4(Ipv4Addr::from(o))
        }
        0x02 if v.len() >= 20 => {
            let mut o: [u8; 16] = v[4..20].try_into().ok()?;
            if let Some(tid) = xor {
                let key = cookie.iter().chain(tid.iter());
                for (b, k) in o.iter_mut().zip(key) {
                    *b ^= k;
                }
            }
            IpAddr::V6(Ipv6Addr::from(o))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

/// Encode a Binding success response (tests and loopback responders).
#[cfg(test)]
pub fn binding_response(tid: &TransactionId, mapped: SocketAddr) -> Vec<u8> {
    let cookie = MAGIC_COOKIE.to_be_bytes();
    let mut value = vec![0u8, 0];
    let port = mapped.port() ^ (MAGIC_COOKIE >> 16) as u16;
    match mapped.ip() {
        IpAddr::V4(ip) => {
            value[1] = 0x01;
            value.extend_from_slice(&port.to_be_bytes());
            value.extend(ip.octets().iter().zip(cookie).map(|(b, c)| b ^ c));
        }
        IpAddr::V6(ip) => {
            value[1] = 0x02;
            value.extend_from_slice(&port.to_be_bytes());
            let key = cookie.iter().chain(tid.iter());
            value.extend(ip.octets().iter().zip(key).map(|(b, k)| b ^ k));
        }
    }
    let mut m = Vec::with_capacity(24 + value.len());
    m.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    m.extend_from_slice(&((4 + value.len()) as u16).to_be_bytes());
    m.extend_from_slice(&cookie);
    m.extend_from_slice(tid);
    m.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
    m.extend_from_slice(&(value.len() as u16).to_be_bytes());
    m.extend_from_slice(&value);
    m
}

/// If `buf` is a Binding request, its transaction id (tests and loopback responders).
#[cfg(test)]
pub fn binding_request_tid(buf: &[u8]) -> Option<TransactionId> {
    if buf.len() < 20 || u16::from_be_bytes([buf[0], buf[1]]) != BINDING_REQUEST {
        return None;
    }
    if u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) != MAGIC_COOKIE {
        return None;
    }
    buf[8..20].try_into().ok()
}

/// `host[:port]` of a `stun:` URL (`stuns:`/`turn:` and anything else: `None`).
pub fn parse_stun_url(url: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("stun:")?;
    let rest = rest.split('?').next().unwrap_or(rest);
    if let Some(v6) = rest.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None => DEFAULT_PORT,
        };
        return Some((host.to_string(), port));
    }
    match rest.rsplit_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok()?)),
        None if !rest.is_empty() => Some((rest.to_string(), DEFAULT_PORT)),
        None => None,
    }
}

/// Resolve a `stun:` URL to its first IPv4 address (the endpoint's socket is IPv4). Blocking
/// (DNS): call off the share thread.
pub fn resolve_stun_url(url: &str) -> Option<SocketAddr> {
    let (host, port) = parse_stun_url(url)?;
    (host.as_str(), port)
        .to_socket_addrs()
        .ok()?
        .find(SocketAddr::is_ipv4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout() {
        let tid = [7u8; 12];
        let r = binding_request(&tid);
        assert_eq!(&r[..4], &[0, 1, 0, 0]);
        assert_eq!(&r[4..8], &[0x21, 0x12, 0xA4, 0x42]);
        assert_eq!(binding_request_tid(&r), Some(tid));
        assert_eq!(binding_response_tid(&r), None);
    }

    #[test]
    fn response_roundtrip_v4_and_v6() {
        let tid = *b"abcdefghijkl";
        for a in [
            "203.0.113.7:4242".parse::<SocketAddr>().unwrap(),
            "[2001:db8::1]:65000".parse().unwrap(),
        ] {
            let r = binding_response(&tid, a);
            assert_eq!(parse_binding_response(&r), Some((tid, a)));
        }
    }

    #[test]
    fn rfc5769_ipv4_response() {
        // RFC 5769 §2.2 sample response (XOR-MAPPED-ADDRESS 192.0.2.1:32853), with SOFTWARE,
        // MESSAGE-INTEGRITY and FINGERPRINT attributes that must be skipped.
        let hex = "0101003c2112a442b7e7a701bc34d686fa87dfae8022000b\
                   7465737420766563746f7220\
                   002000080001a147e112a643\
                   000800142b91f599fd9e90c38c7489f92af9ba53f06be7d7\
                   80280004c07d4c96";
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let (_, a) = parse_binding_response(&bytes).unwrap();
        assert_eq!(a, "192.0.2.1:32853".parse().unwrap());
    }

    #[test]
    fn garbage_is_ignored() {
        assert_eq!(parse_binding_response(&[0u8; 10]), None);
        // RTP-ish first byte.
        let mut r = binding_response(&[1; 12], "1.2.3.4:5".parse().unwrap());
        r[0] = 0x80;
        assert_eq!(parse_binding_response(&r), None);
        // Truncated attribute.
        let r = binding_response(&[1; 12], "1.2.3.4:5".parse().unwrap());
        assert_eq!(parse_binding_response(&r[..r.len() - 2]), None);
    }

    #[test]
    fn stun_urls() {
        assert_eq!(
            parse_stun_url("stun:relay.example:3479"),
            Some(("relay.example".into(), 3479))
        );
        assert_eq!(
            parse_stun_url("stun:relay.example"),
            Some(("relay.example".into(), 3478))
        );
        assert_eq!(parse_stun_url("stun:[::1]:9"), Some(("::1".into(), 9)));
        assert_eq!(parse_stun_url("turn:relay.example:3478"), None);
        assert_eq!(
            resolve_stun_url("stun:127.0.0.1:5000"),
            Some("127.0.0.1:5000".parse().unwrap())
        );
    }
}
