//! Tempo map editing and metronome settings (roadmap v2, `tempo-metronome` node).
//!
//! `Transport::SetTempo` / `SetTimeSignature` keep editing "the point in effect at the
//! playhead"; these commands edit the tempo map as a list of points. All are undoable
//! document edits.
//!
//! Invariants (enforced by the model): a tempo point and a time signature at beat 0 always
//! exist, so the ones at 0 can be edited but not removed or moved away from 0. BPM is
//! clamped to 20..=999 by the controller.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{
    Beats, Decibels, MetronomeSound, TempoCurve, TempoPointId, TimeSignature, TimeSignatureId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TempoCommand {
    /// Insert a tempo point (client-chosen id). Idempotent on an existing id.
    AddTempoPoint {
        id: TempoPointId,
        time: Beats,
        bpm: f64,
        curve: TempoCurve,
    },
    /// Partial edit; `None` fields are unchanged. Send with a gesture while dragging.
    EditTempoPoint {
        id: TempoPointId,
        time: Option<Beats>,
        bpm: Option<f64>,
        curve: Option<TempoCurve>,
    },
    /// Remove tempo points (not the one at beat 0: `InvalidArgument`).
    RemoveTempoPoints { ids: Vec<TempoPointId> },
    AddTimeSignature {
        id: TimeSignatureId,
        time: Beats,
        signature: TimeSignature,
    },
    EditTimeSignature {
        id: TimeSignatureId,
        time: Option<Beats>,
        signature: Option<TimeSignature>,
    },
    /// Remove time-signature changes (not the one at beat 0).
    RemoveTimeSignatures { ids: Vec<TimeSignatureId> },
    /// Metronome settings (the on/off switch stays `Transport::SetMetronome`). `None`
    /// fields are unchanged. Volume is clamped to -144..=+6 dB.
    SetMetronomeSettings {
        volume: Option<Decibels>,
        accent: Option<bool>,
        sound: Option<MetronomeSound>,
    },
}
