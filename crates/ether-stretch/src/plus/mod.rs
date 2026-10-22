//! **Complex+**: a multi-resolution, transient-aware, formant-preserving phase vocoder
//! (`WarpMode::ComplexPlus`). Pure Rust (native and wasm32), written from published research
//! (references in each module); design and measurements in `docs/WARP.md`.
//!
//! - Three resolutions ([`path`]) share one time map: long frames (≈85 ms) for the lows,
//!   medium (≈43 ms) for the mids, short (≈11 ms) for the highs, each synthesising its band
//!   (log-frequency raised-cosine crossovers), all centred on the same analysis cursor.
//! - Phases: phase-gradient heap integration ([`phase`]) on a reference spectrum shared by
//!   the channels (stereo image kept); exact propagation when nothing is stretched.
//! - Transients ([`onset`]): detected ahead of the cursor. At an onset the cursor snaps to the
//!   attack and runs at the source's own speed for a few ms (the attack is reproduced, not
//!   stretched), the deviation from the requested time map being absorbed smoothly after;
//!   rising bins' phases reset; analysis frames straddling a strong attack (in their band)
//!   are gated to one side of it, so no frame smears the attack before it happens
//!   (pre-echo).
//! - Formants ([`envelope`]): true-envelope estimate on the mid resolution; pitch shifting
//!   moves the spectrum and a gain moves the envelope back (amount 0–100 %), optionally
//!   shifted (semitones), with an envelope-detail control (Live's Complex Pro "Envelope").
//!
//! The streaming contract is [`Stretcher`]'s: after producing output sample `o`, the caller
//! has fed input up to `src(o + output_latency) + input_latency`; output `o` is then `src(o)`.
//! Frames are scheduled on a tick grid of output samples; a frame at tick `f` analyses the
//! input at the cursor `src(f)` (interpolated within the call, plus the transient deviation)
//! and is synthesised centred on output sample `f`.

mod envelope;
mod onset;
mod path;
mod phase;

use std::sync::atomic::{AtomicU32, Ordering};

use self::envelope::EnvelopeEstimator;
use self::onset::{BANDS, Onset, OnsetDetector};
use self::path::{Edge, FormantMap, FrameArgs, Gate, Path, PathSpec};
use crate::{Formants, Stretcher, StretcherFactory};

/// Most channels a stretcher handles.
const MAX_CHANNELS: usize = 8;
/// Input frames per `max_block` output frames one internal call takes (larger calls are
/// split).
const INPUT_HEADROOM: usize = 8;
/// Sizes at 48 kHz (scaled to the nearest power of two at other rates).
const TICK_48K: usize = 128;
const LEAD_48K: usize = 1536;
/// Crossovers (Hz) between the long/mid and mid/short resolutions, and the half-width of
/// each transition (octaves).
const SPLIT_LOW: f32 = 550.0;
const SPLIT_HIGH: f32 = 4500.0;
const TRANSITION_OCT: f32 = 0.45;
/// Attack held at the source's speed after an onset (seconds).
const HOLD_SECONDS: f32 = 0.024;
/// After the hold, the cursor's deviation decays at up to this fraction of the nominal
/// speed (the effective speed stays within `1 ± RECOVER` of the requested one).
const RECOVER: f64 = 0.5;
/// Gate ramp before an attack (seconds).
const GATE_RAMP_SECONDS: f32 = 0.0015;
/// The resolution whose spectra feed the formant envelope (the mid one: resolves harmonics
/// of voices, tracks formant motion).
const ENVELOPE_PATH: usize = 1;
/// True-envelope iterations.
const ENVELOPE_ITERS: usize = 8;

static INSTANCES: AtomicU32 = AtomicU32::new(0);

/// Per-channel input ring.
pub(crate) struct InputRing {
    data: Vec<Vec<f32>>,
    mask: usize,
    /// Samples received (index of the next one).
    count: i64,
}

impl InputRing {
    fn new(channels: usize, cap: usize) -> Self {
        let cap = cap.next_power_of_two();
        Self {
            data: (0..channels).map(|_| vec![0.0; cap]).collect(),
            mask: cap - 1,
            count: 0,
        }
    }

    fn reset(&mut self) {
        for d in &mut self.data {
            d.fill(0.0);
        }
        self.count = 0;
    }

