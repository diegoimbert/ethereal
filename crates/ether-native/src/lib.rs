//! Native host: audio backend + RT thread, disk streaming, GC thread, controller thread.
//!
//! Wires `ether-core` (engine), `ether-controller` (document), `ether-media` (decode) and
//! plugins (`ether-clap`, `ether-sandbox`) behind a thread-safe [`NativeHost`] that the
//! Tauri app (`apps/desktop`) drives. Owned by the `native-host` node.
//!
//! # Threads
//! - audio: the cpal callback, or the `null`/`offline` render thread ([`audio`], [`rt`]);
//! - controller: `Controller::handle`/`tick`, engine handle, audio device control ([`host`]);
//! - GC: drops what the audio thread retired;
//! - main: CLAP plugin controllers ([`plugins`]; the process main thread on macOS);
//! - media/scan workers ([`media`], plugin rescans).
//!
//! # Audio backends
//! - `cpal` (default): real device.
//! - `null`: no device. A timer thread calls `Engine::process` at real-time pace (or as
//!   fast as possible in `offline` mode). Selected with `ETHER_AUDIO=null`. Used by CI,
//!   tests and parallel dev instances so nobody contends for the sound card.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

pub mod audio;
pub mod bridge;
pub mod host;
pub mod media;
pub mod plugins;
pub mod recording;
pub mod rt;
pub mod sandbox;
pub mod store;
#[doc(hidden)]
pub mod test_util;

pub use store::{DiskStore, LibraryRoot};

pub use audio::{AudioBackendKind, AudioSettings};
pub use bridge::{NativeBridge, NativeServices};
pub use host::{HostConfig, HostError, HostOptions, NativeHost, Subscriber};
pub use plugins::{DedicatedThread, MainThread};

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

/// Default `projects_root`:
/// - dev builds: `<data_dir>/ethereal-dev/<instance>/projects` where `instance_data_dir` is
///   the per-instance app data dir (`<data_dir>/ethereal-dev/<instance>`);
/// - release: `~/Documents/Ethereal/Projects`.
pub fn default_projects_root(dev: bool, instance_data_dir: &std::path::Path) -> PathBuf {
    if dev {
        return instance_data_dir.join("projects");
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| instance_data_dir.to_path_buf());
    home.join("Documents").join("Ethereal").join("Projects")
}

#[cfg(test)]
mod tests {
    #[test]
    fn sanitize_instance() {
        assert_eq!(super::instance::sanitize("agent a/b_1"), "agent-a-b_1");
    }
}
