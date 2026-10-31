//! Command line of `ether-mcp`.

use std::path::PathBuf;

pub const USAGE: &str = "\
ether-mcp: MCP server (stdio) for Ethereal

USAGE:
  ether-mcp                                  drive the running desktop app (default)
  ether-mcp --server ws://HOST:PORT --token T  drive an ether-server
  ether-mcp --project PATH                   headless engine on a project file

OPTIONS:
  --server <URL>        ether-server URL (ws://host:port/).
  --token <T>           Its token. Prefer $ETHER_SERVER_TOKEN: arguments are visible
                        to other local users in the process list.
  --project <PATH>      A project folder or .ether file (created if missing). No audio
                        output; save_project writes the file.
  --runtime-file <F>    Desktop mode: the app's agent-bridge.json (default: the dev
                        instance $ETHER_INSTANCE, then the installed app; see docs/MCP.md).
  -h, --help            This help.

Desktop mode needs 'Allow AI agents (MCP)' turned on in the app. Logs go to stderr
(RUST_LOG=info for more).";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Desktop { runtime_file: Option<PathBuf> },
    Server { url: String, token: Option<String> },
    Project { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub help: bool,
}

pub fn parse<I: IntoIterator<Item = String>>(
    args: I,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Args, String> {
    let (mut server, mut token, mut project, mut runtime_file, mut help) =
        (None, None, None, None, false);
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--server" => server = Some(value("--server")?),
            "--token" => token = Some(value("--token")?),
            "--project" => project = Some(PathBuf::from(value("--project")?)),
            "--runtime-file" => runtime_file = Some(PathBuf::from(value("--runtime-file")?)),
            "-h" | "--help" => help = true,
            other => return Err(format!("unknown option {other:?} (see --help)")),
        }
    }
    if runtime_file.is_some() && (server.is_some() || project.is_some()) {
        return Err("--runtime-file is for the desktop mode".into());
    }
    let mode = match (server, project) {
        (Some(_), Some(_)) => return Err("--server and --project are exclusive".into()),
        (Some(url), None) => {
            if url.starts_with("wss://") {
                return Err(format!(
                    "--server: wss:// is not supported ({url:?}); ether-server serves ws:// \
                     only. For a server on another machine, forward its port over SSH \
                     (ssh -L PORT:127.0.0.1:PORT HOST) and use ws://127.0.0.1:PORT"
                ));
            }
            if !url.starts_with("ws://") {
                return Err(format!("--server must be a ws:// URL, got {url:?}"));
            }
            Mode::Server {
                url,
                token: token
                    .or_else(|| env("ETHER_SERVER_TOKEN"))
                    .filter(|t| !t.is_empty()),
            }
        }
        (None, Some(path)) => Mode::Project { path },
        (None, None) => {
            if token.is_some() {
                return Err("--token needs --server".into());
            }
            Mode::Desktop { runtime_file }
        }
    };
    Ok(Args { mode, help })
}

/// Whether `url` (`ws://...`) reaches a host other than this machine: its token and
/// traffic then cross the network in clear text.
pub fn is_cleartext_remote(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("ws://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    !ether_server::is_loopback_host(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Result<Args, String> {
        parse(v.iter().map(|s| s.to_string()), |_| None)
    }

    #[test]
    fn modes() {
        assert_eq!(p(&[]).unwrap().mode, Mode::Desktop { runtime_file: None });
        assert_eq!(
            p(&["--server", "ws://h:1", "--token", "t"]).unwrap().mode,
            Mode::Server {
                url: "ws://h:1".into(),
                token: Some("t".into())
            }
        );
        assert_eq!(
            parse(["--server", "ws://h:1"].map(String::from), |k| (k
                == "ETHER_SERVER_TOKEN")
                .then(|| "e".into()))
            .unwrap()
            .mode,
            Mode::Server {
                url: "ws://h:1".into(),
                token: Some("e".into())
            }
        );
        assert_eq!(
            p(&["--project", "a.ether"]).unwrap().mode,
            Mode::Project {
                path: "a.ether".into()
            }
        );
        assert!(p(&["--server", "http://x"]).is_err());
        assert!(p(&["--server", "wss://x:1"]).is_err());
        assert!(p(&["--server", "ws://x", "--project", "a"]).is_err());
        assert!(p(&["--token", "t"]).is_err());
        assert!(p(&["--project", "a", "--runtime-file", "f"]).is_err());
        assert!(p(&["--nope"]).is_err());
        assert!(p(&["--help"]).unwrap().help);
    }

    #[test]
    fn cleartext_remote_servers() {
        for local in [
            "ws://127.0.0.1:9000/",
            "ws://localhost:9000",
            "ws://[::1]:9000/x",
            "ws://u@127.0.0.1:1",
        ] {
            assert!(!is_cleartext_remote(local), "{local}");
        }
        for remote in [
            "ws://192.168.1.4:9000/",
            "ws://studio.example:9000",
            "ws://127.0.0.1.example:1",
            "ws://127.0.0.1@evil:1",
        ] {
            assert!(is_cleartext_remote(remote), "{remote}");
        }
    }
}
