//! Built-in `Eq` device (roadmap v2, owned by the `devices-2` node).
//!
//! Eight-band parametric EQ. Every band has the same five params (on, type, frequency,
//! gain, Q) and can be any of: low cut, low shelf, bell, notch, high shelf, high cut
//! (12 dB/oct cuts). Bands are TPT state-variable filters ([`crate::dsp::svf`]) run in
//! series, in `f64`, followed by an output gain.
//!
//! No zipper noise: parameter events recompute each band's *target* coefficients, and the
//! running coefficients glide to them per sample through two cascaded one-poles (an
//! S-shaped ≈15 ms glide with no slope discontinuity; frequency moves in octaves).
//! Switching a band off glides its output mix to pass-through (same frequency and Q), so
//! on/off and type changes don't click either.
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
//!
//! # Analysis (v0.2, `graphical-eq`, CONTRACTS.md §12.15)
//! While watched, the EQ publishes its input (`SpectrumPre`) and output (`Spectrum`) spectra
//! in the same pass: `process` feeds both mono sums into [`spectrum::Analyzer`]s and runs
//! their FFTs once per `sample_rate / ANALYSIS_HZ` frames, only while `analysis` was asked
//! within the last second; `analysis` copies the latest bins out (and, on a new watch, drops
//! the stale ones: nothing is published until the next block has refreshed them). No allocation on the audio
//! thread (all buffers are sized in `new`).

use ether_core::analysis::{ANALYSIS_HZ, AnalysisSink};
use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessStatus, Smoother,
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
/// Time constant of each of the two coefficient glide stages.
const GLIDE_MS: f32 = 6.0;

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
        layout: Some(layout()),
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

/// Declarative panel (v0.2, `graphical-eq`): the interactive curve over all bands with a
/// pre/post spectrum, then each band's controls and the output.
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::eq_response::EqShape;
    use ether_core::protocol::layout::{EqBandBinding, SpectrumOverlay, Widget, WidgetSize};
    const SHAPES: [EqShape; 6] = [
        EqShape::LowCut,
        EqShape::LowShelf,
        EqShape::Bell,
        EqShape::Notch,
        EqShape::HighShelf,
        EqShape::HighCut,
    ];
    let bands = (0..BANDS)
        .map(|b| EqBandBinding {
            on: Some(params::band(b, params::ON)),
            kind: Some(params::band(b, params::TYPE)),
            shapes: SHAPES.to_vec(),
            freq: params::band(b, params::FREQ),
            gain: Some(params::band(b, params::GAIN)),
            q: Some(params::band(b, params::Q)),
        })
        .collect();
    let mut curve = item(
        Widget::EqCurve {
            bands,
            crossovers: vec![],
            spectrum: SpectrumOverlay::PrePost,
        },
        WidgetSize::Large,
    );
    curve.colspan = 8;
    let mut controls = Vec::new();
    for offset in [
        params::ON,
        params::TYPE,
        params::FREQ,
        params::GAIN,
        params::Q,
    ] {
        for b in 0..BANDS {
            let p = params::band(b, offset);
            controls.push(match offset {
                params::ON => item(Widget::Toggle { param: p }, WidgetSize::Small),
                params::TYPE => item(Widget::Choice { param: p }, WidgetSize::Small),
                _ => knob(p, WidgetSize::Small),
            });
        }
    }
    layout(vec![
        section("curve", None, 4, 8, vec![curve]),
        section("bands", Some("Bands"), 4, 8, controls),
        section(
            "output",
            Some("Output"),
            1,
            1,
            vec![knob(params::OUTPUT, WidgetSize::Medium)],
        ),
    ])
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(Eq::new())
}

