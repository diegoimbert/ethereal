//! Command line of the `ether-server` binary.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use ether_native::LibraryRoot;

pub const USAGE: &str = "\
ether-server: headless Ethereal engine over WebSocket

USAGE: ether-server [OPTIONS]

OPTIONS:
  --port <N>        Listen port. Default: $ETHER_SERVER_PORT, else $ETHER_DEV_PORT + 4
                    (the dev instance's remote port, see `just dev-port`), else OS-chosen.
  --listen <IP>     Interface to listen on (default 127.0.0.1, loopback only). Anything
                    else exposes the engine to the network and requires a token.
  --token <T>       Shared token clients must send (or $ETHER_SERVER_TOKEN). Default: one
                    generated at first start and kept in <data-dir>/config/server-token.
  --no-auth         Serve without a token (loopback only). Upgrades whose Host or Origin
                    header is not loopback are then refused, so other web pages open in
                    a browser on this machine cannot drive the engine.
  --print-token     Print the token on stdout at start (it is never written to logs).
  --name <NAME>     Name shown to clients (default: host name).
  --single-client   Reject a second client while one is connected.
  --data-dir <DIR>  Engine data dir (config, plugin db). Default: per instance in dev builds
                    (<data>/ethereal-dev/<instance>/server), else <data>/ethereal-server.
  --projects <DIR>  Project store root (default: dev: <data-dir>/projects, release:
                    ~/Documents/Ethereal/Projects).
  --library <DIR>   Sample library folder for the browser (repeatable; default: the demo
                    samples plus $ETHER_LIBRARY).
  -h, --help        This help.

Set ETHER_AUDIO=null to run without an audio device.";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Args {
    pub port: Option<u16>,
    pub listen: Option<IpAddr>,
    pub token: Option<String>,
    pub no_auth: bool,
    pub print_token: bool,
    pub name: Option<String>,
    pub single_client: bool,
    pub data_dir: Option<PathBuf>,
    pub projects: Option<PathBuf>,
    pub library: Vec<PathBuf>,
    pub help: bool,
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--port" => {
                let v = value("--port")?;
                a.port = Some(v.parse().map_err(|_| format!("bad port {v:?}"))?);
            }
            "--listen" => {
                let v = value("--listen")?;
                a.listen = Some(v.parse().map_err(|_| format!("bad address {v:?}"))?);
            }
            "--token" => a.token = Some(value("--token")?),
            "--no-auth" => a.no_auth = true,
            "--print-token" => a.print_token = true,
            "--name" => a.name = Some(value("--name")?),
            "--single-client" => a.single_client = true,
            "--data-dir" => a.data_dir = Some(value("--data-dir")?.into()),
            "--projects" => a.projects = Some(value("--projects")?.into()),
            "--library" => a.library.push(value("--library")?.into()),
            "-h" | "--help" => a.help = true,
            other => return Err(format!("unknown option {other:?} (see --help)")),
        }
    }
    if a.no_auth && a.token.is_some() {
        return Err("--no-auth and --token are exclusive".into());
    }
    Ok(a)
}

/// The listen port from flags and environment (see [`USAGE`]).
pub fn port(args: &Args, env: impl Fn(&str) -> Option<String>) -> Result<u16, String> {
    if let Some(p) = args.port {
        return Ok(p);
    }
    if let Some(v) = env("ETHER_SERVER_PORT") {
        return v
            .parse()
            .map_err(|_| format!("bad ETHER_SERVER_PORT {v:?}"));
    }
    if let Some(v) = env("ETHER_DEV_PORT") {
        let base: u16 = v.parse().map_err(|_| format!("bad ETHER_DEV_PORT {v:?}"))?;
        return base
            .checked_add(4)
            .ok_or_else(|| "ETHER_DEV_PORT too large".to_string());
    }
    Ok(0)
}

pub fn bind(args: &Args, port: u16) -> SocketAddr {
    SocketAddr::new(args.listen.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)), port)
}

/// The OS per-user data dir (`dirs::data_dir` equivalent).
pub fn os_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map_or(home, PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("share"))
    }
}

pub fn default_data_dir(dev: bool, instance: &str) -> PathBuf {
    if dev {
        os_data_dir()
            .join("ethereal-dev")
            .join(instance)
            .join("server")
    } else {
        os_data_dir().join("ethereal-server")
    }
}

/// The persisted token of this install (created with owner-only permissions on first use).
pub fn load_or_create_token(data_dir: &Path) -> std::io::Result<String> {
    let path = token_file(data_dir);
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let token = crate::random_hex(24).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(path.parent().expect("has a parent"))?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    opts.open(&path)?.write_all(token.as_bytes())?;
    Ok(token)
}

pub fn token_file(data_dir: &Path) -> PathBuf {
    data_dir.join("config").join("server-token")
}

/// Library roots: explicit `--library` folders, else the demo samples plus `ETHER_LIBRARY`.
pub fn library_roots(args: &Args, data_dir: &Path) -> Vec<LibraryRoot> {
    let named = |i: usize, path: PathBuf| LibraryRoot {
        id: format!("lib{i}"),
        name: path
            .file_name()
            .map_or_else(|| "Library".into(), |n| n.to_string_lossy().into_owned()),
        path,
    };
    if !args.library.is_empty() {
        return args
            .library
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, p)| named(i, p))
            .collect();
    }
    let mut roots = Vec::new();
    let demo = data_dir.join("demo-samples");
    match ether_native::demo_samples::ensure(&demo) {
        Ok(()) => roots.push(LibraryRoot {
            id: "demo".into(),
            name: "Demo Samples".into(),
            path: demo,
        }),
        Err(e) => tracing::warn!(%e, "could not write the demo samples"),
    }
    if let Some(list) = std::env::var_os("ETHER_LIBRARY") {
        roots.extend(
            std::env::split_paths(&list)
                .filter(|p| p.is_dir())
                .enumerate()
                .map(|(i, p)| named(i, p)),
        );
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Result<Args, String> {
        parse(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_flags() {
        let a = args(&[
            "--port",
            "9",
            "--listen",
            "0.0.0.0",
            "--token",
            "t",
            "--library",
            "a",
            "--library",
            "b",
        ])
        .unwrap();
        assert_eq!(a.port, Some(9));
        assert_eq!(bind(&a, 9).to_string(), "0.0.0.0:9");
        assert_eq!(a.library.len(), 2);
        assert!(args(&["--nope"]).is_err());
        assert!(args(&["--port"]).is_err());
        assert!(args(&["--no-auth", "--token", "x"]).is_err());
        assert_eq!(bind(&args(&[]).unwrap(), 5).to_string(), "127.0.0.1:5");
    }

    #[test]
    fn port_from_env() {
        let none = |_: &str| None;
        let a = Args::default();
        assert_eq!(port(&a, none), Ok(0));
        assert_eq!(
            port(&a, |k| (k == "ETHER_DEV_PORT").then(|| "21000".into())),
            Ok(21004)
        );
        assert_eq!(
            port(&a, |_| Some("7".into())),
            Ok(7),
            "ETHER_SERVER_PORT wins"
        );
        let a = Args {
            port: Some(1),
            ..Args::default()
        };
        assert_eq!(port(&a, |_| Some("7".into())), Ok(1));
    }

    #[test]
    fn token_is_persisted() {
        let dir = std::env::temp_dir().join(format!("ether-server-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let t = load_or_create_token(&dir).unwrap();
        assert_eq!(t.len(), 48);
        assert_eq!(load_or_create_token(&dir).unwrap(), t);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
