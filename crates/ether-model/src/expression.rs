//! MIDI expression (v0.3, contracts-4; CONTRACTS.md §13.2): clip expression lanes (CC, pitch
//! bend, channel pressure; owned by `midi-expression`), per-note expressions (poly pressure
//! by `midi-expression`; pitch and timbre by `mpe`) and a track's MPE settings (`mpe`).
//!
//! # Model
//! - An [`ExpressionLane`] belongs to one **MIDI clip** and holds one channel-wide curve:
//!   a MIDI CC (`Cc { controller }`, 0..=119), pitch bend or channel pressure. At most one
//!   lane per `(clip, kind)`. Times are **content-relative beats** (like notes), so lanes
//!   move, loop and duplicate with their clip.
//! - A [`NoteExpression`] belongs to one **note** and holds one per-note curve
//!   ([`NoteExpressionKind`]: `Pitch`, `Pressure`, `Timbre`). At most one per `(note, kind)`.
//!   Times are relative to the note start; points after the note end are ignored at playback
//!   (they are kept so lengthening the note brings them back).
//! - Both hold their curve as **one value** (`points`, sorted by time): a curve is the unit of
//!   editing and of collab last-writer-wins (a recorded performance has thousands of points;
//!   one entity per point would be far too heavy). Editing commands replace a whole curve or
//!   a time range of it (`ExpressionCommand::{SetPoints, ReplaceRange}`).
//!
//! # Values
//! | kind | range | MIDI 1.0 rendering |
//! |---|---|---|
//! | `Cc { controller }` | 0..=1 | CC value `round(v·127)` |
//! | `PitchBend` | -1..=1 | 14-bit bend `8192 + round(v·8191)`; semitones = the receiver's bend range |
//! | `ChannelPressure` | 0..=1 | channel aftertouch `round(v·127)` |
//! | note `Pitch` | -[`MAX_NOTE_PITCH_OFFSET`]..=+ semitones | MPE: per-channel bend over `MpeSettings::note_pitch_range` |
//! | note `Pressure` | 0..=1 | poly aftertouch (non-MPE) / channel pressure on the note's MPE channel |
//! | note `Timbre` | 0..=1 | MPE: CC 74 on the note's channel (non-MPE receivers: ignored) |
//!
//! Curves interpolate between points with the point's [`CurveShape`] (like automation);
//! before the first point the value is the first point's; after the last, the last's. A
//! lane with no points sends nothing. Playback (engine): the controller compiles curves into
//! `ether_core::expression` descs; built-ins and plugins receive channel expression as raw
//! MIDI (`EventKind::Midi`) and per-note expression as `EventKind::NoteExpression` (CLAP
//! note expressions are the reference model; plugins without them get MPE MIDI when the
//! track has [`MpeSettings`], else poly aftertouch for `Pressure` only).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::CurveShape;
use crate::ids::{ClipId, ExpressionLaneId, NoteExpressionId, NoteId};
use crate::value::Beats;

/// Most points in one curve (lane or note expression).
pub const MAX_EXPRESSION_POINTS: usize = 16_384;
/// Highest CC number of an expression lane (120..=127 are channel mode messages).
pub const MAX_EXPRESSION_CC: u8 = 119;
/// Largest per-note pitch offset, in semitones (either direction).
pub const MAX_NOTE_PITCH_OFFSET: f32 = 96.0;

/// What a clip expression lane controls (channel-wide).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExpressionKind {
    /// MIDI control change `controller` (0..=119), value 0..=1.
    Cc { controller: u8 },
    /// Pitch bend, -1..=1.
    PitchBend,
    /// Channel pressure (aftertouch), 0..=1.
    ChannelPressure,
}

impl ExpressionKind {
    /// Value range of this kind.
    pub fn range(self) -> (f32, f32) {
        match self {
            Self::PitchBend => (-1.0, 1.0),
            Self::Cc { .. } | Self::ChannelPressure => (0.0, 1.0),
        }
    }
}

/// What a per-note expression controls (CLAP note expressions: `TUNING`, `PRESSURE`,
/// `BRIGHTNESS`). Append-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub enum NoteExpressionKind {
    /// Pitch offset in semitones (`mpe`), ±[`MAX_NOTE_PITCH_OFFSET`].
    Pitch,
    /// Pressure / poly aftertouch, 0..=1 (`midi-expression`).
    Pressure,
    /// Timbre ("slide", MPE CC 74, CLAP brightness), 0..=1 (`mpe`).
    Timbre,
}

