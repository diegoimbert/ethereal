//! Clip scheduling: maps timeline beats to clip content (offset, looping), generates
//! sample-accurate note events, and renders audio clips from their [`AudioSource`].
//!
//! Everything here is RT-safe: no allocation, bounded work per call.

use crate::graph::{ClipContentDesc, ClipDesc, WarpDesc};
use crate::media::AudioSource;
use crate::mixer::{ActiveNote, MAX_ACTIVE_NOTES};
use crate::event::{EventBuffer, EventKind, ProcessEvent};

/// Shortest content loop honoured (beats); shorter loops play unlooped.
const MIN_LOOP: f64 = 1.0 / 256.0;
/// Upper bound on linear pieces handled per call (a block rarely spans more than 2).
const MAX_PIECES: usize = 256;
/// Anti-click fade at clip edges, in samples.
pub(crate) const DECLICK_SAMPLES: f64 = 64.0;

/// A stretch of timeline `[t0, t1)` where content position is `c0 + (t − t0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Piece {
    pub t0: f64,
    pub t1: f64,
    pub c0: f64,
    /// Where this linear piece ends on the timeline if not cut by the query range (clip end
    /// or content loop end): notes are cut there.
    pub end: f64,
}

/// Call `f` for every linear piece of `clip` (placed at timeline `start`) inside `[r0, r1)`.
pub(crate) fn for_each_piece(
    clip: &ClipDesc,
    start: f64,
    r0: f64,
    r1: f64,
    mut f: impl FnMut(Piece),
) {
    let clip_end = start + clip.length.max(0.0);
    let a = r0.max(start);
    let b = r1.min(clip_end);
    if a >= b {
        return;
    }
    let looping = clip
        .looping
        .filter(|&(ls, le)| le - ls >= MIN_LOOP && clip.offset < le);
    let Some((ls, le)) = looping else {
        f(Piece {
            t0: a,
            t1: b,
            c0: clip.offset + (a - start),
            end: clip_end,
        });
        return;
    };
    let first = le - clip.offset;
    let len = le - ls;
    // Piece k covers clip-relative [first + k·len, first + (k+1)·len) (k = -1: the intro
    // from the offset to the first loop end).
    let p_a = a - start;
    let mut k: i64 = if p_a < first {
        -1
    } else {
        ((p_a - first) / len).floor() as i64
    };
    for _ in 0..MAX_PIECES {
        let (p_start, p_end, c_start) = if k < 0 {
            (0.0, first, clip.offset)
        } else {
            let s = first + k as f64 * len;
            (s, s + len, ls)
        };
        let t_start = start + p_start;
        let t_end = (start + p_end).min(clip_end);
        let t0 = t_start.max(a);
        let t1 = t_end.min(b);
        if t0 >= b {
            break;
        }
        if t0 < t1 {
            f(Piece {
                t0,
                t1,
                c0: c_start + (t0 - t_start),
                end: t_end,
            });
        }
        k += 1;
    }
}

/// Is timeline beat `t` inside the clip, and at which content position?
pub(crate) fn content_at(clip: &ClipDesc, start: f64, t: f64) -> Option<f64> {
    let mut out = None;
    for_each_piece(clip, start, t, t + 1e-9, |p| out = Some(p.c0 + (t - p.t0)));
    out
}

/// Context for converting beats to sample offsets inside the current sub-block.
pub(crate) struct Timing<'a> {
    pub tempo: &'a crate::tempo::TempoMapRt,
    /// Sub-block start/end in beats (`b1` exclusive).
    pub b0: f64,
    pub b1: f64,
    /// Seconds at `b0`.
    pub s0: f64,
    pub sample_rate: f64,
    pub frames: usize,
}

impl Timing<'_> {
    /// Sample offset of timeline beat `t` (`b0 <= t < b1`), clamped into the block.
    #[inline]
    pub(crate) fn offset(&self, t: f64) -> u32 {
        let o = ((self.tempo.beats_to_seconds(t) - self.s0) * self.sample_rate).floor();
        (o.max(0.0) as usize).min(self.frames.saturating_sub(1)) as u32
    }

    /// Timeline beat at sample `o` (linear within the sub-block).
    #[inline]
    pub(crate) fn beat_at(&self, o: f64) -> f64 {
        self.b0 + (self.b1 - self.b0) * o / self.frames as f64
    }

    /// First sample whose beat is `>= t`.
    #[inline]
    pub(crate) fn sample_ceil(&self, t: f64) -> usize {
        let span = self.b1 - self.b0;
        if span <= 0.0 {
            return 0;
        }
        let o = ((t - self.b0) / span * self.frames as f64).ceil();
        (o.max(0.0) as usize).min(self.frames)
    }
}

/// Sounding-note bookkeeping + note-id allocation shared by all tracks.
pub(crate) struct NoteSink<'a> {
    pub events: &'a mut EventBuffer,
    pub notes: &'a mut Vec<ActiveNote>,
    pub next_note_id: &'a mut u32,
}

