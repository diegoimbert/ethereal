//! `Tuner`: audio passes through (or is muted with `Mute Output`, ramped, sample-accurate);
//! the pitch of the selected input is estimated with the McLeod pitch method (normalised
//! square difference function via FFT autocorrelation, key-maximum picking, parabolic
//! interpolation) about [`ANALYSIS_HZ`] times per second while watched, and published as
//! `AnalysisKind::Tuner` frames `[hz|0, note|-1, cents, confidence, level_db]`
//! (CONTRACTS.md §12.4.3), note and cents relative to `Reference` (A4).
//!
//! - Window: ~85 ms (the power of two ≥ 0.085 s, 4096 samples at 48 kHz), which resolves
//!   down to ~25 Hz (five-string bass low B is 31 Hz); range 25 Hz ..= 4.2 kHz.
//! - No pitch (hz 0, note -1) when the window is quieter than [`GATE_DB`] or its clarity
//!   (the NSDF peak) is below [`MIN_CLARITY`]; `confidence` is the clarity either way.
//! - Successive estimates on the same note are lightly smoothed so the needle is steady.
//!
//! Real-time: buffers allocated in `new`/`prepare`; the estimate runs in `process` (at most
//! once per call, only while frames are being collected); `analysis` only copies.

use ether_core::analysis::{ANALYSIS_HZ, AnalysisKind, AnalysisSink};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use super::fft::Fft;
use super::tuner as p;
use crate::util;

/// Below this window level (dBFS) there is no pitch.
pub const GATE_DB: f32 = -60.0;
/// Below this NSDF clarity there is no pitch.
pub const MIN_CLARITY: f32 = 0.8;
/// Key maxima within this fraction of the highest one are candidates (MPM `k`).
const PEAK_K: f32 = 0.9;
const MIN_HZ: f32 = 25.0;
const MAX_HZ: f32 = 4200.0;
const WINDOW_SECONDS: f32 = 0.085;
const MAX_WINDOW: usize = 16_384;
const MUTE_MS: f32 = 10.0;
const IDLE_SECONDS: f32 = 1.0;

/// One estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pitch {
    /// Frequency (Hz), 0 = none.
    pub hz: f32,
    /// Nearest MIDI note, -1 = none.
    pub note: f32,
    pub cents: f32,
    pub confidence: f32,
    pub level_db: f32,
}

impl Pitch {
    const NONE: Self = Self {
        hz: 0.0,
        note: -1.0,
        cents: 0.0,
        confidence: 0.0,
        level_db: -160.0,
    };
}

pub struct Tuner {
    descriptor: DeviceDescriptor,
    values: [f64; p::COUNT],
    sample_rate: f32,
    window: usize,
    fft: Fft,
    /// Mono input history (ring of `window`).
    history: Vec<f32>,
    write: usize,
    frame: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    nsdf: Vec<f32>,
    gain: Smoother,
    hop: usize,
    since_hop: usize,
    idle: usize,
    last: Pitch,
    fresh: bool,
}

/// Non-RT. Analysis window (samples) at `sample_rate`.
fn window_for(sample_rate: f32) -> usize {
    ((sample_rate * WINDOW_SECONDS).max(64.0) as usize)
        .next_power_of_two()
        .min(MAX_WINDOW)
}

impl Tuner {
    /// Non-RT.
    pub fn new(descriptor: DeviceDescriptor) -> Self {
        let mut values = [0.0; p::COUNT];
        for info in &descriptor.params {
            values[info.id.0 as usize] = info.default;
        }
        let mut t = Self {
            descriptor,
            values,
            sample_rate: 48_000.0,
            window: 0,
            fft: Fft::new(2),
            history: Vec::new(),
            write: 0,
            frame: Vec::new(),
            re: Vec::new(),
            im: Vec::new(),
            nsdf: Vec::new(),
            gain: Smoother::new(1.0, MUTE_MS, 48_000.0),
            hop: 1,
            since_hop: 0,
            idle: usize::MAX,
            last: Pitch::NONE,
            fresh: false,
        };
        t.configure(48_000.0);
        t
    }

