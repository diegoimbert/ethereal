//! The relay's STUN responder, TURN REST credentials and ICE advertisement
//! (docs/COLLAB.md §10).
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ether_collab::relay::ice::credentials::{
    CREDENTIAL_TTL_SECS, MAX_SKEW_SECS, check_username, parse_username, unix_now,
};
use ether_collab::relay::ice::peers::is_private_or_local;
use ether_collab::relay::ice::stun::{GLOBAL_PER_SECOND, PER_IP_PER_SECOND};
use ether_collab::relay::ice::{
    IceAdvertiser, IceConfig, PeerFilter, SiteHosts, StunResponder, TurnCredentials, host_name,
};
use ether_collab::relay::server::{RelayServer, RelayServerConfig};
use ether_collab::wire::{COLLAB_PROTOCOL_VERSION, SnapshotData};
use ether_collab::{CollabMessage, ConnectRequest, LinkState};
use ether_protocol::collab::IceServer;
use ether_protocol::model::SiteId;
use stun::fingerprint::FINGERPRINT;
use stun::message::{BINDING_SUCCESS, Getter, Message};
use stun::xoraddr::XorMappedAddress;

const TID: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

/// A minimal (attribute-less) Binding request, as browsers send.
fn request(tid: [u8; 12]) -> Vec<u8> {
    let mut p = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xa4, 0x42];
    p.extend_from_slice(&tid);
    p
}

fn v4(last: u8, port: u16) -> SocketAddr {
    SocketAddr::from(([203, 0, 113, last], port))
}

/// Decode a response and check everything but the mapped address, which is returned.
fn decode(response: &[u8], tid: [u8; 12]) -> SocketAddr {
    let mut m = Message::new();
    m.unmarshal_binary(response).expect("valid STUN");
    assert_eq!(m.typ, BINDING_SUCCESS);
    assert_eq!(m.transaction_id.0, tid, "transaction id echoed");
    assert_eq!(
        m.attributes.0.len(),
        2,
        "XOR-MAPPED-ADDRESS + FINGERPRINT only"
    );
    FINGERPRINT.check(&m).expect("fingerprint");
    let mut xor = XorMappedAddress::default();
    xor.get_from(&m).expect("XOR-MAPPED-ADDRESS");
    SocketAddr::new(xor.ip, xor.port)
}

#[test]
fn ipv4_binding_response_is_40_bytes_and_maps_the_source() {
    let mut r = StunResponder::new();
    let from = v4(7, 54321);
    let resp = r.handle(&request(TID), from, Instant::now()).unwrap();
    assert_eq!(resp.len(), 40);
    assert_eq!(decode(&resp, TID), from);
}

#[test]
fn ipv6_binding_response_is_52_bytes_and_maps_the_source() {
    let mut r = StunResponder::new();
    let from = SocketAddr::new(IpAddr::V6("2001:db8::42".parse().unwrap()), 4000);
    let resp = r.handle(&request(TID), from, Instant::now()).unwrap();
    assert_eq!(resp.len(), 52);
    assert_eq!(decode(&resp, TID), from);
}

#[test]
fn a_v4_mapped_source_is_answered_as_ipv4() {
    let mut r = StunResponder::new();
    let mapped = Ipv4Addr::new(203, 0, 113, 9).to_ipv6_mapped();
    let from = SocketAddr::new(IpAddr::V6(mapped), 1234);
    let resp = r.handle(&request(TID), from, Instant::now()).unwrap();
    assert_eq!(resp.len(), 40);
    assert_eq!(decode(&resp, TID), v4(9, 1234));
}

#[test]
fn a_request_with_a_valid_fingerprint_is_answered() {
    let mut m = Message::new();
    m.build(&[
        Box::new(Message {
            transaction_id: stun::agent::TransactionId(TID),
            ..Default::default()
        }),
        Box::new(stun::message::BINDING_REQUEST),
        Box::new(FINGERPRINT),
    ])
    .unwrap();
    let mut r = StunResponder::new();
    assert!(r.handle(&m.raw, v4(1, 1), Instant::now()).is_some());
    let mut bad = m.raw.clone();
    let n = bad.len();
    bad[n - 1] ^= 1;
    assert!(
        r.handle(&bad, v4(1, 1), Instant::now()).is_none(),
        "bad CRC"
    );
}

