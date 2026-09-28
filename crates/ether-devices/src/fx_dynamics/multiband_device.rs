//! 3-band compressor (`BuiltinDeviceType::MultibandCompressor`).
//!
//! Linkwitz-Riley 24 dB/oct crossovers (`Low / Mid`, `Mid / High`; the second is kept at or
//! above the first). The low band goes through the high crossover's allpass, so the three
//! bands sum to a flat allpass when every band is neutral. Each band has the built-in
//! compressor's feed-forward peak detector and soft-knee gain computer
//! ([`crate::compressor::gain_reduction_db`]) with its own threshold, ratio, attack, release
//! and makeup; `Solo` (any band soloed = only soloed bands) and `Bypass` (band passes
//! uncompressed, no makeup) crossfade in a few ms. With a sidechain source, its full-band
//! peak keys all three detectors (`Node::process_sidechain`). `Mix` blends with the dry
//! signal passed through the same allpasses (phase-matched, no comb filtering).
//!
//! Per-band gain reduction (dB, ≤ 0; the most reduced value since the last read) is
//! published as `AnalysisKind::Levels [low, mid, high]` for the layout meters.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus,
};

use super::multiband_compressor as p;
use super::shared::{
    GainRamp, LrSplit, Params, SIDECHAIN_CHANNELS, Svf, SvfCoefs, samples, usable_sidechain,
};
use crate::compressor::gain_reduction_db;
use crate::util::{amp_to_db, db_to_amp, split_at_events, tau_coef};

const BANDS: usize = 3;
/// Crossfade time of Solo/Bypass changes (ms).
const SWITCH_MS: f32 = 5.0;
/// Crossover glide time constant (ms).
const GLIDE_MS: f32 = 8.0;

/// Param ids of one band.
struct BandIds {
    threshold: ParamId,
    ratio: ParamId,
    attack: ParamId,
    release: ParamId,
    makeup: ParamId,
    solo: ParamId,
    bypass: ParamId,
}

const BAND_IDS: [BandIds; BANDS] = [
    BandIds {
        threshold: p::LOW_THRESHOLD,
        ratio: p::LOW_RATIO,
        attack: p::LOW_ATTACK,
        release: p::LOW_RELEASE,
        makeup: p::LOW_MAKEUP,
        solo: p::LOW_SOLO,
        bypass: p::LOW_BYPASS,
    },
    BandIds {
        threshold: p::MID_THRESHOLD,
        ratio: p::MID_RATIO,
        attack: p::MID_ATTACK,
        release: p::MID_RELEASE,
        makeup: p::MID_MAKEUP,
        solo: p::MID_SOLO,
        bypass: p::MID_BYPASS,
    },
    BandIds {
        threshold: p::HIGH_THRESHOLD,
        ratio: p::HIGH_RATIO,
        attack: p::HIGH_ATTACK,
        release: p::HIGH_RELEASE,
        makeup: p::HIGH_MAKEUP,
        solo: p::HIGH_SOLO,
        bypass: p::HIGH_BYPASS,
    },
];

/// Per-band dynamics state.
#[derive(Clone, Copy, Debug)]
struct Band {
    attack_coef: f32,
    release_coef: f32,
    /// Smoothed gain reduction (dB, >= 0).
    gr_db: f32,
    makeup: GainRamp,
    /// 1 = compressed, 0 = bypassed (crossfade).
    active: GainRamp,
    /// 1 = audible, 0 = muted by another band's solo (crossfade).
    audible: GainRamp,
    /// Largest reduction since the last analysis read.
    meter_gr: f32,
}

impl Band {
    fn new() -> Self {
        Self {
            attack_coef: 0.0,
            release_coef: 0.0,
            gr_db: 0.0,
            makeup: GainRamp::new(1.0),
            active: GainRamp::new(1.0),
            audible: GainRamp::new(1.0),
            meter_gr: 0.0,
        }
    }
}

/// Filter state of one channel.
#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    split_low: LrSplit,
    split_high: LrSplit,
    /// Low band through the high crossover's allpass.
    low_ap: Svf,
    /// Dry path through both allpasses (for `Mix`).
    dry_ap: [Svf; 2],
}

