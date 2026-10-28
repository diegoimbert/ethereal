//! Built-in `GraphicEq` (`graphic-eq`): the default EQ ("EQ" in the library), a curve
//! editor in the style of the big "draw on the spectrum" EQs. The older eight-band
//! [`crate::eq`] is the "Parametric EQ".
//!
//! [`BANDS`] band slots, all unused by default (a flat line, the input bit for bit). The UI
//! places a point by using a free slot: `Used` on, then its type, frequency, gain, Q and
//! slope. Every band is plain automatable params, so automation, undo, presets and MIDI
//! learn work as on any device.
//!
//! Types ([`GraphicEqShape`], `Type` labels): Bell, Low Shelf, Low Cut, High Shelf, High
//! Cut, Notch, Band-pass, Tilt Shelf, Flat Tilt. Cuts and the band-pass take a slope of
//! 6..96 dB/oct (Butterworth, Q = resonance); gain is ignored by the cuts, notch and
//! band-pass, Q by the flat tilt. The response of each band is
//! `ether_protocol::eq_response::graphic_eq_stages`: up to eight decramped (matched)
//! second-order stages, shared with the UI's curve so the drawn response is the device's.
//!
//! Every stage is a TPT state-variable filter ([`crate::dsp::svf`], `f64`), all bands in
//! series, then the output gain; zero latency. No zipper noise: a parameter event recomputes
//! the band's target stages and the running coefficients glide to them per sample (two
//! cascaded one-poles, ≈15 ms, frequency in octaves); a stage switching on or off glides
//! its output mix only (same frequency and damping), so `Used`, `On`, type and slope
//! changes don't click either. Settled pass-through stages are skipped.
//!
//! While watched, the input (`SpectrumPre`) and output (`Spectrum`) spectra are published
//! as by the parametric EQ ([`crate::eq::spectrum`]).
//!
//! # Parameter ids (stable, append-only)
//!
//! Band `b` (0..24) uses ids `7·b .. 7·b+6`:
//!
//! | id     | name    | range                                        |
//! |--------|---------|----------------------------------------------|
//! | `7b+0` | `Used`  | toggle, default off                          |
//! | `7b+1` | `On`    | toggle, default on                           |
//! | `7b+2` | `Type`  | Bell, Low Shelf, Low Cut, High Shelf, High Cut, Notch, Band-pass, Tilt Shelf, Flat Tilt |
//! | `7b+3` | `Freq`  | 10 ..= 30000 Hz (log), default 1000          |
//! | `7b+4` | `Gain`  | -30 ..= 30 dB, default 0                     |
//! | `7b+5` | `Q`     | 0.025 ..= 40 (log), default 1                |
//! | `7b+6` | `Slope` | 6, 12, 18, 24, 36, 48, 72, 96 dB/oct, default 12 |
//! | `168`  | `Output`| -24 ..= 24 dB, default 0                     |

use ether_core::analysis::{ANALYSIS_HZ, AnalysisSink};
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::eq_response::{
    GRAPHIC_EQ_MAX_STAGES, GRAPHIC_EQ_SLOPES, GraphicEqShape, graphic_eq_stages,
};
use ether_core::protocol::layout::{DeviceLayout, GraphicEqBandBinding, SpectrumOverlay, Widget};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessStatus, Smoother,
};

use crate::contract::{
    FactoryPreset, choice, descriptor as build, item, layout, param, section, toggle,
};
use crate::dsp::svf::{Coefs, Svf};
use crate::eq::spectrum;
use crate::util;

/// Number of band slots.
pub const BANDS: usize = 24;
/// Params per band.
pub const BAND_PARAMS: u32 = 7;
/// Stages per band (at most).
pub const STAGES: usize = GRAPHIC_EQ_MAX_STAGES;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;

    /// Offsets inside a band (`band(b, offset)`).
    pub const USED: u32 = 0;
    pub const ON: u32 = 1;
    pub const TYPE: u32 = 2;
    pub const FREQ: u32 = 3;
    pub const GAIN: u32 = 4;
    pub const Q: u32 = 5;
    pub const SLOPE: u32 = 6;

    /// Id of param `offset` of band `b` (0-based).
    pub const fn band(b: usize, offset: u32) -> ParamId {
        ParamId(b as u32 * super::BAND_PARAMS + offset)
    }

    pub const OUTPUT: ParamId = ParamId(super::BANDS as u32 * super::BAND_PARAMS);
}