    /// RT. Samples `start..start + out.len()` of channel `ch` (zeros outside what the ring
    /// holds).
    pub(crate) fn read(&self, ch: usize, start: i64, out: &mut [f32]) {
        let len = out.len() as i64;
        let oldest = (self.count - self.mask as i64).max(0);
        let (a, b) = (start.max(oldest), (start + len).min(self.count));
        if a >= b {
            out.fill(0.0);
            return;
        }
        let (pre, valid) = ((a - start) as usize, (b - a) as usize);
        out[..pre].fill(0.0);
        out[pre + valid..].fill(0.0);
        let data = &self.data[ch];
        let cap = self.mask + 1;
        let from = a as usize & self.mask;
        let first = valid.min(cap - from);
        out[pre..pre + first].copy_from_slice(&data[from..from + first]);
        out[pre + first..pre + valid].copy_from_slice(&data[..valid - first]);
    }
}

struct State {
    channels: usize,
    sr: f32,
    tick: i64,
    lead: f64,
    hold_ticks: u32,
    in_lat: i64,
    out_lat: i64,
    max_out: usize,
    max_in: usize,
    input: InputRing,
    onsets: OnsetDetector,
    /// Long, mid, short.
    paths: [Path; 3],
    /// Tick phase offsets (stagger the expensive frames across instances and paths).
    phase_off: [i64; 3],
    envelope: EnvelopeEstimator,
    log_env: Vec<f32>,
    env_valid: bool,
    out_count: i64,
    /// Clock of the next tick to run.
    next_tick: i64,
    /// Transient deviation of the cursor from the time map (input samples).
    dev: f64,
    hold: u32,
    last_nominal: f64,
    last_cursor: f64,
    /// Onsets before this are consumed.
    onset_from: i64,
    /// Per path: an onset whose first frame at/after it resets phases.
    pending_reset: [Option<i64>; 3],
    gate_ramp: usize,
}

/// Complex+ behind [`Stretcher`]. Unconfigured instances output silence.
pub struct PlusStretcher {
    transpose: f32,
    formants: Formants,
    instance: u32,
    state: Option<Box<State>>,
}

impl Default for PlusStretcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Nearest power of two to `x`.
fn pow2_near(x: f32) -> usize {
    let x = x.max(1.0);
    let lo = (x.log2().floor()).exp2();
    if x / lo < std::f32::consts::SQRT_2 {
        lo as usize
    } else {
        (lo * 2.0) as usize
    }
}

impl PlusStretcher {
    pub fn new() -> Self {
        Self {
            transpose: 0.0,
            formants: Formants::default(),
            instance: INSTANCES.fetch_add(1, Ordering::Relaxed),
            state: None,
        }
    }

    fn channels(&self) -> usize {
        self.state.as_ref().map_or(0, |s| s.channels)
    }
}

impl State {
    fn new(channels: usize, sr: f32, max_block: usize, instance: u32) -> Self {
        let scale = pow2_near(sr / 48_000.0 * 16.0) as f32 / 16.0;
        let at = |n48: usize| ((n48 as f32 * scale) as usize).max(16);
        let tick = at(TICK_48K);
        let lo_t = |c: f32| (c * (-TRANSITION_OCT).exp2(), c * TRANSITION_OCT.exp2());
        let specs = [
            PathSpec {
                n: at(4096),
                hop_ticks: 8,
                lo: None,
                hi: Some(lo_t(SPLIT_LOW)),
                onset_band: 0,
                follow: None,
            },
            PathSpec {
                n: at(2048),
                hop_ticks: 4,
                lo: Some(lo_t(SPLIT_LOW)),
                hi: Some(lo_t(SPLIT_HIGH)),
                onset_band: 1,
                follow: Some(Edge::Lo),
            },
            PathSpec {
                n: at(512),
                hop_ticks: 1,
                lo: Some(lo_t(SPLIT_HIGH)),
                hi: None,
                onset_band: 2,
                follow: Some(Edge::Lo),
            },
        ];
        let n_max = specs.iter().map(|s| s.n).max().unwrap_or(16);
        let lead = at(LEAD_48K);
        let in_lat = (n_max / 2 + lead) as i64;
        let out_lat = (n_max / 2) as i64;
        let max_out = max_block.max(1);
        let max_in = max_out * INPUT_HEADROOM;
        let in_cap = n_max + 2 * lead + max_in + 4 * tick + 64;
        let out_cap = (n_max + out_lat as usize + max_out + 4 * tick).next_power_of_two();
        let paths = specs.map(|s| Path::new(s, sr, channels, tick, out_cap));
        let env_n = paths[1].n();
        let off = (instance % 8) as i64;
        Self {
            channels,
            sr,
            tick: tick as i64,
            lead: lead as f64,
            hold_ticks: ((HOLD_SECONDS * sr) / tick as f32).round() as u32,
            in_lat,
            out_lat,
            max_out,
            max_in,
            input: InputRing::new(channels, in_cap),
            onsets: OnsetDetector::new(sr, at(512), tick, [SPLIT_LOW, SPLIT_HIGH], in_cap),
            paths,
            phase_off: [off, (off + 2) % 4, 0],
            envelope: EnvelopeEstimator::new(env_n),
            log_env: vec![0.0; env_n / 2 + 1],
            env_valid: false,
            out_count: 0,
            next_tick: 0,
            dev: 0.0,
            hold: 0,
            last_nominal: f64::NAN,
            last_cursor: f64::NAN,
            onset_from: i64::MIN,
            pending_reset: [None; 3],
            gate_ramp: ((GATE_RAMP_SECONDS * sr) as usize).max(2),
        }
    }

