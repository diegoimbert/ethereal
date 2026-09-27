//! Metronome click (roadmap v2, owned by the `tempo-metronome` node; see
//! `docs/ROADMAP.md`).
//!
//! Hook point (already wired, base-17): the engine owns one [`Metronome`] and, after the
//! master bus has been written to the hardware outputs of a sub-block, calls
//! [`Metronome::render`] once with that sub-block's `TransportInfo`, and
//! [`Metronome::reset`] on transport jumps. This node only edits this file.
//!
//! Behaviour:
//! - **When**: on every beat (in denominator units: 7/8 clicks on each eighth note) while
//!   playing with `RenderGraphDesc::metronome` on, and during a recording count-in
//!   (`info.recording`, beat before `desc.count_in_end`) whatever `metronome` says. The
//!   count-in click stops for good once the playhead has reached `count_in_end` (until the
//!   end changes or recording stops), so a loop wrapping back before the record start does
//!   not click again.
//! - **Where**: sample-accurate. The engine splits sub-blocks at tempo, signature and loop
//!   boundaries, so inside one sub-block the tempo is constant or one linear ramp and the
//!   signature is fixed. Tempo ramps are linear in *beats* (`ether_model::tempo`), so the
//!   sub-block's ramp is recovered from its start tempo (`info.bpm`), its span in beats and
//!   its duration (a bounded Newton solve), and beats are placed with the shared
//!   `segment_beats_to_seconds` math. A sub-block cut short at a boundary lasts a fraction
//!   of a sample less than its frame count; when it continues the previous sub-block's
//!   ramp (and that ramp fits its duration) the previous slope is kept, so positions stay
//!   exact. A beat starts its click on the first sample at or after it. Bars restart at
//!   every signature change; the first beat of a bar is accented when `desc.accent`.
//! - **Latency**: `render` gets the graph's output latency (PDC); a beat's click is emitted that
//!   many samples after the sub-block crosses the beat, so it lines up with the audio.
//! - **Sound**: synthesized once in [`Metronome::new`] (one normal + one accent buffer per
//!   [`MetronomeSound`]), scaled by `desc.volume`. A new click cuts the previous one.
//!
//! RT rules: no allocation after `new` (pending clicks live in a fixed-size queue, kept
//! sorted by due time so a latency decrease never stalls earlier clicks; clicks that don't
//! fit are counted in [`Metronome::dropped_clicks`]).

use ether_protocol::model::{MetronomeSound, segment_beats_to_seconds};

use crate::graph::MetronomeDesc;
use crate::transport::TransportInfo;

/// Beat comparisons tolerance (beats).
const EPS: f64 = 1e-7;
/// Tolerance when rounding a beat's fractional sample offset up (samples).
const SAMPLE_EPS: f64 = 1e-4;
/// Pending clicks (scheduled, not started yet). Clicks beyond this are dropped.
const QUEUE: usize = 256;
/// Relative level of a normal click (accents are at full level).
const NORMAL_LEVEL: f32 = 0.6;

const SOUNDS: [MetronomeSound; 3] = [
    MetronomeSound::Classic,
    MetronomeSound::Wood,
    MetronomeSound::Beep,
];

fn sound_index(s: MetronomeSound) -> usize {
    match s {
        MetronomeSound::Classic => 0,
        MetronomeSound::Wood => 1,
        MetronomeSound::Beep => 2,
    }
}

/// One click waiting for its start sample.
#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    /// Engine sample time (`TransportInfo::sample_time` clock) of the click's first sample.
    due: u64,
    sound: usize,
    accent: bool,
    gain: f32,
}

/// The click currently sounding.
#[derive(Clone, Copy, Debug)]
struct Voice {
    sound: usize,
    accent: bool,
    gain: f32,
    pos: usize,
}