    fn configure(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        let w = window_for(self.sample_rate);
        if w != self.window {
            self.window = w;
            self.fft = Fft::new(2 * w);
            self.history = vec![0.0; w];
            self.frame = vec![0.0; w];
            self.re = vec![0.0; 2 * w];
            self.im = vec![0.0; 2 * w];
            self.nsdf = vec![0.0; w / 2 + 2];
        }
        self.hop = (self.sample_rate / ANALYSIS_HZ as f32).round().max(1.0) as usize;
        self.gain = Smoother::new(self.target_gain(), MUTE_MS, self.sample_rate);
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    fn target_gain(&self) -> f32 {
        if self.value(p::MUTE) >= 0.5 { 0.0 } else { 1.0 }
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(info) = self.descriptor.params.iter().find(|i| i.id == id) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, info.min, info.max);
        if id == p::MUTE {
            let g = self.target_gain();
            if smooth {
                self.gain.set_target(g);
            } else {
                self.gain.set_immediate(g);
            }
        }
    }

    /// The last estimate (tests).
    pub fn pitch(&self) -> Pitch {
        self.last
    }

    /// RT. McLeod pitch estimate of the latest window → `self.last`.
    fn estimate(&mut self) {
        let w = self.window;
        let start = self.write % w;
        let mut energy = 0.0f64;
        for i in 0..w {
            let x = self.history[(start + i) % w];
            self.frame[i] = x;
            energy += f64::from(x * x);
        }
        let rms = (energy / w as f64).sqrt() as f32;
        let level_db = util::amp_to_db(rms).max(-160.0);
        // Autocorrelation r(τ) = IFFT(|FFT(x zero-padded to 2w)|²).
        for i in 0..2 * w {
            self.re[i] = if i < w { self.frame[i] } else { 0.0 };
            self.im[i] = 0.0;
        }
        self.fft.forward(&mut self.re, &mut self.im);
        for i in 0..2 * w {
            self.re[i] = self.re[i] * self.re[i] + self.im[i] * self.im[i];
            self.im[i] = 0.0;
        }
        // The power spectrum is real and even: its forward transform is 2w · r(τ).
        self.fft.forward(&mut self.re, &mut self.im);
        let scale = 1.0 / (2 * w) as f32;
        // NSDF n(τ) = 2 r(τ) / m(τ), m(τ) = Σ x_j² + x_{j+τ}² (incremental).
        let max_tau = (w / 2).min(self.nsdf.len() - 1);
        let mut m = 2.0 * energy as f32;
        for tau in 0..=max_tau {
            if tau > 0 {
                let a = self.frame[tau - 1];
                let b = self.frame[w - tau];
                m -= a * a + b * b;
            }
            self.nsdf[tau] = if m > 1e-12 {
                (2.0 * self.re[tau] * scale / m).clamp(-1.0, 1.0)
            } else {
                0.0
            };
        }
        let min_tau = ((self.sample_rate / MAX_HZ).floor() as usize).max(2);
        let limit = ((self.sample_rate / MIN_HZ).ceil() as usize).min(max_tau - 1);
        let (tau, clarity) = pick_peak(&self.nsdf[..=max_tau], min_tau, limit);
        let reference = self.value(p::REFERENCE) as f32;
        self.last = if tau > 0.0 && clarity >= MIN_CLARITY && level_db >= GATE_DB {
            let mut hz = self.sample_rate / tau;
            let midi = 69.0 + 12.0 * (hz / reference).log2();
            let note = midi.round().clamp(0.0, 127.0);
            if self.last.note == note && self.last.hz > 0.0 {
                hz = 0.6 * self.last.hz + 0.4 * hz;
            }
            let midi = 69.0 + 12.0 * (hz / reference).log2();
            Pitch {
                hz,
                note,
                cents: ((midi - note) * 100.0).clamp(-50.0, 50.0),
                confidence: clarity,
                level_db,
            }
        } else {
            Pitch {
                confidence: clarity.max(0.0),
                level_db,
                ..Pitch::NONE
            }
        };
        self.fresh = true;
    }
}

