//! Clip scheduling: maps timeline beats to clip content (offset, looping), generates
//! sample-accurate note events, and renders audio clips from their [`AudioSource`].
//!
//! Everything here is RT-safe: no allocation, bounded work per call.

use ether_protocol::model::FadeCurve;

use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::fades::fade_gain;
use crate::graph::{ClipContentDesc, ClipDesc, WarpDesc};
use crate::media::AudioSource;
use crate::mixer::{ActiveNote, MAX_ACTIVE_NOTES};

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
    /// Where this linear piece begins on the timeline if not cut by the query range (clip
    /// start or content loop start): notes before it don't sound in it.
    pub begin: f64,
}

/// Call `f` for every linear piece of `clip` (at timeline `clip.start`) inside `[r0, r1)`.
pub(crate) fn for_each_piece(clip: &ClipDesc, r0: f64, r1: f64, mut f: impl FnMut(Piece)) {
    let start = clip.start;
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
            begin: start,
        });
        return;
    };
    let first = le - clip.offset;
    let len = le - ls;
    // Piece k covers clip-relative [first + k·len, first + (k+1)·len) (k = -1: the intro
    // from the offset to the first loop end).
    let p_a = a - start;
    let k0: i64 = if p_a < first {
        -1
    } else {
        ((p_a - first) / len).floor() as i64
    };
    for k in (k0..).take(MAX_PIECES) {
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
                begin: t_start,
            });
        }
    }
}

/// Is timeline beat `t` inside the clip, and at which content position?
pub(crate) fn content_at(clip: &ClipDesc, t: f64) -> Option<f64> {
    let mut out = None;
    for_each_piece(clip, t, t + 1e-9, |p| out = Some(p.c0 + (t - p.t0)));
    out
}

/// Event ranges are shifted back by this many beats (≈0.002 samples at 120 BPM/48 kHz) so an
/// event exactly on a sub-block boundary, which float error may put a hair before the
/// boundary, is always scheduled in the later sub-block, at offset 0.
pub(crate) const EVENT_SHIFT: f64 = 1e-7;

/// Context for converting beats to sample offsets inside the current sub-block.
///
/// Two modes (CONTRACTS.md §12.7):
/// - **linear** (tempo maps without ramps, and while stopped): beats are linear within the
///   sub-block (`b0..b1`), exactly the v0.1 math, so such projects render bit-identically;
/// - **exact** (the tempo map ramps somewhere, playing): every sample's beat is the
///   closed-form tempo integral of the engine's sample clock since the last timeline jump
///   ([`Exact`]), never an accumulation of sub-block lengths, so positions are identical
///   for every block size and match the tempo map to rounding error over any duration.
pub(crate) struct Timing<'a> {
    pub tempo: &'a crate::tempo::TempoMapRt,
    /// Sub-block start/end in beats (`b1` exclusive).
    pub b0: f64,
    pub b1: f64,
    /// Seconds at `b0`.
    pub s0: f64,
    pub sample_rate: f64,
    pub frames: usize,
    /// Engine sample clock at the sub-block start (`TransportInfo::sample_time`): the
    /// automation grid ([`crate::automation_rt::PARAM_GRID`]) is aligned on it.
    pub sample_time: u64,
    /// The sub-block ends at the loop end: the timeline jumps back after sample
    /// `frames - 1` (mixer ramps aim at the loop end instead of a grid point past it).
    pub wraps: bool,
    /// Exact per-sample beats (tempo ramps), `None` = linear.
    pub exact: Option<Exact<'a>>,
}

/// Exact timing of a sub-block on a ramped tempo map: beat of sample `o` =
/// `tempo.seconds_to_beats(seconds + (since + o) / sample_rate)`, where the anchor
/// (`beat`, `seconds`) is the last timeline jump (play, locate, loop wrap, tempo/signature
/// boundary, new tempo map)
/// and `since` counts the samples rendered since. `beats[o]` caches it for `o` in
/// `0..=frames` (filled by the engine once per sub-block).
#[derive(Clone, Copy)]
pub(crate) struct Exact<'a> {
    pub beat: f64,
    pub seconds: f64,
    pub since: u64,
    pub beats: &'a [f64],
}

