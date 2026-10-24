//! `Loudness`: a loudness meter (EBU R 128 / ITU-R BS.1770-4). Audio passes through
//! untouched; the device measures momentary (400 ms), short-term (3 s) and gated integrated
//! loudness, the loudness range and the true peak ([`crate::dsp::loudness`],
//! [`crate::dsp::true_peak`]), and keeps a 15-minute history of the momentary and
//! short-term values (one point per 100 ms).
//!
//! - It measures all the time, watched or not, so the integrated value covers everything
//!   that played through it since the last reset. With `Reset On Play` (on by default) the
//!   transport starting resets it, so a playback from the top measures that playback.
//! - `Target` only draws the reference line on the history (and colours the readouts); the
//!   audio is never changed.
//! - Analysis: `AnalysisKind::Levels` frames, see [`Frame`] for the layout.
//! - Silence: once the 3 s window holds only silence and the input is silent the node
//!   reports `Silent` (the engine may then skip it; the history pauses meanwhile).
//!
//! Real-time: the meter, history and detectors are allocated in `new` / `prepare`.

use ether_core::analysis::{AnalysisKind, AnalysisSink};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::loudness as p;
use crate::dsp::loudness::{FLOOR, Meter};
use crate::dsp::true_peak::TruePeak;
use crate::util;

/// History length in 100 ms points (15 minutes).
pub const HISTORY: usize = 9000;
/// Most history points per analysis frame (longer windows are bucketed, max per bucket).
pub const FRAME_POINTS: usize = 300;
/// The `History` choices, in 100 ms points.
pub const WINDOWS: [usize; 5] = [100, 300, 600, 3000, 9000];

/// Layout of the `Levels` frame (indices into its values).
pub struct Frame;

impl Frame {
    pub const MOMENTARY: usize = 0;
    pub const SHORT_TERM: usize = 1;
    pub const INTEGRATED: usize = 2;
    /// Loudness range (LU).
    pub const RANGE: usize = 3;
    /// Highest true peak since the reset (dBTP).
    pub const TRUE_PEAK_MAX: usize = 4;
    /// Highest true peak since the previous frame (dBTP).
    pub const TRUE_PEAK: usize = 5;
    /// The shown history window (seconds).
    pub const WINDOW: usize = 6;
    /// Seconds per history point.
    pub const STEP: usize = 7;
    /// `n`, the number of history points that follow: `n` momentary values (oldest
    /// first), then `n` short-term values; the last point is now.
    pub const POINTS: usize = 8;
    pub const HISTORY: usize = 9;
}

fn to_db(amp: f32) -> f32 {
    if amp <= 1e-6 {
        FLOOR
    } else {
        (20.0 * amp.log10()).max(FLOOR)
    }
}

pub struct Loudness {
    descriptor: DeviceDescriptor,
    values: [f64; p::COUNT],
    sample_rate: f32,
    meter: Meter,
    peaks: [TruePeak; 2],
    peak_max: f32,
    peak_now: f32,
    /// Rings of momentary / short-term points; `len` of them are valid (saturating).
    momentary: Vec<f32>,
    short_term: Vec<f32>,
    at: usize,
    len: usize,
    was_playing: bool,
}

