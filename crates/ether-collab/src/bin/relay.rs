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

    use ether_collab::relay::server::{RelayServer, RelayServerConfig, random_hex};

    const USAGE: &str = "\
Usage: ether-collab-relay [options]

  --port <N>        Listen port. Default: $ETHER_COLLAB_PORT, else $ETHER_DEV_PORT + 3
                    (the dev instance's collab port), else OS-chosen.
  --listen <IP>     Listen address (default 127.0.0.1). Non-loopback requires a token.
  --token <T>       Shared token sites must send (or $ETHER_COLLAB_TOKEN). Default: a
                    random one, printed at start.
  --no-token        Serve without a token (loopback only).
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
            ..RelayServerConfig::default()
        };
        let server = RelayServer::start(config).map_err(|e| e.to_string())?;
        // Machine-readable first line (scripts and e2e tests read it).
        println!("ether-collab-relay listening on {}", server.url());
        if let Some(t) = token {
            println!("token: {t}");
        }
        server.wait();
        Ok(())
    }
}
