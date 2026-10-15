//! Time-stretching behind the [`Stretcher`] trait.
//!
//! The trait is always available (native + wasm). Two implementations of the same
//! algorithm (Signalsmith Stretch):
//! - [`SignalsmithStretcher`] (feature `signalsmith`, native only): the vendored C++.
//! - [`RustStretcher`] (feature `rust-stretch`, everywhere): a pure-Rust port, used by the
//!   web host (wasm32 has no C++ toolchain here). Without a stretcher, Complex-warped clips
//!   play Repitch.
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
pub mod signalsmith;

#[cfg(all(feature = "signalsmith", not(target_arch = "wasm32")))]
pub use signalsmith::{Preset, SignalsmithFactory, SignalsmithStretcher};

/// Pure-Rust port of Signalsmith Stretch (`rust-stretch` feature; native + wasm32).
#[cfg(feature = "rust-stretch")]
pub mod rust;

#[cfg(feature = "rust-stretch")]
pub use rust::{RustPreset, RustStretchFactory, RustStretcher};
