//! Per-instance identity so several dev copies of Ethereal (one per worktree/agent) can run
//! side by side without sharing config, caches, plugin DBs or autosaves.
//!
//! - Dev builds (`debug_assertions`): `<data_dir>/ethereal-dev/<instance>/`, where
//!   `<instance>` comes from `ETHER_INSTANCE` (sanitized to `[A-Za-z0-9_-]`) or `default`.
//!   The justfile also overrides the Tauri `identifier` to `dev.ethereal.<instance>` so the
//!   WebView's own storage (localStorage, IndexedDB) is separated per instance too.
//! - Release builds: the normal Tauri app data dir for the bundle identifier.
//!
//! There is deliberately no single-instance plugin.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

/// Environment variable naming the dev instance.
pub const INSTANCE_ENV: &str = "ETHER_INSTANCE";

/// Subdirectories created under the app data dir.
/// `projects` is the dev `projects_root` (engine-side project store).
pub const SUBDIRS: &[&str] = &[
    "config",
    "logs",
    "cache",
    "plugin-db",
    "autosave",
    "tmp",
    "projects",
];

/// The instance id: `ETHER_INSTANCE` sanitized to `[A-Za-z0-9_-]`, or `"default"`.
pub fn instance_id() -> String {
    sanitize(std::env::var(INSTANCE_ENV).ok().as_deref())
}

fn sanitize(raw: Option<&str>) -> String {
    let cleaned: String = raw
        .unwrap_or_default()
        .chars()
        // Same rule as scripts/dev-env.mjs and ether_native::instance::sanitize.
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "default".to_owned()
    } else {
        cleaned
    }
}

/// Resolves the app data dir for this build/instance and creates its standard layout.
pub fn app_data_dir<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<PathBuf> {
    let dir = if cfg!(debug_assertions) {
        app.path()
            .data_dir()?
            .join("ethereal-dev")
            .join(instance_id())
    } else {
        app.path().app_data_dir()?
    };
    ensure_layout(&dir)?;
    Ok(dir)
}

/// Creates `dir` and its [`SUBDIRS`].
pub fn ensure_layout(dir: &Path) -> std::io::Result<()> {
    for sub in SUBDIRS {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_instance_names() {
        assert_eq!(sanitize(None), "default");
        assert_eq!(sanitize(Some("")), "default");
        assert_eq!(sanitize(Some("../../etc")), "------etc");
        assert_eq!(sanitize(Some("node/ui-shell")), "node-ui-shell");
        assert_eq!(sanitize(Some("agent_42-b")), "agent_42-b");
    }
}
