//! MPE (v0.3, owned by the `mpe` node; CONTRACTS.md §13.3). Three RT-safe pieces, all
//! fixed-size (no allocation):
//!
//! - **Zone layout** ([`master_channel`], [`is_member`], [`members`]) and the pitch
//!   conversions between a 14-bit member bend and semitones ([`bend_semitones`],
//!   [`semitones_bend`]).
//! - [`MpeIn`]: live MPE input of a track with `Track::mpe` (monitoring). Member-channel
//!   pitch bend / channel pressure / CC 74 become `EventKind::NoteExpression` (`Pitch` in
//!   semitones over `note_pitch_range`, `Pressure`, `Timbre`) for the note sounding on that
//!   channel. Messages that arrive on a member channel before its note-on (MPE controllers
//!   set the initial state that way) are held and sent right after the note-on. Master
//!   channel and non-zone messages pass through unchanged.
//! - [`MpeOut`]: what a plugin host uses when the plugin has no per-note expression of its
//!   own: **MPE MIDI**. The engine announces a track's MPE settings in-band with the
//!   standard MPE Configuration Message and pitch-bend sensitivities ([`config_messages`],
//!   sent by `ExpressionRt` on MPE tracks when playback starts or the settings change), so
//!   hosts learn the zone and ranges from the event stream itself (as an MPE synth does).
//!   Once a zone is known, each note gets its own member channel (least recently used),
//!   and its `NoteExpression`s become that channel's bend / channel pressure / CC 74.
//!   Without a zone (no MPE on the track), `Pressure` goes out as poly aftertouch and
//!   `Pitch` / `Timbre` are dropped (CONTRACTS.md §13.2).

use ether_protocol::model::{MAX_NOTE_PITCH_OFFSET, MpeSettings, MpeZone, NoteExpressionKind};

use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::expression::{BEND_CENTRE, lane_message, lane_quantum};
use ether_protocol::model::ExpressionKind;

/// The MPE "timbre" controller.
pub const TIMBRE_CC: u8 = 74;

/// Master channel of a zone (0-based: Lower = channel 1, Upper = channel 16).
pub fn master_channel(m: &MpeSettings) -> u8 {
    match m.zone {
        MpeZone::Lower => 0,
        MpeZone::Upper => 15,
    }
}

/// Member channels of a zone (0-based, inclusive): Lower `1..=n`, Upper `15-n..=14`.
pub fn members(m: &MpeSettings) -> std::ops::RangeInclusive<u8> {
    let n = m.member_channels.clamp(1, 15);
    match m.zone {
        MpeZone::Lower => 1..=n,
        MpeZone::Upper => (15 - n)..=14,
    }
}

/// Whether 0-based `channel` is a member channel of the zone.
pub fn is_member(m: &MpeSettings, channel: u8) -> bool {
    members(m).contains(&channel)
}

/// A member channel's 14-bit bend as semitones over `range` (`8192 ± 8191` = `±range`,
/// like the bend lanes; clamped to ±[`MAX_NOTE_PITCH_OFFSET`]).
pub fn bend_semitones(raw: u16, range: f32) -> f32 {
    let v = ((f32::from(raw.min(16383)) - 8192.0) / 8191.0).clamp(-1.0, 1.0);
    (v * range).clamp(-MAX_NOTE_PITCH_OFFSET, MAX_NOTE_PITCH_OFFSET)
}

/// Semitones as a member channel's 14-bit bend over `range` (clamped to the range).
pub fn semitones_bend(semitones: f32, range: f32) -> u16 {
    if range <= 0.0 {
        return BEND_CENTRE;
    }
    lane_quantum(ExpressionKind::PitchBend, semitones / range)
}

/// Messages of the in-band MPE announcement (see the module docs).
pub const CONFIG_MESSAGES: usize = 13;