/// The multiband compressor.
pub struct MultibandCompressor {
    params: Params<{ p::COUNT }>,
    sample_rate: f32,
    bands: [Band; BANDS],
    channels: [Channel; 2],
    /// Current (gliding) crossover frequencies and their coefficients.
    freq: [f32; 2],
    coefs: [SvfCoefs; 2],
    glide_coef: f32,
    output: GainRamp,
    mix: GainRamp,
}

impl Default for MultibandCompressor {
    fn default() -> Self {
        Self::new()
    }
}

impl MultibandCompressor {
    /// Non-RT. A multiband compressor with default parameters (prepared for 48 kHz).
    pub fn new() -> Self {
        let params = Params::new(&super::descriptor(BuiltinDeviceType::MultibandCompressor));
        let mut m = Self {
            params,
            sample_rate: 48_000.0,
            bands: [Band::new(); BANDS],
            channels: [Channel::default(); 2],
            freq: [200.0, 2500.0],
            coefs: [SvfCoefs::new(200.0, 48_000.0), SvfCoefs::new(2500.0, 48_000.0)],
            glide_coef: 0.0,
            output: GainRamp::new(1.0),
            mix: GainRamp::new(1.0),
        };
        m.sync();
        m
    }

    /// Target crossover frequencies (the high one kept at or above the low one).
    fn target_freqs(&self) -> [f32; 2] {
        let lo = self.params.get(p::LOW_MID_FREQ);
        let hi = self.params.get(p::MID_HIGH_FREQ).max(lo);
        [lo, hi]
    }

    fn sync(&mut self) {
        self.glide_coef = tau_coef(GLIDE_MS, self.sample_rate);
        self.freq = self.target_freqs();
        self.update_coefs();
        for b in 0..BANDS {
            self.update_band_times(b);
            let ids = &BAND_IDS[b];
            let band = &mut self.bands[b];
            band.makeup.set(db_to_amp(self.params.get(ids.makeup)), false);
            band.active
                .set(if self.params.on(ids.bypass) { 0.0 } else { 1.0 }, false);
        }
        self.update_solo(false);
        self.output
            .set(db_to_amp(self.params.get(p::OUTPUT)), false);
        self.mix.set(self.params.get(p::MIX) * 0.01, false);
    }

    fn update_coefs(&mut self) {
        let sr = f64::from(self.sample_rate);
        self.coefs = [
            SvfCoefs::new(f64::from(self.freq[0]), sr),
            SvfCoefs::new(f64::from(self.freq[1]), sr),
        ];
    }

    fn update_band_times(&mut self, b: usize) {
        let ids = &BAND_IDS[b];
        let band = &mut self.bands[b];
        band.attack_coef = tau_coef(self.params.get(ids.attack), self.sample_rate);
        band.release_coef = tau_coef(self.params.get(ids.release), self.sample_rate);
    }

    fn update_solo(&mut self, smooth: bool) {
        let any = BAND_IDS.iter().any(|ids| self.params.on(ids.solo));
        let n = samples(SWITCH_MS, self.sample_rate) as u32;
        for (b, ids) in BAND_IDS.iter().enumerate() {
            let target = if !any || self.params.on(ids.solo) { 1.0 } else { 0.0 };
            if smooth {
                self.bands[b].audible.ramp(target, n);
            } else {
                self.bands[b].audible.set(target, false);
            }
        }
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        if !self.params.set(id, value) {
            return;
        }
        match id {
            p::LOW_MID_FREQ | p::MID_HIGH_FREQ if !smooth => {
                self.freq = self.target_freqs();
                self.update_coefs();
            }
            p::OUTPUT => self.output.set(db_to_amp(self.params.get(p::OUTPUT)), smooth),
            p::MIX => self.mix.set(self.params.get(p::MIX) * 0.01, smooth),
            _ => {
                let n = samples(SWITCH_MS, self.sample_rate) as u32;
                for b in 0..BANDS {
                    let ids = &BAND_IDS[b];
                    if id == ids.attack || id == ids.release {
                        self.update_band_times(b);
                    } else if id == ids.makeup {
                        let amp = db_to_amp(self.params.get(ids.makeup));
                        self.bands[b].makeup.set(amp, smooth);
                    } else if id == ids.bypass {
                        let t = if self.params.on(ids.bypass) { 0.0 } else { 1.0 };
                        if smooth {
                            self.bands[b].active.ramp(t, n);
                        } else {
                            self.bands[b].active.set(t, false);
                        }
                    } else if id == ids.solo {
                        self.update_solo(smooth);
                    }
                }
            }
        }
    }