/// Click generator: precomputed click buffers, the pending-click queue and the sounding
/// click.
#[derive(Debug)]
pub struct Metronome {
    sample_rate: f32,
    /// `clicks[sound][accent as usize]`.
    clicks: [[Vec<f32>; 2]; 3],
    queue: [Pending; QUEUE],
    head: usize,
    len: usize,
    voice: Option<Voice>,
    /// `count_in_end` of the desc the count-in state refers to.
    count_in_end: Option<f64>,
    /// The playhead reached `count_in_end` while recording: no more count-in clicks.
    count_in_done: bool,
    /// The last playing sub-block's ramp (continuity for sub-blocks cut at a boundary).
    prev: Option<Ramp>,
    /// Clicks dropped because the pending queue was full.
    dropped: u64,
}

/// Tempo over one playing sub-block: from `start_bpm` at its start to `end_bpm` after
/// `span` beats (linear in beats).
#[derive(Clone, Copy, Debug)]
struct Ramp {
    start_bpm: f64,
    end_bpm: f64,
    span: f64,
    /// Position and engine sample time right after the sub-block.
    end_pos: f64,
    end_time: u64,
}

impl Ramp {
    /// Seconds from the sub-block start to `d` beats into it.
    fn seconds(&self, d: f64) -> f64 {
        segment_beats_to_seconds(self.start_bpm, self.end_bpm, self.span, d)
    }
}

impl Metronome {
    /// Non-RT. Synthesizes the click sounds for `sample_rate`.
    pub fn new(sample_rate: f32) -> Self {
        let clicks = SOUNDS.map(|s| [synth(s, false, sample_rate), synth(s, true, sample_rate)]);
        Self {
            sample_rate,
            clicks,
            queue: [Pending::default(); QUEUE],
            head: 0,
            len: 0,
            voice: None,
            count_in_end: None,
            count_in_done: false,
            prev: None,
            dropped: 0,
        }
    }

    /// Clicks dropped so far because too many were pending (diagnostics).
    pub fn dropped_clicks(&self) -> u64 {
        self.dropped
    }