/// Beat `o` samples after the start of a sub-block `since` samples past the anchor
/// (`beat`, `seconds`): the closed-form tempo integral (see [`Exact`]).
#[inline]
pub(crate) fn exact_beat(
    tempo: &crate::tempo::TempoMapRt,
    sample_rate: f64,
    (beat, seconds): (f64, f64),
    since: u64,
    o: f64,
) -> f64 {
    let k = since as f64 + o;
    if k == 0.0 {
        return beat;
    }
    tempo.seconds_to_beats(seconds + k / sample_rate)
}

impl Timing<'_> {
    /// Beat range `[r0, r1)` whose events belong to this sub-block.
    #[inline]
    pub(crate) fn event_range(&self) -> (f64, f64) {
        (self.b0 - EVENT_SHIFT, self.b1 - EVENT_SHIFT)
    }

    /// Sample offset of timeline beat `t` (`b0 <= t < b1`), clamped into the block.
    #[inline]
    pub(crate) fn offset(&self, t: f64) -> u32 {
        // Absorb float error so events on a sample boundary land on that sample.
        let o = ((self.tempo.beats_to_seconds(t) - self.s0) * self.sample_rate + 1e-4).floor();
        (o.max(0.0) as usize).min(self.frames.saturating_sub(1)) as u32
    }

    /// Timeline beat at sample `o`: linear within the sub-block, or exact on tempo ramps
    /// ([`Exact`]; any `o`, also past the sub-block end).
    #[inline]
    pub(crate) fn beat_at(&self, o: f64) -> f64 {
        match &self.exact {
            None => self.b0 + (self.b1 - self.b0) * o / self.frames as f64,
            Some(e) => {
                let i = o as usize;
                if i as f64 == o
                    && let Some(b) = e.beats.get(i)
                {
                    return *b;
                }
                exact_beat(
                    self.tempo,
                    self.sample_rate,
                    (e.beat, e.seconds),
                    e.since,
                    o,
                )
            }
        }
    }

    /// First sample whose beat is `>= t`.
    #[inline]
    pub(crate) fn sample_ceil(&self, t: f64) -> usize {
        if self.exact.is_some() {
            return self.sample_at_or_after(t);
        }
        let span = self.b1 - self.b0;
        if span <= 0.0 {
            return 0;
        }
        // Absorb float error (1e-6 samples) so a boundary on a sample lands on that sample
        // whatever the sub-block split (adjacent pieces never both render it, or skip it).
        let o = ((t - self.b0) / span * self.frames as f64 - 1e-6).ceil();
        (o.max(0.0) as usize).min(self.frames)
    }

    /// First sample whose beat is `>= t`, through the tempo map (automation breakpoints:
    /// the same sample for every block size, up to 1e-6 samples of float error), clamped
    /// to `0..=frames`.
    #[inline]
    pub(crate) fn sample_at_or_after(&self, t: f64) -> usize {
        let o = ((self.tempo.beats_to_seconds(t) - self.s0) * self.sample_rate - 1e-6).ceil();
        (o.max(0.0) as usize).min(self.frames)
    }
}

