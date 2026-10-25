//! Built-in `Shaper`: three drawn, tempo-synced cycles (`ether_model::ShaperCurves`) that
//! move the volume, a filter's cutoff and the stereo balance, each lane at its own rate.
//!
//! - **Phase**: every lane reads its curve at `beat / lane beats` (`Rate`, a sync rate
//!   from 1/64 to 8 bars). Playing, `beat` is the song position at every sample, so a
//!   lane's cycle starts on every multiple of its length from beat 0, wherever playback
//!   starts, and a bounce is sample-exact. Stopped, the lanes run on at the tempo.
//! - **Volume**: gain `1 - Depth × (1 - y)` (0 % depth: bit-exact unity).
//! - **Cutoff**: a state-variable filter (low-pass, high-pass or band-pass, `Resonance`)
//!   whose cutoff is `Bottom × (Top / Bottom)^(1 - Depth × (1 - y))`: the drawn line sweeps
//!   between the two cutoffs on a log scale, 0 % depth holds it at `Top`. Coefficients
//!   are computed every [`CONTROL`] samples from the smoothed value. Off, the filter fades out
//!   (10 ms) and then isn't run at all.
//! - **Pan**: balance `p = Depth × (2y - 1)`; the opposite side is turned down by
//!   `cos(|p| · π/2)` (no boost: centre is unity, hard left silences the right).
//! - **Smoothing**: each lane's value goes through a one-pole (`Smoothing` ms, default 1)
//!   that tames the hard edges of a drawn curve, a curve swap or a phase jump.
//! - **Mix** blends the dry input back in, **Output** is a final gain.
//! - **Curve edits** arrive as a [`Curves`] through `Node::set_data` (swapped, never
//!   allocated or freed here).
//! - Zero latency; silent in, silent out once the filter rang out.
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | group  | name          | range                                  |
//! |----|--------|---------------|----------------------------------------|
//! | 0  | Volume | `On`          | toggle, default on                     |
//! | 1  | Volume | `Rate`        | sync rates, default 1/4                |
//! | 2  | Volume | `Depth`       | 0 ..= 100 %, default 100               |
//! | 3  | Cutoff | `On`          | toggle, default off                    |
//! | 4  | Cutoff | `Rate`        | sync rates, default 1 bar              |
//! | 5  | Cutoff | `Depth`       | 0 ..= 100 %, default 100               |
//! | 6  | Cutoff | `Bottom`      | 20 ..= 20000 Hz (log), default 200     |
//! | 7  | Cutoff | `Top`         | 20 ..= 20000 Hz (log), default 12000   |
//! | 8  | Cutoff | `Resonance`   | 0 ..= 100 %, default 20                |
//! | 9  | Cutoff | `Filter`      | Low-pass / High-pass / Band-pass       |
//! | 10 | Pan    | `On`          | toggle, default off                    |
//! | 11 | Pan    | `Rate`        | sync rates, default 1/2                |
//! | 12 | Pan    | `Depth`       | 0 ..= 100 %, default 100               |
//! | 13 | Output | `Smoothing`   | 0 ..= 20 ms, default 1                 |
//! | 14 | Output | `Mix`         | 0 ..= 100 %, default 100               |
//! | 15 | Output | `Output`      | -24 ..= 12 dB, default 0               |

use ether_core::node::NodeData;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::{DeviceLayout, LayoutItem, Widget, WidgetSize};
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, ParamId, ShaperCurves, ShaperLane,
};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::contract::{
    FactoryPreset, SYNC_RATE_BEATS, SYNC_RATES, choice, descriptor as build, item, knob, layout,
    param, section, toggle,
};
use crate::dsp::svf::{Coefs, Svf};
use crate::fx_modulation::VolumeLfoCurve as CurveData;
use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const VOLUME_ON: ParamId = ParamId(0);
    pub const VOLUME_RATE: ParamId = ParamId(1);
    pub const VOLUME_DEPTH: ParamId = ParamId(2);
    pub const CUTOFF_ON: ParamId = ParamId(3);
    pub const CUTOFF_RATE: ParamId = ParamId(4);
    pub const CUTOFF_DEPTH: ParamId = ParamId(5);
    pub const LOW: ParamId = ParamId(6);
    pub const HIGH: ParamId = ParamId(7);
    pub const RESONANCE: ParamId = ParamId(8);
    pub const FILTER: ParamId = ParamId(9);
    pub const PAN_ON: ParamId = ParamId(10);
    pub const PAN_RATE: ParamId = ParamId(11);
    pub const PAN_DEPTH: ParamId = ParamId(12);
    pub const SMOOTHING: ParamId = ParamId(13);
    pub const MIX: ParamId = ParamId(14);
    pub const OUTPUT: ParamId = ParamId(15);
    /// Number of params.
    pub const COUNT: usize = 16;
}
use params as p;

