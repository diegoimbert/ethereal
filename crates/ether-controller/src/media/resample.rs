//! Incremental sample-rate conversion, stepped from `tick()` a budget at a time.
//!
//! Identical algorithm and alignment to `ether_media::resample` (rubato `SincFixedIn`,
//! sinc 256 / oversampling 1024 / Blackman-Harris², exact time alignment by padding
//! `q - 1` zeros and dropping `p - 1` outputs for `ratio = p / q`), so the result matches
//! the one-shot version; only the work is split across calls.

use std::sync::Arc;

use ether_media::{DecodedAudio, MediaError, resampled_len};
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

const CHUNK: usize = 1024;

fn err(e: impl std::fmt::Display) -> MediaError {
    MediaError::Resample(e.to_string())
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

struct Running {
    resampler: SincFixedIn<f32>,
    pad: usize,
    drop: usize,
    total_in: usize,
    pos: usize,
    inbuf: Vec<Vec<f32>>,
    outbuf: Vec<Vec<f32>>,
}

pub(crate) struct IncrementalResampler {
    source: Arc<DecodedAudio>,
    target_rate: u32,
    expected: usize,
    out: Vec<Vec<f32>>,
    running: Option<Running>,
    done: bool,
}

impl IncrementalResampler {
    pub fn new(source: Arc<DecodedAudio>, target_rate: u32) -> Result<Self, MediaError> {
        if target_rate == 0 || source.sample_rate == 0 {
            return Err(MediaError::Resample("sample rate must be > 0".into()));
        }
        let frames = source.frames();
        let n_ch = source.channels.len();
        let expected = resampled_len(frames, source.sample_rate, target_rate);
        let trivial = source.sample_rate == target_rate || frames == 0 || n_ch == 0;
        let running = if trivial {
            None
        } else {
            let ratio = target_rate as f64 / source.sample_rate as f64;
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: rubato::calculate_cutoff(256, WindowFunction::BlackmanHarris2),
                oversampling_factor: 1024,
                interpolation: SincInterpolationType::Cubic,
                window: WindowFunction::BlackmanHarris2,
            };
            let resampler = SincFixedIn::<f32>::new(ratio, 1.0, params, CHUNK, n_ch).map_err(err)?;
            let g = gcd(target_rate, source.sample_rate);
            let (p, q) = ((target_rate / g) as usize, (source.sample_rate / g) as usize);
            let inbuf = resampler.input_buffer_allocate(true);
            let outbuf = resampler.output_buffer_allocate(true);
            Some(Running {
                resampler,
                pad: q - 1,
                drop: p - 1,
                total_in: q - 1 + frames,
                pos: 0,
                inbuf,
                outbuf,
            })
        };
        let out = match &running {
            Some(r) => (0..n_ch)
                .map(|_| Vec::with_capacity(r.drop + expected + r.resampler.output_frames_max()))
                .collect(),
            None => Vec::new(),
        };
        Ok(Self {
            source,
            target_rate,
            expected,
            out,
            running,
            done: trivial,
        })
    }

    /// Fraction done, 0..=1.
    pub fn progress(&self) -> f32 {
        match &self.running {
            _ if self.done => 1.0,
            Some(r) => (r.pos as f32 / r.total_in.max(1) as f32).min(1.0),
            None => 1.0,
        }
    }

    /// Process about `budget` input frames. Returns `true` when done.
    pub fn step(&mut self, budget: usize) -> Result<bool, MediaError> {
        let Some(r) = self.running.as_mut() else {
            return Ok(true);
        };
        let stop_at = r.pos.saturating_add(budget.max(1));
        while self.out[0].len() < r.drop + self.expected {
            if r.pos >= stop_at {
                return Ok(false);
            }
            let need = r.resampler.input_frames_next();
            for (dst, src) in r.inbuf.iter_mut().zip(&self.source.channels) {
                dst.resize(need, 0.0);
                for (i, d) in dst.iter_mut().enumerate() {
                    let v = r.pos + i;
                    *d = if v >= r.pad && v < r.total_in { src[v - r.pad] } else { 0.0 };
                }
            }
            let (_, n) = r
                .resampler
                .process_into_buffer(&r.inbuf, &mut r.outbuf, None)
                .map_err(err)?;
            if n == 0 && r.pos > r.total_in + 4 * CHUNK {
                break;
            }
            for (o, b) in self.out.iter_mut().zip(&r.outbuf) {
                o.extend_from_slice(&b[..n]);
            }
            r.pos += need;
        }
        self.done = true;
        Ok(true)
    }

    pub fn finish(self) -> DecodedAudio {
        match self.running {
            None => {
                let mut audio = Arc::unwrap_or_clone(self.source);
                audio.sample_rate = self.target_rate;
                audio
            }
            Some(r) => {
                let mut out = self.out;
                for ch in &mut out {
                    ch.drain(..r.drop.min(ch.len()));
                    ch.resize(self.expected, 0.0);
                }
                DecodedAudio {
                    sample_rate: self.target_rate,
                    channels: out,
                }
            }
        }
    }
}
