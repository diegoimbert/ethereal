//! [`Stretcher`] backed by a pure-Rust port of Signalsmith Stretch (feature `rust-stretch`).
//!
//! Same algorithm as [`crate::signalsmith`] (the C++ is native-only), so it builds for
//! wasm32: the web host uses it for `WarpMode::Complex`. The port follows
//! `signalsmith-stretch.h` v1.3.1 step for step, including split computation (each STFT
//! spread over the following interval, so no 128-frame render quantum carries a whole FFT);
//! Signalsmith Linear's FFT is replaced by `rustfft` (half-bin shifted real FFT in
//! [`fft`]). Signalsmith Stretch is MIT-licensed, © 2022 Geraint Luff / Signalsmith Audio
//! Ltd. (`csrc/LICENSE-signalsmith-stretch.txt`).

mod core;
mod fft;
mod stft;

use self::core::{Core, Inputs};
use crate::{Stretcher, StretcherFactory};

/// Most channels a stretcher handles.
const MAX_CHANNELS: usize = 8;
/// Length of [`RustStretcher::apply_onset_fade`].
const ONSET_FADE_SECONDS: f32 = 0.005;

/// Block / interval sizes for [`RustStretcher`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RustPreset {
    /// Signalsmith `presetDefault` with split computation: 120 ms block, 30 ms interval
    /// (native quality, as `SignalsmithFactory::default()`).
    #[default]
    Default,
    /// [`RustPreset::Default`] computing each STFT in one go (parity tests).
    DefaultUnsplit,
    /// Signalsmith `presetCheaper`: 100 ms block, 40 ms interval, split. Its windows
    /// overlap only 2.5x, so a fresh (reset) stretcher's output starts with a step after
    /// the first interval (a click; the C++ does the same). Not used by the engine.
    Cheaper,
    /// The web (AudioWorklet) preset: 100 ms block, 33.3 ms interval (3x overlap), split.
    /// About 25% cheaper than [`RustPreset::Default`]. A 5 ms onset fade after a reset
    /// keeps a fresh voice from clicking as [`RustPreset::Cheaper`] does.
    Web,
}

impl RustPreset {
    /// `(block, interval, split)` in samples at `sr`.
    fn sizes(self, sr: f32) -> (usize, usize, bool) {
        let at = |seconds: f64| (sr as f64 * seconds) as usize;
        match self {
            // `sampleRate*0.12` etc. in double, truncated (as Signalsmith's presets).
            Self::Default => (at(0.12), at(0.03), true),
            Self::DefaultUnsplit => (at(0.12), at(0.03), false),
            Self::Cheaper => (at(0.1), at(0.04), true),
            Self::Web => (at(0.1), at(0.1) / 3, true),
        }
    }
}

/// Pure-Rust Signalsmith Stretch behind [`Stretcher`]. Unconfigured instances output
/// silence.
pub struct RustStretcher {
    preset: RustPreset,
    transpose: f32,
    core: Option<Core>,
    /// Onset fade ([`RustPreset::Web`]): output samples since the last reset, while
    /// fading, and the fade's `(start, length)`.
    since_reset: usize,
    onset_fade: Option<(usize, usize)>,
}

impl Default for RustStretcher {
    fn default() -> Self {
        Self::new(RustPreset::default())
    }
}

impl RustStretcher {
    pub fn new(preset: RustPreset) -> Self {
        Self {
            preset,
            transpose: 0.0,
            core: None,
            since_reset: 0,
            onset_fade: None,
        }
    }

    /// Raised-cosine fade-in of the first output after a reset ([`RustPreset::Web`]): with
    /// 3x-overlapping windows the first synthesised frame starts on a window edge that is
    /// far from zero, which steps (clicks) at the end of the first interval otherwise.
    fn apply_onset_fade(&mut self, output: &mut [&mut [f32]], frames: usize) {
        let Some((start, len)) = self.onset_fade else {
            return;
        };
        let t0 = self.since_reset;
        if t0 >= start + len {
            return;
        }
        for ch in output.iter_mut() {
            for (i, v) in ch.iter_mut().take(frames).enumerate() {
                let t = t0 + i;
                if t >= start + len {
                    break;
                }
                let x = t.saturating_sub(start) as f32 / len as f32;
                *v *= 0.5 - 0.5 * (std::f32::consts::PI * x).cos();
            }
        }
        self.since_reset = t0.saturating_add(frames);
    }

    fn channels(&self) -> usize {
        self.core.as_ref().map_or(0, |c| c.channels())
    }
}

impl Stretcher for RustStretcher {
    fn configure(&mut self, channels: usize, sample_rate: f32, _max_block: usize) {
        let channels = channels.clamp(1, MAX_CHANNELS);
        let sr = sample_rate.max(1.0).round();
        let (block, interval, split) = self.preset.sizes(sr);
        let mut core = Core::new(channels, block.max(16), interval.max(4), split);
        core.set_transpose_factor((self.transpose / 12.0).exp2());
        self.core = Some(core);
        self.since_reset = 0;
        self.onset_fade = (self.preset == RustPreset::Web).then(|| {
            // Starts where the first synthesised frame is read (one interval in).
            (interval, (sr * ONSET_FADE_SECONDS) as usize + 1)
        });
    }

    fn reset(&mut self) {
        if let Some(c) = &mut self.core {
            c.reset();
            self.since_reset = 0;
        }
    }

    fn input_latency(&self) -> usize {
        self.core.as_ref().map_or(0, Core::input_latency)
    }

    fn output_latency(&self) -> usize {
        self.core.as_ref().map_or(0, Core::output_latency)
    }

    fn set_transpose_semitones(&mut self, semitones: f32) {
        if semitones == self.transpose {
            return;
        }
        self.transpose = semitones;
        if let Some(c) = &mut self.core {
            c.set_transpose_factor((semitones / 12.0).exp2());
        }
    }

    fn seek(&mut self, input: &[&[f32]], playback_rate: f64) {
        let channels = self.channels();
        let Some(core) = &mut self.core else { return };
        let len = input
            .iter()
            .take(channels)
            .map(|c| c.len())
            .min()
            .unwrap_or(0);
        // Align every channel's tail (only the last block + interval is used).
        let mut tails: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        for (t, ch) in tails.iter_mut().zip(input.iter().take(channels)) {
            *t = &ch[ch.len() - len..];
        }
        core.seek(
            Inputs(&tails[..input.len().min(channels)]),
            len,
            playback_rate,
        );
    }

    fn process(
        &mut self,
        input: &[&[f32]],
        input_frames: usize,
        output: &mut [&mut [f32]],
        output_frames: usize,
    ) {
        let channels = self.channels();
        if let Some(core) = &mut self.core {
            core.process(Inputs(input), input_frames, output, output_frames);
            self.apply_onset_fade(output, output_frames);
        }
        // Outputs beyond the configured channels (or everything, unconfigured): silence.
        for ch in output.iter_mut().skip(channels) {
            let n = output_frames.min(ch.len());
            ch[..n].fill(0.0);
        }
    }
}

/// Factory for [`RustStretcher`]s (unconfigured; the caller configures them).
#[derive(Debug, Clone, Copy, Default)]
pub struct RustStretchFactory {
    pub preset: RustPreset,
}

impl StretcherFactory for RustStretchFactory {
    fn create(&self) -> Box<dyn Stretcher> {
        Box::new(RustStretcher::new(self.preset))
    }
}

#[cfg(test)]
mod tests;