/// `Filter` values.
pub mod filter {
    pub const LOW_PASS: usize = 0;
    pub const HIGH_PASS: usize = 1;
    pub const BAND_PASS: usize = 2;
}
const FILTERS: [&str; 3] = ["Low-pass", "High-pass", "Band-pass"];

/// Index of "1/4", "1/2" and "1 bar" in [`SYNC_RATES`].
const SYNC_QUARTER: usize = 8;
const SYNC_HALF: usize = 9;
const SYNC_BAR: usize = 10;
/// Samples between two filter coefficient updates.
pub const CONTROL: usize = 16;
/// Fade of the filter switching on or off (ms).
const FILTER_FADE_MS: f32 = 10.0;

/// The `(on, rate, depth)` params of each lane, in [`ShaperLane::ALL`] order.
pub const LANES: [(ParamId, ParamId, ParamId); 3] = [
    (p::VOLUME_ON, p::VOLUME_RATE, p::VOLUME_DEPTH),
    (p::CUTOFF_ON, p::CUTOFF_RATE, p::CUTOFF_DEPTH),
    (p::PAN_ON, p::PAN_RATE, p::PAN_DEPTH),
];

/// Descriptor of the shaper.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(ty, BuiltinDeviceType::Shaper);
    let pct = |id, name: &str, group: &str, default| {
        param(
            id,
            name,
            group,
            ParamUnit::Percent,
            (0.0, 100.0, default),
            ParamScale::Linear,
        )
    };
    let hz = |id, name: &str, default| {
        param(
            id,
            name,
            "Cutoff",
            ParamUnit::Hertz,
            (20.0, 20_000.0, default),
            ParamScale::Log,
        )
    };
    let mut d = build(
        BuiltinDeviceType::Shaper,
        "Shaper",
        DeviceCategory::AudioEffect,
        vec![
            toggle(0, "On", "Volume", true),
            choice(1, "Rate", "Volume", &SYNC_RATES, SYNC_QUARTER),
            pct(2, "Depth", "Volume", 100.0),
            toggle(3, "On", "Cutoff", false),
            choice(4, "Rate", "Cutoff", &SYNC_RATES, SYNC_BAR),
            pct(5, "Depth", "Cutoff", 100.0),
            hz(6, "Bottom", 200.0),
            hz(7, "Top", 12_000.0),
            pct(8, "Resonance", "Cutoff", 20.0),
            choice(9, "Filter", "Cutoff", &FILTERS, filter::LOW_PASS),
            toggle(10, "On", "Pan", false),
            choice(11, "Rate", "Pan", &SYNC_RATES, SYNC_HALF),
            pct(12, "Depth", "Pan", 100.0),
            param(
                13,
                "Smoothing",
                "Output",
                ParamUnit::Milliseconds,
                (0.0, 20.0, 1.0),
                ParamScale::Linear,
            ),
            pct(14, "Mix", "Output", 100.0),
            param(
                15,
                "Output",
                "Output",
                ParamUnit::Decibels,
                (-24.0, 12.0, 0.0),
                ParamScale::Linear,
            ),
        ],
        2,
        2,
        false,
        0,
    );
    d.layout = Some(panel());
    d
}

/// One section per lane (the curve editor across, its switches and knobs beside it), then
/// the output.
fn panel() -> DeviceLayout {
    use WidgetSize::*;
    let lane = |id: &str, title: &str, lane: ShaperLane, extra: Vec<LayoutItem>| {
        let (on, rate, depth) = LANES[lane as usize];
        let mut items = vec![
            LayoutItem {
                colspan: 4,
                ..item(
                    Widget::ShaperCurve {
                        lane,
                        on,
                        rate,
                        depth,
                    },
                    Large,
                )
            },
            item(Widget::Toggle { param: on }, Small),
            item(Widget::Choice { param: rate }, Small),
            knob(depth, Small),
        ];
        items.extend(extra);
        section(id, Some(title), 4, 4, items)
    };
    layout(vec![
        lane("volume", "Volume", ShaperLane::Volume, vec![]),
        lane(
            "cutoff",
            "Cutoff",
            ShaperLane::Cutoff,
            vec![
                item(Widget::Choice { param: p::FILTER }, Small),
                knob(p::LOW, Small),
                knob(p::HIGH, Small),
                knob(p::RESONANCE, Small),
            ],
        ),
        lane("pan", "Pan", ShaperLane::Pan, vec![]),
        section(
            "output",
            Some("Output"),
            1,
            3,
            vec![
                knob(p::SMOOTHING, Small),
                knob(p::MIX, Small),
                knob(p::OUTPUT, Small),
            ],
        ),
    ])
}