    /// RT. Add the click for the sub-block described by `info` (`frames` samples) into
    /// `out[ch][offset..offset + frames]` (planar hardware outputs; channels may be shorter:
    /// clamp). `enabled` = `RenderGraphDesc::metronome`. Called by `engine.rs` once per
    /// sub-block (already wired).
    ///
    /// `latency` is the graph's total output latency (samples, PDC included): the audio of
    /// timeline position `p` reaches the hardware `latency` samples after `p` is rendered,
    /// so the click of a beat is emitted `latency` samples after the sample where the
    /// sub-block timeline crosses it (pending clicks wait in a preallocated queue).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        desc: &MetronomeDesc,
        enabled: bool,
        info: &TransportInfo,
        latency: u32,
        offset: usize,
        frames: usize,
        out: &mut [&mut [f32]],
    ) {
        if frames == 0 {
            return;
        }
        self.schedule(desc, enabled, info, latency, frames);
        self.play(info.sample_time, offset, frames, out);
    }

    /// RT. Silence any sounding click and forget the scheduled ones (transport jumps).
    pub fn reset(&mut self) {
        self.voice = None;
        self.head = 0;
        self.len = 0;
        self.prev = None;
    }

    /// Queue the clicks of the beats inside this sub-block.
    fn schedule(
        &mut self,
        desc: &MetronomeDesc,
        enabled: bool,
        info: &TransportInfo,
        latency: u32,
        frames: usize,
    ) {
        // Count-in state.
        if desc.count_in_end != self.count_in_end || !info.recording {
            self.count_in_end = desc.count_in_end;
            self.count_in_done = false;
        }
        let count_in_end = match desc.count_in_end {
            Some(end) if info.recording && !self.count_in_done => {
                if info.playing && info.position >= end - EPS {
                    self.count_in_done = true;
                    None
                } else {
                    Some(end)
                }
            }
            _ => None,
        };
        if !info.playing || info.beats_per_sample <= 0.0 {
            self.prev = None;
            return;
        }
        let ramp = self.ramp(info, frames);
        self.prev = Some(ramp);
        let normal = enabled;
        if !(normal || count_in_end.is_some()) {
            return;
        }

        let sr = f64::from(self.sample_rate);
        let b0 = info.position;
        let b1 = ramp.end_pos;
        let den = f64::from(info.time_signature.denominator.max(1));
        let num = i64::from(info.time_signature.numerator.max(1));
        let unit = 4.0 / den;
        let sound = sound_index(desc.sound);

        let mut k = ((b0 - EPS - info.bar_start) / unit).ceil();
        for _ in 0..QUEUE {
            let beat = info.bar_start + k * unit;
            if beat.partial_cmp(&(b1 - EPS)) != Some(std::cmp::Ordering::Less) {
                break;
            }
            let audible = normal || count_in_end.is_some_and(|end| beat < end - EPS);
            if audible {
                let t = ramp.seconds((beat - b0).max(0.0)) * sr;
                let at = (t - SAMPLE_EPS).ceil().max(0.0) as u64;
                let accent = desc.accent && (k as i64).rem_euclid(num) == 0;
                self.push(Pending {
                    due: info.sample_time + at + u64::from(latency),
                    sound,
                    accent,
                    gain: desc.volume,
                });
            }
            k += 1.0;
        }
    }

    /// The tempo ramp of this sub-block (see the module docs).
    fn ramp(&self, info: &TransportInfo, frames: usize) -> Ramp {
        let n = frames as f64;
        let sr = f64::from(self.sample_rate);
        let span = info.beats_per_sample * n;
        let v0 = info.bpm;
        let end_time = info.sample_time + frames as u64;
        let end_pos = info.position + span;
        let make = |end_bpm: f64| Ramp {
            start_bpm: v0,
            end_bpm,
            span,
            end_pos,
            end_time,
        };
        if !(v0.is_finite() && v0 > 0.0 && span > 0.0) {
            // Degenerate: constant at the average rate.
            return make(span / n * sr * 60.0);
        }
        // Same ramp as the previous sub-block, if contiguous and consistent with this one's
        // duration (it may be up to one sample shorter than `frames` at a boundary).
        if let Some(p) = self.prev
            && p.end_time == info.sample_time
            && (p.end_pos - info.position).abs() <= 1e-9
            && (p.end_bpm - v0).abs() <= 1e-9 * v0
        {
            let slope = if p.span > 0.0 {
                (p.end_bpm - p.start_bpm) / p.span
            } else {
                0.0
            };
            let cand = make(v0 + slope * span);
            let t = cand.seconds(span) * sr;
            if cand.end_bpm > 0.0 && t > n - 1.0 - 1e-6 && t <= n + 1e-6 {
                return cand;
            }
        }
        make(v0 * solve_ramp_ratio(n / sr * v0 / (60.0 * span)))
    }

    /// Insert keeping the queue sorted by due time (a latency decrease can schedule a
    /// click before already pending ones).
    fn push(&mut self, p: Pending) {
        if self.len == QUEUE {
            self.dropped += 1;
            return;
        }
        let mut i = self.len;
        while i > 0 && self.queue[(self.head + i - 1) % QUEUE].due > p.due {
            self.queue[(self.head + i) % QUEUE] = self.queue[(self.head + i - 1) % QUEUE];
            i -= 1;
        }
        self.queue[(self.head + i) % QUEUE] = p;
        self.len += 1;
    }

    /// Start due clicks and mix the sounding one into `out`.
    fn play(&mut self, start: u64, offset: usize, frames: usize, out: &mut [&mut [f32]]) {
        let mut j = 0;
        while j < frames {
            while self.len > 0 && self.queue[self.head].due <= start + j as u64 {
                let p = self.queue[self.head];
                self.head = (self.head + 1) % QUEUE;
                self.len -= 1;
                self.voice = Some(Voice {
                    sound: p.sound,
                    accent: p.accent,
                    gain: p.gain,
                    pos: 0,
                });
            }
            let next = if self.len > 0 {
                ((self.queue[self.head].due - start) as usize).min(frames)
            } else {
                frames
            };
            if let Some(v) = &mut self.voice {
                let buf = &self.clicks[v.sound][v.accent as usize];
                let count = (next - j).min(buf.len() - v.pos);
                for o in out.iter_mut().take(2) {
                    let lo = (offset + j).min(o.len());
                    let hi = (offset + j + count).min(o.len());
                    for (d, s) in o[lo..hi].iter_mut().zip(&buf[v.pos..]) {
                        *d += s * v.gain;
                    }
                }
                v.pos += count;
                if v.pos >= buf.len() {
                    self.voice = None;
                }
            }
            j = next;
        }
    }
}

