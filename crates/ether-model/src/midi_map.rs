//! MIDI controller mappings ("MIDI learn"). Roadmap v2, owned by the `midi-learn` node (see
//! `docs/ROADMAP.md`).
//!
//! A mapping binds one incoming MIDI control (a CC or a note on a port/channel) to one
//! target: a device parameter, a mixer control, or a transport action. Mappings are part of
//! the document (saved with the project, undoable), like in Ableton.
//!
//! Runtime flow (see CONTRACTS.md §11.9): the host delivers raw input as
//! `ether_protocol::midi_map::MidiInputEvent`s to the controller, which matches them against
//! the mappings and turns them into ordinary commands (`Device::SetParam`, `Mixer::SetVolume`,
//! transport actions, ...), one gesture per controller movement. Mapped control therefore
//! runs at controller tick rate, not sample-accurately.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::AutomationTarget;
use crate::ids::{MidiMappingId, TrackId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MidiMapping {
    pub id: MidiMappingId,
    pub source: MidiSource,
    pub target: MidiMapTarget,
    /// Normalized output range the control is scaled into (`0 <= min, max <= 1`; `min > max`
    /// inverts the control). Continuous targets map it through the target's `ParamInfo`
    /// (like automation); toggles and actions use the midpoint as threshold.
    pub min: f64,
    pub max: f64,
    pub mode: MidiMapMode,
}

/// Which incoming control a mapping listens to. At most one mapping per source (the model
/// rejects duplicates; `port: None`/`channel: None` are wildcards and count as distinct
/// sources from concrete ones).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct MidiSource {
    /// Input port id (`MidiPort::id` from `Recording::ListInputs`). `None` = any port.
    pub port: Option<String>,
    /// MIDI channel 0..=15. `None` = omni.
    pub channel: Option<u8>,
    pub control: MidiControl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MidiControl {
    /// Control change `number` 0..=127 (value = CC value).
    Cc { number: u8 },
    /// Note `key` 0..=127 (value = velocity on note-on, 0 on note-off).
    Note { key: u8 },
    /// Pitch bend (14-bit value).
    PitchBend,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MidiMapTarget {
    /// A continuous or stepped parameter: device param, track volume/pan or send level.
    /// Same target space (and normalized↔plain mapping) as automation.
    Param {
        target: AutomationTarget,
    },
    TrackMute {
        track: TrackId,
    },
    TrackSolo {
        track: TrackId,
    },
    /// Record-arm (runtime state: not undoable, like `Recording::Arm`).
    TrackArm {
        track: TrackId,
    },
    Transport {
        action: TransportAction,
    },
}

/// Transport actions a MIDI control can trigger (fired when the control crosses the
/// midpoint of `min..max` upwards).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum TransportAction {
    Play,
    Stop,
    TogglePlay,
    ToggleRecord,
    ToggleLoop,
    ToggleMetronome,
    TapTempo,
    /// Jump to the previous / next arrangement marker.
    PreviousMarker,
    NextMarker,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MidiMapMode {
    /// The control value (0..=127, or 14-bit) is the position in `min..max`.
    #[default]
    Absolute,
    /// Endless encoders: each message is an increment, decoded with `encoding`.
    Relative { encoding: RelativeEncoding },
    /// Each press (value crossing the midpoint upwards) flips the target between `min` and
    /// `max`.
    Toggle,
}

/// How relative (endless encoder) CC values encode increments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum RelativeEncoding {
    /// 1..=63 = +1..+63, 65..=127 = -63..-1 (two's complement, 7-bit).
    #[default]
    TwosComplement,
    /// 64 = no change, 65.. = +1.., ..63 = ..-1.
    BinaryOffset,
    /// Bit 6 = sign (1 = negative), bits 0..5 = magnitude.
    SignMagnitude,
}
