//! Plugin GUI mirrors while listening (`plugin-mirror` node; docs/COLLAB.md §9.6).
//!
//! A site listening on a peer hears the host's engine, plugins included: its own plugin
//! instances process nothing useful (the local timeline is held stopped). So while it
//! listens, each plugin device is **swapped for a mirror**: a GUI-only instance
//! (`EngineBridge::create_plugin_mirror`), never activated, never in a graph. The user
//! still opens the plugin's editor (`OpenEditor` falls back to the mirror in the host) and
//! tweaks it: its GUI edits come back through `poll_plugins` as `ParamEdited`, i.e.
//! ordinary undoable, replicated `SetParam`s that the host's live instance applies (so the
//! listener hears the change). Document param changes (remote edits, undo, automation
//! lane edits, the UI's knobs) are pushed into the mirror
//! (`EngineBridge::set_plugin_mirror_param`), so it always shows the shared document.
//!
//! - **When**: from `Listen` (connecting or listening) until the stream ends or the session
//!   is left; the per-site setting ([`EtherController::set_plugin_mirrors`], default on)
//!   turns it off (live instances stay, e.g. to play a plugin instrument while listening).
//!   Devices on **record-armed tracks keep their live instance**: live MIDI through a
//!   plugin instrument and input monitoring still sound locally while listening (§9.5).
//! - **Swap**: the mirror is created from the live instance's current state (else the
//!   document's), then the device's engine node is re-created: a bridge with a mirror
//!   gives the slot a stand-in (dry / silent) node instead of instantiating the plugin.
//!   Back: the mirror is dropped (the bridge keeps its last state) and the node re-created,
//!   so the live instance starts from what the user did in the mirror's GUI.
//! - **Params**: every tick, the document values of each mirrored device that differ from
//!   what was last pushed are pushed (a param reset to its default pushes the default).
//!   The bridge skips values the mirror already shows (a GUI edit's own echo).
//! - **Opaque state** (a preset loaded in the mirror's GUI) replicates at the next save
//!   through the save-time capture (§2.2): the bridge's `plugin_state` reads the mirror.
//!   A state a peer replicates is loaded into the mirror (re-created from it, which
//!   closes its editor; its params are pushed again).
//! - A bridge without mirrors (web, `Unsupported`) is never asked again; a plugin whose
//!   mirror fails (not installed here) keeps its live instance or its missing-plugin
//!   bypass, and is retried only when the device's plugin changes.

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::model::{
    Base64Bytes, DeviceId, DeviceKind, ParamId, PluginFormat, PluginInstance,
};

use crate::store::{Library, ProjectStore};
use crate::{BridgeError, EngineBridge, EtherController, HostServices};

/// What a mirror was built from: a change re-creates it.
#[derive(Clone, Debug, PartialEq)]
struct Sig {
    format: PluginFormat,
    plugin_id: String,
    sandboxed: bool,
}

fn sig_of(plugin: &PluginInstance) -> Sig {
    Sig {
        format: plugin.format,
        plugin_id: plugin.plugin_id.clone(),
        sandboxed: plugin.sandboxed,
    }
}

struct Mirror {
    sig: Sig,
    /// The document's `plugin.state` as of the last check: a different one (a peer's
    /// replicated preset) is loaded into the mirror.
    doc_state: Option<Base64Bytes>,
    /// Values last pushed into the mirror.
    pushed: BTreeMap<ParamId, f64>,
}

/// Mirror state of the controller (one field of `CollabState`: it outlives a session, so
/// leaving one swaps the live instances back).
pub(crate) struct MirrorState {
    /// Setting: swap plugin instances for mirrors while listening.
    enabled: bool,
    /// The bridge has no mirrors (`Unsupported`): never asked again.
    unsupported: bool,
    mirrors: BTreeMap<DeviceId, Mirror>,
    /// Devices whose mirror could not be created, for that plugin.
    failed: BTreeMap<DeviceId, Sig>,
}

