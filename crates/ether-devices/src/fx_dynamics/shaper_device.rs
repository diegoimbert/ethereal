//! Transient shaper (`BuiltinDeviceType::TransientShaper`).
//!
//! Level-independent (differential envelope) design, stereo-linked:
//! 1. `level`: the input peak held over [`HOLD_MS`] (a sliding maximum, so the envelope of
//!    anything above ~35 Hz is ripple-free), in dB, floored at [`FLOOR_DB`] so noise isn't
//!    pumped;
//! 2. `slow`: follows `level` up with the `Attack Time` constant and down instantly, so
//!    `level - slow` (>= 0) is the transient part (right after an onset only);
//! 3. `tail`: follows `level` up instantly and down with the `Release Time` constant, so
//!    `tail - level` (>= 0) is the sustain part (while a sound decays faster than that).
//!
//! The gain in dB is `Attack · (level - slow) + Sustain · (tail - level)` (amounts
//! -100..=100 % = -1..=1), clamped to ±[`MAX_GAIN_DB`]: steady signals pass unchanged and
//! the effect doesn't depend on the input level. `Clip` soft-clips the output at 0 dBFS
//! (tanh). `Mix` is a parallel blend (no latency). The applied shaping gain (dB, the largest
//! |gain| since the last read, sign kept) is published as `AnalysisKind::Levels [gain dB]`
//! for the layout meter.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus,
};

use super::shared::{GainRamp, Params};
use super::transient_shaper as p;
use crate::dsp::sliding_min::SlidingMin;
use crate::util::{db_to_amp, split_at_events, tau_coef};

/// Largest boost/cut of the shaping gain (dB).
pub const MAX_GAIN_DB: f32 = 18.0;
/// Detector floor (dB): quieter input is treated as this level.
const FLOOR_DB: f32 = -60.0;
/// Peak hold of the level detector (ms): bridges the gaps between the peaks of a
/// waveform down to ~35 Hz.
const HOLD_MS: f32 = 15.0;

/// A peak follower in dB with separate attack/release coefficients.
#[derive(Clone, Copy, Debug)]
struct Follower {
    level: f32,
}

impl Follower {
    const IDLE: Self = Self { level: FLOOR_DB };

    #[inline]
    fn tick(&mut self, x_db: f32, attack: f32, release: f32) -> f32 {
        let coef = if x_db > self.level { attack } else { release };
        self.level = x_db + (self.level - x_db) * coef;
        self.level
    }
}

/// The transient shaper.
pub struct TransientShaper {
    params: Params<{ p::COUNT }>,
    sample_rate: f32,
    attack_coef: f32,
    release_coef: f32,
    /// Sliding maximum of the input peak (as a minimum of its negation).
    hold: SlidingMin,
    slow: Follower,
    tail: Follower,
    attack: GainRamp,
    sustain: GainRamp,
    output: GainRamp,
    mix: GainRamp,
    /// Largest |gain| (dB, sign kept) since the last analysis read.
    meter_db: f32,
}

impl Default for TransientShaper {
    fn default() -> Self {
        Self::new()
    }
}

impl TransientShaper {
    /// Non-RT. A shaper with default parameters (prepared for 48 kHz).
    pub fn new() -> Self {
        let params = Params::new(&super::descriptor(BuiltinDeviceType::TransientShaper));
        let mut s = Self {
            params,
            sample_rate: 48_000.0,
            attack_coef: 0.0,
            release_coef: 0.0,
            hold: SlidingMin::new(1),
            slow: Follower::IDLE,
            tail: Follower::IDLE,
            attack: GainRamp::new(0.0),
            sustain: GainRamp::new(0.0),
            output: GainRamp::new(1.0),
            mix: GainRamp::new(1.0),
            meter_db: 0.0,
        };
        s.alloc(48_000.0);
        s
    }

    /// Non-RT: size the peak hold for `sample_rate`.
    fn alloc(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        self.hold = SlidingMin::new((HOLD_MS * 0.001 * self.sample_rate) as usize);
        self.sync();
    }

    fn sync(&mut self) {
        self.update_times();
        for id in [p::ATTACK, p::SUSTAIN, p::OUTPUT, p::MIX] {
            self.apply_param(id, self.params.get(id).into(), false);
        }
    }

    fn update_times(&mut self) {
        self.attack_coef = tau_coef(self.params.get(p::ATTACK_TIME), self.sample_rate);
        self.release_coef = tau_coef(self.params.get(p::RELEASE_TIME), self.sample_rate);
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        if !self.params.set(id, value) {
            return;
        }
        let v = self.params.get(id);
        match id {
            p::ATTACK => self.attack.set(v * 0.01, smooth),
            p::SUSTAIN => self.sustain.set(v * 0.01, smooth),
            p::ATTACK_TIME | p::RELEASE_TIME => self.update_times(),
            p::OUTPUT => self.output.set(db_to_amp(v), smooth),
            p::MIX => self.mix.set(v * 0.01, smooth),
            _ => {}
        }
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let inputs = audio.inputs;
        let clip = self.params.on(p::CLIP);
        for i in start..end {
            let peak = inputs.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()));
            let x_db = if peak > 1e-3 {
                (20.0 * peak.log10()).max(FLOOR_DB)
            } else {
                FLOOR_DB
            };
            let level = -self.hold.push(-x_db);
            let slow = self.slow.tick(level, self.attack_coef, 0.0);
            let tail = self.tail.tick(level, 0.0, self.release_coef);
            let transient = (level - slow).max(0.0);
            let sustain = (tail - level).max(0.0);
            let gain_db = (self.attack.tick() * transient + self.sustain.tick() * sustain)
                .clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
            if gain_db.abs() > self.meter_db.abs() {
                self.meter_db = gain_db;
            }
            let gain = db_to_amp(gain_db);
            let mix = self.mix.tick();
            let out_gain = self.output.tick();
            for (o, out) in audio.outputs.iter_mut().enumerate() {
                let x = inputs.get(o).or(inputs.first()).map_or(0.0, |ch| ch[i]);
                let wet = x + (x * gain - x) * mix;
                let y = wet * out_gain;
                out[i] = if clip { y.tanh() } else { y };
            }
        }
    }
}

impl Node for TransientShaper {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.alloc(config.sample_rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.hold.clear();
        self.slow = Follower::IDLE;
        self.tail = Follower::IDLE;
        self.meter_db = 0.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        if let Some(f) = out.frame(AnalysisKind::Levels) {
            f.push(self.meter_db);
        }
        self.meter_db = 0.0;
    }
}

impl Device for TransientShaper {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::TransientShaper)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