    fn reset(&mut self) {
        self.input.reset();
        self.onsets.reset();
        for p in &mut self.paths {
            p.reset();
        }
        self.env_valid = false;
        self.out_count = 0;
        self.next_tick = self.first_tick();
        self.dev = 0.0;
        self.hold = 0;
        self.last_nominal = f64::NAN;
        self.last_cursor = f64::NAN;
        self.onset_from = i64::MIN;
        self.pending_reset = [None; 3];
    }

    /// First tick whose frames reach output sample 0.
    fn first_tick(&self) -> i64 {
        let half = (self.paths.iter().map(Path::n).max().unwrap_or(16) / 2) as i64;
        (-half).div_euclid(self.tick) * self.tick + self.tick
    }

    fn push_input(&mut self, input: &[&[f32]], from: usize, frames: usize) {
        let mask = self.input.mask;
        let start = self.input.count as usize;
        // Each channel into its ring (missing input reads silence).
        for ch in 0..self.channels {
            let src = input.get(ch).map_or(&[][..], |c| {
                &c[from.min(c.len())..(from + frames).min(c.len())]
            });
            let data = &mut self.input.data[ch];
            for (i, v) in src
                .iter()
                .chain(std::iter::repeat(&0.0))
                .take(frames)
                .enumerate()
            {
                data[(start + i) & mask] = *v;
            }
        }
        // Their mix feeds the onset detector.
        let inv = 1.0 / self.channels as f32;
        for i in 0..frames {
            let at = (start + i) & mask;
            let mono: f32 = self.input.data[..self.channels].iter().map(|d| d[at]).sum();
            self.onsets.push(mono * inv);
        }
        self.input.count += frames as i64;
        self.onsets.run();
    }

    /// The cursor for the tick at nominal input position `nominal`: applies the transient
    /// hold / recovery and snaps to an onset crossed since the previous tick. Returns the
    /// cursor and the onset hit, if any.
    fn cursor(&mut self, nominal: f64) -> (f64, Option<Onset>) {
        let step = if self.last_nominal.is_finite() {
            nominal - self.last_nominal
        } else {
            0.0
        };
        self.last_nominal = nominal;
        let tick = self.tick as f64;
        if self.hold > 0 && self.last_cursor.is_finite() {
            let want = self.last_cursor + tick;
            let dev = want - nominal;
            if dev.abs() <= self.lead {
                self.dev = dev;
                self.hold -= 1;
            } else {
                self.hold = 0;
            }
        } else if self.dev != 0.0 {
            let max = RECOVER * step.abs();
            self.dev -= self.dev.clamp(-max, max);
        }
        let mut c = nominal + self.dev;
        let mut hit = None;
        let prev = if self.last_cursor.is_finite() {
            self.last_cursor
        } else {
            c - step.max(0.0)
        };
        while let Some(o) = self.onsets.next_after(self.onset_from) {
            let pos = o.pos as f64;
            if pos > c {
                break;
            }
            self.onset_from = o.pos + 1;
            // At the source's own speed there is nothing to protect (and snapping would only
            // offset the phases).
            let unity = (step - tick).abs() < 1e-9;
            if pos > prev && !unity && (self.dev + pos - c).abs() <= self.lead {
                // Snap the cursor onto the attack and hold the source's speed.
                self.dev += pos - c;
                c = pos;
                self.hold = self.hold_ticks;
                hit = Some(o);
            }
        }
        self.last_cursor = c;
        (c, hit)
    }