#[test]
fn malformed_and_non_binding_requests_are_dropped() {
    let mut r = StunResponder::new();
    let now = Instant::now();
    let from = v4(1, 1);
    let ok = request(TID);
    assert!(r.handle(&ok[..19], from, now).is_none(), "short");
    assert!(r.handle(&[], from, now).is_none(), "empty");
    let mut cookie = ok.clone();
    cookie[4] = 0;
    assert!(r.handle(&cookie, from, now).is_none(), "cookie");
    let mut long = ok.clone();
    long.extend_from_slice(&[0; 4]);
    assert!(r.handle(&long, from, now).is_none(), "length mismatch");
    let mut lying = ok.clone();
    lying[3] = 8; // claims 8 bytes of attributes, has none
    assert!(r.handle(&lying, from, now).is_none(), "truncated");
    let mut unaligned = ok.clone();
    unaligned[3] = 2;
    unaligned.extend_from_slice(&[0; 2]);
    assert!(r.handle(&unaligned, from, now).is_none(), "unaligned");
    let mut bad_attr = ok.clone();
    bad_attr[3] = 8;
    bad_attr.extend_from_slice(&[0x80, 0x22, 0x00, 0x10, 0, 0, 0, 0]); // length 16 > 4
    assert!(
        r.handle(&bad_attr, from, now).is_none(),
        "attribute overflow"
    );
    for typ in [
        [0x01, 0x01],
        [0x00, 0x11],
        [0x01, 0x11],
        [0x00, 0x03],
        [0x40, 0x01],
    ] {
        let mut p = ok.clone();
        p[0..2].copy_from_slice(&typ);
        assert!(r.handle(&p, from, now).is_none(), "type {typ:?}");
    }
    assert!(r.handle(&[0x40; 64], from, now).is_none(), "channel data");
    assert!(r.handle(&ok, from, now).is_some(), "the valid one passes");
}

#[test]
fn per_ip_cap_is_50_per_second_and_refills() {
    let mut r = StunResponder::new();
    let t0 = Instant::now();
    for port in 0..PER_IP_PER_SECOND as u16 {
        assert!(r.handle(&request(TID), v4(1, 1000 + port), t0).is_some());
    }
    assert_eq!(PER_IP_PER_SECOND, 50);
    let late = t0 + Duration::from_millis(999);
    assert!(r.handle(&request(TID), v4(1, 9), late).is_none(), "51st");
    assert!(
        r.handle(&request(TID), v4(2, 9), late).is_some(),
        "other IP"
    );
    let next = t0 + Duration::from_secs(1);
    for _ in 0..PER_IP_PER_SECOND {
        assert!(r.handle(&request(TID), v4(1, 9), next).is_some());
    }
    assert!(r.handle(&request(TID), v4(1, 9), next).is_none());
}

#[test]
fn global_cap_is_1000_per_second_and_refills() {
    assert_eq!(GLOBAL_PER_SECOND, 1000);
    let mut r = StunResponder::new();
    let t0 = Instant::now();
    let ip = |i: u32| SocketAddr::new(IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i)), 5);
    for i in 0..GLOBAL_PER_SECOND {
        assert!(r.handle(&request(TID), ip(i), t0).is_some(), "{i}");
    }
    assert!(r.handle(&request(TID), ip(5000), t0).is_none(), "1001st");
    assert!(
        r.handle(&request(TID), ip(5000), t0 + Duration::from_millis(500))
            .is_none()
    );
    assert!(
        r.handle(&request(TID), ip(5000), t0 + Duration::from_secs(1))
            .is_some(),
        "refilled"
    );
    assert!(r.tracked_ips() <= ether_collab::relay::ice::stun::MAX_TRACKED_IPS);
}

