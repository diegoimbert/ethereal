//! Auto Filter: a zero-delay-feedback multimode filter swept by an envelope follower and an
//! LFO.
//!
//! - **Filters.** `Low-pass 12`, `High-pass 12`, `Band-pass`, `Notch` and `Peak` are a
//!   trapezoidal (TPT) state-variable filter (A. Simper); `Low-pass 24` / `High-pass 24` are
//!   a TPT 4-pole ladder (V. Zavalishin, "The Art of VA Filter Design") with the feedback
//!   solved without a unit delay; the high-pass taps the ladder stages
//!   (`u - 4y1 + 6y2 - 4y3 + y4`). SVF Q follows the UI's curve: `Q = 0.5 · 24^res`; ladder
//!   feedback `k = 3.9 · res` (self-oscillation at the top). `Peak` is a bell of
//!   `6 + 12·res` dB. Cutoff moves in octaves, so every modulation is musical and the TPT
//!   structures stay stable while it moves every sample.
//! - **Drive** (0..24 dB) boosts the filter input into a `tanh` stage (on the ladder, in the
//!   feedback path, which also tames resonance); the output is compensated by half the
//!   drive in dB. At 0 dB the filter is linear; the saturation fades in over the first 3 dB.
//! - **Envelope follower**: a stereo-linked peak follower (`Attack`/`Release`) on the
//!   sidechain when the device has one ([`Node::process_sidechain`], latency-aligned),
//!   else on the input. Its level, mapped -48..0 dBFS → 0..1, moves the cutoff by
//!   `Amount × 6` octaves (negative = down).
//! - **LFO**: six shapes, free (`Rate` Hz) or tempo-synced (`Sync Rate`, phase locked to
//!   the song position while playing; free-running at the tempo while stopped), `Amount`
//!   = ±4 octaves at 100 %. `Stereo Phase` offsets the right channel (0..180°). A 1 ms
//!   one-pole softens the steps of square / sample & hold.
//! - A filter type change between the SVF and the ladder crossfades over 5 ms.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
    TransportInfo,
};

use super::auto_filter as ids;
use super::shared::{Values, db, flush, glide, hash_bipolar};
use crate::contract::SYNC_RATE_BEATS;
use crate::util::{split_at_events, tau_coef};

const SIDECHAIN_CHANNELS: usize = 2;
/// Octaves of cutoff movement at `Envelope Amount` ±100 %.
const ENV_OCTAVES: f32 = 6.0;
/// Envelope level range mapped to 0..1 (dBFS).
const ENV_FLOOR_DB: f32 = -48.0;
/// Octaves of cutoff movement (peak) at `LFO Amount` 100 %.
const LFO_OCTAVES: f32 = 4.0;
/// Ladder feedback at resonance 100 %.
const LADDER_K_MAX: f32 = 3.9;
const FADE_MS: f32 = 5.0;
const LFO_SMOOTH_MS: f32 = 1.0;

/// Filter type (param `Type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterType {
    LowPass12,
    LowPass24,
    HighPass12,
    HighPass24,
    BandPass,
    Notch,
    Peak,
}

impl FilterType {
    const ALL: [Self; 7] = [
        Self::LowPass12,
        Self::LowPass24,
        Self::HighPass12,
        Self::HighPass24,
        Self::BandPass,
        Self::Notch,
        Self::Peak,
    ];

    fn from_index(i: usize) -> Self {
        Self::ALL[i.min(6)]
    }

    fn ladder(self) -> bool {
        matches!(self, Self::LowPass24 | Self::HighPass24)
    }
}

/// LFO shape (param `LFO Shape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LfoShape {
    Sine,
    Triangle,
    SawUp,
    SawDown,
    Square,
    SampleHold,
}

impl LfoShape {
    fn from_index(i: usize) -> Self {
        [
            Self::Sine,
            Self::Triangle,
            Self::SawUp,
            Self::SawDown,
            Self::Square,
            Self::SampleHold,
        ][i.min(5)]
    }

    /// Value at `cycle + frac` (`frac` in `[0, 1)`), -1..1.
    #[inline]
    fn at(self, cycle: i64, frac: f64) -> f32 {
        let p = frac as f32;
        match self {
            Self::Sine => (2.0 * std::f32::consts::PI * p).sin(),
            Self::Triangle => {
                if p < 0.25 {
                    4.0 * p
                } else if p < 0.75 {
                    2.0 - 4.0 * p
                } else {
                    4.0 * p - 4.0
                }
            }
            Self::SawUp => 2.0 * p - 1.0,
            Self::SawDown => 1.0 - 2.0 * p,
            Self::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::SampleHold => hash_bipolar(cycle),
        }
    }
}