    /// Strongest onsets (in `band`) just before and after `ci`, within `half`.
    fn gate_for(&self, ci: i64, half: i64, band: usize) -> Gate {
        let mut g = Gate {
            ramp: self.gate_ramp,
            ..Gate::default()
        };
        for o in self.onsets.onsets() {
            if o.strength[band] < 1.0 {
                continue;
            }
            if o.pos > ci && o.pos < ci + half {
                if g.pre.is_none() {
                    g.pre = Some(o.pos);
                }
            } else if o.pos <= ci && o.pos > ci - half {
                g.post = Some(o.pos);
            }
        }
        g
    }

    /// Run the ticks up to clock `b_clock`, with the cursor going linearly from `a` (at clock
    /// `a_clock`) to `b` (at `b_clock`), extrapolated before `a_clock`.
    fn run_ticks(
        &mut self,
        a_clock: i64,
        a: f64,
        b_clock: i64,
        b: f64,
        transpose: f32,
        formants: &Formants,
    ) {
        let span = (b_clock - a_clock) as f64;
        let rate = if span > 0.0 { (b - a) / span } else { 0.0 };
        let unity = (rate - 1.0).abs() < 1e-9;
        let pitch = (transpose / 12.0).exp2();
        let formant_target = {
            let amount = formants.amount.clamp(0.0, 1.0);
            pitch.powf(1.0 - amount) * (formants.shift_semitones / 12.0).exp2()
        };
        let formants_on = (formant_target / pitch - 1.0).abs() > 1e-4;
        let cutoff = (formants.envelope.clamp(8.0, 256.0) * 0.5 * self.sr / 44_100.0) as usize;
        while self.next_tick <= b_clock {
            let f = self.next_tick;
            self.next_tick += self.tick;
            let nominal = a + (f - a_clock) as f64 * rate;
            let (c, hit) = self.cursor(nominal);
            if let Some(o) = hit {
                self.pending_reset = [Some(o.pos); 3];
            }
            let idx = f.div_euclid(self.tick);
            // Longest first: each resolution follows the next-longer one in their crossover.
            for pi in 0..self.paths.len() {
                let hop = self.paths[pi].spec.hop_ticks as i64;
                if (idx + self.phase_off[pi]).rem_euclid(hop) != 0 {
                    continue;
                }
                let half = (self.paths[pi].n() / 2) as i64;
                if f + half <= self.out_count {
                    continue;
                }
                let ci = c.round() as i64;
                // At the source's speed frames are reproduced in place: nothing to gate.
                let gate = if unity {
                    Gate::default()
                } else {
                    self.gate_for(ci, half, self.paths[pi].spec.onset_band.min(BANDS - 1))
                };
                let reset = match self.pending_reset[pi] {
                    Some(pos) if ci >= pos => {
                        self.pending_reset[pi] = None;
                        true
                    }
                    _ => false,
                };
                // The mid path also estimates the envelope (used from its next frame on, and
                // by the other paths).
                let want_energy = pi == ENVELOPE_PATH && formants_on;
                let env_bin_hz = self.paths[ENVELOPE_PATH].bin_hz();
                let (longer, rest) = self.paths.split_at_mut(pi);
                let args = FrameArgs {
                    input: &self.input,
                    center: c,
                    clock: f,
                    out_from: self.out_count,
                    pitch,
                    gate,
                    reset,
                    formants: (formants_on && self.env_valid).then(|| FormantMap {
                        log_env: &self.log_env,
                        env_bin_hz,
                        target: formant_target,
                    }),
                    want_energy,
                    master: longer.last().and_then(Path::as_master),
                };
                rest[0].frame(&args);
                if want_energy {
                    let energy = self.paths[ENVELOPE_PATH].energy();
                    self.envelope
                        .estimate(energy, cutoff, ENVELOPE_ITERS, &mut self.log_env);
                    self.env_valid = true;
                }
            }
        }
    }

    fn process(
        &mut self,
        input: &[&[f32]],
        input_frames: usize,
        output: &mut [&mut [f32]],
        output_frames: usize,
        transpose: f32,
        formants: &Formants,
    ) {
        let a_clock = self.out_count + self.out_lat;
        let a = (self.input.count - self.in_lat) as f64;
        self.push_input(input, 0, input_frames);
        let b_clock = a_clock + output_frames as i64;
        let b = (self.input.count - self.in_lat) as f64;
        self.run_ticks(a_clock, a, b_clock, b, transpose, formants);
        for i in 0..output_frames {
            let o = self.out_count + i as i64;
            for ch in 0..self.channels {
                let mut v = 0.0;
                for p in &mut self.paths {
                    v += p.take(ch, o, ch + 1 == self.channels);
                }

                if let Some(out) = output.get_mut(ch)
                    && let Some(s) = out.get_mut(i)
                {
                    *s = v;
                }
            }
        }
        self.out_count += output_frames as i64;
    }
}

