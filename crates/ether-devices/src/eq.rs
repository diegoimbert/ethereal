//! Built-in `Eq` device (roadmap v2, owned by the `devices-2` node).
//!
//! Eight-band parametric EQ. Every band has the same five params (on, type, frequency,
//! gain, Q) and can be any of: low cut, low shelf, bell, notch, high shelf, high cut
//! (12 dB/oct cuts). Bands are TPT state-variable filters ([`crate::dsp::svf`]) run in
//! series, in `f64`, followed by an output gain.
//!
//! No zipper noise: parameter events recompute each band's *target* coefficients, and the
//! running coefficients glide to them per sample (≈5 ms one-pole). Switching a band off
//! glides it to the pass-through coefficients, so on/off and type changes don't click.
//!
//! # Parameter ids (stable, append-only)
//!
//! Band `b` (0..8) uses ids `5·b .. 5·b+4`:
//!
//! | id      | name       | range                         |
//! |---------|------------|-------------------------------|
//! | `5b+0`  | `On`       | toggle                        |
//! | `5b+1`  | `Type`     | Low Cut, Low Shelf, Bell, Notch, High Shelf, High Cut |
//! | `5b+2`  | `Freq`     | 20 ..= 20000 Hz (log)         |
//! | `5b+3`  | `Gain`     | -24 ..= 24 dB                 |
//! | `5b+4`  | `Q`        | 0.1 ..= 18 (log)              |
//! | `40`    | `Output`   | -24 ..= 24 dB                 |
//!
//! Default bands: 1 Low Cut 30 Hz (off), 2 Low Shelf 100 Hz, 3-6 Bell 250 / 1k / 2.5k /
//! 6k Hz, 7 High Shelf 10 kHz, 8 High Cut 18 kHz (off); all gains 0 dB (flat).

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::dsp::svf::{Coefs, Shape, Svf};
use crate::util;

/// Number of bands.
pub const BANDS: usize = 8;
/// Params per band.
pub const BAND_PARAMS: u32 = 5;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;

    /// Offsets inside a band (`band(b) + offset`).
    pub const ON: u32 = 0;
    pub const TYPE: u32 = 1;
    pub const FREQ: u32 = 2;
    pub const GAIN: u32 = 3;
    pub const Q: u32 = 4;

    /// Id of param `offset` of band `b` (0-based).
    pub const fn band(b: usize, offset: u32) -> ParamId {
        ParamId(b as u32 * super::BAND_PARAMS + offset)
    }

    pub const OUTPUT: ParamId = ParamId(40);
}

/// Band type labels (plain values 0..=5 of the `Type` params).
pub const TYPE_LABELS: [&str; 6] = [
    "Low Cut",
    "Low Shelf",
    "Bell",
    "Notch",
    "High Shelf",
    "High Cut",
];
const SHAPES: [Shape; 6] = [
    Shape::LowCut,
    Shape::LowShelf,
    Shape::Bell,
    Shape::Notch,
    Shape::HighShelf,
    Shape::HighCut,
];

/// Default (on, type index, freq) per band.
const DEFAULT_BANDS: [(bool, usize, f64); BANDS] = [
    (false, 0, 30.0),
    (true, 1, 100.0),
    (true, 2, 250.0),
    (true, 2, 1000.0),
    (true, 2, 2500.0),
    (true, 2, 6000.0),
    (true, 4, 10_000.0),
    (false, 5, 18_000.0),
];

const NUM_PARAMS: usize = BANDS * BAND_PARAMS as usize + 1;
/// Coefficient glide time constant.
const GLIDE_MS: f32 = 5.0;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::{choice, param};
    let mut v = Vec::with_capacity(NUM_PARAMS);
    for (b, &(on, ty, freq)) in DEFAULT_BANDS.iter().enumerate() {
        let group = format!("Band {}", b + 1);
        let id = |o: u32| params::band(b, o).0;
        v.push(choice(
            id(params::ON),
            "On",
            &group,
            ParamUnit::Toggle,
            &["Off", "On"],
            on as usize,
        ));
        v.push(choice(
            id(params::TYPE),
            "Type",
            &group,
            ParamUnit::None,
            &TYPE_LABELS,
            ty,
        ));
        v.push(param(
            id(params::FREQ),
            "Freq",
            &group,
            ParamUnit::Hertz,
            (20.0, 20_000.0, freq),
            ParamScale::Log,
        ));
        v.push(param(
            id(params::GAIN),
            "Gain",
            &group,
            ParamUnit::Decibels,
            (-24.0, 24.0, 0.0),
            ParamScale::Linear,
        ));
        v.push(param(
            id(params::Q),
            "Q",
            &group,
            ParamUnit::None,
            (0.1, 18.0, std::f64::consts::FRAC_1_SQRT_2),
            ParamScale::Log,
        ));
    }
    v.push(param(
        params::OUTPUT.0,
        "Output",
        "Output",
        ParamUnit::Decibels,
        (-24.0, 24.0, 0.0),
        ParamScale::Linear,
    ));
    v
}