/// Per-sample filter coefficients (shared by both topologies; each uses its part).
#[derive(Clone, Copy)]
struct Coefs {
    /// `tan(π fc / fs)`.
    g: f32,
    /// SVF damping `1/Q` (for the bell: `1/(Q·A)`).
    k: f32,
    /// Bell: `A² - 1`.
    bell: f32,
    /// Ladder feedback.
    fb: f32,
    /// Input drive gain, saturation blend, output compensation.
    drive: f32,
    sat: f32,
    comp: f32,
}

#[derive(Clone, Copy, Default)]
struct FilterState {
    ic1: f32,
    ic2: f32,
    s: [f32; 4],
}

#[inline]
fn saturate(x: f32, sat: f32) -> f32 {
    if sat <= 0.0 {
        x
    } else {
        x + (x.tanh() - x) * sat
    }
}

impl FilterState {
    fn reset_svf(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    fn reset_ladder(&mut self) {
        self.s = [0.0; 4];
    }

    #[inline]
    fn tick(&mut self, ty: FilterType, x: f32, c: &Coefs) -> f32 {
        let x = x * c.drive;
        let y = if ty.ladder() {
            self.ladder(ty, x, c)
        } else {
            self.svf(ty, saturate(x, c.sat), c)
        };
        y * c.comp
    }

    #[inline]
    fn svf(&mut self, ty: FilterType, v0: f32, c: &Coefs) -> f32 {
        let g = c.g;
        let k = c.k;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = v0 - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        match ty {
            FilterType::LowPass12 => v2,
            FilterType::HighPass12 => v0 - k * v1 - v2,
            FilterType::BandPass => k * v1,
            FilterType::Notch => v0 - k * v1,
            FilterType::Peak => v0 + k * c.bell * v1,
            FilterType::LowPass24 | FilterType::HighPass24 => unreachable!(),
        }
    }

    #[inline]
    fn ladder(&mut self, ty: FilterType, x: f32, c: &Coefs) -> f32 {
        let g = c.g;
        let big_g = g / (1.0 + g);
        let inv = 1.0 / (1.0 + g);
        let b = [
            self.s[0] * inv,
            self.s[1] * inv,
            self.s[2] * inv,
            self.s[3] * inv,
        ];
        let g2 = big_g * big_g;
        let sum = g2 * big_g * b[0] + g2 * b[1] + big_g * b[2] + b[3];
        let k = c.fb;
        let mut u = (x - k * sum) / (1.0 + k * g2 * g2);
        u = saturate(u, c.sat);
        // Four TPT one-poles.
        let mut y = [0.0f32; 4];
        let mut inp = u;
        for (i, yi) in y.iter_mut().enumerate() {
            let v = (inp - self.s[i]) * big_g;
            *yi = v + self.s[i];
            self.s[i] = flush(*yi + v);
            inp = *yi;
        }
        match ty {
            // Partial passband compensation (the feedback costs 1/(1+k) at DC).
            FilterType::LowPass24 => y[3] * (1.0 + k).sqrt(),
            _ => u - 4.0 * y[0] + 6.0 * y[1] - 4.0 * y[2] + y[3],
        }
    }
}

/// The auto filter device.
pub struct AutoFilter {
    values: Values<{ ids::COUNT }>,
    sample_rate: f32,
    ty: FilterType,
    fade_from: FilterType,
    fade_left: u32,
    fade_len: u32,
    /// log2 of the cutoff (Hz).
    cutoff: Smoother,
    res: Smoother,
    /// Drive in dB.
    drive: Smoother,
    env_amount: Smoother,
    att: f32,
    rel: f32,
    env: f32,
    lfo_amount: Smoother,
    lfo_shape: LfoShape,
    lfo_rate: f64,
    lfo_sync: bool,
    lfo_beats: f64,
    /// Right-channel phase offset (cycles).
    lfo_offset: Smoother,
    cycle: i64,
    frac: f64,
    lfo_lp: [f32; 2],
    lfo_lp_a: f32,
    mix: Smoother,
    output: Smoother,
    ch: [FilterState; 2],
}

impl Default for AutoFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoFilter {
    /// Non-RT. An auto filter with default params.
    pub fn new() -> Self {
        let desc = descriptor();
        let z = || Smoother::new(0.0, 0.0, 48_000.0);
        let mut s = Self {
            values: Values::new(&desc),
            sample_rate: 48_000.0,
            ty: FilterType::LowPass12,
            fade_from: FilterType::LowPass12,
            fade_left: 0,
            fade_len: 1,
            cutoff: z(),
            res: z(),
            drive: z(),
            env_amount: z(),
            att: 0.0,
            rel: 0.0,
            env: 0.0,
            lfo_amount: z(),
            lfo_shape: LfoShape::Sine,
            lfo_rate: 1.0,
            lfo_sync: false,
            lfo_beats: 1.0,
            lfo_offset: z(),
            cycle: 0,
            frac: 0.0,
            lfo_lp: [0.0; 2],
            lfo_lp_a: 1.0,
            mix: z(),
            output: z(),
            ch: [FilterState::default(); 2],
        };
        s.sync_all();
        s
    }