#[test]
fn the_per_ip_map_is_bounded() {
    let mut r = StunResponder::with_limits(50, 1_000_000, 16);
    let t0 = Instant::now();
    for i in 0..16 {
        assert!(r.handle(&request(TID), v4(i, 1), t0).is_some());
    }
    assert!(
        r.handle(&request(TID), v4(100, 1), t0).is_none(),
        "map full of live entries"
    );
    assert!(r.handle(&request(TID), v4(3, 2), t0).is_some(), "known IP");
    assert_eq!(r.tracked_ips(), 16);
    // A second later the stale entries are evicted for newcomers.
    let t1 = t0 + Duration::from_secs(1);
    for i in 100..200 {
        assert!(
            r.handle(
                &request(TID),
                v4(i, 1),
                t1 + Duration::from_secs(u64::from(i) - 99)
            )
            .is_some()
        );
        assert!(r.tracked_ips() <= 16);
    }
}

#[test]
fn a_started_relay_answers_stun_on_its_port() {
    let server = RelayServer::start(RelayServerConfig::default()).unwrap();
    let udp = server.udp_addr().expect("STUN on");
    assert_eq!(udp, server.local_addr(), "same IP and port number");
    assert!(
        server.ice_status().starts_with("stun: udp "),
        "{}",
        server.ice_status()
    );
    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buf = [0u8; 256];
    let mut answer = None;
    for _ in 0..5 {
        client.send_to(&request(TID), udp).unwrap();
        if let Ok((n, from)) = client.recv_from(&mut buf) {
            assert_eq!(from, udp);
            answer = Some(buf[..n].to_vec());
            break;
        }
    }
    let answer = answer.expect("a STUN answer");
    assert_eq!(answer.len(), 40);
    assert_eq!(decode(&answer, TID), client.local_addr().unwrap());
    // Junk gets nothing back.
    client.send_to(b"hello", udp).unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    assert!(client.recv_from(&mut buf).is_err());
    server.shutdown();
}

#[test]
fn no_stun_binds_no_udp() {
    let server = RelayServer::start(RelayServerConfig {
        ice: IceConfig {
            stun: false,
            ..IceConfig::default()
        },
        ..RelayServerConfig::default()
    })
    .unwrap();
    assert_eq!(server.udp_addr(), None);
    assert!(server.ice_status().starts_with("stun: off"));
    // The port is free in UDP.
    drop(UdpSocket::bind(server.local_addr()).unwrap());
}

// ─── credentials ───

#[test]
fn credentials_roundtrip() {
    let c = TurnCredentials::new().unwrap();
    let now = unix_now();
    let m = c.mint(SiteId(42), now);
    assert_eq!(m.username, format!("{}:42", now + CREDENTIAL_TTL_SECS));
    assert_eq!(m.expiry, now + 12 * 3600);
    assert!(c.verify(&m.username, &m.credential, now));
    assert!(c.verify(&m.username, &m.credential, now + CREDENTIAL_TTL_SECS - 1));
    assert_eq!(
        c.password_for(&m.username, now).as_deref(),
        Some(&*m.credential)
    );
    // Standard base64 of a 20-byte HMAC-SHA1.
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&m.credential)
        .unwrap();
    assert_eq!(raw.len(), 20);
    assert_eq!(parse_username(&m.username), Some((m.expiry, SiteId(42))));
}

#[test]
fn tampered_credentials_are_rejected() {
    let c = TurnCredentials::new().unwrap();
    let now = unix_now();
    let m = c.mint(SiteId(42), now);
    let other_site = m.username.replace(":42", ":43");
    assert!(!c.verify(&other_site, &m.credential, now), "other site");
    let later = format!("{}:42", m.expiry - 1);
    assert!(!c.verify(&later, &m.credential, now), "other expiry");
    let mut cred = m.credential.clone().into_bytes();
    cred[0] = if cred[0] == b'A' { b'B' } else { b'A' };
    assert!(!c.verify(&m.username, std::str::from_utf8(&cred).unwrap(), now));
    assert!(!c.verify(&m.username, "", now));
    assert!(!c.verify(&m.username, &m.credential[1..], now));
}

