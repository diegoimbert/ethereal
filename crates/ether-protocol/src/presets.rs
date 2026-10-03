//! Device presets (v0.2, `presets` node; CONTRACTS.md §12.5). File format:
//! `ether_model::preset`.
//!
//! - Factory presets ship embedded in `ether-devices` (`ether_devices::factory_presets`), one
//!   folder per device type under `crates/ether-devices/presets/<device-key>/`; read-only.
//! - User presets live engine-side in the user library
//!   (`<library>/Presets/<device-key>/<file>.etherpreset`, or `<library>/Presets/plugins/
//!   <format>/<plugin id>/...`), written through `Library::{write_file, remove_file, rename_file}`. The UI
//!   never touches files. Hosts without a writable library (web without OPFS library, remote
//!   read-only) reply `Unsupported` to `Save`/`Rename`/`Delete`/`SetMeta`.
//! - `Load` is a document command (one undo step); the others are not undoable.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{DeviceId, PresetDevice, PresetMeta, RackChainId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PresetCommand {
    /// Presets for a device type (`device: None` = all), factory first, then user, each sorted
    /// by name. `text` filters by name/tags/author (case-insensitive substring). Replies
    /// `Presets`.
    List {
        device: Option<PresetDevice>,
        text: Option<String>,
    },
    /// Apply a preset to an existing device of the same type (params, kind data, plugin
    /// state). Undoable. `InvalidArgument` for a preset of another type.
    ///
    /// v0.3 (`rack-presets`): loading a rack preset that stores its structure
    /// (`Preset::rack`) replaces the rack's chains, chain devices, the rack's modulators and
    /// the mappings inside it; every new entity gets `derive_id(seed, i)` in preset order
    /// (chains, then each chain's devices, then modulators, then mappings) so collab replays
    /// mint the same ids. `seed` is required for such presets (`InvalidArgument` without
    /// it) and ignored otherwise. Omitted from JSON when `None`.
    Load {
        device: DeviceId,
        preset: PresetRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        seed: Option<RackChainId>,
    },
    /// Save a device's current state as a user preset. Replies `Preset` (the new entry).
    /// An existing user preset with the same name is replaced only when `overwrite`.
    Save {
        device: DeviceId,
        name: String,
        meta: PresetMeta,
        overwrite: bool,
    },
    /// User presets only. Replies `Preset`.
    Rename { preset: PresetRef, name: String },
    /// User presets only.
    Delete { preset: PresetRef },
    /// User presets only. Replies `Preset`.
    SetMeta { preset: PresetRef, meta: PresetMeta },
}

/// Where a preset comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum PresetSource {
    /// Embedded factory preset (read-only).
    Factory,
    /// A file in the user library.
    User,
}

/// Stable preset reference. `id`: factory = `"<device-key>/<slug>"` (e.g.
/// `"poly-synth/warm-pad"`); user = the path relative to the user library's `Presets/`
/// folder (opaque to the UI).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct PresetRef {
    pub source: PresetSource,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetInfo {
    pub preset: PresetRef,
    pub name: String,
    pub device: PresetDevice,
    pub meta: PresetMeta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PresetEvent {
    /// The user preset set changed (saved, renamed, deleted, meta edited, or files changed on
    /// disk): re-`List`.
    Changed,
}
