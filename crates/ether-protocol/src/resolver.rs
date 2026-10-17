//! Resolving missing plugins and audio files (`import-resolver`, owner request; CONTRACTS.md
//! §14.2). Used right after `Project::Import` and at any time later from the project.
//!
//! 1. `Resolve::Propose { installed }` replies [`ResolveProposal`]: every plugin device whose
//!    plugin is not installed (grouped by plugin) with ranked replacement candidates from
//!    deterministic rules ([`MatchRule`]), and every missing audio file.
//! 2. `Resolve::SearchMedia` looks for the missing audio files (library roots, the imported
//!    project's folder and its parents' `Samples/`, an optional folder); results arrive as
//!    [`ResolveEvent::MediaCandidates`], then [`ResolveEvent::MediaSearchDone`].
//! 3. `Resolve::Apply` applies the user's choices in **one undo step** ("Resolve Missing
//!    Items").
//!
//! 4. `Resolve::Replace` swaps **any** device that isn't a rack (built-in, plugin or
//!    missing) for another device of the same role, built-in or plugin, in one undo step
//!    ("Replace Device"), optionally retaining its automation, mappings and parameter
//!    values ([`ReplaceReport`] says what could not be kept).
//!
//! The controller has no plugin catalog (the host scans plugins): the UI passes the
//! installed plugins it got from `Plugin::List`. AI suggestions (optional, UI side, through
//! the AI chat's provider) are never produced here; the UI shows them as [`MatchRule::Ai`]
//! and sends the user's choice with `Apply` like any other.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::media::MediaSource;
use crate::model::{BuiltinDeviceType, DeviceId, MediaId, MissingRole, PluginFormat};
use crate::plugins::PluginDescriptor;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ResolveCommand {
    /// Replies `ResolveProposal`. `installed`: the installed plugins (the UI's
    /// `Plugin::List`); `params`: parameter lists of installed plugins the UI knows
    /// (optional; the controller adds those of plugins loaded in the project). With
    /// `plugins: false` (hosts without plugins, e.g. the web) only audio files are listed.
    Propose {
        installed: Vec<PluginDescriptor>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(optional, as = "Option<Vec<PluginParams>>")]
        params: Vec<PluginParams>,
        #[serde(default = "yes")]
        #[ts(optional, as = "Option<bool>")]
        plugins: bool,
    },
    /// Search the missing audio files (`media`: these only; `None` = every missing one) by
    /// file name, then size, length, sample rate and content hash: the library roots, the
    /// imported project's folder, its `Samples/` and its parents' `Samples/`, and `folder`
    /// (an absolute engine-side folder: the dialog's "Search folder…"; hosts without OS
    /// files reply `Unsupported`). Replies `Unit`; results are events. A new search
    /// replaces a running one.
    SearchMedia {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        media: Option<Vec<MediaId>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        folder: Option<String>,
    },
    /// Stop a running media search (no `MediaSearchDone`). Replies `Unit`.
    CancelSearch,
    /// Apply the chosen replacements, all in one undo step. Items left out stay missing.
    /// Replies `Unit`; a notification lists what could not be applied.
    Apply {
        #[serde(default)]
        plugins: Vec<PluginResolution>,
        #[serde(default)]
        media: Vec<MediaResolution>,
        /// Missing devices (placeholders) replaced by a plugin or a built-in device.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(optional, as = "Option<Vec<DeviceResolution>>")]
        devices: Vec<DeviceResolution>,
    },
    /// Replace any device that isn't a rack (a built-in device, a plugin, installed or
    /// missing, or a missing device placeholder) with another device of the same role, in
    /// one undo step "Replace Device": a plugin (`plugin`) or a built-in device (`builtin`),
    /// exactly one of them. Roles: an audio effect is replaced by an audio effect, an
    /// instrument by an instrument, a MIDI effect by a MIDI effect (checked when both
    /// descriptors are known); racks and drum racks are neither replaced nor replacements
    /// (their chains would go). The device keeps its id, position, enabled state and name
    /// (unless it was the old device's default name), and its sidechain source when the new
    /// device has a sidechain input (else it goes, listed in the report). `retain`: keep the
    /// automation lanes, modulation and MIDI mappings and values of the parameters the new
    /// device also has, matched by id for the same device (the same built-in type, or the
    /// same plugin or another version of it in the same format, whose state is carried) and
    /// by normalized name otherwise; the others are dropped and listed in the report.
    /// Without `retain` the new device starts at its defaults and the device's parameter
    /// lanes and mappings are removed (its on/off lane and modulators stay). `preview`:
    /// reply the report only (nothing changes). Replies `PluginReplaced`.
    Replace {
        device: DeviceId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        plugin: Option<ReplacementPlugin>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        builtin: Option<BuiltinDeviceType>,
        retain: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(optional, as = "Option<bool>")]
        preview: bool,
    },
}

