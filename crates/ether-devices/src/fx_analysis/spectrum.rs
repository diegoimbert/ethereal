//! `SpectrumAnalyzer`: audio passes through untouched; the input is analysed with a
//! Hann-windowed FFT (`Block Size` points) about [`ANALYSIS_HZ`] times per second while
//! the device is watched, reduced to [`BINS`] log-spaced bins (20 Hz ..= 20 kHz or
//! Nyquist) and published as `AnalysisKind::Spectrum` frames (CONTRACTS.md §12.4.3).
//!
//! - Levels are dBFS of a sine: a full-scale sine centred on a bin reads 0 dB. Where a log
//!   bin covers several FFT bins it shows their maximum (peaks stay visible); where it falls
//!   between two (low end, small blocks) the power is interpolated.
//! - `Channel`: `Stereo` averages the left and right power spectra (one packed complex FFT
//!   for both), `Left`/`Right`, `Mid` = (L+R)/2, `Side` = (L−R)/2.
//! - `Averaging` smooths successive frames per bin in dB (exponential, 0 % = raw).
//! - `Slope` tilts the display by that many dB per octave around 1 kHz (pink noise reads
//!   flat at 3 dB/oct).
//! - `Range` and `Peak Hold` are display params, read by the shared renderer's Spectrum
//!   widget (dB floor, UI-held peak line).
//!
//! Real-time: all buffers are allocated in `new`/`prepare` for the largest block size; the
//! FFT runs in `process` (at most once per call, only while frames are being collected,
//! i.e. `analysis` was called within the last second); `analysis` only copies.

use ether_core::analysis::{ANALYSIS_HZ, AnalysisKind, AnalysisSink};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::fft::{Fft, hann};
use super::spectrum_analyzer as p;
use crate::util;

/// Log-spaced bins per frame.
pub const BINS: usize = 256;
/// Lowest analysed frequency.
pub const MIN_HZ: f32 = 20.0;
/// Highest analysed frequency (capped at 0.999 × Nyquist).
pub const MAX_HZ: f32 = 20_000.0;
/// FFT sizes of the `Block Size` choices.
pub const BLOCK_SIZES: [usize; 4] = [1024, 2048, 4096, 8192];
const MAX_N: usize = 8192;
/// Floor of the published levels (silence).
pub const FLOOR_DB: f32 = -160.0;
/// The tilt pivot of `Slope`.
const PIVOT_HZ: f32 = 1000.0;
/// Frames stop being computed after this long without an `analysis` call.
const IDLE_SECONDS: f32 = 1.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Channel {
    Stereo,
    Left,
    Right,
    Mid,
    Side,
}

pub struct SpectrumAnalyzer {
    descriptor: DeviceDescriptor,
    values: [f64; p::COUNT],
    sample_rate: f32,
    fft: Fft,
    /// Hann windows per block size and their sums.
    windows: [Vec<f32>; 4],
    window_sums: [f32; 4],
    /// Input history (L, R), ring of `MAX_N`.
    history: [Vec<f32>; 2],
    write: usize,
    re: Vec<f32>,
    im: Vec<f32>,
    power: Vec<f32>,
    /// Averaged bins (dB, before slope).
    avg: [f32; BINS],
    /// Published bins (dB, slope applied).
    out: [f32; BINS],
    /// Bin centre frequencies and edges (`BINS + 1`), for the current sample rate.
    centres: [f32; BINS],
    edges: [f32; BINS + 1],
    max_hz: f32,
    hop: usize,
    since_hop: usize,
    /// Samples since the last `analysis` call.
    idle: usize,
    /// Whether `avg` holds a frame (else the next frame is taken as is).
    primed: bool,
    fresh: bool,
}

