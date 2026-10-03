//! MIDI expression playback (v0.3, contracts-4; owned by the `midi-expression` node, the
//! per-note pitch/timbre and MPE output parts by `mpe`; CONTRACTS.md §13.2).
//!
//! The controller compiles a MIDI track's clip expression lanes and note expressions
//! (`ether_model::expression`) into [`TrackExpressionDesc`] (`TrackDesc::expression`). The
//! engine renders them where MIDI clips are scheduled (the clip stage of the track job):
//!
//! - **Clip lanes** (CC, pitch bend, channel pressure) become raw MIDI events
//!   (`EventKind::Midi`) on the track's first chain entry, on the same sub-block grid as
//!   automation (`automation_rt::PARAM_GRID` samples, plus one at every breakpoint), only
//!   when the 7/14-bit value changes; on a loop jump or a locate the current value of every
//!   lane is re-sent at offset 0. A clip's lanes are active while the clip plays (looped with
//!   its content). When a clip with a pitch-bend lane stops, bend is reset to centre.
//! - **Note expressions** become `EventKind::NoteExpression { note_id, .. }` for the note's
//!   voice, between its `NoteOn` and `NoteOff`, on the same grid. A value is sent right after
//!   the `NoteOn` (same offset) when the curve doesn't start at 0.
//! - Plugins: CLAP hosts forward `NoteExpression` as `clap_event_note_expression` (TUNING,
//!   PRESSURE, BRIGHTNESS), VST3 as `NoteExpressionValueEvent` when the plugin supports note
//!   expression; otherwise, with `TrackExpressionDesc::mpe` set, the plugin host rewrites
//!   notes to MPE MIDI (one member channel per note, per-channel bend/pressure/CC 74), and
//!   without MPE only `Pressure` is sent, as poly aftertouch.
//!
//! Hooks (`midi-expression` adds them; shared touches listed in ROADMAP.md "v0.3"): the
//! clip-stage call in `engine.rs` ([`ExpressionRt::render`]) and the plugin hosts' event
//! translation. Until then the desc is compiled empty and nothing is sent (v0.2 behaviour).
//! RT rules: [`ExpressionRt::render`] runs on the audio thread (no allocation; the desc's
//! curves are pre-sorted plain data).

use ether_protocol::model::{ClipId, CurveShape, ExpressionKind, MpeSettings, NoteExpressionKind};
use serde::{Deserialize, Serialize};

use crate::event::EventBuffer;

/// One curve: `(time beats, value, shape to next)`, sorted by time. Times are
/// content-relative beats (lanes) or beats from the note start (note expressions).
pub type CurveDesc = Vec<(f64, f32, CurveShape)>;

/// MIDI expression of one track.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackExpressionDesc {
    /// Clips of the track (`TrackDesc::clips` ids) that have any expression.
    pub clips: Vec<ClipExpressionDesc>,
    /// The track's MPE settings (`Track::mpe`), for plugin MPE output.
    pub mpe: Option<MpeSettings>,
}

impl TrackExpressionDesc {
    /// Nothing to render (the common case).
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty() && self.mpe.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipExpressionDesc {
    pub clip: ClipId,
    pub lanes: Vec<ExpressionLaneDesc>,
    pub notes: Vec<NoteExpressionDesc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpressionLaneDesc {
    pub kind: ExpressionKind,
    pub points: CurveDesc,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteExpressionDesc {
    /// Index of the note in the clip's `ClipContentDesc::Midi { notes }` (sorted by start).
    pub note: u32,
    pub kind: NoteExpressionKind,
    pub points: CurveDesc,
}

/// Per-track RT state (last sent values, so only changes are sent). Pre-wired as a stub:
/// `midi-expression` implements it.
#[derive(Debug, Default)]
pub struct ExpressionRt {
    _private: (),
}

impl ExpressionRt {
    /// Non-RT: (re)build for a new snapshot (keeps last-sent values of clips that remain).
    pub fn prepare(&mut self, desc: &TrackExpressionDesc) {
        let _ = desc;
    }

    /// RT: push the expression events of `[start_beat, end_beat)` (a linear sub-block of
    /// `frames` samples) into `events`. Stub: sends nothing.
    pub fn render(
        &mut self,
        desc: &TrackExpressionDesc,
        start_beat: f64,
        end_beat: f64,
        frames: u32,
        events: &mut EventBuffer,
    ) {
        let _ = (desc, start_beat, end_beat, frames, events);
    }

    /// RT: transport jumped (loop, locate): re-send current values at the next render.
    pub fn reset(&mut self) {}
}
