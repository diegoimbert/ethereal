//! [`Stretcher`] backed by Signalsmith Stretch (C++, via the `signalsmith-stretch` crate).
//!
//! The C wrapper takes interleaved buffers while the trait is planar, so each instance owns
//! interleave scratch buffers sized in [`Stretcher::configure`]. Calls larger than the
//! scratch are split into proportional chunks (same time ratio), so `process` and `seek`
//! never allocate.

use signalsmith_stretch::Stretch;

use crate::{Stretcher, StretcherFactory};

/// Input frames per `max_block` output frames the scratch is sized for (8x speed-up in a
/// single call before chunking kicks in).
const INPUT_HEADROOM: usize = 8;

/// Quality preset for [`SignalsmithStretcher`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Signalsmith `presetDefault`: 120 ms block, 30 ms interval.
    #[default]
    Default,
    /// Signalsmith `presetCheaper`: 100 ms block, 40 ms interval, split computation.
    Cheaper,
}

struct Configured {
    inner: Stretch,
    channels: usize,
    /// Interleaved input scratch: `in_cap * channels`.
    in_buf: Vec<f32>,
    in_cap: usize,
    /// Interleaved output scratch: `out_cap * channels`.
    out_buf: Vec<f32>,
    out_cap: usize,
}

/// Signalsmith Stretch behind [`Stretcher`]. Unconfigured instances output silence.
pub struct SignalsmithStretcher {
    preset: Preset,
    transpose: f32,
    state: Option<Configured>,
}

impl Default for SignalsmithStretcher {
    fn default() -> Self {
        Self::new(Preset::Default)
    }
}

impl SignalsmithStretcher {
    pub fn new(preset: Preset) -> Self {
        Self {
            preset,
            transpose: 0.0,
            state: None,
        }
    }
}

impl Stretcher for SignalsmithStretcher {
    fn configure(&mut self, channels: usize, sample_rate: f32, max_block: usize) {
        let channels = channels.max(1);
        // The binding takes an integer rate; round rather than truncate.
        let sr = sample_rate.max(1.0).round() as u32;
        let mut inner = match self.preset {
            Preset::Default => Stretch::preset_default(channels as u32, sr),
            Preset::Cheaper => Stretch::preset_cheaper(channels as u32, sr),
        };
        inner.set_transpose_factor_semitones(self.transpose, None);
        let max_block = max_block.max(1);
        // `seek` only looks at the last block+interval (<= 0.16 s) of its input; keep at
        // least that much so seeks are never truncated further than Signalsmith would.
        let seek_len = (sr as usize * 16).div_ceil(100) + 1;
        let in_cap = (max_block * INPUT_HEADROOM).max(seek_len);
        let out_cap = max_block;
        self.state = Some(Configured {
            inner,
            channels,
            in_buf: vec![0.0; in_cap * channels],
            in_cap,
            out_buf: vec![0.0; out_cap * channels],
            out_cap,
        });
    }

    fn reset(&mut self) {
        if let Some(s) = &mut self.state {
            s.inner.reset();
        }
    }

    fn input_latency(&self) -> usize {
        self.state.as_ref().map_or(0, |s| s.inner.input_latency())
    }

    fn output_latency(&self) -> usize {
        self.state.as_ref().map_or(0, |s| s.inner.output_latency())
    }

    fn set_transpose_semitones(&mut self, semitones: f32) {
        if semitones == self.transpose {
            return;
        }
        self.transpose = semitones;
        if let Some(s) = &mut self.state {
            s.inner.set_transpose_factor_semitones(semitones, None);
        }
    }

    fn seek(&mut self, input: &[&[f32]], playback_rate: f64) {
        let Some(s) = &mut self.state else { return };
        let len = input.iter().map(|c| c.len()).min().unwrap_or(0);
        // Signalsmith only uses the tail of the seek input; keep the most recent frames.
        let n = len.min(s.in_cap);
        interleave(input, len - n, n, s.channels, &mut s.in_buf);
        s.inner.seek(&s.in_buf[..n * s.channels], playback_rate);
    }

