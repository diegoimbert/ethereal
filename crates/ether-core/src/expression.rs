//! MIDI expression playback (v0.3, contracts-4; owned by the `midi-expression` node, the
//! per-note pitch/timbre and MPE output parts by `mpe`; CONTRACTS.md §13.2).
//!
//! The controller compiles a MIDI track's clip expression lanes and note expressions
//! (`ether_model::expression`) into [`TrackExpressionDesc`] (`TrackDesc::expression`). The
//! engine renders them where MIDI clips are scheduled (the clip stage of the track job):
//!
//! - **Clip lanes** (CC, pitch bend, channel pressure) become raw MIDI events
//!   (`EventKind::Midi`, channel 1 like the clip notes) on the track's first chain entry.
//!   A clip's lanes are active while the clip plays (looped with its content, silent while
//!   the clip is muted). When two playing clips have a lane of the same kind, the later
//!   clip (in `TrackDesc::clips` order) wins. When no bend lane drives the bend any more
//!   (the clip stopped, ended or the lane was removed) and the last bend sent was not the
//!   centre, the centre is sent once.
//! - **Note expressions** become `EventKind::NoteExpression { note_id, .. }` for the
//!   voice of the note (`crate::mixer::NoteSource` links a sounding note to its clip note),
//!   between its `NoteOn` (the first value goes out at the note-on's offset, after it) and
//!   its `NoteOff`. Times are beats from the note start (looped notes restart their curve).
//!
//! # Timing (sample accuracy)
//! Values are evaluated at **knots**, exactly like node-param automation
//! (`crate::automation_rt`): every [`PARAM_GRID`] samples of the absolute engine clock, at
//! the sample of every breakpoint (curve point, clip start/end/loop wrap, note start), and
//! at offset 0 after a timeline jump (play, locate, loop wrap; [`ExpressionRt::reset`]).
//! Each knot is evaluated at the exact beat of its sample (tempo ramps included). A message
//! is sent only when its **quantized** value changes (7-bit CC/pressure, 14-bit bend; see
//! [`lane_quantum`] / [`note_quantum`]; a `NoteExpression` carries the quantized value,
//! [`note_value`]), and after a jump every current value is re-sent.
//! So an offline render is the same for every block size.
//!
//! Lane messages of a knot are pushed **before** the knot's note-ons (a bend set on a
//! note's first sample applies to it): the engine renders lanes before scheduling notes
//! ([`ExpressionRt::render_lanes`]) and note expressions after ([`ExpressionRt::render_notes`]).
//!
//! # Plugins
//! CLAP and VST3 hosts receive lanes as MIDI (VST3 through `IMidiMapping`). A `Pressure`
//! note expression goes out as **poly aftertouch** (`0xA0`) when the track has no MPE
//! settings. `mpe` extends the hosts: CLAP note expressions (`TUNING`, `PRESSURE`,
//! `BRIGHTNESS`), VST3 note expression, or MPE MIDI (one member channel per note) using
//! [`TrackExpressionDesc::mpe`].
//!
//! # MPE (`mpe`, CONTRACTS.md §13.3; [`mpe`])
//! Per-note `Pitch` and `Timbre` flow through [`ExpressionRt::render_notes`] like
//! `Pressure`. On a track with MPE settings (`TrackExpressionDesc::mpe`):
//! - the settings are announced in-band to the chain (the MPE Configuration Message and
//!   pitch-bend sensitivities, [`mpe::config_messages`]) when playback starts, when live
//!   input arrives, and whenever they change ([`ExpressionRt::live_input`] /
//!   [`ExpressionRt::render_lanes`]); switching MPE off announces an empty zone;
//! - live input is read as MPE ([`ExpressionRt::live_input`], [`mpe::MpeIn`]): member
//!   channels' bend / pressure / CC 74 reach the instrument as `NoteExpression`.
//!
//! Receivers: the Poly Synth reads `NoteExpression` directly; plugin hosts send CLAP note
//! expressions / VST3 note expression, else MPE MIDI ([`mpe::MpeOut`]).
//!
//! RT rules: the render methods run on the audio thread. They never allocate (the per-note
//! state is pre-allocated by [`ExpressionRt::compile`], knots live on the stack) and are
//! bounded by the curve data overlapping the sub-block.