impl SpectrumAnalyzer {
    /// Non-RT.
    pub fn new(descriptor: DeviceDescriptor) -> Self {
        let mut values = [0.0; p::COUNT];
        for info in &descriptor.params {
            values[info.id.0 as usize] = info.default;
        }
        let windows = BLOCK_SIZES.map(hann);
        let window_sums = [0, 1, 2, 3].map(|i| windows[i].iter().sum::<f32>());
        let mut s = Self {
            descriptor,
            values,
            sample_rate: 48_000.0,
            fft: Fft::new(MAX_N),
            windows,
            window_sums,
            history: [vec![0.0; MAX_N], vec![0.0; MAX_N]],
            write: 0,
            re: vec![0.0; MAX_N],
            im: vec![0.0; MAX_N],
            power: vec![0.0; MAX_N / 2 + 1],
            avg: [FLOOR_DB; BINS],
            out: [FLOOR_DB; BINS],
            centres: [0.0; BINS],
            edges: [0.0; BINS + 1],
            max_hz: MAX_HZ,
            hop: 1,
            since_hop: 0,
            idle: usize::MAX,
            primed: false,
            fresh: false,
        };
        s.configure(48_000.0);
        s
    }

    fn configure(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        self.max_hz = MAX_HZ.min(0.4995 * self.sample_rate).max(MIN_HZ * 2.0);
        let ratio = self.max_hz / MIN_HZ;
        let at = |x: f32| MIN_HZ * ratio.powf(x / (BINS - 1) as f32);
        for i in 0..BINS {
            self.centres[i] = at(i as f32);
        }
        for i in 0..=BINS {
            self.edges[i] = at(i as f32 - 0.5);
        }
        self.hop = (self.sample_rate / ANALYSIS_HZ as f32).round().max(1.0) as usize;
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    fn block_index(&self) -> usize {
        util::index(self.value(p::BLOCK_SIZE), BLOCK_SIZES.len())
    }

    fn channel(&self) -> Channel {
        match util::index(self.value(p::CHANNEL), 5) {
            1 => Channel::Left,
            2 => Channel::Right,
            3 => Channel::Mid,
            4 => Channel::Side,
            _ => Channel::Stereo,
        }
    }

    /// The published bins of the last computed frame (tests).
    pub fn bins(&self) -> &[f32; BINS] {
        &self.out
    }

    /// Bin centre frequencies (tests).
    pub fn centres(&self) -> &[f32; BINS] {
        &self.centres
    }

    /// RT. FFT of the latest `Block Size` samples → `self.out`.
    fn compute(&mut self) {
        let bi = self.block_index();
        let n = BLOCK_SIZES[bi];
        let channel = self.channel();
        let window = &self.windows[bi];
        let start = (self.write + MAX_N - n) % MAX_N;
        let [hl, hr] = &self.history;
        for (i, &w) in window.iter().enumerate() {
            let j = (start + i) % MAX_N;
            let (l, r) = (hl[j], hr[j]);
            let (a, b) = match channel {
                Channel::Stereo => (l, r),
                Channel::Left => (l, 0.0),
                Channel::Right => (r, 0.0),
                Channel::Mid => (0.5 * (l + r), 0.0),
                Channel::Side => (0.5 * (l - r), 0.0),
            };
            self.re[i] = a * w;
            self.im[i] = b * w;
        }
        self.fft.forward(&mut self.re[..n], &mut self.im[..n]);
        // Power normalised so a full-scale sine on a bin reads 1 (0 dB).
        let norm = {
            let g = 2.0 / self.window_sums[bi];
            g * g
        };
        let half = n / 2;
        for k in 0..=half {
            let nk = (n - k) % n;
            let (zr, zi, wr, wi) = (self.re[k], self.im[k], self.re[nk], self.im[nk]);
            let pw = if channel == Channel::Stereo {
                // Unpack the two real spectra of z = l + i·r.
                let (lr, li) = (0.5 * (zr + wr), 0.5 * (zi - wi));
                let (rr, ri) = (0.5 * (zi + wi), -0.5 * (zr - wr));
                0.5 * (lr * lr + li * li + rr * rr + ri * ri)
            } else {
                zr * zr + zi * zi
            };
            self.power[k] = pw * norm;
        }
        let df = self.sample_rate / n as f32;
        let averaging = (self.value(p::AVERAGING) as f32 * 0.01).clamp(0.0, 1.0);
        let keep = if self.primed { 0.95 * averaging } else { 0.0 };
        let slope = self.value(p::SLOPE) as f32;
        for i in 0..BINS {
            let lo = self.edges[i] / df;
            let hi = (self.edges[i + 1] / df).min(half as f32);
            let (k0, k1) = (lo.ceil() as usize, hi.floor() as usize);
            let pw = if k1 >= k0 {
                let mut m = 0.0f32;
                for &v in &self.power[k0..=k1.min(half)] {
                    m = m.max(v);
                }
                m
            } else {
                let c = (self.centres[i] / df).min(half as f32);
                let k = (c.floor() as usize).min(half.saturating_sub(1));
                let t = c - k as f32;
                self.power[k] * (1.0 - t) + self.power[k + 1] * t
            };
            let db = (10.0 * (pw + 1e-30).log10()).max(FLOOR_DB);
            let a = keep * self.avg[i] + (1.0 - keep) * db;
            self.avg[i] = if a.is_finite() { a } else { FLOOR_DB };
            let tilt = slope * (self.centres[i] / PIVOT_HZ).log2();
            self.out[i] = (self.avg[i] + tilt).max(FLOOR_DB);
        }
        self.primed = true;
        self.fresh = true;
    }

    fn set(&mut self, id: ParamId, value: f64) {
        if let Some(info) = self.descriptor.params.iter().find(|i| i.id == id) {
            let v = util::clamp(value, info.min, info.max);
            self.values[id.0 as usize] = v;
        }
    }
}

impl Node for SpectrumAnalyzer {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.configure(config.sample_rate);
        self.reset();
    }

