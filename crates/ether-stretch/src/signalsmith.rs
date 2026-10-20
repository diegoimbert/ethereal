//! [`Stretcher`] backed by Signalsmith Stretch (C++, vendored in `csrc/`, behind the small
//! C ABI of `csrc/wrapper.cpp`).
//!
//! - **Split computation**: `presetDefault` with `splitComputation` spreads
//!   each STFT over the following interval instead of computing it in one burst every few
//!   blocks (a full stereo STFT is ~0.6 ms; with every voice re-seeking on the same sample at
//!   play/locate/loop, the bursts lined up). It adds one interval of output latency, which
//!   the engine compensates like the rest ([`Stretcher::output_latency`]).
//! - **Planar** channel pointers go straight to Signalsmith: no interleaving copies. Missing
//!   input channels read silence and outputs beyond the configured channels are silenced,
//!   through small scratch buffers sized in [`Stretcher::configure`]; only then are calls
//!   longer than the scratch split into proportional chunks (same time ratio), so `process`
//!   and `seek` never allocate.

use std::ffi::c_void;

use crate::{Stretcher, StretcherFactory};

/// Input frames per `max_block` output frames the scratch is sized for (8x speed-up in a
/// single call before chunking kicks in).
const INPUT_HEADROOM: usize = 8;
/// Most channels a stretcher handles.
const MAX_CHANNELS: usize = 8;

unsafe extern "C" {
    fn ether_ss_create(channels: i32, sample_rate: f32, split_computation: i32) -> *mut c_void;
    fn ether_ss_create_cheaper(channels: i32, sample_rate: f32) -> *mut c_void;
    fn ether_ss_destroy(s: *mut c_void);
    fn ether_ss_reset(s: *mut c_void);
    fn ether_ss_input_latency(s: *mut c_void) -> i32;
    fn ether_ss_output_latency(s: *mut c_void) -> i32;
    fn ether_ss_set_transpose_semitones(s: *mut c_void, semitones: f32);
    fn ether_ss_seek(s: *mut c_void, input: *const *const f32, frames: i32, rate: f64);
    fn ether_ss_process(
        s: *mut c_void,
        input: *const *const f32,
        input_frames: i32,
        output: *const *mut f32,
        output_frames: i32,
    );
}

/// Quality preset for [`SignalsmithStretcher`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Signalsmith `presetDefault`: 120 ms block, 30 ms interval, split computation.
    #[default]
    Default,
    /// `presetDefault` computing each STFT in one go (the pre-v0.4 behaviour; for
    /// comparisons).
    DefaultUnsplit,
    /// Signalsmith `presetCheaper`: 100 ms block, 40 ms interval, split computation.
    Cheaper,
}

/// One Signalsmith instance (owned pointer).
struct Instance(*mut c_void);

// SAFETY: the instance is only used through `&mut self` (one thread at a time); Signalsmith
// keeps no thread-local state.
unsafe impl Send for Instance {}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: created by `ether_ss_create*`, dropped once.
        unsafe { ether_ss_destroy(self.0) }
    }
}