impl Stretcher for PlusStretcher {
    fn configure(&mut self, channels: usize, sample_rate: f32, max_block: usize) {
        let channels = channels.clamp(1, MAX_CHANNELS);
        let sr = sample_rate.max(8_000.0).round();
        let mut s = State::new(channels, sr, max_block, self.instance);
        s.reset();
        self.state = Some(Box::new(s));
    }

    fn reset(&mut self) {
        if let Some(s) = &mut self.state {
            s.reset();
        }
    }

    fn input_latency(&self) -> usize {
        self.state.as_ref().map_or(0, |s| s.in_lat as usize)
    }

    fn output_latency(&self) -> usize {
        self.state.as_ref().map_or(0, |s| s.out_lat as usize)
    }

    fn set_transpose_semitones(&mut self, semitones: f32) {
        self.transpose = if semitones.is_finite() {
            semitones.clamp(-48.0, 48.0)
        } else {
            0.0
        };
    }

    fn set_formants(&mut self, formants: Formants) {
        self.formants = formants;
    }

    fn seek(&mut self, input: &[&[f32]], playback_rate: f64) {
        let channels = self.channels();
        let Some(s) = &mut self.state else { return };
        s.reset();
        let len = input
            .iter()
            .take(channels)
            .map(|c| c.len())
            .min()
            .unwrap_or(0);
        let keep = len.min(s.input.mask + 1);
        let from = len - keep;
        let mut tails: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        for (t, ch) in tails.iter_mut().zip(input.iter().take(channels)) {
            *t = &ch[ch.len() - len..];
        }
        s.push_input(&tails[..channels.min(input.len())], from, keep);
        // Pretend the stretcher has been running at `playback_rate`: frames reaching output
        // sample 0 onwards, up to the clock of output 0.
        let rate = if playback_rate.is_finite() {
            playback_rate.clamp(0.0, 64.0)
        } else {
            1.0
        };
        let b_clock = s.out_lat;
        let b = (s.input.count - s.in_lat) as f64;
        let a_clock = s.first_tick() - s.tick;
        let a = b - (b_clock - a_clock) as f64 * rate;
        let (t, f) = (self.transpose, self.formants);
        s.run_ticks(a_clock, a, b_clock, b, t, &f);
    }

    fn process(
        &mut self,
        input: &[&[f32]],
        input_frames: usize,
        output: &mut [&mut [f32]],
        output_frames: usize,
    ) {
        let channels = self.channels();
        let (t, f) = (self.transpose, self.formants);
        if let Some(s) = &mut self.state {
            // Split calls larger than the buffers into proportional chunks.
            let (mut done_in, mut done_out) = (0usize, 0usize);
            loop {
                let left_out = output_frames - done_out;
                let left_in = input_frames - done_in;
                if left_out == 0 && left_in == 0 {
                    break;
                }
                let n_out = left_out.min(s.max_out);
                let mut n_in = if n_out == left_out {
                    // The last output chunk (or input only) takes the rest.
                    left_in
                } else {
                    ((input_frames as f64 * (done_out + n_out) as f64 / output_frames as f64)
                        .round() as usize)
                        .saturating_sub(done_in)
                        .min(left_in)
                };
                n_in = n_in.min(s.max_in);
                let mut ins: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
                for (d, c) in ins.iter_mut().zip(input.iter()) {
                    *d = &c[done_in.min(c.len())..];
                }
                let n_outs = output.len().min(MAX_CHANNELS);
                let mut outs: [&mut [f32]; MAX_CHANNELS] = Default::default();
                for (d, c) in outs.iter_mut().zip(output.iter_mut()) {
                    let len = c.len();
                    *d = &mut c[done_out.min(len)..];
                }
                s.process(
                    &ins[..input.len().min(MAX_CHANNELS)],
                    n_in,
                    &mut outs[..n_outs],
                    n_out,
                    t,
                    &f,
                );
                done_in += n_in;
                done_out += n_out;
            }
        }
        for ch in output.iter_mut().skip(channels) {
            let n = output_frames.min(ch.len());
            ch[..n].fill(0.0);
        }
    }
}

/// Factory for [`PlusStretcher`]s (unconfigured; the caller configures them).
#[derive(Debug, Clone, Copy, Default)]
pub struct PlusStretchFactory;

impl StretcherFactory for PlusStretchFactory {
    fn create(&self) -> Box<dyn Stretcher> {
        Box::new(PlusStretcher::new())
    }
}

#[cfg(test)]
mod tests;