    fn sync_all(&mut self) {
        self.fade_len = ((FADE_MS * 0.001 * self.sample_rate) as u32).max(1);
        self.lfo_lp_a = 1.0 - (-1.0 / (LFO_SMOOTH_MS * 0.001 * self.sample_rate)).exp();
        for i in 0..ids::COUNT {
            self.apply(ParamId(i as u32), false);
        }
        self.fade_left = 0;
    }

    fn apply(&mut self, id: ParamId, smooth: bool) {
        let v = |s: &Self, id| s.values.f(id);
        match id {
            ids::TYPE => {
                let t = FilterType::from_index(self.values.index(ids::TYPE));
                if t != self.ty {
                    if smooth && t.ladder() != self.ty.ladder() {
                        for c in &mut self.ch {
                            if t.ladder() {
                                c.reset_ladder();
                            } else {
                                c.reset_svf();
                            }
                        }
                        self.fade_from = self.ty;
                        self.fade_left = self.fade_len;
                    }
                    self.ty = t;
                }
            }
            ids::CUTOFF => {
                let t = v(self, ids::CUTOFF).log2();
                glide(&mut self.cutoff, t, smooth);
            }
            ids::RESONANCE => {
                let t = v(self, ids::RESONANCE) * 0.01;
                glide(&mut self.res, t, smooth);
            }
            ids::DRIVE => {
                let t = v(self, ids::DRIVE);
                glide(&mut self.drive, t, smooth);
            }
            ids::ENV_AMOUNT => {
                let t = v(self, ids::ENV_AMOUNT) * 0.01;
                glide(&mut self.env_amount, t, smooth);
            }
            ids::ENV_ATTACK => self.att = tau_coef(v(self, ids::ENV_ATTACK), self.sample_rate),
            ids::ENV_RELEASE => self.rel = tau_coef(v(self, ids::ENV_RELEASE), self.sample_rate),
            ids::LFO_AMOUNT => {
                let t = v(self, ids::LFO_AMOUNT) * 0.01;
                glide(&mut self.lfo_amount, t, smooth);
            }
            ids::LFO_SHAPE => self.lfo_shape = LfoShape::from_index(self.values.index(id)),
            ids::LFO_RATE => self.lfo_rate = self.values.get(id).unwrap_or(1.0),
            ids::LFO_SYNC => self.lfo_sync = self.values.on(id),
            ids::LFO_SYNC_RATE => {
                self.lfo_beats =
                    SYNC_RATE_BEATS[self.values.index(id).min(SYNC_RATE_BEATS.len() - 1)]
            }
            ids::LFO_PHASE => {
                let t = v(self, ids::LFO_PHASE) / 360.0;
                glide(&mut self.lfo_offset, t, smooth);
            }
            ids::MIX => {
                let t = v(self, ids::MIX) * 0.01;
                glide(&mut self.mix, t, smooth);
            }
            ids::OUTPUT => {
                let t = db(v(self, ids::OUTPUT));
                glide(&mut self.output, t, smooth);
            }
            _ => {}
        }
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.values.set(id, value) {
            self.apply(id, smooth);
        }
    }

    /// Advance the LFO to frame `i` of the block (free, or locked to the song position).
    #[inline]
    fn lfo_step(&mut self, transport: &TransportInfo, i: usize) {
        if self.lfo_sync && transport.playing {
            let beats = transport.position + i as f64 * transport.beats_per_sample;
            let cycles = beats / self.lfo_beats;
            let c = cycles.floor();
            self.cycle = c as i64;
            self.frac = cycles - c;
            return;
        }
        let hz = if self.lfo_sync {
            transport.bpm / 60.0 / self.lfo_beats
        } else {
            self.lfo_rate
        };
        self.frac += hz / f64::from(self.sample_rate);
        if self.frac >= 1.0 {
            let whole = self.frac.floor();
            self.frac -= whole;
            self.cycle = self.cycle.wrapping_add(whole as i64);
        }
    }