/// The plugin a device switches to (`Resolve::Replace`; an installed plugin's
/// [`PluginDescriptor`] fields).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ReplacementPlugin {
    pub format: PluginFormat,
    pub plugin_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
}

/// How `Resolve::Replace` carries parameters over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ParamMatch {
    /// The same plugin, or another version of it in the same format, or the same built-in
    /// type: its state and parameter ids are kept.
    SameIds,
    /// Matched by normalized parameter name (values converted through their normalized
    /// range when both ranges are known).
    ByName,
    /// The same plugin family in another format, parameter names unknown: by id.
    ByIndex,
    /// Nothing could be matched (the parameter names of one side are unknown).
    Unmatched,
    /// `retain: false`: the new device starts at its defaults.
    Reset,
}

/// What `Resolve::Replace` did (or would do, with `preview`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ReplaceReport {
    pub matched: ParamMatch,
    /// The device state (a plugin's presets and loaded samples) is carried over.
    pub state_kept: bool,
    /// Parameters carried over (with a value, automation or a mapping).
    pub kept: u32,
    /// What could not be kept (parameter names, the plugin state), readable; long lists end
    /// with "and N more parameters".
    pub lost: Vec<String>,
    /// Automation lanes removed (unmatched parameters, or every parameter lane without
    /// `retain`).
    pub lanes_removed: u32,
}

/// Parameter names of an installed plugin (`format` + `id` as in [`PluginDescriptor`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginParams {
    pub format: PluginFormat,
    pub id: String,
    pub params: Vec<ParamName>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ParamName {
    pub id: u32,
    pub name: String,
}

/// A parameter value saved in the project for a missing plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SavedParam {
    pub id: u32,
    pub value: f64,
    /// Known when the imported project named it (Ableton Live sets), or from an earlier
    /// instance.
    pub name: Option<String>,
}

/// What a proposal returns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ResolveProposal {
    pub plugins: Vec<MissingPlugin>,
    pub media: Vec<MissingMedia>,
    /// Missing devices (placeholders for devices Ethereal could not create), grouped by
    /// what they were.
    #[serde(default)]
    pub devices: Vec<MissingDeviceGroup>,
    /// The host can search OS folders (`SearchMedia { folder }`).
    pub folder_search: bool,
}

/// A plugin that is not installed, and the devices using it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MissingPlugin {
    /// Every device using this plugin (same format + id), in project order.
    pub devices: Vec<DeviceId>,
    pub format: PluginFormat,
    pub plugin_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// The project holds a state blob for it.
    pub has_state: bool,
    /// Saved parameter values (of the first device).
    pub params: Vec<SavedParam>,
    /// Installed replacements, best first (empty = no deterministic match).
    pub candidates: Vec<PluginCandidate>,
}

/// Missing devices of one kind (`MissingDevice::source_id`), e.g. every "Live's Hybrid".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MissingDeviceGroup {
    /// Every placeholder of this kind, in project order.
    pub devices: Vec<DeviceId>,
    pub source_id: String,
    /// What it was ("Live's Hybrid").
    pub description: String,
    /// The application it came from ("Ableton Live 12.1"), when known.
    pub origin: String,
    pub role: MissingRole,
    /// Its parameters, with the first device's values (plain).
    pub params: Vec<SavedParam>,
    /// Suggested replacements (built-in devices of the same role, best first).
    pub candidates: Vec<DeviceCandidate>,
}

/// What a missing device is replaced with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum Replacement {
    Builtin { device: BuiltinDeviceType },
    Plugin { plugin: ReplacementPlugin },
}