/// `Type` labels (plain value `i` = `GraphicEqShape::ALL[i]`).
pub const TYPE_LABELS: [&str; 9] = [
    "Bell",
    "Low Shelf",
    "Low Cut",
    "High Shelf",
    "High Cut",
    "Notch",
    "Band-pass",
    "Tilt Shelf",
    "Flat Tilt",
];

/// `Slope` labels (plain value `i` = `GRAPHIC_EQ_SLOPES[i]` dB/oct).
pub const SLOPE_LABELS: [&str; 8] = [
    "6 dB/oct",
    "12 dB/oct",
    "18 dB/oct",
    "24 dB/oct",
    "36 dB/oct",
    "48 dB/oct",
    "72 dB/oct",
    "96 dB/oct",
];
const _: () = assert!(SLOPE_LABELS.len() == GRAPHIC_EQ_SLOPES.len());
const _: () = assert!(TYPE_LABELS.len() == GraphicEqShape::ALL.len());

/// Frequency range of the bands (Hz).
pub const FREQ_RANGE: (f64, f64) = (10.0, 30_000.0);
/// Gain range of the bands (dB).
pub const GAIN_RANGE: f64 = 30.0;
/// Q range of the bands.
pub const Q_RANGE: (f64, f64) = (0.025, 40.0);

const NUM_PARAMS: usize = BANDS * BAND_PARAMS as usize + 1;
/// Time constant of each of the two coefficient glide stages.
const GLIDE_MS: f32 = 6.0;

/// Descriptor of the `GraphicEq` type.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(ty, BuiltinDeviceType::GraphicEq);
    let mut params = Vec::with_capacity(NUM_PARAMS);
    for b in 0..BANDS {
        let group = format!("Band {}", b + 1);
        let id = |o: u32| params::band(b, o).0;
        params.push(toggle(id(params::USED), "Used", &group, false));
        params.push(toggle(id(params::ON), "On", &group, true));
        params.push(choice(id(params::TYPE), "Type", &group, &TYPE_LABELS, 0));
        params.push(param(
            id(params::FREQ),
            "Freq",
            &group,
            ParamUnit::Hertz,
            (FREQ_RANGE.0, FREQ_RANGE.1, 1000.0),
            ParamScale::Log,
        ));
        params.push(param(
            id(params::GAIN),
            "Gain",
            &group,
            ParamUnit::Decibels,
            (-GAIN_RANGE, GAIN_RANGE, 0.0),
            ParamScale::Linear,
        ));
        params.push(param(
            id(params::Q),
            "Q",
            &group,
            ParamUnit::None,
            (Q_RANGE.0, Q_RANGE.1, 1.0),
            ParamScale::Log,
        ));
        params.push(choice(id(params::SLOPE), "Slope", &group, &SLOPE_LABELS, 1));
    }
    params.push(param(
        params::OUTPUT.0,
        "Output",
        "Output",
        ParamUnit::Decibels,
        (-24.0, 24.0, 0.0),
        ParamScale::Linear,
    ));
    let mut d = build(
        BuiltinDeviceType::GraphicEq,
        "EQ",
        DeviceCategory::AudioEffect,
        params,
        2,
        2,
        false,
        0,
    );
    d.layout = Some(device_layout());
    d
}

/// The panel: the graphic editor alone (it holds every band and the output gain).
pub fn device_layout() -> DeviceLayout {
    use ether_core::protocol::layout::WidgetSize;
    let bands = (0..BANDS)
        .map(|b| GraphicEqBandBinding {
            used: params::band(b, params::USED),
            on: params::band(b, params::ON),
            kind: params::band(b, params::TYPE),
            freq: params::band(b, params::FREQ),
            gain: params::band(b, params::GAIN),
            q: params::band(b, params::Q),
            slope: params::band(b, params::SLOPE),
        })
        .collect();
    let mut editor = item(
        Widget::GraphicEq {
            bands,
            output: params::OUTPUT,
            spectrum: SpectrumOverlay::PrePost,
        },
        WidgetSize::Large,
    );
    editor.colspan = 1;
    layout(vec![section("editor", None, 4, 1, vec![editor])])
}

