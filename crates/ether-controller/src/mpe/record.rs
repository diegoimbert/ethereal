//! MPE input while recording (CONTRACTS.md §13.3): on a track with `Track::mpe`, every
//! member channel carries one note at a time, and that channel's pitch bend (scaled by
//! `note_pitch_range`), channel pressure and CC 74 are the note's `Pitch` / `Pressure` /
//! `Timbre`.
//!
//! - Values received on a member channel before its note-on (MPE controllers send the
//!   initial state that way) become the note's first point (time 0).
//! - Messages after the note-off are the release of a note no longer recorded: dropped.
//! - Curves are `Step` and thinned like the other recorded expression
//!   ([`crate::expression::record::thin`]); a curve that never leaves its neutral value
//!   (pitch 0, pressure 0, timbre 64) is not recorded.
//! - Everything outside the zone's member channels (the master channel, other channels)
//!   is read as on any MIDI track (lanes, poly pressure).

use ether_core::expression::mpe::{is_member, member_expression};
use ether_core::protocol::model::*;

use crate::expression::record::thin;
use crate::recording::{NoteSpecDraft, RecordedMidi};

/// A pass's MIDI read as MPE.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MpeRecorded {
    /// Messages outside the member channels (read as on any track).
    pub channel_events: Vec<RecordedMidi>,
    /// `(index in the pass's notes, kind, curve)`, sorted by note then kind.
    pub notes: Vec<(usize, NoteExpressionKind, Vec<ExpressionPoint>)>,
}

/// The neutral value of a kind (what an MPE note starts from).
fn neutral(kind: NoteExpressionKind) -> f32 {
    match kind {
        NoteExpressionKind::Pitch | NoteExpressionKind::Pressure => 0.0,
        NoteExpressionKind::Timbre => 64.0 / 127.0,
    }
}

const KINDS: [NoteExpressionKind; 3] = [
    NoteExpressionKind::Pitch,
    NoteExpressionKind::Pressure,
    NoteExpressionKind::Timbre,
];

fn index(kind: NoteExpressionKind) -> usize {
    KINDS.iter().position(|k| *k == kind).unwrap_or(0)
}

/// A note sounding on a member channel.
struct Held {
    key: u8,
    position: f64,
    note: Option<usize>,
    curves: [Vec<ExpressionPoint>; 3],
}

