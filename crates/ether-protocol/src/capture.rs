//! MIDI capture (v0.3, `capture-midi`; CONTRACTS.md §13.4): the controller always keeps the
//! MIDI played into the engine (every input port, armed or not, playing or stopped) in a
//! bounded ring; `Capture` turns the recent playing into a clip, like Ableton's Capture.
//!
//! Not a document command (never in a `Batch`): `Capture` makes one undoable edit itself
//! (the clip, its notes and expression; when stopped and `adopt_tempo`, the tempo and loop
//! too). The buffer is runtime, site-local and never saved or replicated.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, NoteId, TrackId};

/// Longest span the capture buffer keeps (seconds of playing, newest kept).
pub const CAPTURE_MAX_SECONDS: f64 = 600.0;
/// Most MIDI messages the capture buffer keeps (newest kept).
pub const CAPTURE_MAX_EVENTS: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CaptureCommand {
    /// Create a MIDI clip `clip` on the MIDI track `track` from the captured playing that
    /// reached that track's input (its `TrackInput::Midi` port/channel filter; all ports when
    /// it has none). Notes get `derive_id(seed_notes, i)` in (start, pitch) order.
    ///
    /// - While playing: notes keep their song positions (the clip spans the played bars).
    /// - While stopped: the phrase is placed at the playhead; with `adopt_tempo` the tempo
    ///   is inferred from the playing (and the loop set to the clip) as one undo step with the
    ///   clip. Replies `Captured`; `InvalidState` when nothing was played.
    Capture {
        track: TrackId,
        clip: ClipId,
        seed_notes: NoteId,
        adopt_tempo: bool,
    },
    /// Forget the buffer (also done on project close).
    Clear,
    /// Replies `CaptureStatus`.
    Status,
}

/// Reply to `Capture`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CaptureResult {
    pub clip: ClipId,
    pub start: Beats,
    pub length: Beats,
    pub notes: u32,
    /// The tempo the project was set to (stopped + `adopt_tempo`), else `None`.
    pub bpm: Option<f64>,
}

/// Reply to `Status` and payload of `CaptureEvent::Changed`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct CaptureStatus {
    /// Something can be captured (enables the Capture button).
    pub available: bool,
    /// Note-ons in the buffer.
    pub notes: u32,
    /// Span of the buffer in seconds.
    pub seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CaptureEvent {
    /// `available` changed (sent on change only, not per note).
    Changed { status: CaptureStatus },
}