/// The MPE Configuration Message (RPN 6 on the master channel; `None` turns the zone of
/// `off` off), then the pitch-bend sensitivity (RPN 0, semitones + cents) of the master
/// channel and of the first member channel (MPE: applies to every member), then the null
/// RPN. `off`: the previous settings when MPE is switched off.
pub fn config_messages(m: Option<&MpeSettings>, off: &MpeSettings) -> [[u8; 3]; CONFIG_MESSAGES] {
    let (zone, count) = match m {
        Some(m) => (m, m.member_channels.clamp(1, 15)),
        None => (off, 0),
    };
    let master = master_channel(zone);
    let member = *members(zone).start();
    let cc = |ch: u8, c: u8, v: u8| [0xB0 | (ch & 0x0F), c, v & 0x7F];
    let sens = |r: f32| {
        let r = r.clamp(0.0, 96.0);
        let semis = r.floor();
        (semis as u8, ((r - semis) * 100.0).round().min(99.0) as u8)
    };
    let (ms, mc) = sens(m.map_or(0.0, |m| m.master_pitch_range));
    let (ns, nc) = sens(m.map_or(0.0, |m| m.note_pitch_range));
    let mut out = [
        cc(master, 101, 0),
        cc(master, 100, 6),
        cc(master, 6, count),
        cc(master, 101, 0),
        cc(master, 100, 0),
        cc(master, 6, ms),
        cc(master, 38, mc),
        cc(member, 101, 0),
        cc(member, 100, 0),
        cc(member, 6, ns),
        cc(member, 38, nc),
        cc(member, 101, 127),
        cc(member, 100, 127),
    ];
    if m.is_none() {
        // Off: only the MCM matters; the rest re-selects the null RPN.
        for c in &mut out[3..] {
            *c = cc(master, if c[1] == 100 { 100 } else { 101 }, 127);
        }
    }
    out
}

/// The value of a 7-bit controller as 0..=1.
fn unit(v: u8) -> f32 {
    f32::from(v & 0x7F) / 127.0
}

/// Kind index (`Pitch`, `Pressure`, `Timbre`).
fn kind_index(k: NoteExpressionKind) -> usize {
    match k {
        NoteExpressionKind::Pitch => 0,
        NoteExpressionKind::Pressure => 1,
        NoteExpressionKind::Timbre => 2,
    }
}

const KINDS: [NoteExpressionKind; 3] = [
    NoteExpressionKind::Pitch,
    NoteExpressionKind::Pressure,
    NoteExpressionKind::Timbre,
];

/// The note expression a member-channel message stands for, if any.
pub fn member_expression(m: &MpeSettings, data: [u8; 3]) -> Option<(u8, NoteExpressionKind, f32)> {
    let ch = data[0] & 0x0F;
    if !is_member(m, ch) {
        return None;
    }
    match data[0] & 0xF0 {
        0xE0 => {
            let raw = (u16::from(data[2] & 0x7F) << 7) | u16::from(data[1] & 0x7F);
            Some((
                ch,
                NoteExpressionKind::Pitch,
                bend_semitones(raw, m.note_pitch_range),
            ))
        }
        0xD0 => Some((ch, NoteExpressionKind::Pressure, unit(data[1]))),
        0xB0 if data[1] & 0x7F == TIMBRE_CC => {
            Some((ch, NoteExpressionKind::Timbre, unit(data[2])))
        }
        _ => None,
    }
}

/// Live MPE input state of one track (see the module docs).
#[derive(Clone, Copy, Debug, Default)]
pub struct MpeIn {
    /// The sounding note per channel: `(note_id, key)`.
    held: [Option<(u32, u8)>; 16],
    /// Values received on a channel before its note-on.
    pending: [[Option<f32>; 3]; 16],
}