/// Ratio `r = end_bpm / start_bpm` of a linear-in-beats ramp whose duration is `c` times
/// the duration at a constant `start_bpm`: `c = ln(r) / (r - 1)`. Solves
/// `φ(u) = u / (e^u - 1) = c` for `u = ln r` with Newton's method (φ is decreasing and
/// convex, so it converges from any start); bounded iterations, no allocation.
fn solve_ramp_ratio(c: f64) -> f64 {
    if !c.is_finite() || c <= 0.0 {
        return 1.0;
    }
    if (c - 1.0).abs() < 1e-12 {
        return 1.0;
    }
    let phi = |u: f64| {
        if u.abs() < 1e-8 {
            (1.0 - u / 2.0, -0.5 + u / 6.0)
        } else {
            let m = u.exp_m1();
            (u / m, (m - u * u.exp()) / (m * m))
        }
    };
    // Series start: φ(u) ≈ 1 - u/2.
    let mut u = (2.0 * (1.0 - c)).clamp(-20.0, 20.0);
    for _ in 0..64 {
        let (f, df) = phi(u);
        if df == 0.0 || !df.is_finite() {
            break;
        }
        let step = (f - c) / df;
        u = (u - step).clamp(-20.0, 20.0);
        if step.abs() < 1e-15 * u.abs().max(1.0) {
            break;
        }
    }
    u.exp()
}