    #[inline]
    fn lfo_value(&self, offset: f32) -> f32 {
        let mut f = self.frac + f64::from(offset);
        let mut cycle = self.cycle;
        if f >= 1.0 {
            f -= 1.0;
            cycle = cycle.wrapping_add(1);
        }
        self.lfo_shape.at(cycle, f)
    }

    fn render(
        &mut self,
        transport: &TransportInfo,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
        start: usize,
        end: usize,
    ) {
        let n_ch = audio.outputs.len().min(2);
        let nyq = (0.45 * self.sample_rate).min(20_000.0);
        let sr = self.sample_rate;
        for i in start..end {
            // Envelope follower (stereo-linked peak).
            let level = if sidechain.is_empty() {
                audio.inputs.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()))
            } else {
                sidechain.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()))
            };
            let coef = if level > self.env { self.att } else { self.rel };
            self.env = flush(level + (self.env - level) * coef);
            let env_amount = self.env_amount.tick();
            let env_oct = if env_amount != 0.0 && self.env > 1e-6 {
                let l = ((20.0 * self.env.log10() - ENV_FLOOR_DB) / -ENV_FLOOR_DB).clamp(0.0, 1.0);
                env_amount * l * ENV_OCTAVES
            } else {
                0.0
            };
            // LFO.
            self.lfo_step(transport, i);
            let lfo_amount = self.lfo_amount.tick();
            let offset = self.lfo_offset.tick();
            let raw = [self.lfo_value(0.0), self.lfo_value(offset)];
            let base = self.cutoff.tick() + env_oct;
            let res = self.res.tick();
            let drive_db = self.drive.tick();
            let (q_k, bell) = if self.ty == FilterType::Peak || self.fade_from == FilterType::Peak {
                let a = db((6.0 + 12.0 * res) * 0.5);
                (1.0 / (0.5 * 24f32.powf(res) * a), a * a - 1.0)
            } else {
                (1.0 / (0.5 * 24f32.powf(res)), 0.0)
            };
            let drive = db(drive_db);
            let common = Coefs {
                g: 0.0,
                k: q_k,
                bell,
                fb: LADDER_K_MAX * res,
                drive,
                sat: (drive_db / 3.0).min(1.0),
                comp: db(-0.5 * drive_db),
            };
            let fade = if self.fade_left > 0 {
                self.fade_left -= 1;
                Some(self.fade_left as f32 / self.fade_len as f32)
            } else {
                None
            };
            let mix = self.mix.tick();
            let out_gain = self.output.tick();
            for (c, raw) in raw.into_iter().enumerate().take(n_ch) {
                self.lfo_lp[c] = flush(self.lfo_lp[c] + self.lfo_lp_a * (raw - self.lfo_lp[c]));
                let oct = base + lfo_amount * self.lfo_lp[c] * LFO_OCTAVES;
                let fc = oct.exp2().clamp(20.0, nyq);
                let coefs = Coefs {
                    g: (std::f32::consts::PI * fc / sr).tan(),
                    ..common
                };
                let x = audio
                    .inputs
                    .get(c)
                    .or(audio.inputs.first())
                    .map_or(0.0, |b| b[i]);
                let st = &mut self.ch[c];
                let mut wet = st.tick(self.ty, x, &coefs);
                if let Some(old) = fade {
                    let prev = st.tick(self.fade_from, x, &coefs);
                    wet = wet * (1.0 - old) + prev * old;
                }
                audio.outputs[c][i] = (x + (wet - x) * mix) * out_gain;
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
        let n = sidechain.len().min(SIDECHAIN_CHANNELS);
        let sidechain = if sidechain[..n].iter().all(|c| c.len() >= frames) {
            &sidechain[..n]
        } else {
            &[]
        };
        let transport = ctx.transport;
        split_at_events(
            self,
            ctx.events,
            frames,
            |s, a, b| s.render(transport, audio, sidechain, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.set(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }

    /// Current envelope follower level (linear peak), for tests.
    pub fn envelope(&self) -> f32 {
        self.env
    }
}

fn descriptor() -> DeviceDescriptor {
    super::descriptor(BuiltinDeviceType::AutoFilter)
}

impl Node for AutoFilter {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        self.ch = [FilterState::default(); 2];
        self.env = 0.0;
        self.lfo_lp = [0.0; 2];
        self.fade_left = 0;
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
}

impl Device for AutoFilter {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value, false);
    }
}