/// Non-RT. A new instance playing `device`'s curves.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    let BuiltinDevice::Shaper { curves } = device else {
        panic!("{device:?} is not a shaper");
    };
    Box::new(Shaper::new(curves))
}

/// `FactoryPreset` entries for `presets/shaper/<slug>.etherpreset` files.
macro_rules! presets {
    ($($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!("shaper/", $slug),
            json: include_str!(concat!("../presets/shaper/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::Shaper => presets!(
            "quarter-pump",
            "eighth-chop",
            "filter-rise",
            "auto-pan",
            "gated-sweep"
        ),
        _ => &[],
    }
}

/// The three curves as the node plays them (fixed arrays: no allocation on the audio
/// thread).
#[derive(Clone, Copy, Debug)]
pub struct Curves([CurveData; 3]);

impl Curves {
    /// Non-RT. The playable copy of `curves` (each sanitized).
    pub fn new(curves: &ShaperCurves) -> Self {
        Self(ShaperLane::ALL.map(|l| CurveData::new(curves.lane(l))))
    }

    /// Value of `lane` at `phase` (cycles), `0..=1`.
    #[inline]
    pub fn at(&self, lane: ShaperLane, phase: f64) -> f32 {
        self.0[lane as usize].at(phase)
    }
}

/// Non-RT. `Node::set_data` payload of a curve edit (bridges' `update_builtin`); `None` for
/// another device.
pub fn curve_data(device: &BuiltinDevice) -> Option<NodeData> {
    match device {
        BuiltinDevice::Shaper { curves } => Some(Box::new(Curves::new(curves))),
        _ => None,
    }
}

/// Whether a live node built from `old` takes `new` in place (`Node::set_data` with
/// [`curve_data`]): only the curves of a shaper changed.
pub fn updatable_in_place(old: &BuiltinDevice, new: &BuiltinDevice) -> bool {
    matches!(
        (old, new),
        (BuiltinDevice::Shaper { curves: a }, BuiltinDevice::Shaper { curves: b }) if a != b
    )
}

/// The built-in shaper.
pub struct Shaper {
    values: [f64; p::COUNT],
    ranges: [(f64, f64); p::COUNT],
    sample_rate: f32,
    curves: Curves,
    /// Free-running beat clock (stopped transport).
    beat: f64,
    /// Smoothed lane values (`y` after depth), and their one-pole coefficient.
    lanes: [f32; 3],
    coef: f32,
    depth: [Smoother; 3],
    /// Lane on/off fades (gain and pan glide to neutral, the filter fades out).
    on: [Smoother; 3],
    mix: Smoother,
    output: Smoother,
    svf: [Svf; 2],
    /// The filter's coefficients, recomputed every [`CONTROL`] samples (from the smoothed
    /// lane value, so they move in small steps).
    coefs: Coefs,
    control: usize,
    tail: util::Tail,
}

impl Shaper {
    /// Non-RT.
    pub fn new(curves: &ShaperCurves) -> Self {
        let d = descriptor(BuiltinDeviceType::Shaper);
        let mut values = [0.0; p::COUNT];
        let mut ranges = [(0.0, 0.0); p::COUNT];
        for info in &d.params {
            values[info.id.0 as usize] = info.default;
            ranges[info.id.0 as usize] = (info.min, info.max);
        }
        let mut s = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            curves: Curves::new(curves),
            beat: 0.0,
            lanes: [1.0, 1.0, 0.5],
            coef: 0.0,
            depth: std::array::from_fn(|_| Smoother::new(1.0, 20.0, 48_000.0)),
            on: std::array::from_fn(|_| Smoother::new(0.0, FILTER_FADE_MS, 48_000.0)),
            mix: Smoother::new(1.0, 20.0, 48_000.0),
            output: Smoother::new(1.0, 20.0, 48_000.0),
            svf: [Svf::default(), Svf::default()],
            coefs: Coefs::BYPASS,
            control: 0,
            tail: util::Tail::default(),
        };
        s.prepare_smoothers();
        s
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn on(&self, id: ParamId) -> bool {
        self.values[id.0 as usize] >= 0.5
    }

    fn prepare_smoothers(&mut self) {
        let sr = self.sample_rate;
        for (k, &(on, _, depth)) in LANES.iter().enumerate() {
            self.depth[k] = Smoother::new(self.value(depth) * 0.01, 20.0, sr);
            self.on[k] = Smoother::new(f32::from(u8::from(self.on(on))), FILTER_FADE_MS, sr);
        }
        self.mix = Smoother::new(self.value(p::MIX) * 0.01, 20.0, sr);
        self.output = Smoother::new(util::db_to_amp(self.value(p::OUTPUT)), 20.0, sr);
        self.update_coef();
    }

    fn update_coef(&mut self) {
        let ms = self.value(p::SMOOTHING);
        self.coef = if ms <= 0.0 {
            0.0
        } else {
            util::tau_coef(ms, self.sample_rate)
        };
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        let v = self.value(id);
        let set = |s: &mut Smoother, target: f32| {
            if smooth {
                s.set_target(target);
            } else {
                s.set_immediate(target);
            }
        };
        if let Some(k) = LANES.iter().position(|l| l.0 == id) {
            set(&mut self.on[k], if v >= 0.5 { 1.0 } else { 0.0 });
        } else if let Some(k) = LANES.iter().position(|l| l.2 == id) {
            set(&mut self.depth[k], v * 0.01);
        } else {
            match id {
                p::MIX => set(&mut self.mix, v * 0.01),
                p::OUTPUT => set(&mut self.output, util::db_to_amp(v)),
                p::SMOOTHING => self.update_coef(),
                _ => {}
            }
        }
    }

    /// Cycle length in beats of `lane`.
    fn beats(&self, lane: usize) -> f64 {
        let i = util::index(self.values[LANES[lane].1.0 as usize], SYNC_RATE_BEATS.len());
        SYNC_RATE_BEATS[i]
    }

    /// The filter's coefficients for the (smoothed) cutoff lane value `y`.
    fn filter_coefs(&self, y: f32) -> Coefs {
        let sr = f64::from(self.sample_rate);
        let lo = f64::from(self.value(p::LOW));
        let hi = f64::from(self.value(p::HIGH));
        let f = (lo * (hi / lo).powf(f64::from(y))).clamp(10.0, sr * 0.49);
        let g = (std::f64::consts::PI * f / sr).tan();
        // Resonance 0..100 % → Q 0.5..12 (log).
        let q = 0.5 * 24f64.powf(f64::from(self.value(p::RESONANCE)) * 0.01);
        let k = 1.0 / q;
        let (m0, m1, m2) = match util::index(f64::from(self.value(p::FILTER)), FILTERS.len()) {
            filter::HIGH_PASS => (1.0, -k, -1.0),
            filter::BAND_PASS => (0.0, k, 0.0),
            _ => (0.0, 0.0, 1.0),
        };
        Coefs::from_parts(g, k, m0, m1, m2)
    }

    /// The lane values `y` at `beat` (after depth, before smoothing), `[volume, cutoff, pan]`.
    fn targets(&self, beat: f64, depth: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|l| self.target(l, beat, depth[l]))
    }

    /// The value of lane `l` at `beat` (after depth, before smoothing).
    #[inline]
    fn target(&self, l: usize, beat: f64, depth: f32) -> f32 {
        let y = self.curves.at(ShaperLane::ALL[l], beat / self.beats(l));
        if l == 2 {
            0.5 + depth * (y - 0.5)
        } else {
            1.0 - depth * (1.0 - y)
        }
    }

    crate::dsp::simd::avx2_dispatch! {
        fn render => render_avx2, render_generic(
            &mut self,
            ctx: &ProcessContext<'_>,
            audio: &mut AudioBuffers<'_, '_>,
            start: usize,
            end: usize,
        )
    }

    #[inline(always)]
    fn render_generic(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        start: usize,
        end: usize,
    ) {
        let t = ctx.transport;
        let locked = t.playing && t.beats_per_sample > 0.0;
        let bpm = if t.bpm > 0.0 { t.bpm } else { 120.0 };
        let free_inc = bpm / 60.0 / f64::from(self.sample_rate);
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| {
            inputs
                .get(c)
                .or_else(|| inputs.first())
                .map_or(0.0, |ch| ch[i])
        };
        let k = self.coef;
        for i in start..end {
            self.beat = if locked {
                t.position + t.beats_per_sample * i as f64
            } else {
                self.beat + free_inc
            };
            let depth = [0, 1, 2].map(|l| self.depth[l].tick());
            let on = [0, 1, 2].map(|l| self.on[l].tick());
            // Lanes that are off (and done fading) aren't evaluated.
            for l in 0..3 {
                if on[l] > 0.0 || self.on[l].is_smoothing() {
                    let tg = self.target(l, self.beat, depth[l]);
                    let v = &mut self.lanes[l];
                    *v = crate::dsp::flush32(tg + (*v - tg) * k);
                }
            }
            let [vol, cut, pan] = self.lanes;
            // Off lanes glide to neutral (unity gain, centre).
            let gain = 1.0 + on[0] * (vol - 1.0);
            let p = on[2] * (2.0 * pan - 1.0);
            let side = (p.abs() * std::f32::consts::FRAC_PI_2).cos();
            let (gl, gr) = if p > 0.0 { (side, 1.0) } else { (1.0, side) };
            let mix = self.mix.tick();
            let out_gain = self.output.tick();
            let x = [input(0, i), input(1, i)];
            let mut wet = x;
            if on[1] > 0.0 || self.on[1].is_smoothing() {
                if self.control == 0 {
                    self.coefs = self.filter_coefs(cut);
                    self.control = CONTROL;
                }
                self.control -= 1;
                let c = self.coefs;
                let fade = f64::from(on[1]);
                for (w, svf) in wet.iter_mut().zip(&mut self.svf) {
                    let y = svf.tick(f64::from(*w), &c);
                    *w = (f64::from(*w) + (y - f64::from(*w)) * fade) as f32;
                }
            }
            let wet = [wet[0] * gain * gl, wet[1] * gain * gr];
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = if c < 2 {
                    crate::dsp::flush32((x[c] + (wet[c] - x[c]) * mix) * out_gain)
                } else {
                    0.0
                };
            }
        }
    }
}

