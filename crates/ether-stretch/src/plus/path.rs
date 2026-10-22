//! One resolution of the Complex+ stretcher: a phase vocoder with a fixed FFT size that
//! synthesises one frequency band (the others are left to the other resolutions).
//!
//! Per frame (analysis centred at input sample `c`, synthesis centred at output sample `f`):
//! 1. **Analysis** of every channel through one real FFT of the (gated) rectangular frame:
//!    the Hann spectrum and the Hann-derivative spectrum are 3-tap kernels of it, giving the
//!    reassigned instantaneous frequency (Auger & Flandrin 1995), which doesn't depend on the
//!    analysis hop, so any time ratio works.
//! 2. **Reference** spectrum shared by the channels (their sum; the loudest channel where the
//!    sum cancels): phases are integrated once on it and every channel keeps its analysed
//!    phase offset to it, so the stereo image (inter-channel phase) is preserved exactly.
//! 3. **Pitch shift** in the spectrum: output bin `k` reads input bin `k/p`, with the
//!    instantaneous frequency scaled by `p`. **Formants**: a gain moving the spectral envelope
//!    (true envelope, from the mid resolution) to where it should be.
//! 4. **Phase**: phase-gradient heap integration ([`super::phase`]); exact phase-vocoder
//!    propagation when nothing is stretched (`p = 1`, analysis hop = synthesis hop: then the
//!    output reproduces the input); per-bin phase reset at onsets where the energy jumps.
//! 5. **Synthesis**: inverse FFT, Hann synthesis window, overlap-add with the analysis × synthesis
//!    window (and gate) accumulated alongside, so gated frames normalise correctly.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use realfft::num_complex::Complex32 as C;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use super::phase::{Gradient, PhaseIntegrator, Previous, cis};

/// Bins of the previous frame weaker than this (relative to the frame's peak) don't seed the
/// phase integration.
const PHASE_TOL: f32 = 1e-5;
/// Energy rise (ratio of magnitudes) above which an onset frame resets a bin's phase.
const RESET_RISE: f32 = 1.6;
/// Where the channel sum keeps less than this of the channels' magnitude, the loudest
/// channel is the reference.
const CANCEL: f32 = 0.25;
/// Floor of the window normalisation, relative to the nominal overlap (gated regions dip
/// instead of amplifying leakage).
const NORM_FLOOR: f32 = 0.35;
/// Formant gain limits (dB).
const GAIN_MAX_DB: f32 = 24.0;
const GAIN_MIN_DB: f32 = -48.0;
/// Formant correction fades in between these output frequencies (Hz).
const FORMANT_FADE_HZ: (f32, f32) = (100.0, 200.0);
/// Most a frame's energy may change through the formant correction (dB).
const FORMANT_ENERGY_DB: (f32, f32) = (-3.0, 3.0);

/// `z / |z|` (1 for zero).
#[inline]
fn unit(z: C) -> C {
    let n = z.norm_sqr().sqrt();
    if n > 1e-30 { z / n } else { C::new(1.0, 0.0) }
}

/// Partial model (transposition): how far each modelled lobe reaches (bins), the lobe
/// table's resolution (steps per bin), the weakest peak modelled (relative to the frame's).
const MODEL_LOBE: usize = 8;
const LOBE_STEPS: usize = 64;
const MODEL_FLOOR: f32 = 1e-3;
/// A modelled peak stands this much (magnitude ratio, 6 dB) above the lowest bin within 2
/// of it: noise's countless shallow maxima stay in the residual.
const MODEL_PROMINENCE: f32 = 2.0;
/// Refinement passes of the partial amplitudes.
const MODEL_REFINE: usize = 1;
const NO_PEAK: u32 = u32::MAX;
/// Most bins a modelled lobe touches.
const LOBE_SPAN: usize = 2 * MODEL_LOBE + 2;

/// Static description of a resolution.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PathSpec {
    /// FFT / window size.
    pub n: usize,
    /// Synthesis hop in ticks.
    pub hop_ticks: usize,
    /// Band (Hz): the mask is 1 between `lo.1` and `hi.0`, raised-cosine (in log frequency)
    /// down to 0 at `lo.0` and `hi.1`. `lo = None`: from DC; `hi = None`: to Nyquist.
    pub lo: Option<(f32, f32)>,
    pub hi: Option<(f32, f32)>,
    /// Onset band index (strength used for gating).
    pub onset_band: usize,
    /// The crossover (low or high edge) where this resolution takes its phases from the
    /// next-longer resolution (see [`Master`]).
    pub follow: Option<Edge>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    /// The band's low edge (the only one followed: masters are the longer resolutions).
    Lo,
}

/// Phases of the next-longer resolution, which the shorter one follows in their crossover
/// band (the longer one resolves the partials there best, also when transposing up, where
/// the crossover's source lies lower still). Two resolutions
/// synthesising the same partial with independently integrated phases would partly cancel
/// where their bands overlap; instead, in the crossover the follower takes the master's
/// phase (moved to its own frame time with the instantaneous frequency) and integrates the
/// rest of its band from there. Within a lobe the phase is flat (a centred frame), so no
/// group-delay correction is needed for the bins of a partial (a noisy one only hurt).
pub(crate) struct Master<'a> {
    pub phase: &'a [C],
    pub omega: &'a [f32],
    pub clock: i64,
    pub bin_hz: f32,
    pub k_lo: usize,
    pub k_hi: usize,
}

/// A transient gate for one frame: the analysis frame keeps only what is before (`Pre`) or
/// after (`Post`) the attack at `at` (input sample).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Gate {
    pub pre: Option<i64>,
    pub post: Option<i64>,
    pub ramp: usize,
}