impl MpeIn {
    /// RT: forget every note (settings changed, transport stopped).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// RT. Translate one live event of an MPE track at `offset` into `out`.
    pub fn translate(
        &mut self,
        m: &MpeSettings,
        offset: u32,
        kind: EventKind,
        out: &mut EventBuffer,
    ) {
        let mut push = |kind| {
            out.push(ProcessEvent { offset, kind });
        };
        match kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                ..
            } if is_member(m, channel) => {
                let ch = usize::from(channel & 0x0F);
                self.held[ch] = Some((note_id, key));
                push(kind);
                for (i, v) in std::mem::take(&mut self.pending[ch])
                    .into_iter()
                    .enumerate()
                {
                    if let Some(value) = v {
                        push(EventKind::NoteExpression {
                            note_id,
                            channel,
                            key,
                            expression: KINDS[i],
                            value,
                        });
                    }
                }
            }
            EventKind::NoteOff {
                note_id, channel, ..
            } if is_member(m, channel) => {
                let ch = usize::from(channel & 0x0F);
                if self.held[ch].is_some_and(|h| h.0 == note_id) {
                    self.held[ch] = None;
                }
                self.pending[ch] = [None; 3];
                push(kind);
            }
            EventKind::Midi { data } => match member_expression(m, data) {
                Some((channel, expression, value)) => {
                    let ch = usize::from(channel);
                    match self.held[ch] {
                        Some((note_id, key)) => push(EventKind::NoteExpression {
                            note_id,
                            channel,
                            key,
                            expression,
                            value,
                        }),
                        None => self.pending[ch][kind_index(expression)] = Some(value),
                    }
                }
                None => push(kind),
            },
            _ => push(kind),
        }
    }
}

/// One member channel of [`MpeOut`].
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    note: Option<u32>,
    /// Allocation clock of the last note-on (least recently used first).
    age: u64,
    /// A bend / pressure other than the neutral was sent since the last note-on.
    dirty: bool,
}

/// A zone learned from the MPE Configuration Message.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Zone {
    master: u8,
    first: u8,
    last: u8,
}

/// MPE MIDI output for a plugin without per-note expression (see the module docs).
#[derive(Clone, Debug)]
pub struct MpeOut {
    zone: Option<Zone>,
    note_range: f32,
    /// Selected RPN per channel (`(msb, lsb)`, `(127, 127)` = none).
    rpn: [(u8, u8); 16],
    slots: [Slot; 16],
    clock: u64,
}

impl Default for MpeOut {
    fn default() -> Self {
        Self {
            zone: None,
            note_range: 48.0,
            rpn: [(127, 127); 16],
            slots: [Slot::default(); 16],
            clock: 0,
        }
    }
}

impl MpeOut {
    /// An MPE zone was announced (the track has MPE settings).
    pub fn active(&self) -> bool {
        self.zone.is_some()
    }

    /// The learned per-note pitch range (semitones).
    pub fn note_range(&self) -> f32 {
        self.note_range
    }

    /// RT: learn the MPE configuration from a MIDI message (RPN 6 = MCM, RPN 0 on a member
    /// channel = the per-note pitch range). Hosts that send note expressions natively call
    /// this only to know whether the track is in MPE mode.
    pub fn observe(&mut self, data: [u8; 3]) {
        if data[0] & 0xF0 != 0xB0 {
            return;
        }
        let ch = usize::from(data[0] & 0x0F);
        let v = data[2] & 0x7F;
        match data[1] {
            101 => self.rpn[ch].0 = v,
            100 => self.rpn[ch].1 = v,
            6 => match self.rpn[ch] {
                (0, 6) if ch == 0 || ch == 15 => {
                    let n = v.min(15);
                    self.zone = (n > 0).then(|| {
                        if ch == 0 {
                            Zone {
                                master: 0,
                                first: 1,
                                last: n,
                            }
                        } else {
                            Zone {
                                master: 15,
                                first: 15 - n,
                                last: 14,
                            }
                        }
                    });
                    // MPE: an MCM resets the member pitch range to 48 semitones.
                    self.note_range = 48.0;
                    self.slots = [Slot::default(); 16];
                }
                (0, 0) => {
                    if let Some(z) = self.zone
                        && (z.first..=z.last).contains(&(ch as u8))
                    {
                        self.note_range = f32::from(v).max(1.0);
                    }
                }
                _ => {}
            },
            38 => {
                if self.rpn[ch] == (0, 0)
                    && let Some(z) = self.zone
                    && (z.first..=z.last).contains(&(ch as u8))
                {
                    self.note_range = self.note_range.floor() + f32::from(v.min(99)) / 100.0;
                }
            }
            _ => {}
        }
    }

    fn slot_of(&self, note_id: u32) -> Option<u8> {
        let z = self.zone?;
        (z.first..=z.last).find(|&c| self.slots[usize::from(c)].note == Some(note_id))
    }

