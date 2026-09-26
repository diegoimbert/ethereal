//! Time-stretching behind the [`Stretcher`] trait.
//!
//! The trait is always available (native + wasm). The Signalsmith Stretch implementation
//! is behind the `signalsmith` feature and native-only (C++ via the `signalsmith-stretch`
//! crate). On the web, warped clips fall back to unwarped playback until a JS/WASM build of
//! Signalsmith is wired (ARCHITECTURE.md).
//!
//! Owned by the `stretch` node.

/// A streaming time-stretcher / pitch-shifter. API mirrors Signalsmith Stretch.
///
/// Lifecycle: created + [`Stretcher::configure`]d off the audio thread; `process`, `reset`
/// and `seek` are RT-safe (no allocation after `configure`).
pub trait Stretcher: Send {
    /// Non-RT. Allocate for `channels` at `sample_rate`, blocks up to `max_block` frames.
    fn configure(&mut self, channels: usize, sample_rate: f32, max_block: usize);

    /// RT. Clear internal state.
    fn reset(&mut self);

    /// Input samples needed ahead of the output position.
    fn input_latency(&self) -> usize;
    /// Output delay introduced.
    fn output_latency(&self) -> usize;

    /// RT. Pitch shift in semitones (independent of time ratio).
    fn set_transpose_semitones(&mut self, semitones: f32);

    /// RT. Prime internal buffers from `input` (e.g. after a jump) so output starts
    /// immediately at `playback_rate` (input frames per output frame).
    fn seek(&mut self, input: &[&[f32]], playback_rate: f64);

    /// RT. Consume `input_frames` of `input` and produce `output_frames` into `output`;
    /// the time ratio is `input_frames / output_frames`.
    fn process(
        &mut self,
        input: &[&[f32]],
        input_frames: usize,
        output: &mut [&mut [f32]],
        output_frames: usize,
    );
}

/// Creates stretchers (one per playing warped clip voice). Hosts pass the factory to the
/// engine's clip players; `None` = warping unsupported (web fallback).
pub trait StretcherFactory: Send + Sync {
    fn create(&self) -> Box<dyn Stretcher>;
}

/// Signalsmith Stretch implementation (native, `signalsmith` feature).
#[cfg(all(feature = "signalsmith", not(target_arch = "wasm32")))]
pub mod signalsmith {
    /// Stub: implemented by the `stretch` node.
    pub struct SignalsmithStretcher {
        _private: (),
    }
}