impl Node for Shaper {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.prepare_smoothers();
        self.reset();
    }

    fn reset(&mut self) {
        for s in &mut self.svf {
            s.reset();
        }
        let depth = [0, 1, 2].map(|l| self.depth[l].current());
        self.lanes = self.targets(self.beat, depth);
        self.coefs = self.filter_coefs(self.lanes[1]);
        self.control = 0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let c: &ProcessContext<'_> = ctx;
        // Asleep for a while: the free clock goes on as if it had run.
        let skipped = self.tail.skipped(c);
        if skipped > 0 {
            let bpm = if c.transport.bpm > 0.0 {
                c.transport.bpm
            } else {
                120.0
            };
            self.beat += bpm / 60.0 / f64::from(self.sample_rate) * skipped as f64;
            // The smoothed lanes are where they would have been (as if it had run).
            let t = c.transport;
            let now = if t.playing && t.beats_per_sample > 0.0 {
                t.position - t.beats_per_sample
            } else {
                self.beat
            };
            let depth = [0, 1, 2].map(|l| self.depth[l].current());
            self.lanes = self.targets(now, depth);
        }
        util::split_at_events(
            self,
            c.events,
            c.frames,
            |s, a, b| s.render(c, audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply(param, value, !step);
                }
            },
        );
        let settled = self.depth.iter().chain(&self.on).all(|s| !s.is_smoothing())
            && !self.mix.is_smoothing()
            && !self.output.is_smoothing();
        // The filter rings for a while after the input stops (a resonant low cutoff).
        let hold = if self.on(p::CUTOFF_ON) {
            util::samples(0.5, self.sample_rate)
        } else {
            1
        };
        self.tail.status_of(c, audio, settled, hold)
    }

    fn set_data(&mut self, data: NodeData) -> Option<NodeData> {
        match data.downcast::<Curves>() {
            Ok(mut new) => {
                // The old curves go back in the box (dropped off the audio thread).
                std::mem::swap(&mut self.curves, &mut *new);
                Some(new)
            }
            Err(other) => Some(other),
        }
    }
}

impl Device for Shaper {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(BuiltinDeviceType::Shaper)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}