impl Default for MirrorState {
    fn default() -> Self {
        Self {
            enabled: true,
            unsupported: false,
            mirrors: BTreeMap::new(),
            failed: BTreeMap::new(),
        }
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Per-site setting: swap plugin instances for GUI mirrors while listening on a peer
    /// (default `true`). Applied at the next tick.
    pub fn set_plugin_mirrors(&mut self, enabled: bool) {
        self.collab.mirror.enabled = enabled;
    }

    /// Devices currently shown through a GUI mirror (sorted).
    pub fn plugin_mirrors(&self) -> Vec<DeviceId> {
        self.collab.mirror.mirrors.keys().copied().collect()
    }

    /// Tick: swap plugin instances for mirrors (and back) as listening starts and ends,
    /// then push document param changes into the mirrors.
    pub(crate) fn collab_mirror_tick(&mut self) {
        let wanted = self.collab_mirror_wanted();
        let m = &mut self.collab.mirror;
        if m.mirrors.is_empty() && wanted.is_empty() {
            m.failed.clear();
            return;
        }
        m.failed
            .retain(|d, sig| wanted.get(d).is_some_and(|(s, _)| s == sig));
        let drop: Vec<DeviceId> = m
            .mirrors
            .iter()
            .filter(|(d, mirror)| wanted.get(d).is_none_or(|(s, _)| *s != mirror.sig))
            .map(|(d, _)| *d)
            .collect();
        for device in drop {
            self.collab_mirror_unswap(device);
        }
        for (device, (sig, plugin)) in wanted {
            let m = &self.collab.mirror;
            if m.unsupported {
                break;
            }
            if m.mirrors.contains_key(&device) || m.failed.get(&device) == Some(&sig) {
                continue;
            }
            self.collab_mirror_swap(device, sig, &plugin);
        }
        self.collab_mirror_reload_states();
        self.collab_mirror_push_params();
    }

    /// A peer replicated a new opaque state (§2.2) for a mirrored device: load it into the
    /// mirror (re-created from it; its params are pushed again). A state this site captured
    /// from the mirror itself (its own save) is only recorded.
    fn collab_mirror_reload_states(&mut self) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let changed: Vec<(DeviceId, PluginInstance)> = self
            .collab
            .mirror
            .mirrors
            .iter()
            .filter_map(|(id, m)| match &doc.project.devices.get(id)?.kind {
                DeviceKind::Plugin { plugin } if plugin.state != m.doc_state => {
                    Some((*id, plugin.clone()))
                }
                _ => None,
            })
            .collect();
        for (device, plugin) in changed {
            let current = self.bridge.plugin_state(device).ok().flatten();
            if plugin.state.is_some() && current != plugin.state {
                // Replaces the mirror in place (its engine slot stays a stand-in).
                if self
                    .bridge
                    .create_plugin_mirror(device, &plugin, plugin.state.as_ref())
                    .is_ok()
                    && let Some(m) = self.collab.mirror.mirrors.get_mut(&device)
                {
                    m.pushed.clear();
                }
            }
            if let Some(m) = self.collab.mirror.mirrors.get_mut(&device) {
                m.doc_state = plugin.state;
            }
        }
    }

    /// The plugin devices to show through mirrors now.
    fn collab_mirror_wanted(&self) -> BTreeMap<DeviceId, (Sig, PluginInstance)> {
        let m = &self.collab.mirror;
        let listening = self.collab_listening_host().is_some();
        let Some(doc) = self.doc.as_ref().filter(|_| m.enabled && listening) else {
            return BTreeMap::new();
        };
        if m.unsupported {
            return BTreeMap::new();
        }
        doc.project
            .devices
            .values()
            .filter(|d| !self.armed.contains(&d.track))
            .filter_map(|d| match &d.kind {
                DeviceKind::Plugin { plugin } => Some((d.id, (sig_of(plugin), plugin.clone()))),
                DeviceKind::Builtin { .. } => None,
            })
            .collect()
    }

    fn collab_mirror_swap(&mut self, device: DeviceId, sig: Sig, plugin: &PluginInstance) {
        // The live instance's current state (what the user hears), else the document's.
        let state = self
            .bridge
            .plugin_state(device)
            .ok()
            .flatten()
            .or_else(|| plugin.state.clone());
        let m = &mut self.collab.mirror;
        match self
            .bridge
            .create_plugin_mirror(device, plugin, state.as_ref())
        {
            Ok(()) => {
                m.mirrors.insert(
                    device,
                    Mirror {
                        sig,
                        doc_state: plugin.state.clone(),
                        pushed: BTreeMap::new(),
                    },
                );
                // The engine slot becomes the bridge's stand-in (the live instance goes).
                self.engine.request_recreate(device);
            }
            Err(BridgeError::Unsupported(_)) => m.unsupported = true,
            Err(_) => {
                m.failed.insert(device, sig);
            }
        }
    }

    fn collab_mirror_unswap(&mut self, device: DeviceId) {
        self.collab.mirror.mirrors.remove(&device);
        let _ = self.bridge.destroy_plugin_mirror(device);
        let exists = self
            .doc
            .as_ref()
            .is_some_and(|d| d.project.devices.contains_key(&device));
        if exists {
            // Back to a live instance, from the mirror's last state (kept by the bridge).
            self.engine.request_recreate(device);
        }
    }

    /// Push the document values that differ from what each mirror was last given.
    fn collab_mirror_push_params(&mut self) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let mut pushes: Vec<(DeviceId, ParamId, f64)> = Vec::new();
        for (id, mirror) in &self.collab.mirror.mirrors {
            let Some(device) = doc.project.devices.get(id) else {
                continue;
            };
            for (param, value) in &device.params {
                if mirror.pushed.get(param) != Some(value) {
                    pushes.push((*id, *param, *value));
                }
            }
            // Dropped from the document = back to the plugin's default.
            let reset: BTreeSet<ParamId> = mirror
                .pushed
                .keys()
                .filter(|p| !device.params.contains_key(p))
                .copied()
                .collect();
            if !reset.is_empty() {
                let defaults = self.engine.descriptor(device).map(|d| d.params);
                for param in reset {
                    let default = defaults
                        .as_ref()
                        .and_then(|ps| ps.iter().find(|p| p.id == param))
                        .map(|p| p.default);
                    match default {
                        Some(v) => pushes.push((*id, param, v)),
                        None => pushes.push((*id, param, f64::NAN)),
                    }
                }
            }
        }
        for (device, param, value) in pushes {
            let Some(mirror) = self.collab.mirror.mirrors.get_mut(&device) else {
                continue;
            };
            if value.is_nan() {
                // No known default: stop tracking it (the mirror keeps its value).
                mirror.pushed.remove(&param);
                continue;
            }
            // Recorded even on failure (e.g. a param the plugin no longer has): a failing
            // push is not retried every tick; the next document change tries again.
            mirror.pushed.insert(param, value);
            let _ = self.bridge.set_plugin_mirror_param(device, param, value);
        }
    }
}
