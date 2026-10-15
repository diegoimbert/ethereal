//! Modulation (v0.2, Bitwig-style; owned by the `racks-modulation` node; CONTRACTS.md §12.6).
//!
//! # Model
//! - A [`Modulator`] lives **inside a device** of a track chain (`Modulator::device`: built-in,
//!   plugin, rack or drum rack; not a device on a drum pad or a rack chain in v0.2, so every
//!   host is a chain entry the engine's `pre_node` hook sees; a rack's modulators reach its
//!   chain devices). Its kind ([`ModulatorKind`]) has a fixed param table
//!   (`ether_devices::modulators::descriptor(kind)`, ids append-only), values in
//!   `Modulator::params` (plain units, missing = default), exactly like device params.
//! - A [`ModMapping`] routes a [`ModSource`] to one param of one device with a bipolar
//!   `depth` in `-1..=1` (normalized param units). Sources:
//!   - `Modulator { modulator }`: may target params of the modulator's host device and, when
//!     the host is a rack, params of the devices on the rack's chains; never the rack's own
//!     macros (no modulation of modulation in v0.2).
//!   - `Macro { rack, index }`: macro `index` (`0..RACK_MACROS`) of a rack; may target params
//!     of the devices on that rack's chains, and the rack's own params from
//!     `RACK_SELECTOR_PARAM` on (never its macros).
//! - One mapping per `(source, device, param)`.
//!
//! # Composition (frozen)
//! For a target param with normalized base value `b`:
//! `effective = clamp(b + Σ depth_i · m_i(t), 0, 1)`, mapped to plain with the param's
//! `ParamInfo` scale.
//! - `b` = the arrangement automation value when an enabled lane drives the param (clip
//!   envelopes included, same precedence as v0.1), otherwise the document value (set by the
//!   UI, `SetParam`, MIDI learn, presets). Automation, MIDI learn and knob edits always
//!   address the **base**; modulation never writes the document.
//! - `m_i` ranges: bipolar `-1..=1` for LFOs and random/step sources (`ModulatorKind::bipolar`),
//!   unipolar `0..=1` for envelopes, envelope followers and macros (macro = its normalized
//!   value).
//! - Stepped params (labels) snap after the sum. Modulation is evaluated by the engine on the
//!   same grid as sample-accurate automation (CONTRACTS.md §12.7) and reaches nodes as ordinary
//!   `EventKind::Param` events.
//! - **Plugins:** host-side modulation sends the effective value as parameter changes (CLAP
//!   `CLAP_EVENT_PARAM_VALUE`, VST3 `IParameterChanges`, AU scheduled params). Caveat: the
//!   plugin sees ordinary parameter changes, so its own GUI shows the modulated value, it may
//!   mark its state dirty, and the resolution is the engine's param grid. The document keeps
//!   the base (the controller ignores `ParamEdited` echoes of modulated values). CLAP
//!   `CLAP_EVENT_PARAM_MOD` (non-destructive offsets) may be used later where supported.
//! - The UI shows `base` on the knob and the effective value as a ring
//!   (`AnalysisData::Modulation` frames through the analysis channel, throttled readback
//!   for watched devices).
//!
//! # Structure
//! Deleting a device cascades (controller, children first): mappings targeting it, mappings
//! from its modulators (and from its macros if it is a rack), its modulators. Modulator params
//! are not automatable in v0.2.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{DeviceId, ModMappingId, ModulatorId, TrackId};
use crate::value::{OrderKey, ParamId};

/// Modulator kinds. Append-only (serialized by name).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum ModulatorKind {
    /// Free-running or tempo-synced LFO (bipolar).
    Lfo,
    /// ADSR envelope triggered by the notes reaching the host device (unipolar; polyphonic
    /// hosts get the most recent voice's envelope in v0.2).
    Envelope,
    /// Envelope follower of the audio at the host device's input (unipolar).
    EnvelopeFollower,
    /// Step sequencer, tempo-synced (bipolar).
    Steps,
    /// Sample-and-hold random (bipolar).
    Random,
    /// Key tracking (unipolar): the last key reaching the host device, placed between the
    /// kind's `Low Key` (0) and `High Key` (1) params.
    Keytrack,
    /// Velocity (unipolar): the last note-on velocity reaching the host device, placed
    /// between the kind's `Low` (0) and `High` (1) params.
    Velocity,
}

impl ModulatorKind {
    /// Every kind, in `ListModulators` order.
    pub const ALL: [ModulatorKind; 7] = [
        Self::Lfo,
        Self::Envelope,
        Self::EnvelopeFollower,
        Self::Steps,
        Self::Random,
        Self::Keytrack,
        Self::Velocity,
    ];

    /// Whether the source range is `-1..=1` (else `0..=1`).
    pub fn bipolar(self) -> bool {
        matches!(self, Self::Lfo | Self::Steps | Self::Random)
    }
}

/// A modulator inside a device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Modulator {
    pub id: ModulatorId,
    /// Host device.
    pub device: DeviceId,
    /// Order among the host's modulators (display only).
    pub order: OrderKey,
    pub name: String,
    pub kind: ModulatorKind,
    /// Plain values of the kind's params; missing = default.
    pub params: BTreeMap<ParamId, f64>,
    /// `EnvelopeFollower` only: follow this track's sidechain tap (post-fader, before its
    /// PDC delay, exactly like a device sidechain, CONTRACTS.md §11.10) instead of the host
    /// device's input. A routing edge `source → host track` (no cycles, not the host's own
    /// track). Deleting the source track cuts it. Omitted when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sidechain: Option<TrackId>,
}

/// A modulation source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ModSource {
    Modulator {
        modulator: ModulatorId,
    },
    /// Macro `index` (`0..RACK_MACROS`) of a rack device.
    Macro {
        rack: DeviceId,
        index: u8,
    },
}

/// Source → device param, with a bipolar depth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ModMapping {
    pub id: ModMappingId,
    pub source: ModSource,
    /// Target device and param.
    pub device: DeviceId,
    pub param: ParamId,
    /// `-1..=1`, in normalized param units.
    pub depth: f64,
}
