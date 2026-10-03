//! Recording expression: CC / pitch bend / channel pressure / poly pressure played while
//! recording become the recorded clip's lanes and its notes' `Pressure` expressions.
//!
//! - Channel messages of every channel are merged (plain MIDI; `mpe` reads member channels
//!   per note instead on MPE tracks). CC 120..=127 (channel mode) are not recorded.
//! - Curves are `Step` (the value holds until the next message, exactly what was played),
//!   thinned to at most one point per [`THIN_SECONDS`] per curve: in each window the point
//!   that moves furthest from the last kept value wins (peaks survive), and the final value
//!   is always kept.
//! - Poly pressure (`0xA0`) attaches to the recorded note of that key sounding at that time
//!   (times from its start); pressure outside any note is dropped.

use ether_core::protocol::model::*;

use crate::recording::{NoteSpecDraft, RecordedMidi};

/// Thinning window (seconds).
pub(crate) const THIN_SECONDS: f64 = 0.005;

/// Expression recorded in one pass, times relative to the clip start.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RecordedExpression {
    /// Sorted by kind.
    pub lanes: Vec<(ExpressionKind, Vec<ExpressionPoint>)>,
    /// `(index in the pass's notes, curve)`.
    pub note_pressure: Vec<(usize, Vec<ExpressionPoint>)>,
}

impl RecordedExpression {
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty() && self.note_pressure.is_empty()
    }

    /// The last lane point (relative to the clip start), to size the clip.
    pub fn end(&self) -> Option<f64> {
        self.lanes
            .iter()
            .filter_map(|(_, c)| c.last().map(|p| p.time.0))
            .reduce(f64::max)
    }
}

/// The expression of `events` (absolute positions, already limited to the kept range) in a
/// clip starting at `start` whose recorded notes are `notes` (relative to `start`, from
/// `notes_from_midi`). `gap`: the thinning window in beats.
pub(crate) fn recorded_expression(
    events: &[RecordedMidi],
    start: f64,
    notes: &[NoteSpecDraft],
    gap: f64,
) -> RecordedExpression {
    let mut lanes: Vec<(ExpressionKind, Vec<ExpressionPoint>)> = Vec::new();
    let mut pressure: Vec<(usize, Vec<ExpressionPoint>)> = Vec::new();
    let point = |time: f64, value: f32| ExpressionPoint {
        time: Beats(time.max(0.0)),
        value,
        curve: CurveShape::Step,
    };
    for e in events {
        if e.position < start {
            continue;
        }
        let t = e.position - start;
        let (d1, d2) = (e.data[1] & 0x7F, e.data[2] & 0x7F);
        let lane = match e.data[0] & 0xF0 {
            0xB0 if d1 <= MAX_EXPRESSION_CC => {
                Some((ExpressionKind::Cc { controller: d1 }, f32::from(d2) / 127.0))
            }
            0xE0 => {
                let raw = (i32::from(d2) << 7) | i32::from(d1);
                Some((ExpressionKind::PitchBend, bend_value(raw)))
            }
            0xD0 => Some((ExpressionKind::ChannelPressure, f32::from(d1) / 127.0)),
            0xA0 => {
                // The latest-started note of this key sounding at `t`.
                let note = notes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| {
                        n.pitch == d1 && n.start <= t + 1e-9 && t < n.start + n.duration
                    })
                    .max_by(|a, b| a.1.start.total_cmp(&b.1.start))
                    .map(|(i, n)| (i, n.start));
                if let Some((i, s)) = note {
                    let p = point(t - s, f32::from(d2) / 127.0);
                    match pressure.iter_mut().find(|(j, _)| *j == i) {
                        Some((_, c)) => c.push(p),
                        None => pressure.push((i, vec![p])),
                    }
                }
                None
            }
            _ => None,
        };
        if let Some((kind, v)) = lane {
            let p = point(t, v);
            match lanes.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, c)) => c.push(p),
                None => lanes.push((kind, vec![p])),
            }
        }
    }
    let curves = lanes.iter_mut().map(|l| &mut l.1);
    for c in curves.chain(pressure.iter_mut().map(|p| &mut p.1)) {
        *c = thin(c, gap);
    }
    lanes.sort_by_key(|(k, _)| *k);
    pressure.sort_by_key(|(i, _)| *i);
    RecordedExpression {
        lanes,
        note_pressure: pressure,
    }
}

/// 14-bit bend → -1..=1 (`8192 + round(v·8191)` inverted; 0 clamps to -1).
fn bend_value(raw: i32) -> f32 {
    ((raw - 8192) as f32 / 8191.0).clamp(-1.0, 1.0)
}