impl NoteSink<'_> {
    fn note_on(&mut self, offset: u32, key: u8, velocity: f32, end: f64) {
        if self.notes.len() >= MAX_ACTIVE_NOTES {
            // Voice bookkeeping full: drop the note (reported as an event overflow).
            self.events.push(ProcessEvent {
                offset,
                kind: EventKind::AllNotesOff,
            });
            return;
        }
        let note_id = *self.next_note_id;
        *self.next_note_id = self.next_note_id.wrapping_add(1);
        if self.events.push(ProcessEvent {
            offset,
            kind: EventKind::NoteOn {
                note_id,
                channel: 0,
                key,
                velocity,
            },
        }) {
            self.notes.push(ActiveNote { note_id, key, end });
        }
    }

    /// Note-offs for sounding notes ending in `[.., b1)`.
    pub(crate) fn end_notes(&mut self, timing: &Timing<'_>) {
        let mut i = 0;
        while i < self.notes.len() {
            let n = self.notes[i];
            if n.end < timing.b1 {
                let offset = if n.end <= timing.b0 {
                    0
                } else {
                    timing.offset(n.end)
                };
                self.events.push(ProcessEvent {
                    offset,
                    kind: EventKind::NoteOff {
                        note_id: n.note_id,
                        channel: 0,
                        key: n.key,
                        velocity: 0.0,
                    },
                });
                self.notes.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Note-offs at `offset` for every sounding note (stop, locate, loop jump).
    pub(crate) fn release_all(&mut self, offset: u32) {
        for n in self.notes.drain(..) {
            self.events.push(ProcessEvent {
                offset,
                kind: EventKind::NoteOff {
                    note_id: n.note_id,
                    channel: 0,
                    key: n.key,
                    velocity: 0.0,
                },
            });
        }
    }
}

/// Schedule note-ons of a MIDI clip placed at `start` for the sub-block.
pub(crate) fn schedule_notes(
    clip: &ClipDesc,
    start: f64,
    timing: &Timing<'_>,
    sink: &mut NoteSink<'_>,
) {
    let ClipContentDesc::Midi { notes } = &clip.content else {
        return;
    };
    if clip.muted || notes.is_empty() {
        return;
    }
    for_each_piece(clip, start, timing.b0, timing.b1, |p| {
        let c_end = p.c0 + (p.t1 - p.t0);
        let first = notes.partition_point(|n| n.start < p.c0);
        for n in &notes[first..] {
            if n.start >= c_end {
                break;
            }
            let t = p.t0 + (n.start - p.c0);
            let end = (t + n.duration.max(0.0)).min(p.end);
            if end <= t {
                continue;
            }
            sink.note_on(timing.offset(t), n.key, n.velocity, end);
        }
    });
}

/// Source seconds of content beat `c`: piecewise-linear warp markers (extrapolated with
/// the edge slopes), or, unwarped, `c` at the tempo `ref_bpm`.
#[inline]
pub(crate) fn source_seconds(warp: Option<&WarpDesc>, ref_bpm: f64, c: f64) -> f64 {
    match warp {
        Some(w) if w.markers.len() >= 2 => {
            let m = &w.markers;
            let i = m.partition_point(|&(b, _)| b <= c).clamp(1, m.len() - 1);
            let (b0, s0) = m[i - 1];
            let (b1, s1) = m[i];
            if b1 - b0 <= 0.0 {
                return s0;
            }
            s0 + (c - b0) * (s1 - s0) / (b1 - b0)
        }
        _ => c * 60.0 / ref_bpm,
    }
}

/// Render (add) an audio clip placed at `start` into `out` for the sub-block. Reads through
/// `scratch` (≥ 2 × frames recommended). Returns `false` on a source underrun.
///
/// Warped clips play back by resampling (linear interpolation), i.e. `Repitch`
/// semantics; `Complex` (Signalsmith time-stretch) is not wired in yet and falls back to
/// the same resampling.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_audio(
    clip: &ClipDesc,
    start: f64,
    source: &dyn AudioSource,
    ref_bpm: f64,
    timing: &Timing<'_>,
    out: [&mut [f32]; 2],
    scratch: &mut [f32],
) -> bool {
    let ClipContentDesc::Audio {
        gain,
        fade_in,
        fade_out,
        warp,
        ..
    } = &clip.content
    else {
        return true;
    };
    if clip.muted || *gain == 0.0 {
        return true;
    }
    let sr = timing.sample_rate;
    let clip_end = start + clip.length;
    let bps = (timing.b1 - timing.b0) / timing.frames.max(1) as f64;
    let declick = DECLICK_SAMPLES * bps;
    let fin = fade_in.max(declick);
    let fout = fade_out.max(declick);
    let channels = source.channels().max(1);
    let total = source.frames();
    let [out_l, out_r] = out;
    let mut ok = true;
    let cap = scratch.len();
    for_each_piece(clip, start, timing.b0, timing.b1, |p| {
        let o_a = timing.sample_ceil(p.t0);
        let o_b = timing.sample_ceil(p.t1);
        let frame_at = |o: usize| {
            let t = timing.beat_at(o as f64);
            source_seconds(warp.as_ref(), ref_bpm, p.c0 + (t - p.t0)) * sr
        };
        let mut o = o_a;
        while o < o_b {
            // Chunk so the source window fits in the scratch buffer (per channel half).
            let mut len = (o_b - o).min(256);
            let half = cap / 2;
            let (lo, hi) = loop {
                let f0 = frame_at(o);
                let f1 = frame_at(o + len);
                let lo = f0.min(f1).floor();
                let hi = f0.max(f1).ceil() + 2.0;
                if hi - lo <= half as f64 || len == 1 {
                    break (lo, hi);
                }
                len /= 2;
            };
            let base = lo.max(0.0) as u64;
            let count = ((hi - lo.max(0.0)).max(0.0) as usize).min(half);
            if base < total && count > 0 {
                let (sl, sr_buf) = scratch.split_at_mut(half);
                let sl = &mut sl[..count];
                let sr_buf = &mut sr_buf[..count];
                ok &= source.read(0, base, sl);
                if channels > 1 {
                    ok &= source.read(1, base, sr_buf);
                } else {
                    sr_buf.copy_from_slice(sl);
                }
                for s in o..o + len {
                    let f = frame_at(s);
                    if f < 0.0 {
                        continue;
                    }
                    let rel = f - base as f64;
                    let i = rel.floor();
                    let frac = (rel - i) as f32;
                    let i = i as usize;
                    if i + 1 >= count {
                        continue;
                    }
                    let t = timing.beat_at(s as f64);
                    let g_in = ((t - start) / fin).min(1.0);
                    let g_out = ((clip_end - t) / fout).min(1.0);
                    let g = gain * (g_in.min(g_out).max(0.0) as f32);
                    let l = sl[i] + (sl[i + 1] - sl[i]) * frac;
                    let r = sr_buf[i] + (sr_buf[i + 1] - sr_buf[i]) * frac;
                    out_l[s] += l * g;
                    out_r[s] += r * g;
                }
                source.prefetch_hint(base + count as u64);
            }
            o += len;
        }
    });
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_protocol::model::ClipId;

    fn clip(length: f64, offset: f64, looping: Option<(f64, f64)>) -> ClipDesc {
        ClipDesc {
            id: ClipId::default(),
            start: 0.0,
            length,
            offset,
            looping,
            muted: false,
            content: ClipContentDesc::Midi { notes: vec![] },
            envelopes: vec![],
        }
    }

    fn pieces(c: &ClipDesc, start: f64, r0: f64, r1: f64) -> Vec<(f64, f64, f64, f64)> {
        let mut v = vec![];
        for_each_piece(c, start, r0, r1, |p| v.push((p.t0, p.t1, p.c0, p.end)));
        v
    }

    #[test]
    fn unlooped_piece() {
        let c = clip(4.0, 1.0, None);
        assert_eq!(pieces(&c, 8.0, 0.0, 100.0), vec![(8.0, 12.0, 1.0, 12.0)]);
        assert_eq!(pieces(&c, 8.0, 9.0, 10.0), vec![(9.0, 10.0, 2.0, 12.0)]);
        assert!(pieces(&c, 8.0, 12.0, 13.0).is_empty());
    }

    #[test]
    fn looped_pieces() {
        // Offset 1, loop [0, 2): content 1,(0..2),(0..2)... over 5 beats.
        let c = clip(5.0, 1.0, Some((0.0, 2.0)));
        assert_eq!(
            pieces(&c, 0.0, 0.0, 10.0),
            vec![
                (0.0, 1.0, 1.0, 1.0),
                (1.0, 3.0, 0.0, 3.0),
                (3.0, 5.0, 0.0, 5.0)
            ]
        );
        assert_eq!(pieces(&c, 0.0, 3.5, 4.0), vec![(3.5, 4.0, 0.5, 5.0)]);
        assert_eq!(content_at(&c, 0.0, 4.25), Some(1.25));
    }

    #[test]
    fn warp_mapping() {
        let w = WarpDesc {
            mode: ether_protocol::model::WarpMode::Repitch,
            markers: vec![(0.0, 0.0), (4.0, 1.0), (8.0, 3.0)],
        };
        assert_eq!(source_seconds(Some(&w), 120.0, 2.0), 0.5);
        assert_eq!(source_seconds(Some(&w), 120.0, 6.0), 2.0);
        assert_eq!(source_seconds(Some(&w), 120.0, 10.0), 4.0);
        assert_eq!(source_seconds(None, 120.0, 4.0), 2.0);
    }
}