#[test]
fn expired_credentials_are_rejected() {
    let c = TurnCredentials::new().unwrap();
    let now = unix_now();
    let m = c.mint(SiteId(1), now);
    assert!(!c.verify(&m.username, &m.credential, m.expiry), "at expiry");
    assert!(!c.verify(&m.username, &m.credential, m.expiry + 1));
    assert!(c.password_for(&m.username, m.expiry).is_none());
}

#[test]
fn credentials_too_far_in_the_future_are_rejected() {
    let c = TurnCredentials::new().unwrap();
    let now = unix_now();
    let ok = c.mint(SiteId(1), now + MAX_SKEW_SECS);
    assert!(c.verify(&ok.username, &ok.credential, now), "within skew");
    let far = c.mint(SiteId(1), now + MAX_SKEW_SECS + 1);
    assert!(
        !c.verify(&far.username, &far.credential, now),
        "12 h + 61 s"
    );
    assert!(check_username(&far.username, now).is_none());
}

#[test]
fn unparsable_usernames_are_rejected() {
    let c = TurnCredentials::new().unwrap();
    let now = unix_now();
    let e = now + 100;
    for u in [
        String::new(),
        "abc".into(),
        format!("{e}"),
        format!("{e}:"),
        format!(":{e}"),
        format!("{e}:1:2"),
        format!("+{e}:1"),
        format!("{e}:-1"),
        format!("{e}:x"),
        format!(" {e}:1"),
        format!("{e}:18446744073709551616"),
    ] {
        assert!(parse_username(&u).is_none(), "{u:?}");
        assert!(c.password_for(&u, now).is_none(), "{u:?}");
    }
}

#[test]
fn the_secret_is_random_per_relay() {
    let (a, b) = (
        TurnCredentials::new().unwrap(),
        TurnCredentials::new().unwrap(),
    );
    let now = unix_now();
    let (ma, mb) = (a.mint(SiteId(7), now), b.mint(SiteId(7), now));
    assert_eq!(ma.username, mb.username);
    assert_ne!(ma.credential, mb.credential);
    assert!(!b.verify(&ma.username, &ma.credential, now));
    assert!(!format!("{a:?}").contains(&ma.credential));
}

// ─── peers ───

#[test]
fn peer_filter_denies_local_private_and_own_addresses() {
    let own: IpAddr = "198.51.100.10".parse().unwrap();
    let f = PeerFilter {
        allow_private: false,
        own: vec![own],
    };
    for denied in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.1.1",
        "100.64.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
        "::1",
        "::",
        "fe80::1",
        "fd00::1",
        "ff02::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "198.51.100.10",
    ] {
        let ip: IpAddr = denied.parse().unwrap();
        assert!(!f.allows(ip), "{denied}");
    }
    for allowed in [
        "203.0.113.5",
        "8.8.8.8",
        "2001:4860::8888",
        "::ffff:8.8.8.8",
    ] {
        assert!(f.allows(allowed.parse().unwrap()), "{allowed}");
    }
    let lan = PeerFilter {
        allow_private: true,
        own: vec![own],
    };
    assert!(lan.allows("127.0.0.1".parse().unwrap()));
    assert!(lan.allows("192.168.1.1".parse().unwrap()));
    assert!(lan.allows(own));
    assert!(!lan.allows("0.0.0.0".parse().unwrap()));
    assert!(!lan.allows("224.0.0.1".parse().unwrap()));
    assert!(is_private_or_local(IpAddr::V6(Ipv6Addr::LOCALHOST)));
}

// ─── advertisement ───

