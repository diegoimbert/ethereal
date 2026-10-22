//! One resolution of the Complex+ stretcher: a phase vocoder with a fixed FFT size that
//! synthesises one frequency band (the others are left to the other resolutions).
//!
//! Per frame (analysis centred at input sample `c`, synthesis centred at output sample `f`):
//! 1. **Analysis** of every channel through one real FFT of the (gated) rectangular frame:
//!    the Hann spectrum and the Hann-derivative spectrum are 3-tap kernels of it; a second FFT
//!    of the time-weighted frame gives the time-weighted Hann spectrum. From those, the
//!    reassignment estimates (Auger & Flandrin 1995) of instantaneous frequency and group
//!    delay, which don't depend on the analysis hop, so any time ratio works.
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
/// Half-width of a Hann main lobe (bins).
const MAIN_LOBE: usize = 2;
/// Formant gain limits (dB).
const GAIN_MAX_DB: f32 = 24.0;
const GAIN_MIN_DB: f32 = -48.0;
/// Formant correction fades in between these output frequencies (Hz).
const FORMANT_FADE_HZ: (f32, f32) = (100.0, 200.0);
/// Most a frame's energy may change through the formant correction (dB).
const FORMANT_ENERGY_DB: (f32, f32) = (-9.0, 6.0);

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
/// phase, moved to its own frame time with the instantaneous frequency and to its own bin
/// frequency with the group delay (`φ(ω) ≈ φ(ω_m) − τ·(ω − ω_m)`), and integrates the rest
/// of its band from there.
pub(crate) struct Master<'a> {
    pub phase: &'a [C],
    pub omega: &'a [f32],
    pub tau: &'a [f32],
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
    time_w: Vec<f32>,
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
    tw: Vec<Vec<C>>,
    x: Vec<Vec<C>>,
    sum_r: Vec<C>,
    sum_t: Vec<C>,
    xr: Vec<C>,
    emag: Vec<f32>,
    omega_a: Vec<f32>,
    tau_a: Vec<f32>,
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
    /// Output IF = source IF × `wscale` + `wshift` (rad/sample): a region keeps its bins'
    /// offsets to the peak (`1`, the peak's shift); gap bins scale (`p`, 0).
    wscale: Vec<f32>,
    wshift: Vec<f32>,
    /// Analysis (reference) phase of each output bin's source (unit phasor).
    ana_o: Vec<C>,
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
            time_w: vec![0.0; n],
            frame: vec![0.0; n],
            gate: vec![1.0; n],
            wsyn,
            w2,
            rot: cv(),
            unit: cv(),
            rect: (0..channels).map(|_| cv()).collect(),
            tw: (0..channels).map(|_| cv()).collect(),
            x: (0..channels).map(|_| cv()).collect(),
            sum_r: cv(),
            sum_t: cv(),
            xr: cv(),
            emag: fv(),
            omega_a: fv(),
            tau_a: fv(),
            mag_o: fv(),
            omega_o: fv(),
            tau_o: fv(),
            gain: vec![1.0; bins],
            phase: cv(),
            src: vec![0; bins],
            srcf: fv(),
            amp: fv(),
            wscale: vec![1.0; bins],
            wshift: fv(),
            ana_o: cv(),
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
            tau: &self.tau_o,
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
            for i in 0..h {
                self.time_w[i] = self.frame[h + i] * i as f32;
                self.time_w[h + i] = self.frame[i] * (i as f32 - h as f32);
            }
            let _ = self.fwd.process_with_scratch(
                &mut self.time,
                &mut self.rect[ch],
                &mut self.scratch,
            );
            let _ = self.fwd.process_with_scratch(
                &mut self.time_w,
                &mut self.tw[ch],
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
            let mut t = C::default();
            let mut e = 0.0f32;
            for ch in 0..channels {
                let xk = hann(&self.rect[ch], k);
                self.x[ch][k] = xk;
                e += xk.norm_sqr();
                r += self.rect[ch][k];
                t += self.tw[ch][k];
            }
            self.sum_r[k] = r;
            self.sum_t[k] = t;
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
            let (xr, xd, xt) = if channels > 1 && xr.norm() < CANCEL * self.emag[k] {
                let loud = (0..channels)
                    .max_by(|a, b| {
                        self.x[*a][k]
                            .norm_sqr()
                            .total_cmp(&self.x[*b][k].norm_sqr())
                    })
                    .unwrap_or(0);
                (
                    self.x[loud][k],
                    dhann(&self.rect[loud], k),
                    hann(&self.tw[loud], k),
                )
            } else {
                (xr, dhann(&self.sum_r, k), hann(&self.sum_t, k))
            };
            self.xr[k] = xr;
            let p2 = xr.norm_sqr();
            let wk = bin_step * k as f32;
            if p2 > 1e-24 {
                let dw = -(xd * xr.conj()).im / p2;
                self.omega_a[k] = wk + dw.clamp(-2.5 * bin_step, 2.5 * bin_step);
                self.tau_a[k] = ((xt * xr.conj()).re / p2).clamp(-(half as f32), half as f32);
            } else {
                self.omega_a[k] = wk;
                self.tau_a[k] = 0.0;
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

        // 3. Output bins: pitch map (`src`, `amp`) and formant gain.
        let (k_lo, k_hi) = (self.k_lo, self.k_hi);
        self.map_pitch(p, bin_step, s_lo, s_hi);
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
            let em = {
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
            self.omega_o[k] = self.omega_a[j] * self.wscale[k] + self.wshift[k];
            self.tau_o[k] = self.tau_a[j];
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
        for k in k_lo..k_hi {
            let r = self.xr[self.src[k] as usize];
            let rn = r.norm();
            self.ana_o[k] = if rn > 1e-30 { r / rn } else { C::new(1.0, 0.0) };
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
                let hz = k as f32 * hz_per_bin;
                let at = self.omega_o[k] * self.sr / TAU;
                let km = ((at / m.bin_hz).round().max(0.0) as usize)
                    .clamp(m.k_lo, m.k_hi.max(m.k_lo + 1) - 1);
                let dw = TAU * (hz - km as f32 * m.bin_hz) / self.sr;
                self.phase[k] = m.phase[km] * cis(m.omega[km] * dt - m.tau[km] * dw);
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
        for ch in 0..channels {
            self.synth.fill(C::default());
            let xc = &self.x[ch];
            for k in k_lo..k_hi {
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
                    let m = xc[i0].norm() * (1.0 - fr) + xc[i0 + 1].norm() * fr;
                    let zn = z.norm();
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

    /// Pitch map: which source position each output bin of the band reads (`srcf`, nearest
    /// bin `src`), with what magnitude factor (`amp`) and frequency mapping (`wscale`,
    /// `wshift`). `p = 1`: the identity. Otherwise each spectral peak's region (down to the
    /// minima around it, and at least its main lobe) moves as a whole by the (fractional)
    /// number of bins that puts the peak at `p` times its reassigned frequency, the region's
    /// bins keeping their offsets to it (Laroche & Dolson, "New phase-vocoder techniques for
    /// real-time pitch shifting", JAES 1999), so partials keep their lobe shape and level;
    /// colliding regions add in power. Output bins no region reaches (between partials
    /// shifted up) take the floor of the source around the bin-scaled position.
    fn map_pitch(&mut self, p: f32, bin_step: f32, s_lo: usize, s_hi: usize) {
        let (k_lo, k_hi) = (self.k_lo, self.k_hi);
        if p == 1.0 {
            for k in k_lo..k_hi {
                self.src[k] = k as u32;
                self.srcf[k] = k as f32;
                self.amp[k] = 1.0;
                self.wscale[k] = 1.0;
                self.wshift[k] = 0.0;
            }
            return;
        }
        self.amp[k_lo..k_hi].fill(0.0);
        self.peaks.clear();
        let e = &self.emag;
        let (lo, hi) = (s_lo.max(1), s_hi.saturating_sub(1));
        for j in lo..hi {
            if e[j] > 0.0 && e[j] >= e[j - 1] && e[j] > e[j + 1] {
                self.peaks.push(j as u32);
            }
        }
        // Regions: from the minimum before each peak to the minimum after it, and at least
        // the peak's main lobe (close partials share their lobes' flanks; truncating a lobe
        // would lose its level). Contributions colliding on an output bin add in power; the
        // strongest gives the bin its source (phase, frequency). `best` (in `mag_o`) is that
        // strongest magnitude, `power` (in `gain`) the sum.
        let best = &mut self.mag_o;
        let power = &mut self.gain;
        best[k_lo..k_hi].fill(-1.0);
        power[k_lo..k_hi].fill(0.0);
        let np = self.peaks.len();
        let mut start = s_lo;
        let interp = |x: f32| {
            let i = (x as usize).min(e.len() - 2);
            let f = x - i as f32;
            e[i] * (1.0 - f) + e[i + 1] * f
        };
        for (i, &pk) in self.peaks.iter().enumerate() {
            let pk = pk as usize;
            let end = if i + 1 < np {
                let next = self.peaks[i + 1] as usize;
                (pk + 1..next)
                    .min_by(|a, b| e[*a].total_cmp(&e[*b]))
                    .unwrap_or(next)
            } else {
                s_hi
            };
            let lo = start.min(pk.saturating_sub(MAIN_LOBE)).max(s_lo);
            let hi = end.max(pk + MAIN_LOBE + 1).min(s_hi);
            start = end;
            // Fractional shift (bins) moving the peak's frequency to `p` times it; output bin
            // `k` reads the source at `k - shift`.
            let wshift = self.omega_a[pk] * (p - 1.0);
            let shift = wshift / bin_step;
            let k0 = ((lo as f32 + shift).ceil().max(k_lo as f32)) as usize;
            let k1 = ((hi as f32 - 1.0 + shift).floor() + 1.0).min(k_hi as f32);
            let k1 = if k1 > 0.0 { k1 as usize } else { 0 };
            for k in k0..k1 {
                let jf = k as f32 - shift;
                if jf < 0.0 || jf > (e.len() - 1) as f32 {
                    continue;
                }
                let m = interp(jf);
                power[k] += m * m;
                if m > best[k] {
                    best[k] = m;
                    self.srcf[k] = jf;
                    self.src[k] = (jf.round() as u32).min(e.len() as u32 - 1);
                    self.wscale[k] = 1.0;
                    self.wshift[k] = wshift;
                }
            }
        }
        for k in k_lo..k_hi {
            if best[k] > 0.0 {
                self.amp[k] = power[k].sqrt() / best[k];
            }
        }
        // Gaps: the floor of the source around the bin-scaled position (wider than a main
        // lobe, so between partials it is the valley, not a lobe's flank).
        let w = MAIN_LOBE + 1;
        for k in k_lo..k_hi {
            if self.amp[k] != 0.0 {
                continue;
            }
            let q = (k as f32 / p).round() as usize;
            if q < s_lo + 1 || q + 1 >= s_hi {
                continue;
            }
            let m = e[q.saturating_sub(w).max(s_lo)..(q + w + 1).min(s_hi)]
                .iter()
                .fold(f32::MAX, |m, v| m.min(*v));
            if e[q] > 0.0 {
                self.src[k] = q as u32;
                self.srcf[k] = q as f32;
                self.amp[k] = m / e[q];
                self.wscale[k] = p;
                self.wshift[k] = 0.0;
            }
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