/// Formant processing of a frame: the log envelope of the source (`log_env`, bins of
/// `env_bin_hz`), the pitch ratio's formant target, preservation and shift.
pub(crate) struct FormantMap<'a> {
    pub log_env: &'a [f32],
    pub env_bin_hz: f32,
    /// Output frequency `ν` takes the envelope at `ν / target` (`p^(1-amount) · shift`).
    pub target: f32,
}

impl FormantMap<'_> {
    #[inline]
    fn at(&self, hz: f32) -> f32 {
        let x = (hz / self.env_bin_hz).max(0.0);
        let i = x as usize;
        let last = self.log_env.len() - 1;
        if i >= last {
            return self.log_env[last];
        }
        let f = x - i as f32;
        self.log_env[i] * (1.0 - f) + self.log_env[i + 1] * f
    }
}

pub(crate) struct FrameArgs<'a> {
    pub input: &'a super::InputRing,
    /// Analysis centre (input samples) and synthesis centre (output samples).
    pub center: f64,
    pub clock: i64,
    /// Outputs before this are final (not written).
    pub out_from: i64,
    /// Pitch ratio.
    pub pitch: f32,
    pub gate: Gate,
    /// Reset rising bins' phases (first frame at/after an onset).
    pub reset: bool,
    pub formants: Option<FormantMap<'a>>,
    /// Compute the full-band energy magnitudes (`energy()`) for the envelope estimator.
    pub want_energy: bool,
    /// The resolution to follow in the crossover band (`PathSpec::follow`).
    pub master: Option<Master<'a>>,
}

pub(crate) struct Path {
    pub spec: PathSpec,
    n: usize,
    bins: usize,
    sr: f32,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    scratch: Vec<C>,
    mask: Vec<f32>,
    /// Output bins with a non-zero mask.
    k_lo: usize,
    k_hi: usize,
    /// The band at pitch ratio 1 (`mask1`, `k_lo1..k_hi1`, `copy1`). Partials must be
    /// resolved both where they are analysed and where they are synthesised, and the tighter
    /// spacing is the lower of the two: shifting up, the resolutions split the *source*
    /// spectrum (the output band is the source band times `p`: transposing a low voice up an
    /// octave, its partials still come from the long frames); shifting down, the output one
    /// (bands in place). `mask`, `k_lo..k_hi`, `copy`: the current band ([`Path::set_pitch`]).
    mask1: Vec<f32>,
    k_lo1: usize,
    k_hi1: usize,
    copy1: (usize, usize),
    band_pitch: f32,
    time: Vec<f32>,
    /// The frame in time order, and its gate.
    frame: Vec<f32>,
    gate: Vec<f32>,
    /// Synthesis window scaled by 1/N (the inverse FFT is unnormalised), and the analysis ×
    /// synthesis window (for the normalisation).
    wsyn: Vec<f32>,
    w2: Vec<f32>,
    rot: Vec<C>,
    unit: Vec<C>,
    rect: Vec<Vec<C>>,
    x: Vec<Vec<C>>,
    sum_r: Vec<C>,
    xr: Vec<C>,
    emag: Vec<f32>,
    omega_a: Vec<f32>,
    // Output bins.
    mag_o: Vec<f32>,
    omega_o: Vec<f32>,
    tau_o: Vec<f32>,
    gain: Vec<f32>,
    /// Synthesis phases (unit phasors).
    phase: Vec<C>,
    /// Output bin -> source bin, and the magnitude factor (0: silent bin).
    src: Vec<u32>,
    srcf: Vec<f32>,
    amp: Vec<f32>,
    /// Analysis (reference) phase of each output bin's source (unit phasor).
    ana_o: Vec<C>,
    // Partial model (`model_pitch`).
    lobe_tab: Vec<C>,
    res: Vec<Vec<C>>,
    model: Vec<C>,
    dom: Vec<u32>,
    peak_b: Vec<f32>,
    peak_a: Vec<C>,
    peak_ref: Vec<C>,
    peak_rot: Vec<C>,
    /// Each partial's output lobe: its bins `out_span` and values `out_lobe` (`LOBE_SPAN` each).
    out_span: Vec<(u32, u32)>,
    out_lobe: Vec<C>,
    npk: usize,
    peaks: Vec<u32>,
    prev_mag: Vec<f32>,
    prev_omega: Vec<f32>,
    prev_phase: Vec<C>,
    prev_ana: Vec<C>,
    prev_ana_valid: bool,
    prev_center: i64,
    primed: bool,
    last_clock: i64,
    /// Output bins following the master (crossover band; empty without `follow`).
    copy: (usize, usize),
    /// Synthesis hop (samples).
    synth_hop_samples: i64,
    integrator: PhaseIntegrator,
    synth: Vec<C>,
    /// Overlap-add accumulators (per channel) and window sums, rings of `out_mask + 1`.
    pub acc: Vec<Vec<f32>>,
    pub norm: Vec<f32>,
    out_mask: usize,
    /// Nominal window sum (Hann² at the path's overlap).
    pub nominal: f32,
}