    fn reset(&mut self) {
        for h in &mut self.history {
            h.fill(0.0);
        }
        self.write = 0;
        self.since_hop = 0;
        self.primed = false;
        self.avg = [FLOOR_DB; BINS];
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        // Only analysis settings: no audio effect, so they apply at the next frame.
        for e in ctx.events {
            if let EventKind::Param { param, value } = e.kind {
                self.set(param, value);
            }
        }
        let frames = ctx.frames;
        for (ch, out) in audio.outputs.iter_mut().enumerate() {
            match audio.inputs.get(ch).or(audio.inputs.first()) {
                Some(input) => out[..frames].copy_from_slice(&input[..frames]),
                None => out[..frames].fill(0.0),
            }
        }
        for (ch, h) in self.history.iter_mut().enumerate() {
            let input = audio.inputs.get(ch).or(audio.inputs.first());
            let mut w = self.write;
            for i in 0..frames {
                let x = input.map_or(0.0, |s| s[i]);
                h[w] = if x.is_finite() { x } else { 0.0 };
                w = (w + 1) % MAX_N;
            }
        }
        self.write = (self.write + frames) % MAX_N;
        self.since_hop += frames;
        self.idle = self.idle.saturating_add(frames);
        let watched = (self.idle as f32) < IDLE_SECONDS * self.sample_rate;
        if !watched {
            // Collection stopped: start the averaging afresh when it resumes.
            self.primed = false;
            self.fresh = false;
        }
        if watched && self.since_hop >= self.hop {
            self.since_hop = 0;
            self.compute();
        }
        ProcessStatus::Continue
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        self.idle = 0;
        if !self.fresh {
            return;
        }
        if let Some(f) = out.frame(AnalysisKind::Spectrum) {
            f.push(MIN_HZ);
            f.push(self.max_hz);
            for &v in &self.out {
                f.push(v);
            }
            self.fresh = false;
        }
    }
}

impl Device for SpectrumAnalyzer {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value);
    }
}

/// Non-RT.
pub(crate) fn create() -> SpectrumAnalyzer {
    SpectrumAnalyzer::new(super::descriptor(BuiltinDeviceType::SpectrumAnalyzer))
}