/// Read `events` (one pass, sorted, absolute positions) of a clip starting at `from` as
/// MPE. `notes`: the pass's notes (`notes_from_midi(events, from, ..)`); `gap`: the
/// thinning window in beats.
pub(crate) fn read(
    events: &[RecordedMidi],
    from: f64,
    notes: &[NoteSpecDraft],
    gap: f64,
    m: &MpeSettings,
) -> MpeRecorded {
    let mut out = MpeRecorded::default();
    let mut held: [Option<Held>; 16] = Default::default();
    let mut pending: [[Option<f32>; 3]; 16] = [[None; 3]; 16];
    let mut used = vec![false; notes.len()];
    let point = |time: f64, value: f32| ExpressionPoint {
        time: Beats(time.max(0.0)),
        value,
        curve: CurveShape::Step,
    };
    let finish = |h: Held, out: &mut MpeRecorded| {
        let Some(i) = h.note else {
            return;
        };
        for (k, c) in h.curves.into_iter().enumerate() {
            let kind = KINDS[k];
            if c.iter().all(|p| p.value == neutral(kind)) {
                continue;
            }
            let c = thin(&c, gap);
            if !c.is_empty() {
                out.notes.push((i, kind, c));
            }
        }
    };
    for e in events {
        let ch = e.data[0] & 0x0F;
        if !is_member(m, ch) {
            out.channel_events.push(*e);
            continue;
        }
        let c = usize::from(ch);
        let status = e.data[0] & 0xF0;
        let key = e.data[1] & 0x7F;
        let on = status == 0x90 && e.data[2] & 0x7F > 0;
        let off = status == 0x80 || (status == 0x90 && e.data[2] & 0x7F == 0);
        if (on || (off && held[c].as_ref().is_some_and(|h| h.key == key)))
            && let Some(h) = held[c].take()
        {
            finish(h, &mut out);
        }
        if on {
            let start = e.position - from;
            let note = notes
                .iter()
                .enumerate()
                .position(|(i, n)| !used[i] && n.pitch == key && (n.start - start).abs() < 1e-9);
            if let Some(i) = note {
                used[i] = true;
            }
            let mut curves: [Vec<ExpressionPoint>; 3] = Default::default();
            for (k, v) in std::mem::take(&mut pending[c]).into_iter().enumerate() {
                if let Some(v) = v {
                    curves[k].push(point(0.0, v));
                }
            }
            held[c] = Some(Held {
                key,
                position: e.position,
                note,
                curves,
            });
            continue;
        }
        if off {
            pending[c] = [None; 3];
            continue;
        }
        let Some((_, kind, value)) = member_expression(m, e.data) else {
            continue;
        };
        match &mut held[c] {
            Some(h) => h.curves[index(kind)].push(point(e.position - h.position, value)),
            None => pending[c][index(kind)] = Some(value),
        }
    }
    for h in held.into_iter().flatten() {
        finish(h, &mut out);
    }
    out.notes.sort_by_key(|(i, k, _)| (*i, *k));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::notes_from_midi;

    fn ev(position: f64, data: [u8; 3]) -> RecordedMidi {
        RecordedMidi {
            position,
            data,
            pass: 0,
        }
    }

    #[test]
    fn member_channels_become_note_expressions() {
        let m = MpeSettings::default(); // lower zone, 15 members, ±48
        let events = [
            // Initial state of channel 2 before its note-on.
            ev(10.0, [0xE1, 0, 0x40]),
            ev(10.0, [0xD1, 0, 0]),
            ev(10.0, [0xB1, 74, 64]),
            ev(10.0, [0x91, 60, 100]),
            // Channel 3 starts on the same beat with a bend already up.
            ev(10.0, [0xE2, 0x7F, 0x7F]),
            ev(10.0, [0x92, 64, 90]),
            ev(10.5, [0xE1, 0x7F, 0x7F]), // ch 2 +48
            ev(10.5, [0xD1, 100, 0]),
            ev(10.75, [0xB1, 74, 127]),
            // Master channel: a lane as on any track.
            ev(10.8, [0xB0, 1, 30]),
            ev(11.0, [0x81, 60, 0]),
            ev(11.25, [0xE1, 0, 0]), // release: dropped
            ev(11.5, [0x82, 64, 0]),
        ];
        let notes = notes_from_midi(&events, 10.0, 12.0);
        assert_eq!(notes.len(), 2);
        let r = read(&events, 10.0, &notes, 0.0, &m);
        assert_eq!(r.channel_events, vec![events[9]]);
        let i60 = notes.iter().position(|n| n.pitch == 60).unwrap();
        let i64 = notes.iter().position(|n| n.pitch == 64).unwrap();
        let curve = |i: usize, k| {
            r.notes
                .iter()
                .find(|n| n.0 == i && n.1 == k)
                .map(|n| n.2.iter().map(|p| (p.time.0, p.value)).collect::<Vec<_>>())
        };
        use NoteExpressionKind::*;
        assert_eq!(curve(i60, Pitch), Some(vec![(0.0, 0.0), (0.5, 48.0)]));
        assert_eq!(
            curve(i60, Pressure),
            Some(vec![(0.0, 0.0), (0.5, 100.0 / 127.0)])
        );
        assert_eq!(
            curve(i60, Timbre),
            Some(vec![(0.0, 64.0 / 127.0), (0.75, 1.0)])
        );
        // Bent before its note-on: held from time 0. Neutral timbre/pressure: not recorded.
        assert_eq!(curve(i64, Pitch), Some(vec![(0.0, 48.0)]));
        assert_eq!(curve(i64, Pressure), None);
        assert!(
            r.notes
                .iter()
                .all(|n| n.2.iter().all(|p| p.curve == CurveShape::Step))
        );
    }

    #[test]
    fn the_zone_and_range_decide_what_is_a_member() {
        let m = MpeSettings {
            zone: MpeZone::Upper,
            member_channels: 2,
            note_pitch_range: 12.0,
            ..MpeSettings::default()
        };
        // Channel 15 (0-based 14) is a member of the upper zone of 2, channel 2 is not.
        let events = [
            ev(0.0, [0x9E, 60, 100]),
            ev(0.25, [0xEE, 0x7F, 0x7F]),
            ev(0.5, [0xE1, 0x7F, 0x7F]),
            ev(1.0, [0x8E, 60, 0]),
        ];
        let notes = notes_from_midi(&events, 0.0, 2.0);
        let r = read(&events, 0.0, &notes, 0.0, &m);
        assert_eq!(r.channel_events, vec![events[2]]);
        assert_eq!(r.notes.len(), 1);
        assert_eq!(r.notes[0].2[0].value, 12.0);
        assert_eq!(r.notes[0].2[0].time, Beats(0.25));
    }
}