    fn process(
        &mut self,
        input: &[&[f32]],
        input_frames: usize,
        output: &mut [&mut [f32]],
        output_frames: usize,
    ) {
        let Some(s) = &mut self.state else {
            for ch in output.iter_mut() {
                let n = output_frames.min(ch.len());
                ch[..n].fill(0.0);
            }
            return;
        };
        let chunks = input_frames
            .div_ceil(s.in_cap)
            .max(output_frames.div_ceil(s.out_cap))
            .max(1);
        for k in 0..chunks {
            let i0 = input_frames * k / chunks;
            let i1 = input_frames * (k + 1) / chunks;
            let o0 = output_frames * k / chunks;
            let o1 = output_frames * (k + 1) / chunks;
            let (ni, no) = (i1 - i0, o1 - o0);
            interleave(input, i0, ni, s.channels, &mut s.in_buf);
            let out = &mut s.out_buf[..no * s.channels];
            s.inner.process(&s.in_buf[..ni * s.channels], &mut *out);
            for (c, ch) in output.iter_mut().enumerate() {
                let dst = &mut ch[o0..o1];
                if c >= s.channels {
                    // More outputs than configured channels: silence the extras.
                    dst.fill(0.0);
                    continue;
                }
                for (f, d) in dst.iter_mut().enumerate() {
                    *d = out[f * s.channels + c];
                }
            }
        }
    }
}

/// Copy `n` planar frames starting at `start` into the interleaved `dst`. Missing input
/// channels read as silence.
fn interleave(input: &[&[f32]], start: usize, n: usize, channels: usize, dst: &mut [f32]) {
    for c in 0..channels {
        match input.get(c) {
            Some(src) => {
                for (f, &v) in src[start..start + n].iter().enumerate() {
                    dst[f * channels + c] = v;
                }
            }
            None => {
                for f in 0..n {
                    dst[f * channels + c] = 0.0;
                }
            }
        }
    }
}

/// Factory for [`SignalsmithStretcher`]s (unconfigured; the caller configures them).
#[derive(Debug, Clone, Copy, Default)]
pub struct SignalsmithFactory {
    pub preset: Preset,
}