struct Configured {
    inner: Instance,
    channels: usize,
    /// Silence for missing input channels.
    zeros: Vec<f32>,
    /// Sink for configured channels the caller has no output for.
    junk: Vec<f32>,
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
        let channels = channels.clamp(1, MAX_CHANNELS);
        // Signalsmith sizes its blocks from an integer rate; round rather than truncate.
        let sr = sample_rate.max(1.0).round();
        // SAFETY: plain constructor calls; the pointer is owned by `Instance`.
        let inner = Instance(unsafe {
            match self.preset {
                Preset::Default => ether_ss_create(channels as i32, sr, 1),
                Preset::DefaultUnsplit => ether_ss_create(channels as i32, sr, 0),
                Preset::Cheaper => ether_ss_create_cheaper(channels as i32, sr),
            }
        });
        // SAFETY: valid instance.
        unsafe { ether_ss_set_transpose_semitones(inner.0, self.transpose) };
        let max_block = max_block.max(1);
        // `seek` only looks at the last block+interval (<= 0.16 s) of its input.
        let seek_len = (sr as usize * 16).div_ceil(100) + 1;
        self.state = Some(Configured {
            inner,
            channels,
            zeros: vec![0.0; (max_block * INPUT_HEADROOM).max(seek_len)],
            junk: vec![0.0; max_block],
        });
    }

    fn reset(&mut self) {
        if let Some(s) = &mut self.state {
            // SAFETY: valid instance.
            unsafe { ether_ss_reset(s.inner.0) }
        }
    }

    fn input_latency(&self) -> usize {
        // SAFETY: valid instance.
        self.state.as_ref().map_or(0, |s| {
            unsafe { ether_ss_input_latency(s.inner.0) }.max(0) as usize
        })
    }

    fn output_latency(&self) -> usize {
        // SAFETY: valid instance.
        self.state.as_ref().map_or(0, |s| {
            unsafe { ether_ss_output_latency(s.inner.0) }.max(0) as usize
        })
    }

    fn set_transpose_semitones(&mut self, semitones: f32) {
        if semitones == self.transpose {
            return;
        }
        self.transpose = semitones;
        if let Some(s) = &mut self.state {
            // SAFETY: valid instance.
            unsafe { ether_ss_set_transpose_semitones(s.inner.0, semitones) }
        }
    }

    fn seek(&mut self, input: &[&[f32]], playback_rate: f64) {
        let Some(s) = &mut self.state else { return };
        let len = input.iter().map(|c| c.len()).min().unwrap_or(0);
        // Signalsmith only uses the tail of the seek input: keep what the scratch covers.
        let n = len.min(s.zeros.len());
        let mut ptrs = [std::ptr::null::<f32>(); MAX_CHANNELS];
        for (c, p) in ptrs.iter_mut().enumerate().take(s.channels) {
            *p = match input.get(c) {
                Some(ch) => ch[len - n..].as_ptr(),
                None => s.zeros.as_ptr(),
            };
        }
        // SAFETY: `channels` pointers, each to at least `n` readable frames.
        unsafe { ether_ss_seek(s.inner.0, ptrs.as_ptr(), n as i32, playback_rate) }
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
        // Scratch is only needed for missing channels: then chunk to its size.
        let in_cap = if input.len() < s.channels {
            s.zeros.len()
        } else {
            usize::MAX
        };
        let out_cap = if output.len() < s.channels {
            s.junk.len()
        } else {
            usize::MAX
        };
        let chunks = input_frames
            .div_ceil(in_cap)
            .max(output_frames.div_ceil(out_cap))
            .max(1);
        for k in 0..chunks {
            let i0 = input_frames * k / chunks;
            let i1 = input_frames * (k + 1) / chunks;
            let o0 = output_frames * k / chunks;
            let o1 = output_frames * (k + 1) / chunks;
            let mut ins = [std::ptr::null::<f32>(); MAX_CHANNELS];
            let mut outs = [std::ptr::null_mut::<f32>(); MAX_CHANNELS];
            for c in 0..s.channels {
                ins[c] = match input.get(c) {
                    Some(ch) => ch[i0..i1].as_ptr(),
                    None => s.zeros.as_ptr(),
                };
                outs[c] = match output.get_mut(c) {
                    Some(ch) => ch[o0..o1].as_mut_ptr(),
                    None => s.junk.as_mut_ptr(),
                };
            }
            // SAFETY: `channels` input pointers with `i1 - i0` readable frames and output
            // pointers with `o1 - o0` writable frames, none aliasing (the junk sink is only
            // written, and only shared by channels whose output is discarded).
            unsafe {
                ether_ss_process(
                    s.inner.0,
                    ins.as_ptr(),
                    (i1 - i0) as i32,
                    outs.as_ptr(),
                    (o1 - o0) as i32,
                )
            }
        }
        // More outputs than configured channels: silence the extras.
        for ch in output.iter_mut().skip(s.channels) {
            let n = output_frames.min(ch.len());
            ch[..n].fill(0.0);
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
    fn large_calls_are_handled() {
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
        let first_blocks = |preset: Preset, seek: bool| {
            let mut st = SignalsmithStretcher::new(preset);
            st.configure(1, SR, BLOCK);
            assert!(st.input_latency() > 0 && st.output_latency() > 0);
            let pos = SR as usize / 2;
            if seek {
                st.seek(&[&input[..pos]], 1.0);
            }
            let mut rms_per_block = Vec::new();
            let mut out = vec![0.0; BLOCK];
            for b in 0..12 {
                let src = &input[pos + b * BLOCK..pos + (b + 1) * BLOCK];
                st.process(&[src], BLOCK, &mut [&mut out], BLOCK);
                rms_per_block.push(rms(&out));
            }
            rms_per_block
        };
        // Primed: audible within a couple of blocks (short fade-in), one interval (30 ms,
        // part of the reported output latency) later with split computation. Cold: still
        // silent, waiting out the ~output latency.
        for (preset, audible_from) in [(Preset::DefaultUnsplit, 2), (Preset::Default, 5)] {
            let primed = first_blocks(preset, true);
            let cold = first_blocks(preset, false);
            assert!(
                primed[audible_from..].iter().all(|&r| r > 0.15),
                "{preset:?} primed {primed:?}"
            );
            assert!(
                cold[..8].iter().all(|&r| r < 0.01),
                "{preset:?} cold {cold:?}"
            );
        }
    }

    /// Per-block cost of one stereo voice, unsplit vs split. Ignored; run
    /// in release with `--ignored --nocapture`.
    #[test]
    #[ignore = "perf harness"]
    fn block_costs() {
        let input = sine(440.0, 4 * SR as usize);
        for preset in [Preset::DefaultUnsplit, Preset::Default] {
            let block = 256;
            let mut st = SignalsmithStretcher::new(preset);
            st.configure(2, SR, block);
            let mut l = vec![0.0; block];
            let mut r = vec![0.0; block];
            let mut times = vec![];
            let n_in = block * 5 / 4;
            let mut pos = 0;
            while pos + n_in <= input.len() {
                let src = &input[pos..pos + n_in];
                let t = std::time::Instant::now();
                st.process(&[src, src], n_in, &mut [&mut l, &mut r], block);
                times.push(t.elapsed().as_secs_f64() * 1e6);
                pos += n_in;
            }
            times.sort_by(f64::total_cmp);
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            let p99 = times[times.len() * 99 / 100];
            let max = times[times.len() - 1];
            println!(
                "{preset:?}: mean {mean:.1} µs, p99 {p99:.1} µs, max {max:.1} µs per 256-frame block"
            );
        }
    }

    /// Split computation adds exactly one interval (30 ms) of output latency.
    #[test]
    fn split_computation_adds_one_interval_of_latency() {
        let lat = |preset| {
            let mut st = SignalsmithStretcher::new(preset);
            st.configure(2, SR, BLOCK);
            st.output_latency()
        };
        let interval = (SR * 0.03) as usize;
        assert_eq!(lat(Preset::Default), lat(Preset::DefaultUnsplit) + interval);
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