/// MPM peak picking on `nsdf` (index = lag): the key maximum of each positive lobe after
/// the first negative-going zero crossing (lags `>= min_tau`); the first whose value is
/// within [`PEAK_K`] of the highest wins (lags `<= max_tau`). Returns `(interpolated lag, clarity)`, lag 0 = none.
fn pick_peak(nsdf: &[f32], min_tau: usize, max_tau: usize) -> (f32, f32) {
    let n = nsdf.len();
    // Skip the zero-lag lobe.
    let mut i = 1;
    while i < n && nsdf[i] > 0.0 {
        i += 1;
    }
    let mut best: (usize, f32) = (0, 0.0);
    let mut highest = 0.0f32;
    // Up to 32 key maxima (bounded work, no allocation).
    let mut keys = [(0usize, 0.0f32); 32];
    let mut count = 0;
    while i < n && count < keys.len() {
        while i < n && nsdf[i] <= 0.0 {
            i += 1;
        }
        let mut peak = (0usize, f32::MIN);
        while i < n && nsdf[i] > 0.0 {
            if nsdf[i] > peak.1 && i >= min_tau && i <= max_tau {
                peak = (i, nsdf[i]);
            }
            i += 1;
        }
        if peak.0 > 0 && peak.0 + 1 < n {
            keys[count] = peak;
            count += 1;
            highest = highest.max(peak.1);
        }
    }
    for &(k, v) in &keys[..count] {
        if v >= PEAK_K * highest {
            best = (k, v);
            break;
        }
    }
    let (k, v) = best;
    if k == 0 || k + 1 >= n {
        return (0.0, highest.max(0.0));
    }
    // Parabolic interpolation around the peak.
    let (a, b, c) = (nsdf[k - 1], v, nsdf[k + 1]);
    let den = a - 2.0 * b + c;
    let (shift, value) = if den.abs() > 1e-9 {
        let d = (0.5 * (a - c) / den).clamp(-0.5, 0.5);
        (d, b - 0.25 * (a - c) * d)
    } else {
        (0.0, b)
    };
    (k as f32 + shift, value.clamp(0.0, 1.0))
}

impl Node for Tuner {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.configure(config.sample_rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.history.fill(0.0);
        self.write = 0;
        self.since_hop = 0;
        self.last = Pitch::NONE;
        self.gain.set_immediate(self.target_gain());
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let frames = ctx.frames;
        let inputs = audio.inputs;
        let outputs = &mut *audio.outputs;
        util::split_at_events(
            self,
            ctx.events,
            frames,
            |t, a, b| {
                let input = util::index(t.value(p::INPUT), 3);
                let w = t.window;
                let left = inputs.first().copied();
                let right = inputs.get(1).copied().or(left);
                for i in a..b {
                    let l = left.map_or(0.0, |s| s[i]);
                    let r = right.map_or(0.0, |s| s[i]);
                    let x = match input {
                        1 => l,
                        2 => r,
                        _ => 0.5 * (l + r),
                    };
                    t.history[t.write] = if x.is_finite() { x } else { 0.0 };
                    t.write = (t.write + 1) % w;
                    let g = t.gain.tick();
                    for (ch, out) in outputs.iter_mut().enumerate() {
                        let s = inputs.get(ch).or(inputs.first()).map_or(0.0, |s| s[i]);
                        out[i] = s * g;
                    }
                }
            },
            |t, kind| {
                if let EventKind::Param { param, value } = *kind {
                    t.set(param, value, true);
                }
            },
        );
        self.since_hop += frames;
        self.idle = self.idle.saturating_add(frames);
        let watched = (self.idle as f32) < IDLE_SECONDS * self.sample_rate;
        if !watched {
            self.fresh = false;
            self.last = Pitch::NONE;
        }
        if watched && self.since_hop >= self.hop {
            self.since_hop = 0;
            self.estimate();
        }
        if self.gain.current() == 0.0 && !self.gain.is_smoothing() {
            ProcessStatus::Silent
        } else {
            ProcessStatus::Continue
        }
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        self.idle = 0;
        if !self.fresh {
            return;
        }
        if let Some(f) = out.frame(AnalysisKind::Tuner) {
            let q = self.last;
            for v in [q.hz, q.note, q.cents, q.confidence, q.level_db] {
                f.push(v);
            }
            self.fresh = false;
        }
    }
}

impl Device for Tuner {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value, false);
    }
}

/// Non-RT.
pub(crate) fn create() -> Tuner {
    Tuner::new(super::descriptor(BuiltinDeviceType::Tuner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_covers_low_notes() {
        assert_eq!(window_for(48_000.0), 4096);
        assert_eq!(window_for(44_100.0), 4096);
        assert_eq!(window_for(96_000.0), 8192);
        assert_eq!(window_for(192_000.0), 16_384);
    }
}