/// Thin a time-sorted `Step` curve to at most one point per `gap` beats (+ the final value),
/// dropping repeats. See the module docs.
pub(crate) fn thin(points: &[ExpressionPoint], gap: f64) -> Vec<ExpressionPoint> {
    let mut out: Vec<ExpressionPoint> = Vec::with_capacity(points.len().min(1024));
    let mut i = 0;
    while i < points.len() {
        let w0 = points[i].time.0;
        let mut j = i + 1;
        while j < points.len() && points[j].time.0 < w0 + gap {
            j += 1;
        }
        // The first point that moves furthest from the last kept value.
        let last = out.last().map(|p| p.value);
        let mut pick = points[i];
        if let Some(l) = last {
            for p in &points[i..j] {
                if (p.value - l).abs() > (pick.value - l).abs() {
                    pick = *p;
                }
            }
        }
        if last != Some(pick.value) {
            out.push(ExpressionPoint {
                time: Beats(w0),
                ..pick
            });
        }
        i = j;
    }
    if let (Some(final_), Some(kept)) = (points.last(), out.last())
        && final_.value != kept.value
    {
        let time = Beats(final_.time.0.max(kept.time.0));
        out.push(ExpressionPoint { time, ..*final_ });
    }
    out.truncate(MAX_EXPRESSION_POINTS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(position: f64, data: [u8; 3]) -> RecordedMidi {
        RecordedMidi {
            position,
            data,
            pass: 0,
        }
    }

    #[test]
    fn channel_messages_become_lanes() {
        let events = [
            ev(1.0, [0xB1, 1, 0]),
            ev(1.5, [0xB0, 1, 127]),
            ev(2.0, [0xE0, 0x7F, 0x7F]),
            ev(2.5, [0xE0, 0, 0x40]),
            ev(3.0, [0xD0, 64, 0]),
            ev(3.0, [0xB0, 123, 0]), // channel mode: not recorded
            ev(0.5, [0xB0, 1, 5]),   // before the clip
        ];
        let r = recorded_expression(&events, 1.0, &[], 0.0);
        let kinds: Vec<ExpressionKind> = r.lanes.iter().map(|l| l.0).collect();
        assert_eq!(
            kinds,
            vec![
                ExpressionKind::Cc { controller: 1 },
                ExpressionKind::PitchBend,
                ExpressionKind::ChannelPressure
            ]
        );
        let cc = &r.lanes[0].1;
        assert_eq!(cc.len(), 2);
        assert_eq!((cc[0].time, cc[0].value), (Beats(0.0), 0.0));
        assert_eq!((cc[1].time, cc[1].value), (Beats(0.5), 1.0));
        assert!(cc.iter().all(|p| p.curve == CurveShape::Step));
        let bend = &r.lanes[1].1;
        assert_eq!(bend[0].value, 1.0);
        assert_eq!(bend[1].value, 0.0);
        assert!((r.lanes[2].1[0].value - 64.0 / 127.0).abs() < 1e-6);
    }

    #[test]
    fn poly_pressure_follows_its_note() {
        let notes = [
            NoteSpecDraft {
                pitch: 60,
                velocity: 1.0,
                start: 0.0,
                duration: 1.0,
            },
            NoteSpecDraft {
                pitch: 64,
                velocity: 1.0,
                start: 0.5,
                duration: 1.0,
            },
        ];
        let events = [
            ev(10.25, [0xA0, 60, 10]),
            ev(10.75, [0xA0, 64, 100]),
            ev(10.80, [0xA0, 64, 50]),
            ev(12.0, [0xA0, 64, 50]), // after the note: dropped
            ev(10.5, [0xA0, 61, 50]), // no such note
        ];
        let r = recorded_expression(&events, 10.0, &notes, 0.0);
        assert!(r.lanes.is_empty());
        assert_eq!(r.note_pressure.len(), 2);
        assert_eq!(r.note_pressure[0].0, 0);
        assert_eq!(r.note_pressure[0].1[0].time, Beats(0.25));
        let n1 = &r.note_pressure[1];
        assert_eq!(n1.0, 1);
        assert_eq!(n1.1.len(), 2);
        assert!((n1.1[0].time.0 - 0.25).abs() < 1e-9);
        assert!((n1.1[1].value - 50.0 / 127.0).abs() < 1e-6);
    }

    #[test]
    fn thinning_keeps_peaks_and_the_final_value() {
        let pts: Vec<ExpressionPoint> = (0..100)
            .map(|i| ExpressionPoint {
                time: Beats(i as f64 * 0.001),
                value: if i == 37 { 1.0 } else { 0.5 },
                curve: CurveShape::Step,
            })
            .chain(std::iter::once(ExpressionPoint {
                time: Beats(0.0995),
                value: 0.25,
                curve: CurveShape::Step,
            }))
            .collect();
        let out = thin(&pts, 0.01);
        assert!(out.len() <= 12, "{}", out.len());
        assert!(out.iter().any(|p| p.value == 1.0), "the peak survives");
        assert_eq!(out.last().unwrap().value, 0.25);
        // Sorted, at most one point per window (+ the final value).
        assert!(out.windows(2).all(|w| w[0].time <= w[1].time));
        // Repeats are dropped.
        let flat = vec![pts[0]; 10];
        assert_eq!(thin(&flat, 0.01).len(), 1);
    }
}