/// Descriptor of the `Eq` type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Eq,
        },
        name: "EQ".to_owned(),
        category: DeviceCategory::AudioEffect,
        params: param_infos(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
    }
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(Eq::new())
}

#[derive(Clone, Copy, Debug)]
struct Band {
    current: Coefs,
    target: Coefs,
    /// `current != target` (gliding).
    moving: bool,
    state: [Svf; 2],
}

/// The built-in parametric EQ.
#[derive(Debug)]
pub struct Eq {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    glide: f64,
    bands: [Band; BANDS],
    output: Smoother,
}

impl Default for Eq {
    fn default() -> Self {
        Self::new()
    }
}

impl Eq {
    /// Non-RT. An EQ with default parameters.
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let band = Band {
            current: Coefs::BYPASS,
            target: Coefs::BYPASS,
            moving: false,
            state: [Svf::default(); 2],
        };
        let mut eq = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            glide: 0.0,
            bands: [band; BANDS],
            output: Smoother::new(1.0, 20.0, 48_000.0),
        };
        eq.sync_all();
        eq
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

    /// Target coefficients of band `b` from its params.
    pub(crate) fn band_coefs(&self, b: usize) -> Coefs {
        let v = |o: u32| self.values[params::band(b, o).0 as usize];
        if v(params::ON) < 0.5 {
            return Coefs::BYPASS;
        }
        let shape = SHAPES[util::index(v(params::TYPE), SHAPES.len())];
        Coefs::new(
            shape,
            v(params::FREQ),
            v(params::GAIN),
            v(params::Q),
            self.sample_rate as f64,
        )
    }

    fn update_band(&mut self, b: usize, smooth: bool) {
        let target = self.band_coefs(b);
        let band = &mut self.bands[b];
        band.target = target;
        if smooth {
            band.moving = band.current != target;
        } else {
            band.current = target;
            band.moving = false;
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

    /// Magnitude response (linear) of the settled EQ at `freq` Hz, output gain included.
    pub fn magnitude(&self, freq: f64) -> f64 {
        let sr = self.sample_rate as f64;
        let bands: f64 = (0..BANDS)
            .map(|b| self.band_coefs(b).magnitude(freq, sr))
            .product();
        bands * util::db_to_amp(self.values[params::OUTPUT.0 as usize] as f32) as f64
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let inputs = audio.inputs;
        let channels = audio.outputs.len().min(2);
        for i in start..end {
            let mut x = [0.0f64; 2];
            for (c, v) in x.iter_mut().enumerate().take(channels) {
                *v = inputs.get(c).map_or(0.0, |ch| ch[i]) as f64;
            }
            for band in &mut self.bands {
                if band.moving {
                    band.moving = band.current.glide(&band.target, self.glide);
                } else if band.current == Coefs::BYPASS {
                    // Settled and off: exact pass-through. Clear the state so the band
                    // starts from rest when switched back on (its m1/m2 glide from 0).
                    band.state = [Svf::default(); 2];
                    continue;
                }
                for (c, v) in x.iter_mut().enumerate().take(channels) {
                    *v = band.state[c].tick(*v, &band.current);
                }
            }
            let gain = self.output.tick() as f64;
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = if c < channels {
                    (x[c] * gain) as f32
                } else {
                    0.0
                };
            }
        }
    }
}

impl Node for Eq {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            band.state.iter_mut().for_each(Svf::reset);
        }
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
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }
}

impl Device for Eq {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