/// Sounding-note bookkeeping + note-id allocation of one track (ids are per track, so they
/// never depend on processing order).
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
            if n.end < timing.event_range().1 {
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

/// Schedule note-ons of a MIDI clip for the sub-block.
pub(crate) fn schedule_notes(clip: &ClipDesc, timing: &Timing<'_>, sink: &mut NoteSink<'_>) {
    let ClipContentDesc::Midi { notes } = &clip.content else {
        return;
    };
    if clip.muted || notes.is_empty() {
        return;
    }
    let (r0, r1) = timing.event_range();
    for_each_piece(clip, r0, r1, |p| {
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

/// Note chasing: note-ons at the start of the sub-block for the clip's notes that are
/// already sounding at `timing.b0` (playback started, located or looped back into the
/// middle of them). They end where they would have. Notes starting in the sub-block are
/// `schedule_notes`'s. Scans the notes before the position: bounded by the clip's notes,
/// and only on the first sub-block after a jump.
pub(crate) fn chase_notes(clip: &ClipDesc, timing: &Timing<'_>, sink: &mut NoteSink<'_>) {
    let ClipContentDesc::Midi { notes } = &clip.content else {
        return;
    };
    if clip.muted || notes.is_empty() {
        return;
    }
    let (r0, _) = timing.event_range();
    for_each_piece(clip, r0, r0 + EVENT_SHIFT, |p| {
        let first = notes.partition_point(|n| n.start < p.c0);
        for n in &notes[..first] {
            let t = p.t0 + (n.start - p.c0);
            if t < p.begin - EVENT_SHIFT {
                continue; // before this piece (e.g. ahead of the content loop start)
            }
            let end = (t + n.duration.max(0.0)).min(p.end);
            if end > timing.b0 {
                sink.note_on(0, n.key, n.velocity, end);
            }
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

/// Gain envelope of an audio clip on the timeline: clip gain × fade-in × fade-out, with the
/// fade law [`crate::fades::fade_gain`] (fade-outs mirrored in time). Fades shorter than
/// the anti-click ramp ([`DECLICK_SAMPLES`]) use a linear ramp of that length instead.
/// Shared by [`render_audio`] and the stretched (Complex warp) path.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClipEnvelope {
    start: f64,
    end: f64,
    fin: f64,
    fout: f64,
    curve_in: FadeCurve,
    curve_out: FadeCurve,
    gain: f32,
}

impl ClipEnvelope {
    /// `None` when the clip is silent (not audio, muted or zero gain).
    #[inline]
    pub(crate) fn new(clip: &ClipDesc, timing: &Timing<'_>) -> Option<Self> {
        let ClipContentDesc::Audio {
            gain,
            fade_in,
            fade_out,
            fade_in_curve,
            fade_out_curve,
            ..
        } = &clip.content
        else {
            return None;
        };
        if clip.muted || *gain == 0.0 {
            return None;
        }
        let bps = (timing.b1 - timing.b0) / timing.frames.max(1) as f64;
        let declick = DECLICK_SAMPLES * bps;
        let pick = |len: f64, curve: FadeCurve| {
            if len > declick {
                (len, curve)
            } else {
                (declick, FadeCurve::Linear)
            }
        };
        let (fin, curve_in) = pick(*fade_in, *fade_in_curve);
        let (fout, curve_out) = pick(*fade_out, *fade_out_curve);
        Some(Self {
            start: clip.start,
            end: clip.start + clip.length,
            fin,
            fout,
            curve_in,
            curve_out,
            gain: *gain,
        })
    }

    /// Gain at timeline beat `t`.
    #[inline]
    pub(crate) fn at(&self, t: f64) -> f32 {
        let x_in = (t - self.start) / self.fin;
        let x_out = (self.end - t) / self.fout;
        let g_in = if x_in >= 1.0 {
            1.0
        } else {
            fade_gain(self.curve_in, x_in as f32)
        };
        let g_out = if x_out >= 1.0 {
            1.0
        } else {
            fade_gain(self.curve_out, x_out as f32)
        };
        self.gain * g_in.min(g_out)
    }
}

/// RT. Read `l.len()` frames of `source` starting at frame `start` (may be negative:
/// silence before the source start) into `l`/`r` (mono sources are duplicated). With
/// `reversed`, the frames are those of the reversed media (`R[i] = M[N - 1 - i]`, see
/// `ether_model::AudioContent::reversed`): the forward window is read, then flipped.
/// Used by the stretched (Complex warp) path.
pub(crate) fn read_frames(
    source: &dyn AudioSource,
    reversed: bool,
    start: i64,
    l: &mut [f32],
    r: &mut [f32],
) -> bool {
    let n = l.len();
    let start = if reversed {
        source.frames() as i64 - start - n as i64
    } else {
        start
    };
    let skip = if start < 0 {
        ((-start) as u64).min(n as u64) as usize
    } else {
        0
    };
    l[..skip].fill(0.0);
    r[..skip].fill(0.0);
    let mut ok = true;
    if skip < n {
        let from = start.max(0) as u64;
        ok = source.read(0, from, &mut l[skip..]);
        if source.channels() > 1 {
            ok &= source.read(1, from, &mut r[skip..]);
        } else {
            r[skip..].copy_from_slice(&l[skip..]);
        }
    }
    if reversed {
        l.reverse();
        r.reverse();
    }
    ok
}

/// Render (add) an audio clip into `out` for the sub-block. Reads through
/// `scratch` (≥ 2 × frames recommended). Returns `false` on a source underrun.
///
/// Resamples (linear interpolation): unwarped and `Repitch` clips, and `Complex` clips
/// without a stretcher (`crate::warp` handles the stretched path). Transpose changes the
/// playback rate. Fades follow [`ClipEnvelope`].
///
/// **Reverse** (`reversed`): the clip plays the reversed media `R[i] = M[N - 1 - i]`. The
/// whole content → source mapping (offset, clip loop, warp markers, transpose) is computed
/// on that reversed timeline exactly as for a forward clip, and only the final frame lookup
/// is mirrored, so a reversed clip is sample-for-sample the forward rendering of a
/// reversed file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_audio(
    clip: &ClipDesc,
    source: &dyn AudioSource,
    ref_bpm: f64,
    timing: &Timing<'_>,
    out: [&mut [f32]; 2],
    scratch: &mut [f32],
) -> bool {
    let ClipContentDesc::Audio {
        warp,
        transpose,
        reversed,
        ..
    } = &clip.content
    else {
        return true;
    };
    let Some(env) = ClipEnvelope::new(clip, timing) else {
        return true;
    };
    let reversed = *reversed;
    let sr = timing.sample_rate;
    let channels = source.channels().max(1);
    let total = source.frames();
    let last = total as f64 - 1.0;
    let [out_l, out_r] = out;
    let mut ok = true;
    let cap = scratch.len();
    for_each_piece(clip, timing.b0, timing.b1, |p| {
        let o_a = timing.sample_ceil(p.t0);
        let o_b = timing.sample_ceil(p.t1);
        // Media frame (fractional) played at output sample `o`.
        let frame_at = |o: usize| {
            let t = timing.beat_at(o as f64);
            let f = crate::warp::repitch_source_seconds(
                warp.as_ref(),
                *transpose,
                ref_bpm,
                clip.offset,
                p.c0 + (t - p.t0),
            ) * sr;
            if reversed { last - f } else { f }
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
                    if f < 0.0 || (reversed && f > last) {
                        continue;
                    }
                    let rel = f - base as f64;
                    let i = rel.floor();
                    let frac = (rel - i) as f32;
                    let i = i as usize;
                    if i + 1 >= count {
                        continue;
                    }
                    let g = env.at(timing.beat_at(s as f64));
                    let l = sl[i] + (sl[i + 1] - sl[i]) * frac;
                    let r = sr_buf[i] + (sr_buf[i + 1] - sr_buf[i]) * frac;
                    out_l[s] += l * g;
                    out_r[s] += r * g;
                }
                if reversed {
                    source.prefetch_hint(base.saturating_sub(count as u64));
                } else {
                    source.prefetch_hint(base + count as u64);
                }
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

    fn clip(start: f64, length: f64, offset: f64, looping: Option<(f64, f64)>) -> ClipDesc {
        ClipDesc {
            id: ClipId::NIL,
            start,
            length,
            offset,
            looping,
            muted: false,
            content: ClipContentDesc::Midi { notes: vec![] },
            envelopes: vec![],
        }
    }

    fn pieces(c: &ClipDesc, r0: f64, r1: f64) -> Vec<(f64, f64, f64, f64)> {
        let mut v = vec![];
        for_each_piece(c, r0, r1, |p| v.push((p.t0, p.t1, p.c0, p.end)));
        v
    }

    #[test]
    fn unlooped_piece() {
        let c = clip(8.0, 4.0, 1.0, None);
        assert_eq!(pieces(&c, 0.0, 100.0), vec![(8.0, 12.0, 1.0, 12.0)]);
        assert_eq!(pieces(&c, 9.0, 10.0), vec![(9.0, 10.0, 2.0, 12.0)]);
        assert!(pieces(&c, 12.0, 13.0).is_empty());
    }

    #[test]
    fn looped_pieces() {
        // Offset 1, loop [0, 2): content 1,(0..2),(0..2)... over 5 beats.
        let c = clip(0.0, 5.0, 1.0, Some((0.0, 2.0)));
        assert_eq!(
            pieces(&c, 0.0, 10.0),
            vec![
                (0.0, 1.0, 1.0, 1.0),
                (1.0, 3.0, 0.0, 3.0),
                (3.0, 5.0, 0.0, 5.0)
            ]
        );
        assert_eq!(pieces(&c, 3.5, 4.0), vec![(3.5, 4.0, 0.5, 5.0)]);
        assert_eq!(content_at(&c, 4.25), Some(1.25));
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