impl NoteExpressionKind {
    /// Value range of this kind.
    pub fn range(self) -> (f32, f32) {
        match self {
            Self::Pitch => (-MAX_NOTE_PITCH_OFFSET, MAX_NOTE_PITCH_OFFSET),
            Self::Pressure | Self::Timbre => (0.0, 1.0),
        }
    }
}

/// One breakpoint of an expression curve.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ExpressionPoint {
    /// Content-relative beats (lanes) or beats from the note start (note expressions), >= 0.
    pub time: Beats,
    /// In the kind's range (see the module table).
    pub value: f32,
    /// Shape from this point to the next.
    pub curve: CurveShape,
}

/// A channel-wide expression curve of a MIDI clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ExpressionLane {
    pub id: ExpressionLaneId,
    /// A MIDI clip.
    pub clip: ClipId,
    pub kind: ExpressionKind,
    /// Sorted by time (ties keep their order: a jump), at most [`MAX_EXPRESSION_POINTS`].
    pub points: Vec<ExpressionPoint>,
}

/// A per-note expression curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NoteExpression {
    pub id: NoteExpressionId,
    pub note: NoteId,
    pub kind: NoteExpressionKind,
    /// Sorted by time from the note start, at most [`MAX_EXPRESSION_POINTS`].
    pub points: Vec<ExpressionPoint>,
}

/// Which MPE zone a track's input and output use (MIDI MPE spec: the lower zone's master
/// channel is 1 with member channels 2.., the upper zone's master is 16 with members ..15).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum MpeZone {
    #[default]
    Lower,
    Upper,
}

/// MPE settings of a MIDI track (`Track::mpe`, owned by `mpe`). `Some` = the track's MIDI
/// input is read as MPE (each member channel's bend/pressure/CC 74 become the note's
/// `Pitch`/`Pressure`/`Timbre` expressions when recording and monitoring) and plugins
/// without CLAP/VST3 note expressions receive MPE MIDI. `None` = plain MIDI (v0.2).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MpeSettings {
    pub zone: MpeZone,
    /// Member channels, 1..=15.
    pub member_channels: u8,
    /// Per-note pitch-bend range in semitones (MPE default 48), 1..=96.
    pub note_pitch_range: f32,
    /// Master-channel pitch-bend range in semitones (default 2), 0..=96.
    pub master_pitch_range: f32,
}

impl Default for MpeSettings {
    fn default() -> Self {
        Self {
            zone: MpeZone::Lower,
            member_channels: 15,
            note_pitch_range: 48.0,
            master_pitch_range: 2.0,
        }
    }
}

/// Check a curve: finite, non-negative, sorted times; values in `range`; valid tensions;
/// at most [`MAX_EXPRESSION_POINTS`]. `Err` carries a message.
pub fn check_points(points: &[ExpressionPoint], range: (f32, f32)) -> Result<(), String> {
    if points.len() > MAX_EXPRESSION_POINTS {
        return Err(format!(
            "an expression curve holds at most {MAX_EXPRESSION_POINTS} points"
        ));
    }
    let mut last = 0.0f64;
    for p in points {
        if !(p.time.0.is_finite() && p.time.0 >= 0.0) {
            return Err("expression point times must be finite and >= 0".into());
        }
        if p.time.0 < last {
            return Err("expression points must be sorted by time".into());
        }
        last = p.time.0;
        if !(p.value.is_finite() && p.value >= range.0 && p.value <= range.1) {
            return Err(format!(
                "expression value {} outside {}..={}",
                p.value, range.0, range.1
            ));
        }
        if let CurveShape::Curve { tension } = p.curve
            && !(tension.is_finite() && (-1.0..=1.0).contains(&tension))
        {
            return Err("curve tension must be in -1..=1".into());
        }
    }
    Ok(())
}

/// Check MPE settings ranges.
pub fn check_mpe(m: &MpeSettings) -> Result<(), String> {
    if !(1..=15).contains(&m.member_channels) {
        return Err("MPE member channels must be 1..=15".into());
    }
    if !(m.note_pitch_range.is_finite() && (1.0..=96.0).contains(&m.note_pitch_range)) {
        return Err("MPE note pitch range must be 1..=96 semitones".into());
    }
    if !(m.master_pitch_range.is_finite() && (0.0..=96.0).contains(&m.master_pitch_range)) {
        return Err("MPE master pitch range must be 0..=96 semitones".into());
    }
    Ok(())
}