#[test]
fn host_header_parsing() {
    assert_eq!(
        host_name("Relay.Example.com:4443").as_deref(),
        Some("relay.example.com")
    );
    assert_eq!(host_name("127.0.0.1:9").as_deref(), Some("127.0.0.1"));
    assert_eq!(host_name("[::1]:9").as_deref(), Some("[::1]"));
    assert_eq!(host_name("[::1]").as_deref(), Some("[::1]"));
    assert_eq!(host_name("example.com").as_deref(), Some("example.com"));
    for bad in [
        "",
        ":9",
        "a b",
        "evil.com/x",
        "a:b",
        "[nope]:1",
        "[::1]x",
        "x@y",
        "a?b",
    ] {
        assert_eq!(host_name(bad), None, "{bad:?}");
    }
}

fn advertiser(turn: Option<Arc<TurnCredentials>>, hosts: Arc<SiteHosts>) -> IceAdvertiser {
    IceAdvertiser {
        public_host: None,
        fallback_host: Some("192.0.2.1".into()),
        port: 7003,
        turn,
        hosts,
    }
}

#[test]
fn advertised_urls_use_the_site_host_then_public_host_then_fallback() {
    let hosts = Arc::new(SiteHosts::default());
    let a = advertiser(None, hosts.clone());
    let now = unix_now();
    assert_eq!(
        a.servers(SiteId(1), now),
        vec![IceServer {
            urls: vec!["stun:192.0.2.1:7003".into()],
            username: None,
            credential: None,
        }],
        "fallback: the listen address"
    );
    hosts.claim(SiteId(1), 10, "relay.example".into());
    assert_eq!(
        a.servers(SiteId(1), now)[0].urls,
        ["stun:relay.example:7003"]
    );
    // A later claim of the same site id (another connection) does not redirect it.
    hosts.claim(SiteId(1), 11, "evil.example".into());
    assert_eq!(
        a.servers(SiteId(1), now)[0].urls,
        ["stun:relay.example:7003"]
    );
    hosts.release(SiteId(1), 10);
    assert_eq!(
        a.servers(SiteId(1), now)[0].urls,
        ["stun:evil.example:7003"]
    );
    hosts.release(SiteId(1), 11);
    assert!(hosts.is_empty());
    let public = IceAdvertiser {
        public_host: Some("2001:db8::1".into()),
        ..a.clone()
    };
    assert_eq!(
        public.servers(SiteId(1), now)[0].urls,
        ["stun:[2001:db8::1]:7003"]
    );
    let nowhere = IceAdvertiser {
        fallback_host: None,
        ..a
    };
    assert!(nowhere.servers(SiteId(1), now).is_empty());
}

#[test]
fn turn_servers_carry_per_site_credentials() {
    let creds = Arc::new(TurnCredentials::new().unwrap());
    let a = advertiser(Some(creds.clone()), Arc::default());
    let now = unix_now();
    let servers = a.servers(SiteId(99), now);
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[0].urls, ["stun:192.0.2.1:7003"]);
    assert_eq!(servers[1].urls, ["turn:192.0.2.1:7003?transport=udp"]);
    let (u, c) = (
        servers[1].username.clone().unwrap(),
        servers[1].credential.clone().unwrap(),
    );
    assert_eq!(u, format!("{}:99", now + CREDENTIAL_TTL_SECS));
    assert!(creds.verify(&u, &c, now));
    // The provider closure uses the wall clock and ignores the relay clock.
    let provider = a.into_provider();
    let s = provider(SiteId(99), 0);
    let u = s[1].username.as_deref().unwrap();
    assert!(creds.verify(u, s[1].credential.as_deref().unwrap(), unix_now()));
}

// ─── through a running relay ───