impl StretcherFactory for SignalsmithFactory {
    fn create(&self) -> Box<dyn Stretcher> {
        Box::new(SignalsmithStretcher::new(self.preset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;
    const BLOCK: usize = 512;

    fn sine(freq: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * 0.5)
            .collect()
    }

    /// Streams `input` through a stretcher block by block at `ratio` (input/output) using
    /// only the trait, and returns the output with the output latency trimmed.
    fn stretch(st: &mut dyn Stretcher, input: &[f32], ratio: f64, semitones: f32) -> Vec<f32> {
        st.configure(1, SR, BLOCK);
        st.set_transpose_semitones(semitones);
        let out_total = (input.len() as f64 / ratio).round() as usize;
        let lat = st.output_latency();
        let mut out = Vec::with_capacity(out_total + lat);
        let mut block = vec![0.0f32; BLOCK];
        let zeros = vec![0.0f32; BLOCK * INPUT_HEADROOM];
        let (mut produced, mut consumed) = (0usize, 0usize);
        while produced < out_total + lat {
            let n_out = BLOCK.min(out_total + lat - produced);
            let target_in = ((produced + n_out) as f64 * ratio).round() as usize;
            let n_in = target_in - consumed;
            let src: &[f32] = if consumed + n_in <= input.len() {
                &input[consumed..consumed + n_in]
            } else {
                &zeros[..n_in] // tail: flush with silence
            };
            st.process(&[src], n_in, &mut [&mut block[..n_out]], n_out);
            out.extend_from_slice(&block[..n_out]);
            produced += n_out;
            consumed += n_in;
        }
        out.drain(..lat);
        out.truncate(out_total);
        out
    }

    /// Frequency estimate from zero crossings over the steady middle of the signal.
    fn freq(sig: &[f32]) -> f32 {
        let mid = &sig[sig.len() / 4..sig.len() * 3 / 4];
        let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f32 * SR / mid.len() as f32
    }

    fn rms(sig: &[f32]) -> f32 {
        (sig.iter().map(|v| v * v).sum::<f32>() / sig.len() as f32).sqrt()
    }

    fn check(ratio: f64, semitones: f32) {
        let f0 = 440.0;
        let input = sine(f0, SR as usize); // 1 s
        let mut st = SignalsmithStretcher::default();
        let out = stretch(&mut st, &input, ratio, semitones);
        assert_eq!(out.len(), (input.len() as f64 / ratio).round() as usize);
        // Energy present across the whole stretched duration (not silent or cut short).
        let q = out.len() / 8;
        for seg in out[q..out.len() - q].chunks(q) {
            assert!(rms(seg) > 0.15, "ratio {ratio}: segment rms {}", rms(seg));
        }
        let want = f0 * 2f32.powf(semitones / 12.0);
        let got = freq(&out);
        assert!(
            (got - want).abs() / want < 0.03,
            "ratio {ratio} st {semitones}: {got} Hz vs {want} Hz"
        );
    }

    #[test]
    fn slow_down_keeps_pitch() {
        check(0.5, 0.0); // 2x longer
    }

    #[test]
    fn speed_up_keeps_pitch() {
        check(1.5, 0.0);
    }

    #[test]
    fn transpose_up_an_octave_without_changing_duration() {
        check(1.0, 12.0);
    }

    #[test]
    fn transpose_down_while_stretching() {
        check(0.75, -7.0);
    }

    #[test]
    fn large_calls_are_chunked() {
        let mut st = SignalsmithStretcher::default();
        st.configure(2, SR, 64);
        let input = sine(440.0, 64 * 40);
        let mut l = vec![0.0; 64 * 10];
        let mut r = vec![0.0; 64 * 10];
        // 4x speed-up over 10 blocks of output in one call (exceeds both scratches).
        st.process(
            &[&input, &input],
            input.len(),
            &mut [&mut l, &mut r],
            64 * 10,
        );
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        assert_eq!(l, r);
    }

    #[test]
    fn seek_primes_output() {
        let input = sine(440.0, SR as usize);
        // Output right after a jump to 0.5 s, with or without priming via `seek`.
        let first_blocks = |seek: bool| {
            let mut st = SignalsmithStretcher::default();
            st.configure(1, SR, BLOCK);
            assert!(st.input_latency() > 0 && st.output_latency() > 0);
            let pos = SR as usize / 2;
            if seek {
                st.seek(&[&input[..pos]], 1.0);
            }
            let mut rms_per_block = Vec::new();
            let mut out = vec![0.0; BLOCK];
            for b in 0..8 {
                let src = &input[pos + b * BLOCK..pos + (b + 1) * BLOCK];
                st.process(&[src], BLOCK, &mut [&mut out], BLOCK);
                rms_per_block.push(rms(&out));
            }
            rms_per_block
        };
        let primed = first_blocks(true);
        let cold = first_blocks(false);
        // Primed: audible within a couple of blocks (short fade-in). Cold: still silent,
        // waiting out the ~output latency.
        assert!(primed[2..].iter().all(|&r| r > 0.15), "primed {primed:?}");
        assert!(cold.iter().all(|&r| r < 0.01), "cold {cold:?}");
    }

    #[test]
    fn unconfigured_outputs_silence() {
        let mut st = SignalsmithFactory::default().create();
        let mut out = vec![1.0; 16];
        st.process(&[&[0.5; 16]], 16, &mut [&mut out], 16);
        assert!(out.iter().all(|&v| v == 0.0));
        assert_eq!(st.input_latency(), 0);
    }
}