/// Synthesize one click buffer. Every click starts at full level on its first sample
/// (a click, not a note), then decays.
fn synth(sound: MetronomeSound, accent: bool, sample_rate: f32) -> Vec<f32> {
    let sr = sample_rate.max(1.0);
    let level = if accent { 1.0 } else { NORMAL_LEVEL };
    let tau = std::f32::consts::TAU;
    let (seconds, f): (f32, &dyn Fn(f32) -> f32) = match sound {
        // Short sine blip, higher on accents.
        MetronomeSound::Classic => (0.05, &|t: f32| {
            let hz = if accent { 1600.0 } else { 1000.0 };
            (tau * hz * t).cos() * (-t / 0.012).exp()
        }),
        // Woodblock-like: two inharmonic damped partials.
        MetronomeSound::Wood => (0.04, &|t: f32| {
            let base = if accent { 1100.0 } else { 800.0 };
            0.6 * (tau * base * t).cos() * (-t / 0.008).exp()
                + 0.4 * (tau * base * 2.63 * t).cos() * (-t / 0.004).exp()
        }),
        // Square-wave beep with a short release.
        MetronomeSound::Beep => (0.06, &|t: f32| {
            let hz = if accent { 1320.0 } else { 880.0 };
            let sq = if (hz * t).fract() < 0.5 { 1.0 } else { -1.0 };
            let release = ((0.06 - t) / 0.01).clamp(0.0, 1.0);
            0.5 * sq * release
        }),
    };
    let len = ((seconds * sr) as usize).max(1);
    let mut buf: Vec<f32> = (0..len).map(|i| f(i as f32 / sr)).collect();
    let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        for s in &mut buf {
            *s *= level / peak;
        }
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_protocol::model::TimeSignature;

    const SR: f32 = 48_000.0;

    fn info(position: f64, bpm: f64, sample_time: u64) -> TransportInfo {
        TransportInfo {
            playing: true,
            sample_time,
            position,
            bpm,
            beats_per_sample: bpm / 60.0 / f64::from(SR),
            time_signature: TimeSignature {
                numerator: 4,
                denominator: 4,
            },
            bar_start: (position / 4.0).floor() * 4.0,
            ..TransportInfo::STOPPED
        }
    }

    fn onsets(buf: &[f32]) -> Vec<usize> {
        let mut v = Vec::new();
        let mut prev = 0.0f32;
        for (i, s) in buf.iter().enumerate() {
            if *s != 0.0 && prev == 0.0 {
                v.push(i);
            }
            prev = *s;
        }
        v
    }

    fn run(
        m: &mut Metronome,
        desc: &MetronomeDesc,
        enabled: bool,
        total: usize,
        block: usize,
        latency: u32,
    ) -> Vec<f32> {
        let mut l = vec![0.0f32; total];
        let mut r = vec![0.0f32; total];
        let mut pos = 0.0;
        let mut t = 0u64;
        while (t as usize) < total {
            let n = block.min(total - t as usize);
            let i = info(pos, 120.0, t);
            let off = t as usize;
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            m.render(desc, enabled, &i, latency, off, n, &mut outs);
            pos += i.beats_per_sample * n as f64;
            t += n as u64;
        }
        assert_eq!(l, r);
        l
    }

    #[test]
    fn clicks_on_beats_with_latency() {
        let desc = MetronomeDesc::default();
        let mut m = Metronome::new(SR);
        let out = run(&mut m, &desc, true, 100_000, 333, 0);
        let on = onsets(&out);
        assert_eq!(&on[..4], &[0, 24_000, 48_000, 72_000]);
        let mut m = Metronome::new(SR);
        let out = run(&mut m, &desc, true, 100_000, 512, 1_000);
        assert_eq!(&onsets(&out)[..4], &[1_000, 25_000, 49_000, 73_000]);
        // Accent on the downbeat only.
        let peak = |i: usize| out[i..i + 50].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak(1_000) > peak(25_000) * 1.3);
    }

    #[test]
    fn silent_when_disabled_or_stopped() {
        let desc = MetronomeDesc::default();
        let mut m = Metronome::new(SR);
        assert!(
            run(&mut m, &desc, false, 50_000, 256, 0)
                .iter()
                .all(|s| *s == 0.0)
        );
        let mut m = Metronome::new(SR);
        let mut l = vec![0.0f32; 512];
        let mut outs: [&mut [f32]; 1] = [&mut l];
        m.render(&desc, true, &TransportInfo::STOPPED, 0, 0, 512, &mut outs);
        assert!(l.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn short_channels_are_clamped() {
        let desc = MetronomeDesc::default();
        let mut m = Metronome::new(SR);
        let mut l = vec![0.0f32; 100];
        let mut outs: [&mut [f32]; 1] = [&mut l];
        m.render(&desc, true, &info(0.0, 120.0, 0), 0, 0, 512, &mut outs);
        assert!(l[0] != 0.0);
    }

    #[test]
    fn sounds_differ_and_start_loud() {
        let a = synth(MetronomeSound::Classic, false, SR);
        let b = synth(MetronomeSound::Wood, false, SR);
        let c = synth(MetronomeSound::Beep, true, SR);
        assert!(a != b[..a.len().min(b.len())].to_vec());
        for buf in [&a, &b, &c] {
            assert!(buf[0].abs() > 0.1);
            assert!(buf.iter().all(|s| s.abs() <= 1.0 + 1e-6));
        }
    }

    #[test]
    fn queue_stays_sorted_and_counts_drops() {
        let mut m = Metronome::new(SR);
        let at = |due| Pending {
            due,
            ..Pending::default()
        };
        for due in [500, 100, 300, 200] {
            m.push(at(due));
        }
        let dues: Vec<u64> = (0..m.len)
            .map(|i| m.queue[(m.head + i) % QUEUE].due)
            .collect();
        assert_eq!(dues, vec![100, 200, 300, 500]);
        for i in 0..QUEUE as u64 {
            m.push(at(1000 + i));
        }
        assert_eq!(m.len, QUEUE);
        assert_eq!(m.dropped_clicks(), 4);
    }

    #[test]
    fn ramp_ratio_inverts_the_segment_duration() {
        for r in [1.0, 1.001, 2.0, 49.95, 0.02, 0.5, 500.0] {
            let c = if r == 1.0 {
                1.0
            } else {
                f64::ln(r) / (r - 1.0)
            };
            let got = solve_ramp_ratio(c);
            assert!((got - r).abs() <= 1e-9 * r, "{r}: {got}");
        }
    }
}