pub mod mpe;

use ether_protocol::model::{ClipId, CurveShape, ExpressionKind, MpeSettings, NoteExpressionKind};
use serde::{Deserialize, Serialize};

use crate::automation::curve_fraction;
use crate::automation_rt::{PARAM_GRID, grid};
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::{ClipContentDesc, ClipDesc};
use crate::mixer::{ActiveNote, MAX_ACTIVE_NOTES};
use crate::sched::{self, Timing};

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

    /// The expression of clip `id`.
    pub fn clip(&self, id: ClipId) -> Option<&ClipExpressionDesc> {
        self.clips.iter().find(|c| c.clip == id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipExpressionDesc {
    pub clip: ClipId,
    /// Sorted by kind (one lane per kind).
    pub lanes: Vec<ExpressionLaneDesc>,
    /// Sorted by `(note, kind)` (one curve per note and kind).
    pub notes: Vec<NoteExpressionDesc>,
}

impl ClipExpressionDesc {
    /// The note expressions of note index `note`.
    pub fn note_curves(&self, note: u32) -> &[NoteExpressionDesc] {
        let a = self.notes.partition_point(|n| n.note < note);
        let b = a + self.notes[a..].partition_point(|n| n.note == note);
        &self.notes[a..b]
    }
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

/// Value of a curve at `t` (same law as automation, `crate::automation::evaluate`): the
/// first value before the first point, the last after the last, each segment shaped by its
/// start point's [`CurveShape`]. `None` without points.
pub fn evaluate(points: &[(f64, f32, CurveShape)], t: f64) -> Option<f32> {
    let first = points.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    let i = points.partition_point(|p| p.0 <= t);
    if i >= points.len() {
        return Some(points[points.len() - 1].1);
    }
    let (t0, v0, curve) = points[i - 1];
    let (t1, v1, _) = points[i];
    let span = t1 - t0;
    if span <= 0.0 {
        return Some(v1);
    }
    let x = (t - t0) / span;
    let f = match curve {
        CurveShape::Linear => x,
        CurveShape::Step => 0.0,
        CurveShape::Curve { tension } => curve_fraction(x, tension),
    };
    Some((f64::from(v0) + f64::from(v1 - v0) * f) as f32)
}

/// Pitch-bend centre (14-bit).
pub const BEND_CENTRE: u16 = 8192;

/// The MIDI 1.0 value of a lane value (see the table in `ether_model::expression`): CC and
/// channel pressure `round(v·127)`, bend `8192 + round(v·8191)`.
pub fn lane_quantum(kind: ExpressionKind, v: f32) -> u16 {
    match kind {
        ExpressionKind::PitchBend => {
            (8192.0 + (v.clamp(-1.0, 1.0) * 8191.0).round()).clamp(0.0, 16383.0) as u16
        }
        ExpressionKind::Cc { .. } | ExpressionKind::ChannelPressure => {
            (v.clamp(0.0, 1.0) * 127.0).round() as u16
        }
    }
}

/// The MIDI message of a lane at quantized value `q` (channel 1).
pub fn lane_message(kind: ExpressionKind, q: u16) -> [u8; 3] {
    match kind {
        ExpressionKind::Cc { controller } => [0xB0, controller & 0x7F, (q & 0x7F) as u8],
        ExpressionKind::PitchBend => [0xE0, (q & 0x7F) as u8, ((q >> 7) & 0x7F) as u8],
        ExpressionKind::ChannelPressure => [0xD0, (q & 0x7F) as u8, 0],
    }
}

/// Change resolution of a note expression: `Pressure`/`Timbre` 7-bit (`round(v·127)`, the
/// poly aftertouch / CC 74 value), `Pitch` 1/128 semitone (`round((v + 96)·128)`, finer
/// than a 14-bit bend over the MPE default ±48 semitones).
pub fn note_quantum(kind: NoteExpressionKind, v: f32) -> u16 {
    match kind {
        NoteExpressionKind::Pitch => {
            let m = ether_protocol::model::MAX_NOTE_PITCH_OFFSET;
            ((v.clamp(-m, m) + m) * 128.0).round() as u16
        }
        NoteExpressionKind::Pressure | NoteExpressionKind::Timbre => {
            (v.clamp(0.0, 1.0) * 127.0).round() as u16
        }
    }
}

/// The value a note-expression quantum stands for (what `NoteExpression::value` carries,
/// so a rendering never depends on float error in the curve evaluation).
pub fn note_value(kind: NoteExpressionKind, q: u16) -> f32 {
    match kind {
        NoteExpressionKind::Pitch => {
            f32::from(q) / 128.0 - ether_protocol::model::MAX_NOTE_PITCH_OFFSET
        }
        NoteExpressionKind::Pressure | NoteExpressionKind::Timbre => f32::from(q) / 127.0,
    }
}

const UNSENT: u16 = u16::MAX;
/// Controllers an expression lane can drive (`ether_model::MAX_EXPRESSION_CC` + 1).
const N_CC: usize = 120;
/// Knots (event offsets) per sub-block beyond which breakpoints are dropped (the grid still
/// covers them).
const MAX_KNOTS: usize = 256;
/// Per-note values remembered (sounding notes × kinds).
const NOTE_SLOTS: usize = MAX_ACTIVE_NOTES * 3;

/// Last quantized value sent for one note expression of a sounding note.
#[derive(Clone, Copy, Debug)]
struct NoteLast {
    note_id: u32,
    kind: NoteExpressionKind,
    q: u16,
}

/// Per-track RT state: the last value sent per channel message and per note expression of
/// a sounding note (only changes are sent). Built by [`ExpressionRt::compile`] with the
/// snapshot (non-RT), carried over on snapshot swaps by [`ExpressionRt::inherit`].
#[derive(Debug)]
pub struct ExpressionRt {
    cc: [u16; N_CC],
    bend: u16,
    pressure: u16,
    /// A bend lane drove the bend at the last knot (reset to centre when none does).
    bend_driven: bool,
    notes: Vec<NoteLast>,
    resend_lanes: bool,
    resend_notes: bool,
    /// MPE (`mpe`): the settings last announced to the chain, whether they must be
    /// announced again (transport stopped), and the live MPE input state.
    announced: Option<MpeSettings>,
    reannounce: bool,
    mpe_in: mpe::MpeIn,
}

impl Default for ExpressionRt {
    fn default() -> Self {
        Self {
            cc: [UNSENT; N_CC],
            bend: UNSENT,
            pressure: UNSENT,
            bend_driven: false,
            notes: Vec::new(),
            resend_lanes: false,
            resend_notes: false,
            announced: None,
            reannounce: false,
            mpe_in: mpe::MpeIn::default(),
        }
    }
}

/// Sorted, unique event offsets of a sub-block, each with the beat it must be evaluated at
/// least at (a breakpoint's own beat, so a `Step` switches exactly there).
struct Knots {
    at: [(u32, f64); MAX_KNOTS],
    len: usize,
}

impl Knots {
    fn new() -> Self {
        Self {
            at: [(0, f64::NEG_INFINITY); MAX_KNOTS],
            len: 0,
        }
    }

    fn insert(&mut self, offset: usize, min_beat: f64) {
        let o = offset as u32;
        let knots = &mut self.at[..self.len];
        match knots.binary_search_by(|k| k.0.cmp(&o)) {
            Ok(i) => knots[i].1 = knots[i].1.max(min_beat),
            Err(i) if self.len < MAX_KNOTS => {
                self.at.copy_within(i..self.len, i + 1);
                self.at[i] = (o, min_beat);
                self.len += 1;
            }
            Err(_) => {}
        }
    }

    /// A breakpoint at timeline beat `t`, if its sample is in the sub-block.
    fn breakpoint(&mut self, timing: &Timing<'_>, lo: f64, t: f64) {
        if t > lo && t < timing.b1 {
            let o = timing.sample_at_or_after(t);
            if o < timing.frames {
                self.insert(o, t);
            }
        }
    }

    fn grid(&mut self, timing: &Timing<'_>) {
        for o in grid(timing) {
            self.insert(o, f64::NEG_INFINITY);
        }
    }

    fn as_slice(&self) -> &[(u32, f64)] {
        &self.at[..self.len]
    }
}

const _: () = assert!(PARAM_GRID > 0);

impl ExpressionRt {
    /// Non-RT: the state for a new snapshot of a track with `desc` (allocates the per-note
    /// state when the track has note expressions).
    pub fn compile(desc: &TrackExpressionDesc) -> Self {
        let mut rt = Self::default();
        rt.prepare(desc);
        rt
    }

    /// Non-RT: (re)allocate for `desc`.
    pub fn prepare(&mut self, desc: &TrackExpressionDesc) {
        let has_notes = desc.clips.iter().any(|c| !c.notes.is_empty());
        if has_notes && self.notes.capacity() < NOTE_SLOTS {
            self.notes.reserve_exact(NOTE_SLOTS - self.notes.len());
        }
    }

    /// RT. Carry the sent values over from the previous snapshot's state of this track (no
    /// allocation: per-note values beyond this state's capacity are dropped and re-sent).
    pub(crate) fn inherit(&mut self, old: &mut ExpressionRt) {
        self.cc = old.cc;
        self.bend = old.bend;
        self.pressure = old.pressure;
        self.bend_driven = old.bend_driven;
        self.resend_lanes = old.resend_lanes;
        self.resend_notes = old.resend_notes;
        self.announced = old.announced;
        self.reannounce = old.reannounce;
        self.mpe_in = old.mpe_in;
        self.notes.clear();
        for n in old.notes.drain(..) {
            if self.notes.len() == self.notes.capacity() {
                break;
            }
            self.notes.push(n);
        }
    }

    /// RT: the timeline jumped (play, locate, loop wrap): re-send every current value at the
    /// next render.
    pub fn reset(&mut self) {
        self.resend_lanes = true;
        self.resend_notes = true;
    }

    /// RT: the transport stopped. Centres a bend a lane left off-centre (at `offset`) and
    /// re-sends everything at the next play.
    pub(crate) fn stop(&mut self, offset: u32, events: &mut EventBuffer) {
        if self.bend_driven && self.bend != BEND_CENTRE {
            events.push(ProcessEvent {
                offset,
                kind: EventKind::Midi {
                    data: lane_message(ExpressionKind::PitchBend, BEND_CENTRE),
                },
            });
            self.bend = BEND_CENTRE;
        }
        self.bend_driven = false;
        self.notes.clear();
        self.reannounce = true;
        self.reset();
    }

    /// RT (`mpe`): announce the track's MPE settings to the chain at `offset` when they
    /// changed since the last announcement (or playback restarted): an empty zone for the
    /// previous settings when MPE was switched off or moved, then the new configuration.
    fn announce(&mut self, desc: &TrackExpressionDesc, offset: u32, events: &mut EventBuffer) {
        let again = self.reannounce && desc.mpe.is_some();
        self.reannounce = false;
        if desc.mpe == self.announced && !again {
            return;
        }
        let mut push = |data: [u8; 3]| {
            events.push(ProcessEvent {
                offset,
                kind: EventKind::Midi { data },
            });
        };
        if let Some(old) = &self.announced
            && desc.mpe.is_none_or(|m| m.zone != old.zone)
        {
            for d in mpe::config_messages(None, old) {
                push(d);
            }
        }
        if let Some(m) = &desc.mpe {
            for d in mpe::config_messages(Some(m), m) {
                push(d);
            }
        }
        if desc.mpe != self.announced {
            self.mpe_in.clear();
        }
        self.announced = desc.mpe;
    }

    /// RT (`mpe`): one live MIDI input event for this track at `offset` (monitoring). On an
    /// MPE track, member-channel messages become `NoteExpression`s ([`mpe::MpeIn`]);
    /// otherwise the event is pushed unchanged.
    pub fn live_input(
        &mut self,
        desc: &TrackExpressionDesc,
        offset: u32,
        kind: EventKind,
        events: &mut EventBuffer,
    ) {
        self.announce(desc, offset, events);
        match &desc.mpe {
            Some(m) => self.mpe_in.translate(m, offset, kind, events),
            None => {
                events.push(ProcessEvent { offset, kind });
            }
        }
    }

    /// RT. Push the clip lanes' MIDI messages of the sub-block into `events` (before the
    /// sub-block's note-ons). `clips`: the track's clips starting before the sub-block end.
    pub(crate) fn render_lanes(
        &mut self,
        desc: &TrackExpressionDesc,
        clips: &[ClipDesc],
        timing: &Timing<'_>,
        events: &mut EventBuffer,
    ) {
        let resend = std::mem::take(&mut self.resend_lanes);
        if resend {
            self.cc = [UNSENT; N_CC];
            self.pressure = UNSENT;
            self.bend = UNSENT;
        }
        self.announce(desc, 0, events);
        if desc.clips.is_empty() && !self.bend_driven {
            return;
        }
        let lo = timing.beat_at(-1.0);
        let mut knots = Knots::new();
        if resend {
            knots.insert(0, f64::NEG_INFINITY);
        }
        knots.grid(timing);
        let active = |c: &ClipDesc| !c.muted && c.start + c.length > lo;
        for clip in clips.iter().filter(|c| active(c)) {
            let Some(cx) = desc.clip(clip.id) else {
                continue;
            };
            if cx.lanes.is_empty() {
                continue;
            }
            sched::for_each_piece(clip, lo, timing.b1, |p| {
                knots.breakpoint(timing, lo, p.t0);
                knots.breakpoint(timing, lo, p.t1);
                let c_end = p.c0 + (p.t1 - p.t0);
                for lane in &cx.lanes {
                    let first = lane.points.partition_point(|q| q.0 < p.c0);
                    for q in &lane.points[first..] {
                        if q.0 >= c_end {
                            break;
                        }
                        knots.breakpoint(timing, lo, p.t0 + (q.0 - p.c0));
                    }
                }
            });
        }
        for &(o, min_beat) in knots.as_slice() {
            let beat = timing.beat_at(f64::from(o)).max(min_beat);
            let mut cc = [UNSENT; N_CC];
            let mut bend = UNSENT;
            let mut pressure = UNSENT;
            for clip in clips.iter().filter(|c| !c.muted) {
                let Some(cx) = desc.clip(clip.id) else {
                    continue;
                };
                if cx.lanes.is_empty() {
                    continue;
                }
                let Some(c) = sched::content_at(clip, beat) else {
                    continue;
                };
                for lane in &cx.lanes {
                    let Some(v) = evaluate(&lane.points, c) else {
                        continue;
                    };
                    let q = lane_quantum(lane.kind, v);
                    match lane.kind {
                        ExpressionKind::Cc { controller } => {
                            if let Some(slot) = cc.get_mut(controller as usize) {
                                *slot = q;
                            }
                        }
                        ExpressionKind::PitchBend => bend = q,
                        ExpressionKind::ChannelPressure => pressure = q,
                    }
                }
            }
            let mut send = |kind: ExpressionKind, q: u16| {
                events.push(ProcessEvent {
                    offset: o,
                    kind: EventKind::Midi {
                        data: lane_message(kind, q),
                    },
                });
            };
            for (i, &q) in cc.iter().enumerate() {
                if q != UNSENT && q != self.cc[i] {
                    send(
                        ExpressionKind::Cc {
                            controller: i as u8,
                        },
                        q,
                    );
                    self.cc[i] = q;
                }
            }
            if pressure != UNSENT && pressure != self.pressure {
                send(ExpressionKind::ChannelPressure, pressure);
                self.pressure = pressure;
            }
            if bend != UNSENT {
                self.bend_driven = true;
            } else if self.bend_driven {
                bend = BEND_CENTRE;
                self.bend_driven = false;
            }
            if bend != UNSENT && bend != self.bend {
                send(ExpressionKind::PitchBend, bend);
                self.bend = bend;
            }
        }
    }

    /// RT. Push the note expressions of the sounding notes for the sub-block into `events`
    /// (after the sub-block's note-ons and before its note-offs were handled: `notes` still
    /// holds the notes ending in it). `clips`: as in [`Self::render_lanes`].
    pub(crate) fn render_notes(
        &mut self,
        desc: &TrackExpressionDesc,
        clips: &[ClipDesc],
        timing: &Timing<'_>,
        notes: &[ActiveNote],
        events: &mut EventBuffer,
    ) {
        let resend = std::mem::take(&mut self.resend_notes);
        if resend {
            self.notes.clear();
        }
        // Forget the notes that stopped sounding.
        self.notes
            .retain(|l| notes.iter().any(|n| n.note_id == l.note_id));
        if self.notes.capacity() == 0 || !desc.clips.iter().any(|c| !c.notes.is_empty()) {
            return;
        }
        let lo = timing.beat_at(-1.0);
        let (_, r1) = timing.event_range();
        // The curves of a sounding note, if its clip note still is the one it started as.
        let curves = |n: &ActiveNote| -> &[NoteExpressionDesc] {
            let Some(cx) = desc.clip(n.source.clip) else {
                return &[];
            };
            let curves = cx.note_curves(n.source.note);
            if curves.is_empty() {
                return curves;
            }
            let same = clips.iter().any(|c| {
                c.id == n.source.clip
                    && matches!(&c.content, ClipContentDesc::Midi { notes }
                        if notes.get(n.source.note as usize).is_some_and(|d| d.key == n.key))
            });
            if same { curves } else { &[] }
        };
        // Sample range `[on, off)` of a note in this sub-block.
        let span = |n: &ActiveNote| -> (u32, u32) {
            let on = if n.source.start < timing.b0 - sched::EVENT_SHIFT {
                0
            } else {
                timing.offset(n.source.start)
            };
            let off = if n.end < r1 {
                if n.end <= timing.b0 {
                    0
                } else {
                    timing.offset(n.end)
                }
            } else {
                timing.frames as u32
            };
            (on, off)
        };
        let mut knots = Knots::new();
        if resend {
            knots.insert(0, f64::NEG_INFINITY);
        }
        knots.grid(timing);
        let mut any = false;
        for n in notes {
            let cs = curves(n);
            if cs.is_empty() {
                continue;
            }
            any = true;
            let (on, off) = span(n);
            if on >= off {
                continue;
            }
            if n.source.start >= timing.b0 - sched::EVENT_SHIFT {
                // Started in this sub-block: its first value goes with the note-on.
                knots.insert(on as usize, n.source.start);
            }
            for c in cs {
                let first = c.points.partition_point(|q| n.source.start + q.0 <= lo);
                for q in &c.points[first..] {
                    let t = n.source.start + q.0;
                    if t >= timing.b1 || t >= n.end {
                        break;
                    }
                    knots.breakpoint(timing, lo, t);
                }
            }
        }
        if !any {
            return;
        }
        for &(o, min_beat) in knots.as_slice() {
            let beat = timing.beat_at(f64::from(o)).max(min_beat);
            for n in notes {
                let cs = curves(n);
                if cs.is_empty() {
                    continue;
                }
                let (on, off) = span(n);
                if o < on || o >= off {
                    continue;
                }
                let rel = (beat - n.source.start).max(0.0);
                for c in cs {
                    let Some(v) = evaluate(&c.points, rel) else {
                        continue;
                    };
                    let q = note_quantum(c.kind, v);
                    let slot = self
                        .notes
                        .iter()
                        .position(|l| l.note_id == n.note_id && l.kind == c.kind);
                    match slot {
                        Some(i) if self.notes[i].q == q => continue,
                        Some(i) => self.notes[i].q = q,
                        None if self.notes.len() < self.notes.capacity() => {
                            self.notes.push(NoteLast {
                                note_id: n.note_id,
                                kind: c.kind,
                                q,
                            })
                        }
                        // Out of slots: send, but without change tracking.
                        None => {}
                    }
                    events.push(ProcessEvent {
                        offset: o,
                        kind: EventKind::NoteExpression {
                            note_id: n.note_id,
                            channel: 0,
                            key: n.key,
                            expression: c.kind,
                            value: note_value(c.kind, q),
                        },
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_evaluate_like_automation() {
        let pts = vec![
            (1.0, 0.0, CurveShape::Linear),
            (2.0, 1.0, CurveShape::Step),
            (3.0, 0.5, CurveShape::Curve { tension: 1.0 }),
            (4.0, 1.0, CurveShape::Linear),
        ];
        assert_eq!(evaluate(&[], 1.0), None);
        assert_eq!(evaluate(&pts, 0.0), Some(0.0));
        assert_eq!(evaluate(&pts, 1.5), Some(0.5));
        assert_eq!(evaluate(&pts, 2.5), Some(1.0));
        assert_eq!(evaluate(&pts, 3.0), Some(0.5));
        // x⁴ at tension 1.
        assert!((evaluate(&pts, 3.5).unwrap() - (0.5 + 0.5 * 0.0625)).abs() < 1e-6);
        assert_eq!(evaluate(&pts, 9.0), Some(1.0));
        // A jump (two points at one time) takes the later value from that time on.
        let jump = vec![
            (0.0, 0.0, CurveShape::Linear),
            (1.0, 0.0, CurveShape::Linear),
            (1.0, 1.0, CurveShape::Linear),
        ];
        assert_eq!(evaluate(&jump, 1.0), Some(1.0));
        let pts_f64: Vec<(f64, f64, CurveShape)> =
            pts.iter().map(|p| (p.0, f64::from(p.1), p.2)).collect();
        for i in 0..80 {
            let t = i as f64 * 0.05;
            let a = evaluate(&pts, t).unwrap();
            let b = crate::automation::evaluate(&pts_f64, t).unwrap();
            assert!((f64::from(a) - b).abs() < 1e-6, "{t}");
        }
    }

    #[test]
    fn quantization_matches_midi_1() {
        let bend = ExpressionKind::PitchBend;
        assert_eq!(lane_quantum(bend, 0.0), BEND_CENTRE);
        assert_eq!(lane_quantum(bend, 1.0), 16383);
        assert_eq!(lane_quantum(bend, -1.0), 1);
        assert_eq!(lane_message(bend, 16383), [0xE0, 0x7F, 0x7F]);
        assert_eq!(lane_message(bend, BEND_CENTRE), [0xE0, 0, 0x40]);
        let cc = ExpressionKind::Cc { controller: 74 };
        assert_eq!(lane_quantum(cc, 0.5), 64);
        assert_eq!(lane_message(cc, 64), [0xB0, 74, 64]);
        assert_eq!(
            lane_message(ExpressionKind::ChannelPressure, 127),
            [0xD0, 127, 0]
        );
        assert_eq!(note_quantum(NoteExpressionKind::Pressure, 1.0), 127);
        assert_eq!(note_quantum(NoteExpressionKind::Pitch, 0.0), 96 * 128);
        assert_ne!(
            note_quantum(NoteExpressionKind::Pitch, 0.01),
            note_quantum(NoteExpressionKind::Pitch, 0.0)
        );
        for kind in [
            NoteExpressionKind::Pitch,
            NoteExpressionKind::Pressure,
            NoteExpressionKind::Timbre,
        ] {
            let (lo, hi) = kind.range();
            let step = (hi - lo) / f32::from(note_quantum(kind, hi) - note_quantum(kind, lo));
            for v in [lo, hi, (lo + hi) / 2.0, lo + 0.3 * (hi - lo)] {
                let back = note_value(kind, note_quantum(kind, v));
                assert!((back - v).abs() <= step / 2.0 + 1e-6, "{kind:?} {v} {back}");
            }
            assert_eq!(note_value(kind, note_quantum(kind, lo)), lo);
            assert_eq!(note_value(kind, note_quantum(kind, hi)), hi);
        }
    }
}
