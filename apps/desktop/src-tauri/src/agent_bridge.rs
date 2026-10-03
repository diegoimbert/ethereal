//! Agent bridge (`agent-api`; docs/MCP.md): lets `ether-mcp` (Claude Code, Claude Desktop,
//! ...) drive this running app.
//!
//! Opt-in ("Allow AI agents (MCP)", default off; persisted in
//! `<app data dir>/config/agent.json`). When enabled, `ether_server::agent_bridge` listens on
//! a random loopback port with a fresh token, against the same engine (and controller) as
//! the UI, and writes `{port, token, pid, version}` to `<app data dir>/agent-bridge.json`
//! (mode 0600). Disabling stops the listener and deletes that file; so does quitting.
//!
//! Tauri commands: `agent_bridge_status() -> AgentBridgeStatus` and
//! `agent_bridge_set_enabled(enabled) -> AgentBridgeStatus`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ether_core::protocol::ServerMessage;
use ether_native::{NativeHost, Subscriber};
use ether_server::agent_bridge::{AgentBridge, RUNTIME_FILE};
use serde::{Deserialize, Serialize};

/// What `agent_bridge_status` returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentBridgeStatus {
    pub enabled: bool,
    /// Loopback port while enabled.
    pub port: Option<u16>,
    pub connected_clients: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    bridge_enabled: bool,
}

fn settings_file(data_dir: &Path) -> PathBuf {
    data_dir.join("config").join("agent.json")
}

/// The persisted "Allow AI agents (MCP)" setting (default off).
pub fn load_enabled(data_dir: &Path) -> bool {
    std::fs::read_to_string(settings_file(data_dir))
        .ok()
        .and_then(|j| serde_json::from_str::<Settings>(&j).ok())
        .is_some_and(|s| s.bridge_enabled)
}

fn save_enabled(data_dir: &Path, enabled: bool) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&Settings {
        bridge_enabled: enabled,
    })
    .map_err(|e| e.to_string())?;
    ether_native::store::atomic_write(&settings_file(data_dir), json.as_bytes())
        .map_err(|e| e.to_string())
}

/// Splits what the engine emits between the UI and the bridge. Installed once as the
/// host's subscriber; `ether_connect` / `ether_disconnect` only swap the UI part.
#[derive(Default)]
pub struct Fanout {
    ui: Mutex<Option<Subscriber>>,
    bridge: Mutex<Option<AgentBridge>>,
}

impl Fanout {
    pub fn set_ui(&self, ui: Option<Subscriber>) {
        if let Ok(mut s) = self.ui.lock() {
            *s = ui;
        }
    }

    pub fn deliver(&self, m: ServerMessage) {
        let m = match self.bridge.lock() {
            Ok(b) => match b.as_ref() {
                Some(b) => b.route(m),
                None => Some(m),
            },
            Err(_) => Some(m),
        };
        let Some(m) = m else { return };
        let ui = self.ui.lock().ok().and_then(|s| s.clone());
        if let Some(ui) = ui {
            ui(m);
        }
    }
}

/// Bridge state managed by Tauri.
pub struct AgentBridgeState {
    pub fanout: Arc<Fanout>,
    data_dir: PathBuf,
    instance: String,
}

impl AgentBridgeState {
    pub fn new(data_dir: PathBuf, instance: String) -> Self {
        Self {
            fanout: Arc::default(),
            data_dir,
            instance,
        }
    }

    pub fn runtime_file(&self) -> PathBuf {
        self.data_dir.join(RUNTIME_FILE)
    }

    pub fn status(&self) -> AgentBridgeStatus {
        let b = self.fanout.bridge.lock().ok();
        let b = b.as_ref().and_then(|b| b.as_ref());
        AgentBridgeStatus {
            enabled: b.is_some(),
            port: b.map(|b| b.port()),
            connected_clients: b.map_or(0, |b| b.client_count()),
        }
    }

    /// Start or stop the bridge (no-op if already in that state).
    pub fn set_running(&self, host: Option<Arc<NativeHost>>, enabled: bool) -> Result<(), String> {
        if enabled {
            let mut slot = self
                .fanout
                .bridge
                .lock()
                .map_err(|_| "bridge lock poisoned")?;
            if slot.is_none() {
                let host = host.ok_or("engine is not running")?;
                *slot = Some(
                    AgentBridge::start(
                        Box::new(host),
                        self.runtime_file(),
                        self.instance.clone(),
                        env!("CARGO_PKG_VERSION"),
                    )
                    .map_err(|e| format!("could not start the agent bridge: {e}"))?,
                );
            }
        } else {
            // Stop outside the lock: the engine keeps delivering meanwhile.
            let bridge = self
                .fanout
                .bridge
                .lock()
                .map_err(|_| "bridge lock poisoned")?
                .take();
            if let Some(b) = bridge {
                b.stop();
            }
        }
        Ok(())
    }

    /// `agent_bridge_set_enabled`: persist, then apply.
    pub fn set_enabled(&self, host: Option<Arc<NativeHost>>, enabled: bool) -> Result<(), String> {
        save_enabled(&self.data_dir, enabled)?;
        self.set_running(host, enabled)
    }

    /// At startup: apply the persisted setting (and clean a runtime file a crash left).
    pub fn restore(&self, host: Arc<NativeHost>) {
        let _ = std::fs::remove_file(self.runtime_file());
        if load_enabled(&self.data_dir)
            && let Err(e) = self.set_running(Some(host), true)
        {
            tracing::warn!(%e, "agent bridge");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_defaults_off_and_persists() {
        let dir = std::env::temp_dir().join(format!("ether-desktop-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!load_enabled(&dir), "default off");
        save_enabled(&dir, true).unwrap();
        assert!(load_enabled(&dir));
        save_enabled(&dir, false).unwrap();
        assert!(!load_enabled(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_status_and_idempotent_disable() {
        let dir = std::env::temp_dir().join(format!("ether-desktop-agent2-{}", std::process::id()));
        let state = AgentBridgeState::new(dir.clone(), "test".into());
        assert_eq!(
            state.status(),
            AgentBridgeStatus {
                enabled: false,
                port: None,
                connected_clients: 0
            }
        );
        state.set_running(None, false).unwrap();
        assert!(state.set_running(None, true).is_err(), "no engine");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