impl Path {
    /// Non-RT.
    pub(crate) fn new(
        spec: PathSpec,
        sr: f32,
        channels: usize,
        tick: usize,
        out_cap: usize,
    ) -> Self {
        let n = spec.n;
        let bins = n / 2 + 1;
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        let scratch = vec![C::default(); fwd.get_scratch_len().max(inv.get_scratch_len())];
        let window: Vec<f32> = (0..n)
            .map(|i| {
                let m = i as f32 - (n / 2) as f32;
                0.5 + 0.5 * (TAU * m / n as f32).cos()
            })
            .collect();
        let wsyn: Vec<f32> = window.iter().map(|w| w / n as f32).collect();
        let w2: Vec<f32> = window.iter().map(|w| w * w).collect();
        let bin_hz = sr / n as f32;
        let edge = |hz: f32, (a, b): (f32, f32)| -> f32 {
            // 0 at a, 1 at b, raised cosine in log frequency.
            if hz <= a {
                0.0
            } else if hz >= b {
                1.0
            } else {
                let x = (hz / a).ln() / (b / a).ln();
                0.5 - 0.5 * (PI * x).cos()
            }
        };
        let mask: Vec<f32> = (0..bins)
            .map(|k| {
                let hz = k as f32 * bin_hz;
                let lo = spec.lo.map_or(1.0, |e| edge(hz, e));
                let hi = spec.hi.map_or(1.0, |e| 1.0 - edge(hz, e));
                lo * hi
            })
            .collect();
        let k_lo = mask.iter().position(|m| *m > 0.0).unwrap_or(bins);
        let k_hi = mask.iter().rposition(|m| *m > 0.0).map_or(k_lo, |k| k + 1);
        // Crossover bins (mask below 1 on the followed edge).
        let copy = match spec.follow {
            Some(Edge::Lo) => (k_lo, (k_lo..k_hi).find(|k| mask[*k] >= 1.0).unwrap_or(k_hi)),
            None => (0, 0),
        };
        let hop = (spec.hop_ticks * tick) as f32;
        // Σ Hann² over shifts of `hop`: 3N/(8 hop).
        let nominal = 3.0 * n as f32 / (8.0 * hop);
        let cv = || vec![C::default(); bins];
        let fv = || vec![0.0f32; bins];
        Self {
            spec,
            n,
            bins,
            sr,
            fwd,
            inv,
            scratch,
            mask: mask.clone(),
            k_lo,
            k_hi,
            mask1: mask,
            k_lo1: k_lo,
            k_hi1: k_hi,
            copy1: copy,
            band_pitch: 1.0,
            time: vec![0.0; n],
            frame: vec![0.0; n],
            gate: vec![1.0; n],
            wsyn,
            w2,
            rot: cv(),
            unit: cv(),
            rect: (0..channels).map(|_| cv()).collect(),
            x: (0..channels).map(|_| cv()).collect(),
            sum_r: cv(),
            xr: cv(),
            emag: fv(),
            omega_a: fv(),
            mag_o: fv(),
            omega_o: fv(),
            tau_o: fv(),
            gain: vec![1.0; bins],
            phase: cv(),
            src: vec![0; bins],
            srcf: fv(),
            amp: fv(),
            ana_o: cv(),
            lobe_tab: lobe_table(n),
            res: (0..channels).map(|_| cv()).collect(),
            model: cv(),
            dom: vec![NO_PEAK; bins],
            peak_b: vec![0.0; bins],
            peak_a: vec![C::default(); bins * channels],
            peak_ref: cv(),
            peak_rot: cv(),
            out_span: vec![(0, 0); bins],
            out_lobe: vec![C::default(); bins * LOBE_SPAN],
            npk: 0,
            peaks: Vec::with_capacity(bins),
            prev_mag: fv(),
            prev_omega: fv(),
            prev_phase: cv(),
            prev_ana: cv(),
            prev_ana_valid: false,
            prev_center: 0,
            primed: false,
            last_clock: 0,
            copy,
            synth_hop_samples: (spec.hop_ticks * tick) as i64,
            integrator: PhaseIntegrator::new(bins),
            synth: cv(),
            acc: (0..channels).map(|_| vec![0.0; out_cap]).collect(),
            norm: vec![0.0; out_cap],
            out_mask: out_cap - 1,
            nominal,
        }
    }

    pub(crate) fn n(&self) -> usize {
        self.n
    }

    pub(crate) fn bin_hz(&self) -> f32 {
        self.sr / self.n as f32
    }

