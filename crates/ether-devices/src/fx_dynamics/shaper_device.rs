//! Transient shaper (`BuiltinDeviceType::TransientShaper`).
//!
//! Level-independent (differential envelope) design: three stereo-linked peak followers
//! of the input level in dB:
//! - `fast`: instant attack, `Release Time` release (the signal's peak envelope);
//! - `slow`: `Attack Time` attack, `Release Time` release (lags behind onsets);
//! - `short`: instant attack, `Attack Time` release (follows the decay closely).
//!
//! `fast - slow` (>= 0) is the transient part (only right after an onset) and `fast -
//! short` (>= 0) the sustain/tail part. The gain in dB is `Attack · (fast - slow) +
//! Sustain · (fast - short)` (amounts -100..=100 % = -1..=1), clamped to ±[`MAX_GAIN_DB`],
//! so steady signals pass unchanged and the effect doesn't depend on the input level.
//! Levels below [`FLOOR_DB`] are floored so noise isn't pumped. `Clip` soft-clips the
//! output at 0 dBFS (tanh). `Mix` is a parallel blend (no latency). The applied shaping
//! gain (dB, the largest |gain| since the last read, sign kept) is published as
//! `AnalysisKind::Levels [gain dB]` for the layout meter.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus,
};

use super::shared::{GainRamp, Params};
use super::transient_shaper as p;
use crate::util::{db_to_amp, split_at_events, tau_coef};

/// Largest boost/cut of the shaping gain (dB).
pub const MAX_GAIN_DB: f32 = 18.0;
/// Detector floor (dB): quieter input is treated as this level.
const FLOOR_DB: f32 = -60.0;
/// Attack time of the "instant" followers (ms).
const INSTANT_MS: f32 = 0.1;

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
    instant: f32,
    attack_coef: f32,
    release_coef: f32,
    fast: Follower,
    slow: Follower,
    short: Follower,
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
            instant: 0.0,
            attack_coef: 0.0,
            release_coef: 0.0,
            fast: Follower::IDLE,
            slow: Follower::IDLE,
            short: Follower::IDLE,
            attack: GainRamp::new(0.0),
            sustain: GainRamp::new(0.0),
            output: GainRamp::new(1.0),
            mix: GainRamp::new(1.0),
            meter_db: 0.0,
        };
        s.sync();
        s
    }

    fn sync(&mut self) {
        self.instant = tau_coef(INSTANT_MS, self.sample_rate);
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
            let fast = self.fast.tick(x_db, self.instant, self.release_coef);
            let slow = self.slow.tick(x_db, self.attack_coef, self.release_coef);
            let short = self.short.tick(x_db, self.instant, self.attack_coef);
            let transient = (fast - slow).max(0.0);
            let tail = (fast - short).max(0.0);
            let gain_db = (self.attack.tick() * transient + self.sustain.tick() * tail)
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
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync();
        self.reset();
    }

    fn reset(&mut self) {
        self.fast = Follower::IDLE;
        self.slow = Follower::IDLE;
        self.short = Follower::IDLE;
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
