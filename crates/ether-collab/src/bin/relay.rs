//! `ether-collab-relay`: the collaboration relay (docs/COLLAB.md).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = native::run() {
        eprintln!("ether-collab-relay: {e}");
        std::process::exit(2);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use ether_collab::relay::ice::{IceConfig, TURN_SUPPORTED};
    use ether_collab::relay::server::{RelayServer, RelayServerConfig, random_hex};

    const USAGE: &str = "\
Usage: ether-collab-relay [options]

  --port <N>        Listen port. Default: $ETHER_COLLAB_PORT, else $ETHER_DEV_PORT + 3
                    (the dev instance's collab port), else OS-chosen.
  --listen <IP>     Listen address (default 127.0.0.1). Non-loopback requires a token.
  --token <T>       Shared token sites must send (or $ETHER_COLLAB_TOKEN). Default: a
                    random one, printed at start.
  --no-token        Serve without a token (loopback only).
  --no-stun         Do not bind UDP: no STUN (by default the relay answers STUN Binding
                    requests on UDP, same IP and port number as the WebSocket listener).
  --turn            EXPERIMENTAL: also run a TURN server on that UDP port (needs the
                    `turn` build feature and a token; sites get per-site 12 h
                    credentials). Known denial-of-service limitations (docs/COLLAB.md
                    §10): do not expose it publicly yet.
  --public-host <H> Host name in the advertised stun:/turn: URLs. Default: the host name
                    each site used to reach the relay.
  --public-ip <IP>  TURN relayed address (default: the listen address; required when
                    listening on 0.0.0.0 or ::).
  --turn-ports <lo-hi>  TURN relayed port range (default 49152-65535).
  --turn-allow-private  Let TURN relay to loopback/link-local/private/own addresses
                    (LAN tests only).
  -h, --help        This help.

Sites join with the relay URL (ws://host:port), a session name and the token.";

    pub fn run() -> Result<(), String> {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info".into()),
            )
            .with_writer(std::io::stderr)
            .init();
        let mut port: Option<u16> = None;
        let mut listen: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let mut token = std::env::var("ETHER_COLLAB_TOKEN")
            .ok()
            .filter(|t| !t.is_empty());
        let mut no_token = false;
        let mut ice = IceConfig::default();
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
            match a.as_str() {
                "--port" => {
                    let v = value("--port")?;
                    port = Some(v.parse().map_err(|_| format!("bad port {v:?}"))?);
                }
                "--listen" => {
                    let v = value("--listen")?;
                    listen = v.parse().map_err(|_| format!("bad address {v:?}"))?;
                }
                "--token" => token = Some(value("--token")?),
                "--no-token" => no_token = true,
                "--no-stun" => ice.stun = false,
                "--turn" => {
                    if !TURN_SUPPORTED {
                        return Err("--turn: this relay was built without the `turn` feature \
                             (cargo build -p ether-collab --features turn)"
                            .into());
                    }
                    ice.turn = true;
                }
                "--public-host" => {
                    let v = value("--public-host")?;
                    let plain_name = ether_collab::relay::ice::host_name(&v)
                        .is_some_and(|h| h.eq_ignore_ascii_case(&v));
                    if !plain_name && v.parse::<IpAddr>().is_err() {
                        return Err(format!("bad host name {v:?}"));
                    }
                    ice.public_host = Some(v);
                }
                "--public-ip" => {
                    let v = value("--public-ip")?;
                    ice.public_ip = Some(v.parse().map_err(|_| format!("bad address {v:?}"))?);
                }
                "--turn-ports" => {
                    let v = value("--turn-ports")?;
                    let range = v
                        .split_once('-')
                        .and_then(|(lo, hi)| {
                            Some((lo.parse::<u16>().ok()?, hi.parse::<u16>().ok()?))
                        })
                        .filter(|(lo, hi)| *lo > 0 && lo <= hi)
                        .ok_or(format!("bad port range {v:?} (expected lo-hi)"))?;
                    ice.turn_ports = Some(range);
                }
                "--turn-allow-private" => ice.turn_allow_private = true,
                "-h" | "--help" => {
                    println!("{USAGE}");
                    return Ok(());
                }
                other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
            }
        }
        let port = match port {
            Some(p) => p,
            None => match std::env::var("ETHER_COLLAB_PORT") {
                Ok(v) => v
                    .parse()
                    .map_err(|_| format!("bad ETHER_COLLAB_PORT {v:?}"))?,
                Err(_) => match std::env::var("ETHER_DEV_PORT") {
                    Ok(v) => {
                        let base: u16 =
                            v.parse().map_err(|_| format!("bad ETHER_DEV_PORT {v:?}"))?;
                        base.checked_add(3).ok_or("ETHER_DEV_PORT too large")?
                    }
                    Err(_) => 0,
                },
            },
        };
        let token = if no_token {
            None
        } else {
            Some(match token {
                Some(t) => t,
                None => random_hex(16)?,
            })
        };
        let config = RelayServerConfig {
            bind: SocketAddr::new(listen, port),
            token: token.clone(),
            ice,
            ..RelayServerConfig::default()
        };
        let server = RelayServer::start(config).map_err(|e| e.to_string())?;
        // Machine-readable first line (scripts and e2e tests read it).
        println!("ether-collab-relay listening on {}", server.url());
        if let Some(t) = token {
            println!("token: {t}");
        }
        println!("{}", server.ice_status());
        server.wait();
        Ok(())
    }
}