    /// This resolution's last frame, for the followers ([`Master`]).
    pub(crate) fn as_master(&self) -> Option<Master<'_>> {
        self.primed.then(|| Master {
            phase: &self.prev_phase,
            omega: &self.prev_omega,
            clock: self.last_clock,
            bin_hz: self.bin_hz(),
            k_lo: self.k_lo,
            k_hi: self.k_hi,
        })
    }

    /// Full-band energy magnitudes of the last frame (valid with `want_energy`).
    pub(crate) fn energy(&self) -> &[f32] {
        &self.emag
    }

    /// RT.
    pub(crate) fn reset(&mut self) {
        for a in &mut self.acc {
            a.fill(0.0);
        }
        self.norm.fill(0.0);
        self.primed = false;
        self.prev_ana_valid = false;
        self.prev_mag.fill(0.0);
    }

    /// Gate value at frame offset `m` (samples from the centre).
    #[inline]
    fn gate_at(g: &Gate, ci: i64, m: i64) -> f32 {
        let mut v = 1.0;
        let r = g.ramp as i64;
        if let Some(at) = g.pre {
            let e = at - ci;
            if m >= e {
                return 0.0;
            }
            if m > e - r {
                v *= 0.5 + 0.5 * (PI * (m - (e - r)) as f32 / r as f32).cos();
            }
        }
        if let Some(at) = g.post {
            let s = at - ci;
            if m < s - r {
                return 0.0;
            }
            if m < s {
                v *= 0.5 - 0.5 * (PI * (m - (s - r)) as f32 / r as f32).cos();
            }
        }
        v
    }

    /// RT. Move the band for pitch ratio `p` (see `mask1`).
    fn set_pitch(&mut self, p: f32) {
        // Shifting down, the output partials are the closer ones: the bands stay in place.
        let p = p.max(1.0);
        if p == self.band_pitch {
            return;
        }
        self.band_pitch = p;
        let bins = self.bins;
        if p == 1.0 {
            self.mask.copy_from_slice(&self.mask1);
            (self.k_lo, self.k_hi, self.copy) = (self.k_lo1, self.k_hi1, self.copy1);
            return;
        }
        let last = self.mask1[bins - 1];
        for k in 0..bins {
            let x = k as f32 / p;
            let i = x as usize;
            self.mask[k] = if i + 1 < bins {
                let f = x - i as f32;
                self.mask1[i] * (1.0 - f) + self.mask1[i + 1] * f
            } else {
                last
            };
        }
        self.k_lo = self.mask.iter().position(|m| *m > 0.0).unwrap_or(bins);
        self.k_hi = self
            .mask
            .iter()
            .rposition(|m| *m > 0.0)
            .map_or(self.k_lo, |k| k + 1);
        let scale = |k: usize| ((k as f32 * p).round() as usize).clamp(self.k_lo, self.k_hi);
        self.copy = if self.copy1.0 < self.copy1.1 {
            (scale(self.copy1.0), scale(self.copy1.1))
        } else {
            (0, 0)
        };
    }

    /// RT. Analyse, process and overlap-add one frame.
    pub(crate) fn frame(&mut self, a: &FrameArgs<'_>) {
        self.set_pitch(a.pitch);
        let n = self.n;
        let half = (n / 2) as i64;
        let bins = self.bins;
        let channels = self.acc.len();
        let ci = a.center.round() as i64;
        let gated = a.gate.pre.is_some() || a.gate.post.is_some();
        let p = a.pitch;

        // Source bins needed: [s_lo, s_hi) (kernels read ±1).
        let (s_lo, s_hi) = if a.want_energy {
            (0, bins)
        } else {
            let lo = ((self.k_lo as f32 / p).floor() as usize).saturating_sub(2);
            let hi = (((self.k_hi as f32 / p).ceil() as usize) + 3).min(bins);
            (lo.min(bins), hi)
        };

        // 1. Analysis. The frame is centred (sample `ci + m` at FFT index `m mod N`), so
        // phases are relative to the frame centre.
        if gated {
            for (i, g) in self.gate.iter_mut().enumerate() {
                *g = Self::gate_at(&a.gate, ci, i as i64 - half);
            }
        }
        let h = n / 2;
        for ch in 0..channels {
            a.input.read(ch, ci - half, &mut self.frame);
            if gated {
                for (v, g) in self.frame.iter_mut().zip(&self.gate) {
                    *v *= g;
                }
            }
            self.time[..h].copy_from_slice(&self.frame[h..]);
            self.time[h..].copy_from_slice(&self.frame[..h]);
            let _ = self.fwd.process_with_scratch(
                &mut self.time,
                &mut self.rect[ch],
                &mut self.scratch,
            );
        }
        let neighbours = |s: &[C], k: usize| -> (C, C) {
            let below = if k == 0 { s[1].conj() } else { s[k - 1] };
            let above = if k + 1 >= s.len() {
                s[s.len() - 2].conj()
            } else {
                s[k + 1]
            };
            (below, above)
        };
        let hann = |s: &[C], k: usize| {
            let (b, u) = neighbours(s, k);
            s[k] * 0.5 + (b + u) * 0.25
        };
        let dhann_scale = PI / (2.0 * n as f32);
        let dhann = |s: &[C], k: usize| {
            let (b, u) = neighbours(s, k);
            (b - u) * C::new(0.0, dhann_scale)
        };
        for k in s_lo..s_hi {
            let mut r = C::default();
            let mut e = 0.0f32;
            for ch in 0..channels {
                let xk = hann(&self.rect[ch], k);
                self.x[ch][k] = xk;
                e += xk.norm_sqr();
                r += self.rect[ch][k];
            }
            self.sum_r[k] = r;
            self.emag[k] = e.sqrt();
        }
        let bin_step = TAU / n as f32;
        let mut peak = 0.0f32;
        for k in s_lo..s_hi {
            // Reference: channel sum (kernels on the summed FFTs), or the loudest channel
            // where the sum cancels.
            let mut xr = C::default();
            for ch in 0..channels {
                xr += self.x[ch][k];
            }
            let (xr, xd) =
                if channels > 1 && xr.norm_sqr() < CANCEL * CANCEL * self.emag[k] * self.emag[k] {
                    let loud = (0..channels)
                        .max_by(|a, b| {
                            self.x[*a][k]
                                .norm_sqr()
                                .total_cmp(&self.x[*b][k].norm_sqr())
                        })
                        .unwrap_or(0);
                    (self.x[loud][k], dhann(&self.rect[loud], k))
                } else {
                    (xr, dhann(&self.sum_r, k))
                };
            self.xr[k] = xr;
            let p2 = xr.norm_sqr();
            let wk = bin_step * k as f32;
            if p2 > 1e-24 {
                let dw = -(xd * xr.conj()).im / p2;
                self.omega_a[k] = wk + dw.clamp(-2.5 * bin_step, 2.5 * bin_step);
            } else {
                self.omega_a[k] = wk;
            }
            peak = peak.max(self.emag[k]);
        }

        // Silence: nothing to synthesise (the window sum still counts, so neighbours
        // normalise correctly), and the next frame starts fresh.
        if peak < 1e-9 {
            self.add_norm(a, ci);
            self.primed = false;
            self.prev_ana_valid = false;
            return;
        }

        // 3. Output bins: pitch map (`src`, `amp`; transposing, the partial model) and formant
        // gain.
        let (k_lo, k_hi) = (self.k_lo, self.k_hi);
        let modeled = p != 1.0;
        if modeled {
            self.model_pitch(p, bin_step, s_lo, s_hi);
        } else {
            self.map_identity();
        }
        let hz_per_bin = self.sr / n as f32;
        let (mut plain_energy, mut formant_energy) = (0.0f32, 0.0f32);
        for k in k_lo..k_hi {
            if self.amp[k] == 0.0 {
                self.mag_o[k] = 0.0;
                self.omega_o[k] = bin_step * k as f32;
                self.tau_o[k] = 0.0;
                self.gain[k] = 0.0;
                continue;
            }
            let j = self.src[k] as usize;
            let jf = self.srcf[k];
            let em = if modeled {
                self.mag_o[k]
            } else {
                let i = (jf as usize).min(bins - 2);
                let f = jf - i as f32;
                self.emag[i] * (1.0 - f) + self.emag[i + 1] * f
            };
            let mut g = self.amp[k];
            if let Some(f) = &a.formants {
                let hz = k as f32 * hz_per_bin;
                // Fade the correction in above the lowest frequencies, where formants don't
                // matter and the envelope (mirrored around DC) is unreliable.
                let fade = ((hz - FORMANT_FADE_HZ.0) / (FORMANT_FADE_HZ.1 - FORMANT_FADE_HZ.0))
                    .clamp(0.0, 1.0);
                let ln = f.at(hz / f.target) - f.at(jf * hz_per_bin);
                let lim = |db: f32| db / 20.0 * std::f32::consts::LN_10;
                let ln = ln.clamp(lim(GAIN_MIN_DB), lim(GAIN_MAX_DB)) * fade;
                plain_energy += (em * g) * (em * g);
                g *= ln.exp();
                formant_energy += (em * g) * (em * g);
            }
            self.gain[k] = g;
            self.mag_o[k] = em * g;
            if !modeled {
                self.omega_o[k] = self.omega_a[j];
            }
        }
        // The formant correction redistributes the frame's energy; bound how much it changes
        // its total (a sparse spectrum, e.g. a lone partial, has no meaningful envelope).
        if a.formants.is_some() && plain_energy > 0.0 && formant_energy > 0.0 {
            let ratio = formant_energy / plain_energy;
            let lim = |db: f32| 10f32.powf(db / 10.0);
            let fix =
                (ratio.clamp(lim(FORMANT_ENERGY_DB.0), lim(FORMANT_ENERGY_DB.1)) / ratio).sqrt();
            if fix != 1.0 {
                for k in k_lo..k_hi {
                    self.gain[k] *= fix;
                    self.mag_o[k] *= fix;
                }
            }
        }

        // 4. Phase.
        let exact = self.primed
            && p == 1.0
            && self.prev_ana_valid
            && (ci - self.prev_center) == self.synth_hop_samples;
        if !modeled {
            for k in k_lo..k_hi {
                self.ana_o[k] = unit(self.xr[self.src[k] as usize]);
            }
        }
        let ana_phase = |s: &Self, k: usize| -> C { s.ana_o[k] };
        // Crossover bins follow the master. Not on a first frame (every resolution starts from
        // its analysis phases, which agree at the source's speed), nor when reproducing the
        // input exactly (they still agree).
        let mut fixed = (0, 0);
        if let Some(m) = a.master.as_ref().filter(|_| !exact && self.primed) {
            let (c0, c1) = self.copy;
            let dt = (a.clock - m.clock) as f32;
            for k in c0..c1 {
                // The master bin at this bin's instantaneous frequency: the centre of the
                // partial dominating it (a lobe centre has a meaningful phase; the master bin
                // at the same centre frequency may be a valley between finer-resolved partials).
                let at = self.omega_o[k] * self.sr / TAU;
                let km = ((at / m.bin_hz).round().max(0.0) as usize)
                    .clamp(m.k_lo, m.k_hi.max(m.k_lo + 1) - 1);
                self.phase[k] = m.phase[km] * cis(m.omega[km] * dt);
            }
            fixed = (c0, c1);
        }
        if !self.primed {
            for k in k_lo..k_hi {
                if k < fixed.0 || k >= fixed.1 {
                    self.phase[k] = ana_phase(self, k);
                }
            }
        } else if exact {
            for k in k_lo..k_hi {
                let z = self.prev_phase[k] * self.ana_o[k] * self.prev_ana[k].conj();
                self.phase[k] = z * (1.5 - 0.5 * z.norm_sqr());
            }
        } else {
            self.integrator.integrate(
                k_lo,
                k_hi,
                fixed,
                self.synth_hop_samples as f32,
                bin_step,
                PHASE_TOL,
                &Gradient {
                    mag: &self.mag_o,
                    omega: &self.omega_o,
                    tau: &self.tau_o,
                    ana: Some(&self.ana_o),
                },
                &Previous {
                    mag: &self.prev_mag,
                    omega: &self.prev_omega,
                    phase: &self.prev_phase,
                },
                &mut self.phase,
            );
        }
        if a.reset && self.primed {
            for k in k_lo..k_hi {
                if (k < fixed.0 || k >= fixed.1) && self.mag_o[k] > RESET_RISE * self.prev_mag[k] {
                    self.phase[k] = ana_phase(self, k);
                }
            }
        }

        // 5. Synthesis per channel: the bin's phase and gain (`rot`) and the reference's
        // conjugate unit phasor (`unit`) are shared; each channel keeps its offset to the
        // reference (its analysed value times `unit`), at its interpolated magnitude.
        for k in k_lo..k_hi {
            let g = self.gain[k] * self.mask[k];
            if g == 0.0 {
                self.rot[k] = C::default();
                continue;
            }
            self.rot[k] = self.phase[k] * g;
            self.unit[k] = self.ana_o[k].conj();
        }
        let lo_out = (a.out_from - (a.clock - half)).clamp(0, n as i64) as usize;
        if modeled {
            // Each partial turns by its centre bin's synthesis phase (relative to its modelled
            // analysis phase there); the residual by its bin's.
            for i in 0..self.npk {
                let kc = (self.peak_b[i] * p).round();
                self.peak_rot[i] =
                    if kc >= k_lo as f32 && kc < k_hi as f32 && self.dom[kc as usize] == i as u32 {
                        let kc = kc as usize;
                        self.phase[kc] * self.ana_o[kc].conj()
                    } else {
                        C::default()
                    };
            }
            for k in k_lo..k_hi {
                self.rot[k] = self.phase[k] * self.ana_o[k].conj() * (self.gain[k] * self.mask[k]);
            }
        }
        for ch in 0..channels {
            self.synth.fill(C::default());
            if modeled {
                self.synth_modeled(ch, p);
            }
            let xc = &self.x[ch];
            for k in k_lo..k_hi {
                if modeled {
                    break;
                }
                if self.rot[k] == C::default() {
                    continue;
                }
                let r = self.src[k] as usize;
                let jf = self.srcf[k];
                let z = xc[r] * self.unit[k];
                self.synth[k] = if jf == r as f32 {
                    // On the bin: |z| is the magnitude.
                    z * self.rot[k]
                } else {
                    let i0 = (jf as usize).min(bins - 2);
                    let fr = jf - i0 as f32;
                    let m =
                        xc[i0].norm_sqr().sqrt() * (1.0 - fr) + xc[i0 + 1].norm_sqr().sqrt() * fr;
                    let zn = z.norm_sqr().sqrt();
                    let u = if zn > 1e-30 { z / zn } else { C::new(1.0, 0.0) };
                    u * self.rot[k] * m
                };
            }
            self.synth[0].im = 0.0;
            self.synth[bins - 1].im = 0.0;
            let _ =
                self.inv
                    .process_with_scratch(&mut self.synth, &mut self.time, &mut self.scratch);
            // Frame sample i (offset i - N/2) is FFT index (i + N/2) mod N.
            let acc = &mut self.acc[ch];
            let base = a.clock - half;
            for i in lo_out..n {
                let idx = if i < h { i + h } else { i - h };
                acc[(base + i as i64) as usize & self.out_mask] += self.time[idx] * self.wsyn[i];
            }
        }
        self.add_norm(a, ci);

        // Keep for the next frame.
        std::mem::swap(&mut self.prev_mag, &mut self.mag_o);
        self.prev_omega[k_lo..k_hi].copy_from_slice(&self.omega_o[k_lo..k_hi]);
        self.prev_phase[k_lo..k_hi].copy_from_slice(&self.phase[k_lo..k_hi]);
        if p == 1.0 {
            self.prev_ana[k_lo..k_hi].copy_from_slice(&self.ana_o[k_lo..k_hi]);
            self.prev_ana_valid = true;
        } else {
            self.prev_ana_valid = false;
        }
        self.prev_center = ci;
        self.last_clock = a.clock;
        self.primed = true;
    }

    /// Transposing (`p != 1`): the source spectrum as windowed sinusoids plus a residual
    /// (McAulay & Quatieri's sinusoidal model, Serra's deterministic + stochastic split, on the
    /// phase vocoder's own frames). Each peak (strongest first) gets the amplitude that best
    /// fits its main lobe (least squares on the 3 bins around it, at its reassigned
    /// frequency), and its whole lobe, side lobes included, is subtracted. The partials are
    /// then synthesised at `p` times their frequency with the exact Hann lobe shape
    /// ([`Path::lobe`]), so the valleys between them come out as deep as the output spacing
    /// makes them (copying source bins would carry the source's shallower valleys, lifted
    /// by close neighbours' side lobes); the residual (noise, transients, what the model
    /// misses) is bin-scaled (`k` reads `k/p`). Fills, per output bin: `mag_o` (before the
    /// formant gain), `omega_o`, `ana_o`, the residual's source bin `src`, the source
    /// position for the formants `srcf`, the dominant partial `dom`.
    fn model_pitch(&mut self, p: f32, bin_step: f32, s_lo: usize, s_hi: usize) {
        let (k_lo, k_hi) = (self.k_lo, self.k_hi);
        let channels = self.acc.len();
        let lobe = MODEL_LOBE as f32;
        for ch in 0..channels {
            self.res[ch][s_lo..s_hi].copy_from_slice(&self.x[ch][s_lo..s_hi]);
        }
        // Peaks, strongest first.
        self.peaks.clear();
        let e = &self.emag;
        let max = e[s_lo..s_hi].iter().fold(0.0f32, |m, v| m.max(*v));
        let floor = max * MODEL_FLOOR;
        for j in (s_lo + 1)..s_hi.saturating_sub(1) {
            if e[j] > floor
                && e[j] >= e[j - 1]
                && e[j] > e[j + 1]
                && e[j]
                    > MODEL_PROMINENCE
                        * e[j.saturating_sub(2).max(s_lo)..(j + 3).min(s_hi)]
                            .iter()
                            .fold(f32::MAX, |m, v| m.min(*v))
            {
                self.peaks.push(j as u32);
            }
        }
        self.peaks
            .sort_unstable_by(|a, b| e[*b as usize].total_cmp(&e[*a as usize]));
        self.npk = 0;
        for pi in 0..self.peaks.len() {
            let pk = self.peaks[pi] as usize;
            let b = self.omega_a[pk] / bin_step;
            if (b - pk as f32).abs() > 1.0 || pk < s_lo + 1 || pk + 2 > s_hi {
                continue;
            }
            // Least-squares amplitude on the main lobe's 3 bins, per channel.
            let w3 = [-1.0f32, 0.0, 1.0].map(|o| self.lobe(pk as f32 + o - b));
            let den: f32 = w3.iter().map(|w| w.norm_sqr()).sum();
            if den <= 0.0 {
                continue;
            }
            let i = self.npk;
            let mut aref = C::default();
            for ch in 0..channels {
                let mut num = C::default();
                for (o, w) in w3.iter().enumerate() {
                    num += self.res[ch][pk - 1 + o] * w.conj();
                }
                let a = num / den;
                self.peak_a[i * channels + ch] = a;
                aref += a;
            }
            // Subtract the whole lobe.
            let j0 = ((b - lobe).ceil().max(s_lo as f32)) as usize;
            let j1 = ((b + lobe).floor() as usize + 1).min(s_hi);
            for j in j0..j1 {
                let w = self.lobe(j as f32 - b);
                for ch in 0..channels {
                    let a = self.peak_a[i * channels + ch];
                    self.res[ch][j] -= a * w;
                }
            }
            self.peak_b[i] = b;
            self.peak_ref[i] = aref;
            self.npk += 1;
        }
        // Refine: each partial re-fitted with the others removed (overlapping lobes bias the
        // first, greedy estimates): its frequency by a Gauss-Newton step on the reference
        // (amplitude and lobe derivative fitted on 5 bins), then its amplitudes.
        for _ in 0..MODEL_REFINE {
            for i in 0..self.npk {
                let b = self.peak_b[i];
                let pk = b.round() as usize;
                if pk < s_lo + 2 || pk + 3 > s_hi {
                    continue;
                }
                // Reference with this partial put back.
                let (mut g11, mut g12, mut g22) = (0.0f32, C::default(), 0.0f32);
                let (mut h1, mut h2) = (C::default(), C::default());
                for j in pk - 2..=pk + 2 {
                    let d = j as f32 - b;
                    let w = self.lobe(d);
                    let dw = (self.lobe(d + 0.01) - self.lobe(d - 0.01)) * 50.0;
                    let mut r = C::default();
                    for ch in 0..channels {
                        r += self.res[ch][j] + self.peak_a[i * channels + ch] * w;
                    }
                    g11 += w.norm_sqr();
                    g12 += w.conj() * dw;
                    g22 += dw.norm_sqr();
                    h1 += w.conj() * r;
                    h2 += dw.conj() * r;
                }
                // [g11 g12; g12* g22] [a; c] = [h1; h2]; r ≈ a·W(j-b) + c·W'(j-b), and a lobe
                // at b + δ is ≈ a·W − aδ·W' (W' = dW/dd), so δ = −Re(c/a).
                let det = g11 * g22 - g12.norm_sqr();
                let mut nb = b;
                if det > 1e-20 {
                    let a = (h1 * g22 - g12 * h2) / det;
                    let c = (h2 * g11 - g12.conj() * h1) / det;
                    if a.norm_sqr() > 0.0 {
                        nb = b + (-(c / a).re).clamp(-0.25, 0.25);
                    }
                }
                let j0 = ((b.min(nb) - lobe).ceil().max(s_lo as f32)) as usize;
                let j1 = ((b.max(nb) + lobe).floor() as usize + 1).min(s_hi);
                // The lobe at the old and the new frequency over the bins it touches.
                let len = (j1 - j0).min(LOBE_SPAN);
                let mut wo = [C::default(); LOBE_SPAN];
                let mut wn = [C::default(); LOBE_SPAN];
                for o in 0..len {
                    let j = (j0 + o) as f32;
                    wo[o] = self.lobe(j - b);
                    wn[o] = self.lobe(j - nb);
                }
                let at = |j: usize| j - j0;
                let den: f32 = (pk - 1..=pk + 1).map(|j| wn[at(j)].norm_sqr()).sum();
                if den <= 0.0 {
                    continue;
                }
                let mut aref = C::default();
                for ch in 0..channels {
                    let old = self.peak_a[i * channels + ch];
                    let mut num = C::default();
                    for j in pk - 1..=pk + 1 {
                        num += (self.res[ch][j] + old * wo[at(j)]) * wn[at(j)].conj();
                    }
                    let a = num / den;
                    for o in 0..len {
                        self.res[ch][j0 + o] -= a * wn[o] - old * wo[o];
                    }
                    self.peak_a[i * channels + ch] = a;
                    aref += a;
                }
                self.peak_b[i] = nb;
                self.peak_ref[i] = aref;
            }
        }
        // The partials' lobes at their new frequencies (`model`: their sum on the reference,
        // `gain` as scratch: the strongest one's magnitude per bin).
        self.dom[k_lo..k_hi].fill(NO_PEAK);
        self.gain[k_lo..k_hi].fill(0.0);
        self.model[k_lo..k_hi].fill(C::default());
        for i in 0..self.npk {
            let pos = self.peak_b[i] * p;
            let k0 = ((pos - lobe).ceil().max(k_lo as f32)) as usize;
            let k1 = (((pos + lobe).floor() + 1.0).max(0.0) as usize)
                .min(k_hi)
                .max(k0);
            let k1 = k1.min(k0 + LOBE_SPAN);
            self.out_span[i] = (k0 as u32, k1 as u32);
            for k in k0..k1 {
                let w = self.lobe(k as f32 - pos);
                self.out_lobe[i * LOBE_SPAN + (k - k0)] = w;
                let c = self.peak_ref[i] * w;
                self.model[k] += c;
                let m = c.norm_sqr();
                if m > self.gain[k] {
                    self.gain[k] = m;
                    self.dom[k] = i as u32;
                }
            }
        }
        let bins = self.bins;
        for k in k_lo..k_hi {
            let q = k as f32 / p;
            let r = (q.round() as usize)
                .clamp(s_lo, s_hi.max(s_lo + 1) - 1)
                .min(bins - 1);
            let inside = q < (s_hi as f32) && r >= s_lo;
            let mut rref = C::default();
            if inside {
                for ch in 0..channels {
                    rref += self.res[ch][r];
                }
            }
            // Bin-scaled, the residual keeps its power density: × 1/√p in amplitude.
            let rscale = p.recip().sqrt();
            let (mr, mm) = (rref.norm_sqr() * rscale * rscale, self.model[k].norm_sqr());
            self.mag_o[k] = (mr + mm).sqrt();
            self.src[k] = r as u32;
            self.amp[k] = if inside || mm > 0.0 { 1.0 } else { 0.0 };
            let d = self.dom[k];
            if d != NO_PEAK && self.gain[k] >= mr {
                let d = d as usize;
                let k0 = self.out_span[d].0 as usize;
                self.srcf[k] = self.peak_b[d];
                self.omega_o[k] = self.peak_b[d] * bin_step * p;
                self.ana_o[k] = unit(self.peak_ref[d] * self.out_lobe[d * LOBE_SPAN + (k - k0)]);
            } else {
                self.dom[k] = NO_PEAK;
                self.srcf[k] = q.min((bins - 1) as f32);
                self.omega_o[k] = self.omega_a[r] * p;
                self.ana_o[k] = unit(rref);
            }
        }
    }

    /// RT. Channel `ch`'s transposed spectrum (`model_pitch`): the residual at each bin,
    /// turned like its bin, plus every partial's lobe at its new frequency, turned like its
    /// centre bin.
    fn synth_modeled(&mut self, ch: usize, p: f32) {
        let (k_lo, k_hi) = (self.k_lo, self.k_hi);
        let channels = self.acc.len();
        let rscale = p.recip().sqrt();
        for k in k_lo..k_hi {
            if self.amp[k] != 0.0 {
                self.synth[k] = self.res[ch][self.src[k] as usize] * (self.rot[k] * rscale);
            }
        }
        for i in 0..self.npk {
            let turn = self.peak_rot[i];
            if turn == C::default() {
                continue;
            }
            let a = self.peak_a[i * channels + ch] * turn;
            let (k0, k1) = self.out_span[i];
            for k in k0 as usize..k1 as usize {
                let g = self.gain[k] * self.mask[k];
                self.synth[k] += a * self.out_lobe[i * LOBE_SPAN + (k - k0 as usize)] * g;
            }
        }
    }

    /// The Hann spectrum of a unit complex sinusoid `d` bins away (the analysis's centred
    /// frame), from the table built in `new`.
    #[inline]
    fn lobe(&self, d: f32) -> C {
        let x = (d + (MODEL_LOBE + 1) as f32) * LOBE_STEPS as f32;
        if x < 0.0 || x >= (self.lobe_tab.len() - 1) as f32 {
            return C::default();
        }
        let i = x as usize;
        let f = x - i as f32;
        self.lobe_tab[i] * (1.0 - f) + self.lobe_tab[i + 1] * f
    }

    /// Not transposing: each output bin reads its own source bin.
    fn map_identity(&mut self) {
        for k in self.k_lo..self.k_hi {
            self.src[k] = k as u32;
            self.srcf[k] = k as f32;
            self.amp[k] = 1.0;
        }
    }

    fn add_norm(&mut self, a: &FrameArgs<'_>, _ci: i64) {
        let n = self.n;
        let half = (n / 2) as i64;
        let gated = a.gate.pre.is_some() || a.gate.post.is_some();
        let base = a.clock - half;
        let lo_out = (a.out_from - base).clamp(0, n as i64) as usize;
        for i in lo_out..n {
            let w2 = self.w2[i];
            let v = if gated { w2 * self.gate[i] } else { w2 };
            self.norm[(base + i as i64) as usize & self.out_mask] += v;
        }
    }

    /// RT. Take (and clear) output sample `o` of channel `ch`, normalised.
    #[inline]
    pub(crate) fn take(&mut self, ch: usize, o: i64, last_channel: bool) -> f32 {
        let i = o as usize & self.out_mask;
        let v = self.acc[ch][i];
        self.acc[ch][i] = 0.0;
        let norm = self.norm[i].max(NORM_FLOOR * self.nominal);
        if last_channel {
            self.norm[i] = 0.0;
        }
        v / norm
    }
}