/// Non-RT. A new instance with default params (no band used: pass-through).
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    assert_eq!(device.device_type(), BuiltinDeviceType::GraphicEq);
    Box::new(GraphicEq::new())
}

/// `FactoryPreset` entries for `presets/graphic-eq/<slug>.etherpreset` files.
macro_rules! presets {
    ($($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!("graphic-eq/", $slug),
            json: include_str!(concat!("../presets/graphic-eq/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets.
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::GraphicEq => presets!(
            "low-cut-80-hz",
            "vocal-presence",
            "warm-master",
            "telephone"
        ),
        _ => &[],
    }
}

/// One stage's running coefficients gliding to its target.
#[derive(Clone, Copy, Debug)]
struct Stage {
    current: Coefs,
    /// First glide stage (`target` → `mid` → `current`).
    mid: Coefs,
    target: Coefs,
}

impl Stage {
    const BYPASS: Self = Self {
        current: Coefs::BYPASS,
        mid: Coefs::BYPASS,
        target: Coefs::BYPASS,
    };

    /// Retarget (`smooth` = glide, else jump). A stage turning off keeps its filter and
    /// fades its mix out; one turning on from pass-through takes the new filter at once
    /// (inaudible: its mix is the dry input) and fades its mix in.
    fn set(&mut self, target: Option<Coefs>, smooth: bool) {
        let target = match target {
            Some(t) => t,
            None => self.current.bypassed(),
        };
        if !smooth {
            *self = Self {
                current: target,
                mid: target,
                target,
            };
            return;
        }
        if self.current.is_bypass() && self.mid.is_bypass() {
            let pass = Coefs::from_parts(target.g, target.k, 1.0, 0.0, 0.0);
            self.current = pass;
            self.mid = pass;
        }
        self.target = target;
    }

    fn moving(&self) -> bool {
        self.current != self.target
    }
}

#[derive(Clone, Copy, Debug)]
struct Band {
    stages: [Stage; STAGES],
    /// Stages `..len` may be audible (the others are settled pass-through).
    len: usize,
    /// Some stage is gliding.
    moving: bool,
}

/// The built-in graphic EQ.
#[derive(Debug)]
pub struct GraphicEq {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    glide: f64,
    bands: [Band; BANDS],
    /// Per band, stage and channel: the filter states.
    states: Box<[[[Svf; 2]; STAGES]; BANDS]>,
    output: Smoother,
    pre: spectrum::Analyzer,
    post: spectrum::Analyzer,
    /// Frames since `analysis` was last called (spectra are computed only while watched).
    since_asked: usize,
    /// Frames until the next spectrum update.
    until_hop: usize,
    tail: util::Tail,
}

impl Default for GraphicEq {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphicEq {
    /// Non-RT. A graphic EQ with default parameters.
    pub fn new() -> Self {
        let infos = descriptor(BuiltinDeviceType::GraphicEq).params;
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            debug_assert_eq!(info.id.0 as usize, i);
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let mut eq = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            glide: 0.0,
            bands: [Band {
                stages: [Stage::BYPASS; STAGES],
                len: 0,
                moving: false,
            }; BANDS],
            states: Box::new([[[Svf::ZERO; 2]; STAGES]; BANDS]),
            output: Smoother::new(1.0, 20.0, 48_000.0),
            pre: spectrum::Analyzer::new(),
            post: spectrum::Analyzer::new(),
            since_asked: usize::MAX,
            until_hop: 0,
            tail: util::Tail::default(),
        };
        eq.sync_all();
        eq
    }

    fn value(&self, b: usize, offset: u32) -> f64 {
        self.values[params::band(b, offset).0 as usize]
    }

    fn sync_all(&mut self) {
        self.glide = util::tau_coef(GLIDE_MS, self.sample_rate) as f64;
        self.output = Smoother::new(
            util::db_to_amp(self.values[params::OUTPUT.0 as usize] as f32),
            20.0,
            self.sample_rate,
        );
        for b in 0..BANDS {
            self.update_band(b, false);
        }
    }

    /// The stages band `b` should run now (`None` past its count, or when unused / off).
    fn band_targets(&self, b: usize) -> [Option<Coefs>; STAGES] {
        let mut out = [None; STAGES];
        if self.value(b, params::USED) < 0.5 || self.value(b, params::ON) < 0.5 {
            return out;
        }
        let shape = GraphicEqShape::ALL
            [util::index(self.value(b, params::TYPE), GraphicEqShape::ALL.len())];
        let slope = util::index(self.value(b, params::SLOPE), GRAPHIC_EQ_SLOPES.len());
        let st = graphic_eq_stages(
            shape,
            self.value(b, params::FREQ),
            self.value(b, params::GAIN),
            self.value(b, params::Q),
            slope,
            self.sample_rate as f64,
        );
        for (o, c) in out.iter_mut().zip(st.active()) {
            *o = Some(Coefs::from_parts(c.g, c.k, c.m0, c.m1, c.m2));
        }
        out
    }

    fn update_band(&mut self, b: usize, smooth: bool) {
        let targets = self.band_targets(b);
        let band = &mut self.bands[b];
        for (s, t) in band.stages.iter_mut().zip(targets) {
            s.set(t, smooth);
        }
        band.moving = band.stages.iter().any(Stage::moving);
        band.len = live_len(&band.stages);
        if !smooth {
            // Jumped: stages that are now pass-through start from rest when used again.
            for (s, st) in band.stages.iter().zip(self.states[b].iter_mut()) {
                if s.current.is_bypass() {
                    *st = [Svf::ZERO; 2];
                }
            }
        }
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let i = id.0 as usize;
        let Some(&(min, max)) = self.ranges.get(i) else {
            return;
        };
        self.values[i] = util::clamp(value, min, max);
        if id == params::OUTPUT {
            let amp = util::db_to_amp(self.values[i] as f32);
            if smooth {
                self.output.set_target(amp);
            } else {
                self.output.set_immediate(amp);
            }
        } else {
            self.update_band(i / BAND_PARAMS as usize, smooth);
        }
    }

    /// Bands with at least one audible stage.
    pub fn used_bands(&self) -> usize {
        self.bands.iter().filter(|b| b.len > 0).count()
    }

    /// Magnitude response (linear) of the settled EQ at `freq` Hz, output gain included.
    pub fn magnitude(&self, freq: f64) -> f64 {
        let sr = self.sample_rate as f64;
        let bands: f64 = self
            .bands
            .iter()
            .flat_map(|b| b.stages.iter())
            .map(|s| s.target.magnitude(freq, sr))
            .product();
        bands * util::db_to_amp(self.values[params::OUTPUT.0 as usize] as f32) as f64
    }

    /// RT. After a block of `frames`: refresh both spectra once per analysis period while
    /// watched (`analysis` asked within the last second).
    fn advance_analysis(&mut self, frames: usize) {
        let sr = self.sample_rate as usize;
        if self.since_asked >= sr {
            return;
        }
        self.since_asked = self.since_asked.saturating_add(frames);
        if self.until_hop > frames {
            self.until_hop -= frames;
            return;
        }
        self.until_hop = (sr / ANALYSIS_HZ as usize).max(1);
        self.pre.compute(self.sample_rate);
        self.post.compute(self.sample_rate);
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let inputs = audio.inputs;
        let channels = audio.outputs.len().min(2);
        let gliding = self.bands.iter().any(|b| b.moving);
        for i in start..end {
            let mut x = [0.0f64; 2];
            for (c, v) in x.iter_mut().enumerate().take(channels) {
                *v = inputs.get(c).or(inputs.first()).map_or(0.0, |ch| ch[i]) as f64;
            }
            // Always fed (two stores per sample) so a new watch starts from real audio.
            self.pre.push(mono(&x, channels));
            let mut y = x;
            for (band, states) in self.bands.iter_mut().zip(self.states.iter_mut()) {
                if band.len == 0 {
                    continue;
                }
                if gliding && band.moving {
                    let mut moving = false;
                    for s in &mut band.stages[..band.len] {
                        if s.moving() {
                            let a = s.mid.glide(&s.target, self.glide);
                            let c = s.current.glide(&s.mid, self.glide);
                            moving |= a | c;
                        }
                    }
                    band.moving = moving;
                    if !moving {
                        band.len = live_len(&band.stages);
                    }
                }
                for (s, st) in band.stages[..band.len].iter().zip(states.iter_mut()) {
                    if s.current.is_bypass() && !s.moving() {
                        // Settled pass-through: start from rest when used again.
                        *st = [Svf::ZERO; 2];
                        continue;
                    }
                    for c in 0..channels {
                        y[c] = st[c].tick(y[c], &s.current);
                    }
                }
            }
            let gain = self.output.tick() as f64;
            self.post.push(mono(&y, channels) * gain as f32);
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = if c < channels {
                    (y[c] * gain) as f32
                } else {
                    0.0
                };
            }
        }
    }
}

/// `1 +` the index of the last stage that isn't settled pass-through.
fn live_len(stages: &[Stage; STAGES]) -> usize {
    stages
        .iter()
        .rposition(|s| !(s.current.is_bypass() && s.target.is_bypass()))
        .map_or(0, |i| i + 1)
}

/// Mono sum of the first `channels` (≤ 2) values.
fn mono(x: &[f64; 2], channels: usize) -> f32 {
    match channels {
        0 => 0.0,
        1 => x[0] as f32,
        _ => ((x[0] + x[1]) * 0.5) as f32,
    }
}

impl Node for GraphicEq {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        *self.states = [[[Svf::ZERO; 2]; STAGES]; BANDS];
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply_param(param, value, !step);
                }
            },
        );
        self.advance_analysis(ctx.frames);
        let settled = !self.output.is_smoothing() && self.bands.iter().all(|b| !b.moving);
        let hold = util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        if self.since_asked >= self.sample_rate as usize {
            // Just (re)watched: drop the stale spectra, publish from the next process block.
            self.until_hop = 0;
            self.pre.restart();
            self.post.restart();
        }
        self.since_asked = 0;
        let max_hz = spectrum::max_hz(self.sample_rate);
        for (kind, a) in [
            (AnalysisKind::SpectrumPre, &self.pre),
            (AnalysisKind::Spectrum, &self.post),
        ] {
            if !a.ready() {
                continue;
            }
            if let Some(f) = out.frame(kind) {
                f.push(spectrum::MIN_HZ);
                f.push(max_hz);
                for &db in a.bins() {
                    f.push(db);
                }
            }
        }
    }
}