    /// The member channel for a new note: a free one used least recently, else the least
    /// recently used one.
    fn allocate(&mut self, z: Zone) -> u8 {
        let range = z.first..=z.last;
        let pick = range
            .clone()
            .filter(|&c| self.slots[usize::from(c)].note.is_none())
            .min_by_key(|&c| self.slots[usize::from(c)].age)
            .or_else(|| range.min_by_key(|&c| self.slots[usize::from(c)].age))
            .unwrap_or(z.first);
        self.clock += 1;
        pick
    }

    /// RT. Translate one engine event into what an MPE-MIDI plugin gets (`emit` receives
    /// engine events; `NoteExpression`s become `EventKind::Midi`).
    pub fn translate(&mut self, kind: &EventKind, mut emit: impl FnMut(EventKind)) {
        match *kind {
            EventKind::Midi { data } => {
                self.observe(data);
                emit(*kind);
            }
            EventKind::NoteOn {
                note_id,
                key,
                velocity,
                ..
            } if self.zone.is_some() => {
                let Some(z) = self.zone else { return };
                let ch = self.allocate(z);
                let slot = &mut self.slots[usize::from(ch)];
                if slot.dirty {
                    // The previous note of this channel left it bent / pressed.
                    let centre = lane_message(ExpressionKind::PitchBend, BEND_CENTRE);
                    emit(EventKind::Midi {
                        data: [0xE0 | ch, centre[1], centre[2]],
                    });
                    emit(EventKind::Midi {
                        data: [0xD0 | ch, 0, 0],
                    });
                }
                *slot = Slot {
                    note: Some(note_id),
                    age: self.clock,
                    dirty: false,
                };
                emit(EventKind::NoteOn {
                    note_id,
                    channel: ch,
                    key,
                    velocity,
                });
            }
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                velocity,
            } => match self.slot_of(note_id) {
                Some(ch) => {
                    self.slots[usize::from(ch)].note = None;
                    emit(EventKind::NoteOff {
                        note_id,
                        channel: ch,
                        key,
                        velocity,
                    });
                }
                None => emit(EventKind::NoteOff {
                    note_id,
                    channel,
                    key,
                    velocity,
                }),
            },
            EventKind::NoteChoke { note_id, key, .. } => match self.slot_of(note_id) {
                Some(ch) => {
                    self.slots[usize::from(ch)].note = None;
                    emit(EventKind::NoteChoke {
                        note_id,
                        channel: ch,
                        key,
                    });
                }
                None => emit(*kind),
            },
            EventKind::AllNotesOff => {
                for s in &mut self.slots {
                    s.note = None;
                }
                emit(*kind);
            }
            EventKind::NoteExpression {
                note_id,
                channel,
                key,
                expression,
                value,
            } => {
                if self.zone.is_none() {
                    if expression == NoteExpressionKind::Pressure {
                        emit(EventKind::Midi {
                            data: [
                                0xA0 | (channel & 0x0F),
                                key & 0x7F,
                                (value.clamp(0.0, 1.0) * 127.0).round() as u8,
                            ],
                        });
                    }
                    return;
                }
                let Some(ch) = self.slot_of(note_id) else {
                    return;
                };
                let data = match expression {
                    NoteExpressionKind::Pitch => {
                        let q = semitones_bend(value, self.note_range);
                        [0xE0 | ch, (q & 0x7F) as u8, ((q >> 7) & 0x7F) as u8]
                    }
                    NoteExpressionKind::Pressure => {
                        [0xD0 | ch, (value.clamp(0.0, 1.0) * 127.0).round() as u8, 0]
                    }
                    NoteExpressionKind::Timbre => [
                        0xB0 | ch,
                        TIMBRE_CC,
                        (value.clamp(0.0, 1.0) * 127.0).round() as u8,
                    ],
                };
                if expression != NoteExpressionKind::Timbre {
                    self.slots[usize::from(ch)].dirty = true;
                }
                emit(EventKind::Midi { data });
            }
            _ => emit(*kind),
        }
    }
}

#[cfg(test)]
mod tests;