#[derive(Clone, Copy, Debug)]
struct Band {
    current: Coefs,
    /// First glide stage (`target` → `mid` → `current`).
    mid: Coefs,
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
    /// Input / output spectrum (`SpectrumPre` / `Spectrum` analysis frames).
    pre: spectrum::Analyzer,
    post: spectrum::Analyzer,
    /// Frames since `analysis` was last called (spectra are computed only while watched).
    since_asked: usize,
    /// Frames until the next spectrum update.
    until_hop: usize,
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
            mid: Coefs::BYPASS,
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
            pre: spectrum::Analyzer::new(),
            post: spectrum::Analyzer::new(),
            since_asked: usize::MAX,
            until_hop: 0,
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
        let shape = SHAPES[util::index(v(params::TYPE), SHAPES.len())];
        let coefs = Coefs::new(
            shape,
            v(params::FREQ),
            v(params::GAIN),
            v(params::Q),
            self.sample_rate as f64,
        );
        if v(params::ON) < 0.5 {
            coefs.bypassed()
        } else {
            coefs
        }
    }

    fn update_band(&mut self, b: usize, smooth: bool) {
        let target = self.band_coefs(b);
        let band = &mut self.bands[b];
        band.target = target;
        if smooth {
            band.moving = band.current != target;
        } else {
            band.current = target;
            band.mid = target;
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
            // Always fed (two stores per sample) so a new watch starts from real audio.
            self.pre.push(mono(&x, channels));
            for band in &mut self.bands {
                if band.moving {
                    let a = band.mid.glide(&band.target, self.glide);
                    let b = band.current.glide(&band.mid, self.glide);
                    band.moving = a | b;
                } else if band.current.is_bypass() {
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
            self.post.push(mono(&x, channels) * gain as f32);
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
        self.advance_analysis(ctx.frames);
        ProcessStatus::Continue
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

/// Mono sum of the first `channels` (≤ 2) values.
fn mono(x: &[f64; 2], channels: usize) -> f32 {
    match channels {
        0 => 0.0,
        1 => x[0] as f32,
        _ => ((x[0] + x[1]) * 0.5) as f32,
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

/// Spectrum analysis of the EQ's input and output (RT after construction).
pub mod spectrum {
    use std::f32::consts::PI;

    /// FFT size (≈ 11.7 Hz resolution at 48 kHz).
    pub const FFT_SIZE: usize = 4096;
    /// Log-spaced output bins (CONTRACTS §12.4.3: ≤ 256).
    pub const BINS: usize = 192;
    /// Lowest bin frequency.
    pub const MIN_HZ: f32 = 20.0;
    /// Highest bin frequency.
    pub const MAX_HZ: f32 = 20_000.0;
    /// dB floor of the published bins.
    pub const FLOOR_DB: f32 = -120.0;
    /// Weight of the newest spectrum in the per-bin power average (display smoothing).
    const SMOOTH: f32 = 0.5;

    /// Highest bin frequency at `sample_rate` (below Nyquist).
    pub fn max_hz(sample_rate: f32) -> f32 {
        MAX_HZ.min(sample_rate * 0.49)
    }

    /// One mono spectrum: a sliding window of the last [`FFT_SIZE`] samples, a Hann-windowed
    /// radix-2 FFT, and [`BINS`] log-spaced dB bins (peak power of the FFT bins each covers,
    /// interpolated where bins are narrower than the FFT resolution).
    #[derive(Debug)]
    pub struct Analyzer {
        history: Box<[f32]>,
        write: usize,
        window: Box<[f32]>,
        /// `cos`, `sin` of `2π·k/N` for `k < N/2`.
        twiddle: Box<[(f32, f32)]>,
        re: Box<[f32]>,
        im: Box<[f32]>,
        /// Smoothed power per output bin.
        power: Box<[f32]>,
        bins: Box<[f32]>,
        ready: bool,
    }

    impl Default for Analyzer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Analyzer {
        /// Non-RT. All buffers are allocated here.
        pub fn new() -> Self {
            let n = FFT_SIZE;
            let window = (0..n)
                .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
                .collect();
            let twiddle = (0..n / 2)
                .map(|k| {
                    let a = 2.0 * PI * k as f32 / n as f32;
                    (a.cos(), a.sin())
                })
                .collect();
            Self {
                history: vec![0.0; n].into_boxed_slice(),
                write: 0,
                window,
                twiddle,
                re: vec![0.0; n].into_boxed_slice(),
                im: vec![0.0; n].into_boxed_slice(),
                power: vec![0.0; BINS].into_boxed_slice(),
                bins: vec![FLOOR_DB; BINS].into_boxed_slice(),
                ready: false,
            }
        }

        /// RT. Append one sample.
        #[inline]
        pub fn push(&mut self, x: f32) {
            self.history[self.write] = x;
            self.write = (self.write + 1) & (FFT_SIZE - 1);
        }

        /// The latest bins (dB), valid once [`Self::ready`].
        pub fn bins(&self) -> &[f32] {
            &self.bins
        }

        /// RT. Forget the computed spectrum (the next [`Self::compute`] starts unsmoothed).
        pub fn restart(&mut self) {
            self.ready = false;
        }

        /// A spectrum has been computed since the last [`Self::restart`].
        pub fn ready(&self) -> bool {
            self.ready
        }

        /// RT. Recompute the bins from the last [`FFT_SIZE`] samples.
        pub fn compute(&mut self, sample_rate: f32) {
            let n = FFT_SIZE;
            for i in 0..n {
                self.re[i] = self.history[(self.write + i) & (n - 1)] * self.window[i];
                self.im[i] = 0.0;
            }
            fft(&mut self.re, &mut self.im, &self.twiddle);
            // Full-scale sine through a Hann window peaks at N/4.
            let norm = (4.0 / n as f32) * (4.0 / n as f32);
            let hz_per_bin = sample_rate / n as f32;
            let top = max_hz(sample_rate);
            let ratio = top / MIN_HZ;
            let power_at = |re: &[f32], im: &[f32], k: usize| {
                let k = k.min(n / 2);
                (re[k] * re[k] + im[k] * im[k]) * norm
            };
            for b in 0..BINS {
                let edge = |t: f32| MIN_HZ * ratio.powf(t / BINS as f32);
                let (lo, hi) = (
                    edge(b as f32) / hz_per_bin,
                    edge(b as f32 + 1.0) / hz_per_bin,
                );
                let (k0, k1) = (lo.ceil() as usize, hi.floor() as usize);
                let p = if k1 >= k0 {
                    (k0..=k1).fold(0.0f32, |m, k| m.max(power_at(&self.re, &self.im, k)))
                } else {
                    // Narrower than one FFT bin: interpolate at the centre.
                    let c = (lo + hi) * 0.5;
                    let k = c.floor() as usize;
                    let t = c - k as f32;
                    power_at(&self.re, &self.im, k) * (1.0 - t)
                        + power_at(&self.re, &self.im, k + 1) * t
                };
                let s = if self.ready {
                    self.power[b] + SMOOTH * (p - self.power[b])
                } else {
                    p
                };
                // Flush tiny values (denormals) to zero.
                self.power[b] = if s < 1e-20 { 0.0 } else { s };
                self.bins[b] = (10.0 * self.power[b].max(1e-12).log10()).max(FLOOR_DB);
            }
            self.ready = true;
        }
    }

    /// In-place iterative radix-2 FFT (`re.len()` a power of two, `twiddle` from `new`).
    fn fft(re: &mut [f32], im: &mut [f32], twiddle: &[(f32, f32)]) {
        let n = re.len();
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            let half = len / 2;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (c, s) = twiddle[k * step];
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * c + im[b] * s;
                    let ti = im[b] * c - re[b] * s;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}