impl Device for GraphicEq {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(BuiltinDeviceType::GraphicEq)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const SR: f32 = 48_000.0;

    fn prepared() -> GraphicEq {
        let mut eq = GraphicEq::new();
        eq.prepare(&PrepareConfig {
            sample_rate: SR,
            max_block_size: 256,
            max_events_per_block: 16,
        });
        eq
    }

    fn use_band(eq: &mut GraphicEq, b: usize, ty: usize, freq: f64, gain: f64, q: f64) {
        eq.set_param(params::band(b, params::USED), 1.0);
        eq.set_param(params::band(b, params::TYPE), ty as f64);
        eq.set_param(params::band(b, params::FREQ), freq);
        eq.set_param(params::band(b, params::GAIN), gain);
        eq.set_param(params::band(b, params::Q), q);
    }

    /// Runs `input` (both channels) through `eq` in blocks; returns the left output.
    fn run(eq: &mut GraphicEq, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len());
        for chunk in input.chunks(256) {
            let (mut l, mut r) = (vec![0.0f32; chunk.len()], vec![0.0f32; chunk.len()]);
            {
                let ins: [&[f32]; 2] = [chunk, chunk];
                let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
                let mut audio = AudioBuffers {
                    inputs: &ins,
                    outputs: &mut outs,
                };
                eq.render(&mut audio, 0, chunk.len());
            }
            out.extend(l);
        }
        out
    }

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (0.5 * (2.0 * PI * freq * i as f64 / SR as f64).sin()) as f32)
            .collect()
    }

    /// Gain (dB) of the second half of `out` relative to the input sine (amplitude 0.5).
    fn gain_db(out: &[f32]) -> f64 {
        let tail = &out[out.len() / 2..];
        let rms =
            (tail.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        20.0 * (rms * 2f64.sqrt() / 0.5).log10()
    }

    #[test]
    fn no_band_is_a_bit_exact_pass_through() {
        let mut eq = prepared();
        let mut seed = 7u32;
        let noise: Vec<f32> = (0..4096)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        let out = run(&mut eq, &noise);
        assert!(
            out.iter()
                .zip(&noise)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
        assert_eq!(eq.used_bands(), 0);
        // A used band at 0 dB is flat too.
        use_band(&mut eq, 3, 0, 1000.0, 0.0, 1.0);
        let out = run(&mut eq, &noise);
        assert!(
            out.iter()
                .zip(&noise)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }

    #[test]
    fn a_bell_boosts_its_frequency() {
        let mut eq = prepared();
        use_band(&mut eq, 0, 0, 1000.0, 6.0, 1.0);
        let at = gain_db(&run(&mut eq, &sine(1000.0, 48_000)));
        assert!((at - 6.0).abs() < 0.05, "{at}");
        let far = gain_db(&run(&mut eq, &sine(50.0, 48_000)));
        assert!(far.abs() < 0.1, "{far}");
        assert!((20.0 * eq.magnitude(1000.0).log10() - 6.0).abs() < 1e-6);
        // Off and unused bands pass.
        eq.set_param(params::band(0, params::ON), 0.0);
        assert!((gain_db(&run(&mut eq, &sine(1000.0, 48_000)))).abs() < 0.01);
        // Switched on while playing: glides there, then settles on the exact filter.
        eq.apply_param(params::band(0, params::ON), 1.0, true);
        let at = gain_db(&run(&mut eq, &sine(1000.0, 48_000)));
        assert!((at - 6.0).abs() < 0.05, "{at}");
        assert!(eq.bands[0].stages[0].current == eq.bands[0].stages[0].target);
        // Unused while playing: fades out, then the band is skipped.
        eq.apply_param(params::band(0, params::USED), 0.0, true);
        run(&mut eq, &sine(1000.0, 48_000));
        assert_eq!(eq.used_bands(), 0);
    }

    #[test]
    fn cuts_follow_their_slope() {
        let mut eq = prepared();
        use_band(&mut eq, 5, 2, 1000.0, 0.0, std::f64::consts::FRAC_1_SQRT_2);
        for (slope, per_oct) in [(1usize, 12.0), (3, 24.0), (7, 96.0)] {
            eq.set_param(params::band(5, params::SLOPE), slope as f64);
            let at = gain_db(&run(&mut eq, &sine(1000.0, 24_000)));
            assert!((at + 3.01).abs() < 0.1, "{per_oct}: {at} at the cutoff");
            let below = gain_db(&run(&mut eq, &sine(500.0, 24_000)));
            assert!(below < -per_oct * 0.7, "{per_oct}: {below} an octave below");
        }
    }

    /// Changing params while playing glides: no sample jumps far above the input's.
    #[test]
    fn param_changes_glide_without_blowing_up() {
        let mut eq = prepared();
        let input = sine(220.0, 256);
        let mut ctx_frames = 0;
        for step in 0..200 {
            let b = step % BANDS;
            // Through the audio path's event handling (smooth).
            eq.apply_param(params::band(b, params::USED), 1.0, true);
            eq.apply_param(params::band(b, params::TYPE), (step % 9) as f64, true);
            eq.apply_param(params::band(b, params::SLOPE), (step % 8) as f64, true);
            eq.apply_param(
                params::band(b, params::FREQ),
                50.0 * (1 + step % 40) as f64,
                true,
            );
            eq.apply_param(
                params::band(b, params::GAIN),
                ((step % 13) as f64 - 6.0) * 2.0,
                true,
            );
            let out = run(&mut eq, &input);
            ctx_frames += out.len();
            assert!(
                out.iter().all(|v| v.is_finite() && v.abs() < 1e4),
                "step {step}"
            );
        }
        assert!(ctx_frames > 0);
    }
}