impl Loudness {
    /// Non-RT.
    pub fn new(descriptor: DeviceDescriptor) -> Self {
        let mut values = [0.0; p::COUNT];
        for info in &descriptor.params {
            values[info.id.0 as usize] = info.default;
        }
        Self {
            descriptor,
            values,
            sample_rate: 48_000.0,
            meter: Meter::new(48_000.0),
            peaks: [TruePeak::new(48_000.0), TruePeak::new(48_000.0)],
            peak_max: 0.0,
            peak_now: 0.0,
            momentary: vec![FLOOR; HISTORY],
            short_term: vec![FLOOR; HISTORY],
            at: 0,
            len: 0,
            was_playing: false,
        }
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    fn set(&mut self, id: ParamId, value: f64) {
        let i = id.0 as usize;
        if let Some(info) = self.descriptor.params.iter().find(|q| q.id == id) {
            self.values[i] = util::clamp(value, info.min, info.max);
        }
    }

    /// RT. Forget every measurement.
    fn clear(&mut self) {
        self.meter.reset();
        for d in &mut self.peaks {
            d.reset();
        }
        self.peak_max = 0.0;
        self.peak_now = 0.0;
        self.at = 0;
        self.len = 0;
    }

    /// The meter (tests).
    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    /// Highest true peak since the reset (dBTP).
    pub fn true_peak_max(&self) -> f32 {
        to_db(self.peak_max)
    }
}

impl Node for Loudness {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.meter = Meter::new(self.sample_rate);
        self.peaks = [
            TruePeak::new(self.sample_rate),
            TruePeak::new(self.sample_rate),
        ];
        self.clear();
    }

    fn reset(&mut self) {
        // A transport jump is not a new measurement: only the detectors' state goes.
        for d in &mut self.peaks {
            d.reset();
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            if let EventKind::Param { param, value, .. } = e.kind {
                self.set(param, value);
            }
        }
        let playing = ctx.transport.playing;
        if playing && !self.was_playing && self.value(p::RESET_ON_PLAY) >= 0.5 {
            self.clear();
        }
        self.was_playing = playing;
        let frames = ctx.frames;
        let left = audio.inputs.first().copied();
        let right = audio.inputs.get(1).copied();
        for i in 0..frames {
            let l = left.map_or(0.0, |s| s[i]);
            let r = right.map(|s| s[i]);
            let mut peak = self.peaks[0].push(l);
            if let Some(r) = r {
                peak = peak.max(self.peaks[1].push(r));
            }
            self.peak_now = self.peak_now.max(peak);
            if let Some(reading) = self.meter.push(l, r) {
                self.momentary[self.at] = reading.momentary;
                self.short_term[self.at] = reading.short_term;
                self.at = (self.at + 1) % HISTORY;
                self.len = (self.len + 1).min(HISTORY);
            }
        }
        self.peak_max = self.peak_max.max(self.peak_now);
        for (ch, out) in audio.outputs.iter_mut().enumerate() {
            match audio.inputs.get(ch).or(audio.inputs.first()) {
                Some(s) => out[..frames].copy_from_slice(&s[..frames]),
                None => out[..frames].fill(0.0),
            }
        }
        if util::all_zero(audio.inputs, frames) && self.meter.is_quiet() {
            ProcessStatus::Silent
        } else {
            ProcessStatus::Continue
        }
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        let Some(f) = out.frame(AnalysisKind::Levels) else {
            return;
        };
        let window = WINDOWS[util::index(self.value(p::HISTORY), WINDOWS.len())];
        let bucket = window.div_ceil(FRAME_POINTS);
        let shown = self.len.min(window);
        let n = shown.div_ceil(bucket);
        for v in [
            self.meter.momentary(),
            self.meter.short_term(),
            self.meter.integrated(),
            self.meter.range(),
            to_db(self.peak_max),
            to_db(self.peak_now),
            window as f32 * 0.1,
            bucket as f32 * 0.1,
            n as f32,
        ] {
            f.push(v);
        }
        self.peak_now = 0.0;
        // Buckets end at the newest point; the oldest one may be partial.
        for ring in [&self.momentary, &self.short_term] {
            for b in (0..n).rev() {
                let hi = b * bucket;
                let lo = ((b + 1) * bucket).min(shown);
                let mut m = FLOOR;
                for k in hi..lo {
                    m = m.max(ring[(self.at + HISTORY - 1 - k) % HISTORY]);
                }
                f.push(m);
            }
        }
    }
}

impl Device for Loudness {
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
pub(crate) fn create() -> Loudness {
    Loudness::new(super::descriptor(BuiltinDeviceType::Loudness))
}
