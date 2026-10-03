//! The desktop app's agent bridge (`agent-api`; docs/MCP.md): an opt-in loopback listener
//! that speaks the remote-engine protocol against the controller the UI already uses, so
//! `ether-mcp` (and any other remote-engine client holding the token) can drive the
//! running app.
//!
//! - **Same server code.** It is a [`Server`] [attached](Server::attach) to the app's
//!   engine: same handshake, auth, limits and per-client id/gesture remapping.
//! - **Shared engine.** The UI keeps talking to the engine directly. The bridge's router
//!   numbers its requests from [`REQUEST_BASE`] and its gestures from [`GESTURE_BASE`]
//!   (the UI's ids stay far below), and [`AgentBridge::route`] splits what the engine emits:
//!   replies to bridge requests go to the bridge only; everything else goes to the UI and
//!   to every bridge client.
//! - **Discovery.** While enabled, `{port, token, pid, version}` is in the runtime file
//!   ([`RUNTIME_FILE`] in the app data dir), created with mode 0600 (owner-only) on Unix.
//!   Stopping the bridge (or dropping it) deletes the file.
//! - **Auth.** Always a fresh random token (24 bytes, hex), loopback only.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ether_protocol::ServerMessage;
use serde::{Deserialize, Serialize};

use crate::router::Router;
use crate::{Engine, Server, ServerConfig, ServerError, random_hex};

/// File name of the runtime file, in the desktop app's data dir.
pub const RUNTIME_FILE: &str = "agent-bridge.json";
/// First request id of the bridge's router (the UI's ids never get this high).
pub const REQUEST_BASE: u32 = 0x8000_0000;
/// First gesture id of the bridge's router (UI gestures stay below, the controller's own
/// start at `0x8000_0000`).
pub const GESTURE_BASE: u32 = 0x4000_0000;
/// The desktop app's release bundle identifier (its app data dir name).
pub const DESKTOP_IDENTIFIER: &str = "dev.ethereal.app";

/// Contents of the runtime file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeInfo {
    /// Loopback port of the listener (`ws://127.0.0.1:<port>/`).
    pub port: u16,
    /// Token for `ClientHello::token`.
    pub token: String,
    /// Process id of the app.
    pub pid: u32,
    /// App version.
    pub version: String,
}

impl RuntimeInfo {
    pub fn url(&self) -> String {
        format!("ws://127.0.0.1:{}/", self.port)
    }
}

/// Write `info` to `path` (atomically; owner-only permissions on Unix).
pub fn write_runtime_file(path: &Path, info: &RuntimeInfo) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let json = serde_json::to_vec_pretty(info).map_err(std::io::Error::other)?;
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Read a runtime file.
pub fn read_runtime_file(path: &Path) -> std::io::Result<RuntimeInfo> {
    let json = std::fs::read_to_string(path)?;
    serde_json::from_str(&json).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Where the desktop app keeps its runtime file, most specific first: the dev build of
/// instance `instance` (`<data>/ethereal-dev/<instance>/`), then the release build
/// (`<data>/dev.ethereal.app/`). `<data>` is the OS data dir ([`crate::cli::os_data_dir`]).
pub fn desktop_runtime_candidates(instance: &str) -> Vec<PathBuf> {
    let data = crate::cli::os_data_dir();
    vec![
        data.join("ethereal-dev").join(instance).join(RUNTIME_FILE),
        data.join(DESKTOP_IDENTIFIER).join(RUNTIME_FILE),
    ]
}

/// A running bridge. Dropping it stops the listener and deletes the runtime file.
pub struct AgentBridge {
    server: Option<Server>,
    router: Arc<Router>,
    runtime_file: PathBuf,
    info: RuntimeInfo,
}

impl std::fmt::Debug for AgentBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentBridge")
            .field("port", &self.info.port)
            .field("runtime_file", &self.runtime_file)
            .finish()
    }
}

impl AgentBridge {
    /// Listen on a random loopback port for `engine` and write `runtime_file`. The caller
    /// must pass everything the engine emits through [`AgentBridge::route`].
    pub fn start(
        engine: Box<dyn Engine>,
        runtime_file: PathBuf,
        instance: String,
        version: &str,
    ) -> Result<Self, ServerError> {
        let token = random_hex(24).map_err(ServerError::Host)?;
        let router = Arc::new(Router::with_id_base(REQUEST_BASE, GESTURE_BASE));
        let server = Server::attach(
            ServerConfig {
                token: Some(token.clone()),
                name: Some("Ethereal (desktop)".into()),
                instance,
                // The UI's own uploads are not the bridge's to clean up.
                projects_root: None,
                ..ServerConfig::default()
            },
            engine,
            router.clone(),
        )?;
        let info = RuntimeInfo {
            port: server.local_addr().port(),
            token,
            pid: std::process::id(),
            version: version.to_string(),
        };
        write_runtime_file(&runtime_file, &info)?;
        tracing::info!(port = info.port, file = %runtime_file.display(), "agent bridge enabled");
        Ok(Self {
            server: Some(server),
            router,
            runtime_file,
            info,
        })
    }

    pub fn port(&self) -> u16 {
        self.info.port
    }

    pub fn runtime_file(&self) -> &Path {
        &self.runtime_file
    }

    /// Connected bridge clients.
    pub fn client_count(&self) -> usize {
        self.router.client_count()
    }

    /// Route one message the engine emitted. Returns it back when the app's own consumer
    /// (the UI) must get it too: everything but the replies to bridge requests.
    pub fn route(&self, message: ServerMessage) -> Option<ServerMessage> {
        match &message {
            ServerMessage::Reply(r) if self.router.owns_reply(r.id) => {
                self.router.outbound(message);
                None
            }
            ServerMessage::Reply(_) => Some(message),
            _ => {
                if self.router.client_count() > 0 {
                    self.router.outbound(message.clone());
                }
                Some(message)
            }
        }
    }

    /// Stop listening (closing every bridge connection) and delete the runtime file.
    pub fn stop(mut self) {
        self.stop_impl();
    }

    fn stop_impl(&mut self) {
        if let Some(server) = self.server.take() {
            server.shutdown();
            match std::fs::remove_file(&self.runtime_file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!(%e, "could not delete the agent bridge runtime file"),
            }
            tracing::info!("agent bridge disabled");
        }
    }
}

impl Drop for AgentBridge {
    fn drop(&mut self) {
        self.stop_impl();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_file_roundtrip_is_owner_only() {
        let dir = std::env::temp_dir().join(format!("ether-agent-bridge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(RUNTIME_FILE);
        let info = RuntimeInfo {
            port: 4242,
            token: "t".into(),
            pid: 1,
            version: "0.0.1".into(),
        };
        write_runtime_file(&path, &info).unwrap();
        write_runtime_file(&path, &info).unwrap();
        assert_eq!(read_runtime_file(&path).unwrap(), info);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidates_end_with_the_file_name() {
        for p in desktop_runtime_candidates("x") {
            assert!(p.ends_with(RUNTIME_FILE));
        }
    }
}
