//! Native host: audio backend + RT thread, disk streaming, GC thread, controller thread.
//!
//! Wires `ether-core` (engine), `ether-controller` (document), `ether-media` (decode) and
//! plugins (`ether-clap`, `ether-sandbox`) behind a thread-safe [`NativeHost`] that the
//! Tauri app (`apps/desktop`) drives. Owned by the `native-host` node.
//!
//! # Audio backends
//! - `cpal` (default): real device.
//! - `null`: no device. A timer thread calls `Engine::process` at real-time pace (or as
//!   fast as possible in `offline` mode). Selected with `ETHER_AUDIO=null`. Used by CI,
//!   tests and parallel dev instances so nobody contends for the sound card.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

/// Which audio backend to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioBackendKind {
    Cpal,
    /// Real-time paced, no device.
    Null,
    /// Render as fast as possible (tests, bounce).
    Offline,
}

impl AudioBackendKind {
    /// From `ETHER_AUDIO` (`cpal` | `null` | `offline`), default `Cpal`.
    pub fn from_env() -> Self {
        match std::env::var("ETHER_AUDIO").as_deref() {
            Ok("null") => Self::Null,
            Ok("offline") => Self::Offline,
            _ => Self::Cpal,
        }
    }
}

/// Dev-instance identity (see README "Running multiple dev instances").
pub mod instance {
    /// `ETHER_INSTANCE`, sanitized to `[A-Za-z0-9_-]`, or `"default"`.
    pub fn instance_id() -> String {
        std::env::var("ETHER_INSTANCE")
            .ok()
            .map(|s| sanitize(&s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "default".to_string())
    }

    pub fn sanitize(s: &str) -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct HostConfig {
    pub backend: AudioBackendKind,
    /// Per-instance app data dir (plugin DB, logs, autosave, caches).
    pub data_dir: PathBuf,
    pub instance: String,
}

/// The running native host. Methods are callable from any thread (Tauri commands).
pub struct NativeHost {
    _private: (),
}

impl NativeHost {
    /// Start controller, GC and audio threads.
    pub fn start(config: HostConfig) -> Result<Self, String> {
        let _ = config;
        todo!("native-host node")
    }

    /// Forward a JSON-encoded `ClientMessage`; responses/events arrive on the sink passed
    /// to `subscribe`.
    pub fn send(&self, message_json: &str) -> Result<(), String> {
        let _ = message_json;
        todo!("native-host node")
    }

    /// Register the sink for JSON-encoded `ServerMessage`s (Tauri channel).
    pub fn subscribe(&self, sink: Box<dyn Fn(String) + Send + Sync>) {
        let _ = sink;
        todo!("native-host node")
    }

    pub fn shutdown(self) {
        todo!("native-host node")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sanitize_instance() {
        assert_eq!(super::instance::sanitize("agent a/b_1"), "agent-a-b_1");
    }
}
