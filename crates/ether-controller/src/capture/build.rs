//! From buffered messages to a take: which messages form the phrase, its notes (note on/off
//! pairs) and its channel expression (CC, pitch bend, channel pressure). Pure; positions are
//! whatever the caller uses (song beats while playing, seconds while stopped).

use ether_core::protocol::model::Beats;
use ether_core::protocol::model::{
    CurveShape, ExpressionKind, ExpressionPoint, MAX_EXPRESSION_CC, MAX_EXPRESSION_POINTS,
};

/// While stopped, a silence (nothing held) longer than this starts a new phrase: Capture
/// takes the last phrase.
pub const PHRASE_GAP_SECONDS: f64 = 6.0;
/// Expression played up to this long before the first note belongs to the phrase.
pub const PRE_ROLL_SECONDS: f64 = 0.5;
/// Shortest captured note (beats).
pub const MIN_NOTE: f64 = 1.0 / 64.0;

/// One buffered message as the builder sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Msg {
    pub data: [u8; 3],
    /// Seconds (stopped) or song beats (playing), non-decreasing except at loop wraps.
    pub at: f64,
}

/// A note before it gets an id; `start` / `duration` in the caller's unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub pitch: u8,
    pub velocity: f32,
    pub start: f64,
    pub duration: f64,
}

pub fn is_note_on(data: [u8; 3]) -> bool {
    data[0] & 0xf0 == 0x90 && data[2] > 0
}

fn is_note_off(data: [u8; 3]) -> bool {
    let s = data[0] & 0xf0;
    s == 0x80 || (s == 0x90 && data[2] == 0)
}

/// Index of the first message of the last phrase of a stopped take (`at` in seconds): the
/// last note-on played after a silence of at least `gap` (nothing held). Expression up to
/// [`PRE_ROLL_SECONDS`] before it is kept. `None` without notes.
pub fn last_phrase_start(msgs: &[Msg], gap: f64) -> Option<usize> {
    let mut held: Vec<(u8, u8)> = Vec::new();
    let mut quiet_since: Option<f64> = None;
    let mut start = None;
    for (i, m) in msgs.iter().enumerate() {
        let (ch, key) = (m.data[0] & 0x0f, m.data[1]);
        if is_note_on(m.data) {
            if held.is_empty() && quiet_since.is_none_or(|q| m.at - q >= gap) {
                start = Some(i);
            }
            held.push((ch, key));
        } else if is_note_off(m.data)
            && let Some(j) = held.iter().position(|h| *h == (ch, key))
        {
            held.remove(j);
            if held.is_empty() {
                quiet_since = Some(m.at);
            }
        }
    }
    let start = start?;
    let t = msgs[start].at - PRE_ROLL_SECONDS;
    Some(msgs[..start].partition_point(|m| m.at < t))
}