/// A suggested replacement of a missing device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeviceCandidate {
    pub replacement: Replacement,
    /// Display name ("Reverb").
    pub name: String,
    /// 0..=1.
    pub confidence: f32,
    /// Plain-language reason ("same kind of device: reverb").
    pub reason: String,
}

/// Replace missing devices (`Resolve::Apply`): values, automation lanes and mappings move to
/// the replacement's parameters by normalized name; the rest are dropped (listed in a
/// notification).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeviceResolution {
    pub devices: Vec<DeviceId>,
    pub replacement: Replacement,
}

/// How a candidate was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum MatchRule {
    /// The same plugin id in another format (VST3 class id derived from the VST2 unique
    /// id; AU subtype = VST2 unique id).
    SameId,
    /// The same name and vendor in another format.
    SameName,
    /// The same plugin family (name without version, architecture or format suffixes) and
    /// vendor: another version or format ("Kontakt 7" → "Kontakt").
    SameFamily,
    /// Similar name (fuzzy, above the threshold).
    Similar,
    /// Suggested by the AI provider (UI only; never produced by the controller).
    Ai,
    /// Picked by the user from the installed plugins (UI only).
    Manual,
}

/// An installed plugin proposed as the replacement of a missing one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginCandidate {
    pub format: PluginFormat,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub rule: MatchRule,
    /// 0..=1.
    pub confidence: f32,
    /// Plain-language reason ("same plugin family and vendor: Kontakt 7 → Kontakt").
    pub reason: String,
    /// What the replacement does with the saved state and parameters.
    pub plan: ParamPlan,
    /// What is lost by this replacement (state, unmapped parameters).
    pub lost: Vec<String>,
}

/// What happens to the saved state and parameters when a plugin is replaced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ParamPlan {
    /// The state blob is carried over (the target can load it: same format and family);
    /// parameter ids are kept.
    KeepState,
    /// The state is dropped; saved parameter values (and their automation) move to the
    /// target's parameters (`from` → `to`); unmapped ones are dropped.
    Map { map: Vec<ParamMap> },
    /// The controller decides with the deterministic rules (a manual pick): keep the state
    /// when the target can load it, else map parameters by name (or by index for the same
    /// family).
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ParamMap {
    pub from: u32,
    pub to: u32,
}

/// An audio file the project references but that cannot be read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MissingMedia {
    pub media: MediaId,
    pub name: String,
    /// The path it was referenced at (external references).
    pub path: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    #[ts(type = "number")]
    pub frames: u64,
    /// The project knows its content hash (else format and length are hints from the
    /// imported project).
    pub hashed: bool,
}

/// A file that may be a missing audio file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MediaCandidate {
    pub source: MediaSource,
    /// Where it is, for display (absolute path, or `<library>/<path>`).
    pub display: String,
    /// 0..=1 (1 = same content).
    pub confidence: f32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginResolution {
    /// The devices to switch (all using the same missing plugin, usually).
    pub devices: Vec<DeviceId>,
    pub format: PluginFormat,
    pub plugin_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub plan: ParamPlan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MediaResolution {
    pub media: MediaId,
    /// A file (`Path`, `Location`) or an upload (the dialog's file picker on the web).
    pub source: MediaSource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ResolveEvent {
    /// A media search advanced (folders listed so far).
    MediaProgress { scanned: u32 },
    /// Candidates for one missing file, best first (sent once per searched file, possibly
    /// empty).
    MediaCandidates {
        media: MediaId,
        candidates: Vec<MediaCandidate>,
    },
    /// The search finished.
    MediaSearchDone { scanned: u32 },
}

fn yes() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propose_defaults_on_the_wire() {
        let c: ResolveCommand =
            serde_json::from_str(r#"{"type":"Propose","installed":[]}"#).unwrap();
        assert_eq!(
            c,
            ResolveCommand::Propose {
                installed: vec![],
                params: vec![],
                plugins: true
            }
        );
        let a: ResolveCommand = serde_json::from_str(r#"{"type":"Apply"}"#).unwrap();
        assert_eq!(
            a,
            ResolveCommand::Apply {
                plugins: vec![],
                media: vec![],
                devices: vec![]
            }
        );
    }
}