    /// Current gain reduction (dB, >= 0) of band `b` (0 = low).
    pub fn gain_reduction(&self, b: usize) -> f32 {
        self.bands[b].gr_db
    }

    /// Glide the crossovers one sample towards their targets.
    #[inline]
    fn glide(&mut self) {
        let target = self.target_freqs();
        if target == self.freq {
            return;
        }
        for (f, t) in self.freq.iter_mut().zip(target) {
            let (lf, lt) = (f.ln(), t.ln());
            let next = (lt + (lf - lt) * self.glide_coef).exp();
            *f = if (next - t).abs() < 1e-3 * t { t } else { next };
        }
        self.update_coefs();
    }

    fn render(
        &mut self,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
        start: usize,
        end: usize,
    ) {
        let thresholds = BAND_IDS.map(|ids| self.params.get(ids.threshold));
        let ratios = BAND_IDS.map(|ids| self.params.get(ids.ratio));
        let inputs = audio.inputs;
        let n_ch = audio.outputs.len().min(2);
        let mut split = [[0.0f64; BANDS]; 2];
        let mut dry = [0.0f64; 2];
        for i in start..end {
            self.glide();
            let (c_lo, c_hi) = (self.coefs[0], self.coefs[1]);
            let mut peak = [0.0f32; BANDS];
            for (c, st) in self.channels.iter_mut().enumerate().take(n_ch) {
                let x = f64::from(inputs.get(c).or(inputs.first()).map_or(0.0, |ch| ch[i]));
                let (low, rest) = st.split_low.tick(x, &c_lo);
                let low = st.low_ap.allpass(low, &c_hi);
                let (mid, high) = st.split_high.tick(rest, &c_hi);
                split[c] = [low, mid, high];
                let d = st.dry_ap[0].allpass(x, &c_lo);
                dry[c] = st.dry_ap[1].allpass(d, &c_hi);
                for (pk, v) in peak.iter_mut().zip(split[c]) {
                    *pk = pk.max(v.abs() as f32);
                }
            }
            if !sidechain.is_empty() {
                let key = sidechain.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()));
                peak = [key; BANDS];
            }
            let mut gains = [0.0f64; BANDS];
            for b in 0..BANDS {
                let band = &mut self.bands[b];
                let target = gain_reduction_db(amp_to_db(peak[b]), thresholds[b], ratios[b]);
                let coef = if target > band.gr_db {
                    band.attack_coef
                } else {
                    band.release_coef
                };
                band.gr_db = target + (band.gr_db - target) * coef;
                if band.gr_db < 1e-6 {
                    band.gr_db = 0.0;
                }
                let active = band.active.tick();
                let comp = db_to_amp(-band.gr_db) * band.makeup.tick();
                let gain = 1.0 + (comp - 1.0) * active;
                gains[b] = f64::from(gain * band.audible.tick());
                let shown = band.gr_db * active;
                if shown > band.meter_gr {
                    band.meter_gr = shown;
                }
            }
            let mix = f64::from(self.mix.tick());
            let out_gain = f64::from(self.output.tick());
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                let c = c.min(n_ch.saturating_sub(1));
                let wet: f64 = split[c].iter().zip(gains).map(|(v, g)| v * g).sum();
                let y = (dry[c] + (wet - dry[c]) * mix) * out_gain;
                out[i] = y as f32;
            }
        }
    }

    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        let frames = ctx.frames;
        let sidechain = usable_sidechain(sidechain, frames);
        split_at_events(
            self,
            ctx.events,
            frames,
            |s, a, b| s.render(audio, sidechain, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }
}

impl Node for MultibandCompressor {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync();
        self.reset();
    }

    fn reset(&mut self) {
        self.channels = [Channel::default(); 2];
        for b in &mut self.bands {
            b.gr_db = 0.0;
            b.meter_gr = 0.0;
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, &[])
    }

    fn sidechain_inputs(&self) -> u16 {
        SIDECHAIN_CHANNELS as u16
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, sidechain)
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        if let Some(f) = out.frame(AnalysisKind::Levels) {
            for b in &mut self.bands {
                f.push(-b.meter_gr);
                b.meter_gr = 0.0;
            }
        }
    }
}

impl Device for MultibandCompressor {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::MultibandCompressor)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
