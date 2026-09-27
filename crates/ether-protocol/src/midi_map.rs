//! MIDI learn / controller mappings (roadmap v2, `midi-learn` node).
//!
//! Mappings are document entities (`Project::midi_mappings`, see `ether_model::midi_map`);
//! `Map`/`Unmap`/`Edit` are undoable. Learn mode is runtime state:
//!
//! 1. `Learn { target: Some(t) }` arms learning for target `t` (`Event::MidiMap
//!    { LearnChanged }`).
//! 2. The next incoming control message (from any port) creates or replaces the mapping for
//!    `t` (one undo step, patch emitted) and ends learning (`Learned` + `LearnChanged`).
//! 3. `Learn { target: None }` cancels.
//!
//! Host → controller input reuses the recording input path: natively every port is opened
//! by `ether-native/src/recording/midi.rs` (`MidiInputs::refresh`), whose midir callback
//! already forwards each short message (`[u8; 3]`, channel voice only) to the engine as
//! `ether_core::recording::LiveMidi`. The midi-learn node extends that callback to also know
//! the port id (`MidiPort::id`) and push a [`MidiInputEvent`] into a controller-bound queue
//! drained by `EngineBridge::poll_midi_input` (controller tick). Web: Web MIDI later, same
//! event. Messages keep flowing to monitored/armed tracks; a message consumed by a mapping
//! is *not* filtered from them (like Ableton "remote" + "track").

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{MidiMapMode, MidiMapTarget, MidiMapping, MidiMappingId, MidiSource};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MidiMapCommand {
    /// Enter (`Some`) or leave (`None`) learn mode for a target. Not undoable.
    Learn {
        target: Option<MidiMapTarget>,
    },
    /// Create a mapping explicitly (client-chosen id). A mapping with the same source is
    /// replaced in the same undo step.
    Map {
        mapping: MidiMapping,
    },
    /// Partial edit; `None` fields unchanged.
    Edit {
        id: MidiMappingId,
        min: Option<f64>,
        max: Option<f64>,
        mode: Option<MidiMapMode>,
    },
    Unmap {
        ids: Vec<MidiMappingId>,
    },
    /// Replies `MidiMappings` (the same data as the mirror, sorted by source).
    List,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MidiMapEvent {
    /// Learn mode changed (sent on change and on connect).
    LearnChanged { target: Option<MidiMapTarget> },
    /// A mapping was created by learning (it also arrives as a patch).
    Learned { mapping: MidiMappingId },
    /// Throttled (~10 Hz) activity for UI feedback: the last source seen.
    Activity { source: MidiSource },
}

/// One incoming MIDI short message, host → controller.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MidiInputEvent {
    /// `MidiPort::id` of the input port.
    pub port: String,
    /// Status byte + up to 2 data bytes (channel voice messages only).
    pub data: [u8; 3],
    /// Host receive time, ms on the controller clock (`HostServices::now_ms`).
    pub time_ms: f64,
}