/// Non-RT. [`Path::lobe`]'s table: `W(d) = Σ w[m]·e^{-2πi·d·m/N}` over the centred Hann
/// frame (`w = ½ + ¼e^{2πim/N} + ¼e^{-2πim/N}`), i.e. `½D(d) + ¼D(d−1) + ¼D(d+1)` with the
/// Dirichlet kernel `D(d) = Σ_{m=-N/2}^{N/2-1} e^{-2πi·d·m/N}`, for `d` within
/// `±(MODEL_LOBE + 1)` bins.
fn lobe_table(n: usize) -> Vec<C> {
    use std::f64::consts::PI;
    let nf = n as f64;
    let dirichlet = |d: f64| -> (f64, f64) {
        let th = 2.0 * PI * d / nf;
        if th.abs() < 1e-9 {
            return (nf, 0.0);
        }
        // e^{iθN/2}·(1 − e^{−iθN}) / (1 − e^{−iθ}).
        let (a_re, a_im) = ((th * nf / 2.0).cos(), (th * nf / 2.0).sin());
        let (b_re, b_im) = (1.0 - (th * nf).cos(), (th * nf).sin());
        let (c_re, c_im) = (1.0 - th.cos(), th.sin());
        let (num_re, num_im) = (a_re * b_re - a_im * b_im, a_re * b_im + a_im * b_re);
        let den = c_re * c_re + c_im * c_im;
        (
            (num_re * c_re + num_im * c_im) / den,
            (num_im * c_re - num_re * c_im) / den,
        )
    };
    let span = (MODEL_LOBE + 1) as f64;
    let count = (2.0 * span * LOBE_STEPS as f64) as usize + 1;
    (0..count)
        .map(|i| {
            let d = i as f64 / LOBE_STEPS as f64 - span;
            let (r0, i0) = dirichlet(d);
            let (r1, i1) = dirichlet(d - 1.0);
            let (r2, i2) = dirichlet(d + 1.0);
            C::new(
                (0.5 * r0 + 0.25 * (r1 + r2)) as f32,
                (0.5 * i0 + 0.25 * (i1 + i2)) as f32,
            )
        })
        .collect()
}
