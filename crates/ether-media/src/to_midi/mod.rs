//! Audio → MIDI detection (v0.3, owned by the `audio-to-midi` node; CONTRACTS.md §13.5).
//! Pure DSP on decoded audio (native + wasm); the controller (`audio_to_midi/`) runs it in
//! bounded slices from its tick and turns the notes into a clip.
//!
//! - **Melody**: monophonic pitch tracking (YIN / pYIN-style with a voicing threshold from
//!   `sensitivity`), note segmentation by pitch stability and energy onsets, pitches
//!   quantized to semitones, velocity from the onset energy.
//! - **Harmony**: simple polyphonic detection (constant-Q / harmonic-sum spectrum peaks
//!   with onset gating), notes in `min_pitch..=max_pitch`.
//! - **Drums**: onset detection (spectral flux) classified by band energy into kick / snare /
//!   hi-hat keys.
//!
//! Notes are in **source seconds** (the controller maps them through the clip's warp to
//! beats and drops those outside the clip's window).

use crate::DecodedAudio;

/// A detected note, in source seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectedNote {
    pub start: f64,
    pub duration: f64,
    pub pitch: u8,
    /// 0..=1.
    pub velocity: f32,
}

/// Detection mode (mirrors `ether_protocol::audio_to_midi::AudioToMidiMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Melody,
    Harmony,
    Drums,
}

/// Detection settings (mirrors `ether_protocol::audio_to_midi::AudioToMidiOptions`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub sensitivity: f32,
    pub min_duration: f64,
    pub min_pitch: u8,
    pub max_pitch: u8,
    pub kick_key: u8,
    pub snare_key: u8,
    pub hihat_key: u8,
}

/// An incremental detector: [`Detector::step`] does a bounded amount of work, so the
/// controller can run it from its tick without stalling.
pub struct Detector {
    _private: (),
}

impl Detector {
    /// Prepare a detection of `audio` (mono-summed internally).
    pub fn new(audio: std::sync::Arc<DecodedAudio>, mode: Mode, options: Options) -> Self {
        let _ = (audio, mode, options);
        Self { _private: () }
    }

    /// Analyse up to `frames` more input frames. Returns the progress 0..=1; `1.0` = done.
    /// Placeholder: done at once.
    pub fn step(&mut self, frames: usize) -> f32 {
        let _ = frames;
        1.0
    }

    /// The detected notes (sorted by start, then pitch) once [`Detector::step`] returned
    /// `1.0`. Placeholder: none.
    pub fn notes(&self) -> Vec<DetectedNote> {
        Vec::new()
    }
}