/// Notes from note on/off pairs, sorted by (start, pitch). A note still held ends at `end`; a
/// note-off before its note-on (a loop wrap, `loop_end` given) ends the note at `loop_end`.
pub fn notes(msgs: &[Msg], end: f64, loop_end: Option<f64>) -> Vec<Note> {
    let mut held: Vec<(u8, u8, f64, f32)> = Vec::new();
    let mut out = Vec::new();
    let close = |t0: f64, t1: f64| -> f64 {
        let t1 = if t1 < t0 { loop_end.unwrap_or(t0) } else { t1 };
        (t1 - t0).max(0.0)
    };
    for m in msgs {
        let (ch, key) = (m.data[0] & 0x0f, m.data[1] & 0x7f);
        let on = is_note_on(m.data);
        if !(on || is_note_off(m.data)) {
            continue;
        }
        if let Some(i) = held.iter().position(|h| h.0 == ch && h.1 == key) {
            let (_, _, t0, v) = held.remove(i);
            out.push(Note {
                pitch: key,
                velocity: v,
                start: t0,
                duration: close(t0, m.at),
            });
        }
        if on {
            held.push((ch, key, m.at, f32::from(m.data[2] & 0x7f) / 127.0));
        }
    }
    for (_, key, t0, v) in held {
        out.push(Note {
            pitch: key,
            velocity: v,
            start: t0,
            duration: close(t0, end),
        });
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    out
}

/// The channel expression kind and value (`ExpressionKind` value ranges) of a message.
pub fn expression(data: [u8; 3]) -> Option<(ExpressionKind, f32)> {
    match data[0] & 0xf0 {
        0xb0 if data[1] <= MAX_EXPRESSION_CC => Some((
            ExpressionKind::Cc {
                controller: data[1],
            },
            f32::from(data[2] & 0x7f) / 127.0,
        )),
        0xd0 => Some((
            ExpressionKind::ChannelPressure,
            f32::from(data[1] & 0x7f) / 127.0,
        )),
        0xe0 => {
            let raw = i32::from(data[1] & 0x7f) | (i32::from(data[2] & 0x7f) << 7);
            Some((
                ExpressionKind::PitchBend,
                ((raw - 8192) as f32 / 8191.0).clamp(-1.0, 1.0),
            ))
        }
        _ => None,
    }
}

/// Channel expression lanes (step curves, one per kind in first-played order) with times
/// mapped by `to_beats` (clip-relative beats; earlier points collapse onto 0). Repeated
/// values are dropped; a lane over [`MAX_EXPRESSION_POINTS`] is decimated.
pub fn lanes(
    msgs: &[Msg],
    to_beats: impl Fn(f64) -> f64,
) -> Vec<(ExpressionKind, Vec<ExpressionPoint>)> {
    let mut out: Vec<(ExpressionKind, Vec<(f64, f32)>)> = Vec::new();
    for m in msgs {
        let Some((kind, value)) = expression(m.data) else {
            continue;
        };
        let t = to_beats(m.at);
        if !t.is_finite() {
            continue;
        }
        let i = match out.iter().position(|l| l.0 == kind) {
            Some(i) => i,
            None => {
                out.push((kind, Vec::new()));
                out.len() - 1
            }
        };
        out[i].1.push((t.max(0.0), value));
    }
    out.into_iter()
        .map(|(kind, mut pts)| {
            // Loop passes interleave: sort by time (stable, so a jump keeps its order).
            pts.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut curve: Vec<ExpressionPoint> = Vec::with_capacity(pts.len());
            for (t, v) in pts {
                if let Some(last) = curve.last_mut() {
                    if last.time.0 == t {
                        last.value = v;
                        continue;
                    }
                    if last.value == v {
                        continue;
                    }
                }
                curve.push(ExpressionPoint {
                    time: Beats(t),
                    value: v,
                    curve: CurveShape::Step,
                });
            }
            (kind, decimate(curve))
        })
        .collect()
}

fn decimate(curve: Vec<ExpressionPoint>) -> Vec<ExpressionPoint> {
    if curve.len() <= MAX_EXPRESSION_POINTS {
        return curve;
    }
    let step = curve.len().div_ceil(MAX_EXPRESSION_POINTS);
    let last = *curve.last().expect("non-empty");
    let mut out: Vec<ExpressionPoint> = curve.into_iter().step_by(step).collect();
    if out.last() != Some(&last) {
        if out.len() == MAX_EXPRESSION_POINTS {
            out.pop();
        }
        out.push(last);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(at: f64, key: u8) -> Msg {
        Msg {
            data: [0x90, key, 100],
            at,
        }
    }
    fn off(at: f64, key: u8) -> Msg {
        Msg {
            data: [0x80, key, 0],
            at,
        }
    }

    #[test]
    fn pairs_notes_and_closes_held_ones() {
        let msgs = [
            on(0.0, 60),
            on(0.5, 64),
            off(1.0, 60),
            Msg {
                data: [0x90, 64, 0],
                at: 1.5,
            },
            on(2.0, 67),
        ];
        let n = notes(&msgs, 3.0, None);
        assert_eq!(n.len(), 3);
        assert_eq!((n[0].pitch, n[0].start, n[0].duration), (60, 0.0, 1.0));
        assert_eq!((n[1].pitch, n[1].start, n[1].duration), (64, 0.5, 1.0));
        assert_eq!((n[2].pitch, n[2].start, n[2].duration), (67, 2.0, 1.0));
        assert!((n[0].velocity - 100.0 / 127.0).abs() < 1e-6);
    }

    #[test]
    fn a_note_held_over_a_loop_wrap_ends_at_the_loop_end() {
        let msgs = [on(7.5, 60), off(0.25, 60)];
        let n = notes(&msgs, 1.0, Some(8.0));
        assert_eq!((n[0].start, n[0].duration), (7.5, 0.5));
    }

    #[test]
    fn the_last_phrase_starts_after_a_long_silence() {
        let msgs = [
            on(0.0, 60),
            off(0.5, 60),
            Msg {
                data: [0xb0, 1, 10],
                at: 9.8,
            },
            on(10.0, 62),
            off(10.5, 62),
            on(11.0, 64), // short gap: same phrase
            off(11.5, 64),
        ];
        assert_eq!(last_phrase_start(&msgs, PHRASE_GAP_SECONDS), Some(2));
        assert_eq!(last_phrase_start(&msgs[..2], PHRASE_GAP_SECONDS), Some(0));
        assert_eq!(
            last_phrase_start(
                &[Msg {
                    data: [0xb0, 1, 1],
                    at: 0.0
                }],
                6.0
            ),
            None
        );
        // A note held across the silence keeps one phrase.
        let held = [
            on(0.0, 48),
            on(1.0, 60),
            off(1.5, 60),
            on(9.0, 62),
            off(9.5, 62),
            off(10.0, 48),
        ];
        assert_eq!(last_phrase_start(&held, PHRASE_GAP_SECONDS), Some(0));
    }

    #[test]
    fn expression_values_follow_the_contract_ranges() {
        assert_eq!(
            expression([0xb1, 1, 127]),
            Some((ExpressionKind::Cc { controller: 1 }, 1.0))
        );
        assert_eq!(expression([0xb0, 120, 0]), None);
        assert_eq!(
            expression([0xd0, 0, 0]),
            Some((ExpressionKind::ChannelPressure, 0.0))
        );
        assert_eq!(
            expression([0xe0, 0, 64]),
            Some((ExpressionKind::PitchBend, 0.0))
        );
        assert_eq!(
            expression([0xe0, 0x7f, 0x7f]),
            Some((ExpressionKind::PitchBend, 1.0))
        );
        assert_eq!(
            expression([0xe0, 0, 0]),
            Some((ExpressionKind::PitchBend, -1.0))
        );
        assert_eq!(expression([0xa0, 60, 10]), None);
    }

    #[test]
    fn lanes_are_step_curves_without_repeats() {
        let msgs = [
            Msg {
                data: [0xb0, 64, 127],
                at: 0.5,
            },
            Msg {
                data: [0xb0, 64, 127],
                at: 1.0,
            },
            Msg {
                data: [0xb0, 64, 0],
                at: 2.0,
            },
            Msg {
                data: [0xe0, 0, 64],
                at: -1.0,
            },
            Msg {
                data: [0xe0, 0, 96],
                at: -0.5,
            },
        ];
        let l = lanes(&msgs, |t| t);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].0, ExpressionKind::Cc { controller: 64 });
        let pts: Vec<(f64, f32)> = l[0].1.iter().map(|p| (p.time.0, p.value)).collect();
        assert_eq!(pts, vec![(0.5, 1.0), (2.0, 0.0)]);
        assert!(l[0].1.iter().all(|p| p.curve == CurveShape::Step));
        // Pre-roll collapses onto 0 with the latest value.
        assert_eq!(l[1].1.len(), 1);
        assert_eq!(l[1].1[0].time.0, 0.0);
        assert!((l[1].1[0].value - 0.5).abs() < 0.01);
    }

    #[test]
    fn long_lanes_are_decimated_to_the_cap() {
        let msgs: Vec<Msg> = (0..40_000)
            .map(|i| Msg {
                data: [0xb0, 1, (i % 128) as u8],
                at: f64::from(i) * 0.001,
            })
            .collect();
        let l = lanes(&msgs, |t| t);
        assert!(l[0].1.len() <= MAX_EXPRESSION_POINTS);
        assert_eq!(l[0].1.last().unwrap().time.0, 39.999);
        ether_core::protocol::model::check_points(&l[0].1, (0.0, 1.0)).unwrap();
    }
}
