//! Grain Delay: a delay whose echoes are chopped into short overlapping grains that can be
//! transposed, scattered in time (spray) and randomly detuned, with feedback (in the spirit
//! of Live's Grain Delay).
//!
//! The input (plus the fed-back output) goes into a delay line. A new grain starts every
//! `1 / Frequency` seconds and lasts two such periods under a raised-cosine (Hann) window,
//! so two grains overlap at any time and their windows sum to one (no level change). Each
//! grain reads the line from `Time` (or the synced division) plus a random `Spray` behind
//! the write head, at speed `2^(Pitch / 12)` (plus a random `Random Pitch` detune, up to
//! ±12 semitones at 100 %): a grain played faster than the line fills rises in pitch, a
//! slower one falls. A rising grain would catch up with the write head, so it starts far
//! enough back to stay behind it for its whole length. Spray is drawn per channel (a wider
//! image); the pitch of a grain is shared by both channels.
//!
//! - **Frequency** (1 ..= 150 Hz) sets the grain rate and so the grain size: low
//!   frequencies give long, smooth grains (a crude pitch shifter), high ones a buzzy,
//!   granular texture.
//! - **Feedback** sends the wet output back into the line (soft-clipped), so transposed
//!   echoes keep transposing (cascading arpeggios at ±12, ±7 ...).
//! - Reads use 4-point Hermite interpolation. The line (4 s) is allocated in `prepare`.
//!   No latency is reported (it is a delay).
//!
//! # Params (`BuiltinDeviceType::GrainDelay`, stable and append-only)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Delay | `Sync` | toggle, default off |
//! | 1 | Delay | `Time` | 1 ..= 1000 Milliseconds (log), default 100 |
//! | 2 | Delay | `Sync Rate` | sync rate (default 1/8) |
//! | 3 | Delay | `Feedback` | 0 ..= 95 Percent, default 0 |
//! | 4 | Grains | `Spray` | 0 ..= 500 Milliseconds, default 10 |
//! | 5 | Grains | `Frequency` | 1 ..= 150 Hertz (log), default 60 |
//! | 6 | Grains | `Pitch` | -36 ..= 12 Semitones, default 0 |
//! | 7 | Grains | `Random Pitch` | 0 ..= 100 Percent, default 0 |
//! | 8 | Output | `Mix` | 0 ..= 100 Percent, default 50 |

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::layout::{DeviceLayout, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::contract::{
    FactoryPreset, SYNC_EIGHTH, SYNC_RATE_BEATS, SYNC_RATES, choice, descriptor as build, item,
    knob, layout, param, section, toggle,
};
use crate::util::{self, split_at_events};

/// Param ids (stable, append-only).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const SYNC: ParamId = ParamId(0);
    pub const TIME: ParamId = ParamId(1);
    pub const SYNC_RATE: ParamId = ParamId(2);
    pub const FEEDBACK: ParamId = ParamId(3);
    pub const SPRAY: ParamId = ParamId(4);
    pub const FREQUENCY: ParamId = ParamId(5);
    pub const PITCH: ParamId = ParamId(6);
    pub const RANDOM_PITCH: ParamId = ParamId(7);
    pub const MIX: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}
use params as ids;

/// Delay line length (s).
pub const LINE_SECONDS: f32 = 4.0;
/// `Random Pitch` at 100 %: up to this many semitones either way.
pub const RANDOM_PITCH_SEMITONES: f32 = 12.0;
/// Grain slots (two overlap in steady state; more absorb a sudden rate change).
const SLOTS: usize = 4;
/// Samples kept between a read and the write head (the Hermite read's look-ahead).
const GUARD: f64 = 3.0;

