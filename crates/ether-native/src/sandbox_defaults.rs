//! Settings: the plugins inserted sandboxed by default (`PluginCommand::ListSandboxDefaults`).
//! A per-machine list, persisted as `<plugin-db>/sandbox-defaults.json` next to the plugin
//! list and the folder settings. It only picks the flag of a new instance (the controller
//! asks [`ether_controller::EngineBridge::sandbox_by_default`]); each instance keeps its
//! own flag in the project after that.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::SandboxDefault;

/// File name of the list in the plugin DB folder.
pub const SANDBOX_DEFAULTS_FILE: &str = "sandbox-defaults.json";

/// The list. Thread-safe; cheap to clone (clones share it).
#[derive(Clone, Debug, Default)]
pub struct SandboxDefaults {
    inner: Arc<Mutex<Vec<SandboxDefault>>>,
    file: Option<PathBuf>,
}

fn same(d: &SandboxDefault, format: PluginFormat, plugin_id: &str) -> bool {
    d.format == format && d.plugin_id.eq_ignore_ascii_case(plugin_id)
}

impl SandboxDefaults {
    /// Load `<plugin_db_dir>/sandbox-defaults.json` (empty when absent or unreadable).
    pub fn open(plugin_db_dir: &Path) -> Self {
        let file = plugin_db_dir.join(SANDBOX_DEFAULTS_FILE);
        let list = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(list)),
            file: Some(file),
        }
    }

    /// The plugins, in the order added.
    pub fn list(&self) -> Vec<SandboxDefault> {
        self.inner.lock().map(|l| l.clone()).unwrap_or_default()
    }

    /// Whether a new instance of this plugin starts sandboxed.
    pub fn contains(&self, format: PluginFormat, plugin_id: &str) -> bool {
        self.inner
            .lock()
            .map(|l| l.iter().any(|d| same(d, format, plugin_id)))
            .unwrap_or(false)
    }

    /// Add or remove a plugin. Adding one already listed only updates its name. An empty id
    /// is ignored.
    pub fn set(&self, format: PluginFormat, plugin_id: &str, name: Option<&str>, on: bool) {
        let plugin_id = plugin_id.trim();
        if plugin_id.is_empty() {
            return;
        }
        let mut l = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let at = l.iter().position(|d| same(d, format, plugin_id));
        match (at, on) {
            (Some(i), true) => {
                if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
                    l[i].name = n.to_owned();
                }
            }
            (None, true) => l.push(SandboxDefault {
                format,
                plugin_id: plugin_id.to_owned(),
                name: name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or(plugin_id)
                    .to_owned(),
            }),
            (Some(i), false) => {
                l.remove(i);
            }
            (None, false) => return,
        }
        if let Some(file) = &self.file
            && let Ok(json) = serde_json::to_string_pretty(&*l)
            && let Err(e) = crate::store::atomic_write(file, json.as_bytes())
        {
            tracing::warn!(%e, "failed to write the sandbox defaults");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn add_remove_match_and_persist() {
        let tmp = TempDir::new("sandbox-defaults");
        let db = tmp.path().join("plugin-db");
        let s = SandboxDefaults::open(&db);
        assert!(s.list().is_empty());
        assert!(!s.contains(PluginFormat::Vst3, "ABCDEF"));

        s.set(PluginFormat::Vst3, "ABCDEF", Some("Verb"), true);
        s.set(PluginFormat::Clap, "com.acme.synth", None, true);
        // Already listed (ids compare case-insensitively): only the name changes.
        s.set(PluginFormat::Vst3, "abcdef", Some("Verb 2"), true);
        // Ignored: an empty id, removing one not listed.
        s.set(PluginFormat::Clap, "  ", Some("x"), true);
        s.set(PluginFormat::Au, "aufx:dely:appl", None, false);

        assert!(s.contains(PluginFormat::Vst3, "abcdef"));
        // Ids are per format.
        assert!(!s.contains(PluginFormat::Clap, "ABCDEF"));
        let again = SandboxDefaults::open(&db).list();
        assert_eq!(
            again,
            [
                SandboxDefault {
                    format: PluginFormat::Vst3,
                    plugin_id: "ABCDEF".into(),
                    name: "Verb 2".into()
                },
                SandboxDefault {
                    format: PluginFormat::Clap,
                    plugin_id: "com.acme.synth".into(),
                    name: "com.acme.synth".into()
                },
            ]
        );

        s.set(PluginFormat::Vst3, "ABCDEF", None, false);
        assert!(!s.contains(PluginFormat::Vst3, "ABCDEF"));
        assert_eq!(SandboxDefaults::open(&db).list().len(), 1);
    }

    #[test]
    fn a_damaged_file_is_an_empty_list() {
        let tmp = TempDir::new("sandbox-defaults-bad");
        std::fs::create_dir_all(tmp.path()).unwrap();
        std::fs::write(tmp.path().join(SANDBOX_DEFAULTS_FILE), "{not json").unwrap();
        assert!(SandboxDefaults::open(tmp.path()).list().is_empty());
    }
}