/// Join `session` as the creator site `site` and return the first `IceServers` received.
pub fn ice_servers_of_a_synced_site(server: &RelayServer, token: Option<&str>) -> Vec<IceServer> {
    let mut t = ether_collab::connect(&ConnectRequest {
        server: server.url(),
        session: "jam".into(),
        token: token.map(str::to_string),
        client: "ice test".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while t.state() != LinkState::Open {
        assert!(Instant::now() < deadline, "link: {:?}", t.state());
        std::thread::sleep(Duration::from_millis(5));
    }
    t.send(&CollabMessage::Hello {
        site: SiteId(5),
        actor: None,
        name: "s".into(),
        protocol_version: COLLAB_PROTOCOL_VERSION,
    });
    t.send(&CollabMessage::SyncRequest {
        site: SiteId(5),
        version: ether_protocol::model::Base64Bytes(vec![]),
    });
    let mut got = Vec::new();
    let mut sent_snapshot = false;
    loop {
        assert!(Instant::now() < deadline, "no IceServers: {got:?}");
        let mut inbox = Vec::new();
        t.poll(&mut inbox);
        for m in inbox {
            match m {
                CollabMessage::SyncRequest { .. } if !sent_snapshot => {
                    sent_snapshot = true;
                    t.send(&CollabMessage::Snapshot {
                        data: SnapshotData {
                            epoch: 0,
                            index: 0,
                            sites: BTreeMap::new(),
                            ether: "{}".into(),
                        }
                        .encode(),
                    });
                }
                CollabMessage::IceServers { servers } => {
                    t.close();
                    return servers;
                }
                other => got.push(format!("{other:?}").chars().take(60).collect::<String>()),
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_synced_site_is_advertised_the_relay_stun_server() {
    let server = RelayServer::start(RelayServerConfig {
        token: Some("tok".into()),
        ..RelayServerConfig::default()
    })
    .unwrap();
    let servers = ice_servers_of_a_synced_site(&server, Some("tok"));
    let port = server.local_addr().port();
    assert_eq!(
        servers,
        vec![IceServer {
            urls: vec![format!("stun:127.0.0.1:{port}")],
            username: None,
            credential: None,
        }],
        "STUN only without --turn"
    );
}

#[test]
fn public_host_wins_and_a_tokenless_relay_serves_stun_only() {
    let server = RelayServer::start(RelayServerConfig {
        token: None,
        ice: IceConfig {
            public_host: Some("relay.example".into()),
            turn: true,
            ..IceConfig::default()
        },
        ..RelayServerConfig::default()
    })
    .unwrap();
    assert!(
        server.ice_status().starts_with("stun: udp"),
        "{}",
        server.ice_status()
    );
    let servers = ice_servers_of_a_synced_site(&server, None);
    let port = server.local_addr().port();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].urls, [format!("stun:relay.example:{port}")]);
}

// ─── TURN (feature `turn`) ───

#[cfg(feature = "turn")]
mod turn_server {
    use webrtc_util::Conn;

    use super::*;

    fn start(allow_private: bool) -> RelayServer {
        RelayServer::start(RelayServerConfig {
            token: Some("tok".into()),
            ice: IceConfig {
                turn: true,
                turn_allow_private: allow_private,
                ..IceConfig::default()
            },
            ..RelayServerConfig::default()
        })
        .unwrap()
    }

    /// Allocate with the given credentials, send "ping" to a loopback peer through the
    /// relay (and "pong" back), and return what the peer received.
    async fn relay_ping(
        udp: SocketAddr,
        username: String,
        password: String,
    ) -> Result<Option<Vec<u8>>, turn::Error> {
        let conn = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await?);
        let local = conn.local_addr()?;
        let client = turn::client::Client::new(turn::client::ClientConfig {
            stun_serv_addr: udp.to_string(),
            turn_serv_addr: udp.to_string(),
            username,
            password,
            realm: String::new(),
            software: String::new(),
            rto_in_ms: 0,
            conn,
            vnet: None,
        })
        .await?;
        client.listen().await?;
        let mapped = client.send_binding_request().await?;
        assert_eq!(mapped, local, "STUN through the TURN port");
        let relay = client.allocate().await?;
        let relayed = relay.local_addr()?;
        assert_eq!(relayed.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
        relay.send_to(b"ping", peer.local_addr()?).await?;
        let mut buf = [0u8; 64];
        let got = match tokio::time::timeout(Duration::from_millis(1500), peer.recv_from(&mut buf))
            .await
        {
            Ok(r) => {
                let (n, from) = r?;
                assert_eq!(from.port(), relayed.port());
                Some(buf[..n].to_vec())
            }
            Err(_) => None,
        };
        if got.is_some() {
            peer.send_to(b"pong", relayed).await?;
            let mut back = [0u8; 64];
            let (n, _) = tokio::time::timeout(Duration::from_secs(2), relay.recv_from(&mut back))
                .await
                .expect("pong relayed back")?;
            assert_eq!(&back[..n], b"pong");
        }
        let _ = relay.close().await;
        client.close().await?;
        Ok(got)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn turn_allocates_and_relays_with_advertised_credentials() {
        let server = start(true);
        assert!(
            server.ice_status().starts_with("turn: udp"),
            "{}",
            server.ice_status()
        );
        let udp = server.udp_addr().unwrap();
        let servers = ice_servers_of_a_synced_site(&server, Some("tok"));
        let port = server.local_addr().port();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].urls, [format!("stun:127.0.0.1:{port}")]);
        assert_eq!(
            servers[1].urls,
            [format!("turn:127.0.0.1:{port}?transport=udp")]
        );
        let username = servers[1].username.clone().unwrap();
        let password = servers[1].credential.clone().unwrap();
        assert!(username.ends_with(":5"));
        let got = runtime()
            .block_on(relay_ping(udp, username, password))
            .unwrap();
        assert_eq!(got.as_deref(), Some(&b"ping"[..]));
        // Plain STUN on the TURN port is still the capped 40-byte answer.
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client.send_to(&request(TID), udp).unwrap();
        let mut buf = [0u8; 128];
        let (n, _) = client.recv_from(&mut buf).unwrap();
        assert_eq!(n, 40);
        assert_eq!(decode(&buf[..n], TID), client.local_addr().unwrap());
    }

    #[test]
    fn turn_refuses_private_peers_by_default() {
        let server = start(false);
        let udp = server.udp_addr().unwrap();
        let servers = ice_servers_of_a_synced_site(&server, Some("tok"));
        let got = runtime()
            .block_on(relay_ping(
                udp,
                servers[1].username.clone().unwrap(),
                servers[1].credential.clone().unwrap(),
            ))
            .unwrap();
        assert_eq!(got, None, "loopback peer denied");
    }

    #[test]
    fn a_username_holds_at_most_4_allocations() {
        let server = start(true);
        let udp = server.udp_addr().unwrap();
        let servers = ice_servers_of_a_synced_site(&server, Some("tok"));
        let (username, password) = (
            servers[1].username.clone().unwrap(),
            servers[1].credential.clone().unwrap(),
        );
        runtime().block_on(async {
            let mut clients = Vec::new();
            let mut relays = Vec::new();
            for i in 0..5 {
                let conn = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
                let client = turn::client::Client::new(turn::client::ClientConfig {
                    stun_serv_addr: udp.to_string(),
                    turn_serv_addr: udp.to_string(),
                    username: username.clone(),
                    password: password.clone(),
                    realm: String::new(),
                    software: String::new(),
                    rto_in_ms: 0,
                    conn,
                    vnet: None,
                })
                .await
                .unwrap();
                client.listen().await.unwrap();
                let r = client.allocate().await;
                assert_eq!(r.is_ok(), i < 4, "allocation {i}");
                if let Ok(r) = r {
                    relays.push(r);
                }
                clients.push(client);
            }
            // Closing one frees a slot.
            let first = relays.remove(0);
            first.close().await.unwrap();
            drop(first);
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(clients[4].allocate().await.is_ok(), "slot freed");
            for c in clients {
                let _ = c.close().await;
            }
        });
    }

    #[test]
    fn turn_rejects_credentials_from_another_secret() {
        let server = start(true);
        let udp = server.udp_addr().unwrap();
        let forged = TurnCredentials::new().unwrap().mint(SiteId(5), unix_now());
        let r = runtime().block_on(relay_ping(udp, forged.username, forged.credential));
        assert!(r.is_err(), "allocation with a foreign secret");
    }
}