/// Descriptor of the grain delay.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(ty, BuiltinDeviceType::GrainDelay);
    let mut d = build(
        BuiltinDeviceType::GrainDelay,
        "Grain Delay",
        DeviceCategory::AudioEffect,
        vec![
            toggle(0, "Sync", "Delay", false),
            param(
                1,
                "Time",
                "Delay",
                ParamUnit::Milliseconds,
                (1.0, 1000.0, 100.0),
                ParamScale::Log,
            ),
            choice(2, "Sync Rate", "Delay", &SYNC_RATES, SYNC_EIGHTH),
            param(
                3,
                "Feedback",
                "Delay",
                ParamUnit::Percent,
                (0.0, 95.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                4,
                "Spray",
                "Grains",
                ParamUnit::Milliseconds,
                (0.0, 500.0, 10.0),
                ParamScale::Linear,
            ),
            param(
                5,
                "Frequency",
                "Grains",
                ParamUnit::Hertz,
                (1.0, 150.0, 60.0),
                ParamScale::Log,
            ),
            // Whole semitones on the knob, like every transpose (the grains play any
            // ratio: automation between steps glides).
            ParamInfo {
                step: Some(1.0),
                ..param(
                    6,
                    "Pitch",
                    "Grains",
                    ParamUnit::Semitones,
                    (-36.0, 12.0, 0.0),
                    ParamScale::Linear,
                )
            },
            param(
                7,
                "Random Pitch",
                "Grains",
                ParamUnit::Percent,
                (0.0, 100.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                8,
                "Mix",
                "Output",
                ParamUnit::Percent,
                (0.0, 100.0, 50.0),
                ParamScale::Linear,
            ),
        ],
        2,
        2,
        false,
        0,
    );
    d.layout = Some(device_layout());
    d
}

/// The panel: an xy pad of grain frequency × pitch (like Live's), the grain knobs, the
/// delay controls, the mix.
fn device_layout() -> DeviceLayout {
    use WidgetSize::*;
    let mut pad = item(
        Widget::XyPad {
            x: ids::FREQUENCY,
            y: ids::PITCH,
        },
        Large,
    );
    pad.colspan = 2;
    layout(vec![
        section(
            "grains",
            Some("Grains"),
            2,
            2,
            vec![
                pad,
                knob(ids::SPRAY, Medium),
                knob(ids::RANDOM_PITCH, Medium),
            ],
        ),
        section(
            "delay",
            Some("Delay"),
            1,
            2,
            vec![
                knob(ids::TIME, Medium),
                knob(ids::FEEDBACK, Medium),
                item(Widget::Toggle { param: ids::SYNC }, Small),
                item(
                    Widget::Choice {
                        param: ids::SYNC_RATE,
                    },
                    Small,
                ),
            ],
        ),
        section("output", Some("Output"), 1, 1, vec![knob(ids::MIX, Large)]),
    ])
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    assert_eq!(device.device_type(), BuiltinDeviceType::GrainDelay);
    Box::new(GrainDelay::new())
}

/// `FactoryPreset` entries for `presets/grain-delay/<slug>.etherpreset` files.
macro_rules! presets {
    ($($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!("grain-delay/", $slug),
            json: include_str!(concat!("../presets/grain-delay/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::GrainDelay => presets!(
            "octave-up-echo",
            "shimmer-cascade",
            "granular-cloud",
            "detuned-slapback",
            "sub-drop"
        ),
        _ => &[],
    }
}

/// Plain values of the params, clamped (NaN = default).
struct Values {
    v: [f64; ids::COUNT],
    range: [(f64, f64, f64); ids::COUNT],
}

impl Values {
    fn new() -> Self {
        let d = descriptor(BuiltinDeviceType::GrainDelay);
        let mut s = Self {
            v: [0.0; ids::COUNT],
            range: [(0.0, 0.0, 0.0); ids::COUNT],
        };
        for (i, p) in d.params.iter().enumerate() {
            assert_eq!(p.id, ParamId(i as u32));
            s.v[i] = p.default;
            s.range[i] = (p.min, p.max, p.default);
        }
        s
    }

    fn set(&mut self, id: ParamId, value: f64) -> bool {
        let i = id.0 as usize;
        let Some(&(lo, hi, def)) = self.range.get(i) else {
            return false;
        };
        self.v[i] = if value.is_nan() {
            def
        } else {
            value.clamp(lo, hi)
        };
        true
    }

    fn f(&self, id: ParamId) -> f64 {
        self.v[id.0 as usize]
    }
}

/// xorshift32: deterministic per instance, RT-safe. Uniform in `[0, 1)`.
#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    #[inline]
    fn next(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        f64::from(x >> 8) / f64::from(1u32 << 24)
    }
}

/// One grain: its read delays (per channel, samples), speed, age and length.
#[derive(Clone, Copy, Default)]
struct Grain {
    delay: [f64; 2],
    /// `1 - speed`: how much the delay grows per sample.
    drift: f64,
    age: u32,
    len: u32,
}

impl Grain {
    fn active(&self) -> bool {
        self.age < self.len
    }
}

/// The grain delay device.
pub struct GrainDelay {
    values: Values,
    sample_rate: f32,
    lines: [Vec<f32>; 2],
    write: usize,
    grains: [Grain; SLOTS],
    /// Samples until the next grain starts.
    countdown: f64,
    rng: Rng,
    feedback: Smoother,
    mix: Smoother,
    /// Silence reporting.
    tail: util::Tail,
}

impl Default for GrainDelay {
    fn default() -> Self {
        Self::new()
    }
}

impl GrainDelay {
    /// Non-RT. A grain delay with default params (line allocated for 48 kHz).
    pub fn new() -> Self {
        let mut s = Self {
            values: Values::new(),
            sample_rate: 48_000.0,
            lines: [Vec::new(), Vec::new()],
            write: 0,
            grains: [Grain::default(); SLOTS],
            countdown: 0.0,
            rng: Rng(0x6EA1_DE1A),
            feedback: Smoother::new(0.0, 0.0, 48_000.0),
            mix: Smoother::new(0.5, 0.0, 48_000.0),
            tail: util::Tail::default(),
        };
        s.allocate();
        s.sync_all();
        s
    }

    fn allocate(&mut self) {
        let n = (LINE_SECONDS * self.sample_rate).ceil() as usize + 8;
        for l in &mut self.lines {
            if l.len() != n {
                *l = vec![0.0; n];
            }
        }
        self.write = 0;
    }

    fn sync_all(&mut self) {
        for i in 0..ids::COUNT {
            self.apply(ParamId(i as u32), false);
        }
    }

    fn apply(&mut self, id: ParamId, smooth: bool) {
        let ramp = |s: &mut Smoother, v: f32| {
            if smooth {
                s.ramp_param(v, ether_core::automation_rt::PARAM_GRID as u32);
            } else {
                s.set_immediate(v);
            }
        };
        match id {
            ids::FEEDBACK => ramp(&mut self.feedback, (self.values.f(id) * 0.01) as f32),
            ids::MIX => ramp(&mut self.mix, (self.values.f(id) * 0.01) as f32),
            // The others are read when a grain starts.
            _ => {}
        }
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.values.set(id, value) {
            self.apply(id, smooth);
        }
    }

    /// Base delay (samples) for the current params and tempo.
    fn base_delay(&self, bpm: f64) -> f64 {
        let sr = f64::from(self.sample_rate);
        let seconds = if self.values.f(ids::SYNC) >= 0.5 {
            let i = (self.values.f(ids::SYNC_RATE).round().max(0.0) as usize)
                .min(SYNC_RATE_BEATS.len() - 1);
            let bpm = if bpm > 0.0 { bpm } else { 120.0 };
            SYNC_RATE_BEATS[i] * 60.0 / bpm
        } else {
            self.values.f(ids::TIME) * 0.001
        };
        seconds * sr
    }

    /// Start a grain in a free slot (else the oldest).
    fn spawn(&mut self, bpm: f64) {
        let sr = f64::from(self.sample_rate);
        let period = sr / self.values.f(ids::FREQUENCY).max(1.0);
        let len = (2.0 * period).round().max(2.0);
        let rp = self.values.f(ids::RANDOM_PITCH) * 0.01 * f64::from(RANDOM_PITCH_SEMITONES);
        let semis = self.values.f(ids::PITCH) + rp * (2.0 * self.rng.next() - 1.0);
        let speed = (semis / 12.0).exp2();
        let drift = 1.0 - speed;
        let max = (self.lines[0].len() as f64) - GUARD - 2.0;
        let base = self.base_delay(bpm);
        let spray = self.values.f(ids::SPRAY) * 0.001 * sr;
        // A rising grain loses `-drift · len` samples of delay over its life.
        let min_start = GUARD + (-drift * len).max(0.0);
        let mut delay = [0.0; 2];
        for d in &mut delay {
            let start = base + spray * self.rng.next();
            // A falling grain gains `drift · len`: keep its end inside the line.
            let max_start = max - (drift * len).max(0.0);
            *d = start.max(min_start).min(max_start.max(min_start));
        }
        let slot = self
            .grains
            .iter()
            .position(|g| !g.active())
            .unwrap_or_else(|| {
                // All busy (the rate just jumped up): replace the one nearest its end.
                let mut best = 0;
                for (i, g) in self.grains.iter().enumerate() {
                    if g.len - g.age < self.grains[best].len - self.grains[best].age {
                        best = i;
                    }
                }
                best
            });
        self.grains[slot] = Grain {
            delay,
            drift,
            age: 0,
            len: len as u32,
        };
        self.countdown += period;
    }

    /// 4-point Hermite read of line `c`, `d` samples behind the last write.
    #[inline]
    fn read(&self, c: usize, d: f64) -> f32 {
        let line = &self.lines[c];
        let n = line.len();
        let i = d as usize;
        let t = (d - i as f64) as f32;
        let at = |k: usize| line[(self.write + n - (k % n)) % n];
        let (xm1, x0, x1, x2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, bpm: f64, start: usize, end: usize) {
        let n_ch = audio.outputs.len().min(2);
        let len = self.lines[0].len();
        for i in start..end {
            self.countdown -= 1.0;
            while self.countdown <= 0.0 {
                self.spawn(bpm);
            }
            // Wet: the sum of the active grains.
            let mut wet = [0.0f32; 2];
            for g in 0..SLOTS {
                let gr = self.grains[g];
                if !gr.active() {
                    continue;
                }
                let ph = gr.age as f32 / gr.len as f32;
                let w = 0.5 - 0.5 * (std::f32::consts::TAU * ph).cos();
                for (c, out) in wet.iter_mut().enumerate().take(n_ch) {
                    // The line's last write is the previous input sample: one less.
                    *out += w * self.read(c, gr.delay[c] - 1.0);
                }
                let gr = &mut self.grains[g];
                gr.age += 1;
                for d in &mut gr.delay {
                    *d += gr.drift;
                }
            }
            let fb = self.feedback.tick();
            let mix = self.mix.tick();
            self.write = (self.write + 1) % len;
            for (c, &y) in wet.iter().enumerate().take(n_ch) {
                let x = audio
                    .inputs
                    .get(c)
                    .or(audio.inputs.first())
                    .map_or(0.0, |b| b[i]);
                let into = x + fb * y.tanh();
                self.lines[c][self.write] = if into.abs() < 1e-20 { 0.0 } else { into };
                audio.outputs[c][i] = x + (y - x) * mix;
            }
        }
    }
}

impl Node for GrainDelay {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        for l in &mut self.lines {
            l.fill(0.0);
        }
        self.grains = [Grain::default(); SLOTS];
        self.countdown = 0.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let bpm = ctx.transport.bpm;
        split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, bpm, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.set(param, value, true);
                }
            },
        );
        // Whatever the line holds can come back up to its length later.
        let settled = !self.feedback.is_smoothing() && !self.mix.is_smoothing();
        let hold = self.lines[0].len() + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }
}

impl Device for GrainDelay {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(BuiltinDeviceType::GrainDelay)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.v.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value, false);
    }
}
