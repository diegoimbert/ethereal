//! Audio → MIDI detection (v0.3, owned by the `audio-to-midi` node; CONTRACTS.md §13.5).
//! Pure DSP on decoded audio (native + wasm, no dependency beyond std); the controller
//! (`audio_to_midi/`) runs it in bounded slices from its tick and turns the notes into a
//! clip.
//!
//! Every mode first mixes to mono and decimates (a chunk at a time, [`prep`]): to ≈11–12 kHz
//! for melody and harmony (pitches up to C8), to ≤ 48 kHz for drums (hats need the top).
//!
//! - **Melody** ([`melody`]): monophonic pitch tracking (YIN with a voicing threshold from
//!   `sensitivity`), note segmentation by pitch stability and energy re-attacks, pitches
//!   quantised to semitones, velocity from the attack level.
//! - **Harmony** ([`harmony`]): onsets by spectral flux, then per inter-onset segment an
//!   iterative harmonic-sum selection over a high-resolution spectrum, notes in
//!   `min_pitch..=max_pitch`.
//! - **Drums** ([`drums`]): onsets per band (spectral flux) classified by the energy each
//!   band gains into kick / snare / hi-hat keys (several per onset).
//!
//! Onsets are sharpened on a 1–2.5 ms envelope ([`prep::Envelope::refine_onset`]), so they
//! land within a few milliseconds of the attack. Notes are in **source seconds** (the
//! controller maps them through the clip's warp to beats and drops those outside the clip's
//! window).
//!
//! Cost: roughly linear in the audio length (FFT frames every 3–4 ms); see the PR for the
//! measured real-time factors.

mod drums;
mod fft;
mod harmony;
mod melody;
mod onset;
mod prep;

use std::sync::Arc;

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

impl Default for Options {
    fn default() -> Self {
        Self {
            sensitivity: 0.5,
            min_duration: 0.05,
            min_pitch: 21,
            max_pitch: 108,
            kick_key: 36,
            snare_key: 38,
            hihat_key: 42,
        }
    }
}

enum Analyser {
    Melody(melody::Melody),
    Harmony(harmony::Harmony),
    Drums(drums::Drums),
}

/// Share of the progress taken by the mono/decimation stage.
const PREP_SHARE: f32 = 0.1;

/// An incremental detector: [`Detector::step`] does a bounded amount of work, so the
/// controller can run it from its tick without stalling.
pub struct Detector {
    audio: Arc<DecodedAudio>,
    options: Options,
    prep: prep::Prep,
    /// Analysis rate (Hz).
    rate: f64,
    analyser: Option<Analyser>,
    mode: Mode,
    notes: Option<Vec<DetectedNote>>,
}

impl std::fmt::Debug for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Detector")
            .field("mode", &self.mode)
            .field("progress", &self.progress())
            .finish()
    }
}

impl Detector {
    /// Prepare a detection of `audio` (mono-summed internally).
    pub fn new(audio: Arc<DecodedAudio>, mode: Mode, options: Options) -> Self {
        let sr = audio.sample_rate.max(1) as f64;
        let factor = match mode {
            Mode::Melody | Mode::Harmony => (sr / 11_025.0).round().max(1.0) as usize,
            Mode::Drums => (sr / 44_100.0).floor().max(1.0) as usize,
        };
        let prep = prep::Prep::new(&audio, factor);
        // Nothing to analyse (or a rate too low to analyse): done at once.
        let trivial = audio.frames() == 0 || audio.sample_rate < 4_000;
        Self {
            rate: sr / factor as f64,
            audio,
            options,
            prep,
            analyser: None,
            mode,
            notes: trivial.then(Vec::new),
        }
    }

    /// Analyse up to `frames` more input frames. Returns the progress 0..=1; `1.0` = done.
    pub fn step(&mut self, frames: usize) -> f32 {
        if self.notes.is_some() {
            return 1.0;
        }
        let mut budget = frames.max(1);
        if !self.prep.done() {
            let used = self.prep.step(&self.audio, budget);
            budget = budget.saturating_sub(used);
            if !self.prep.done() {
                return self.progress();
            }
        }
        let factor = self.prep.factor;
        let x = &self.prep.out;
        let analyser = self.analyser.get_or_insert_with(|| match self.mode {
            Mode::Melody => {
                Analyser::Melody(melody::Melody::new(self.rate, x.len(), &self.options))
            }
            Mode::Harmony => Analyser::Harmony(harmony::Harmony::new(self.rate, x.len())),
            Mode::Drums => Analyser::Drums(drums::Drums::new(self.rate, x.len())),
        });
        // The budget is in input frames; the analysers count analysis samples.
        let budget = budget.div_ceil(factor).max(1);
        let opts = &self.options;
        let done = match analyser {
            Analyser::Melody(a) => {
                a.step(x, budget);
                a.done()
            }
            Analyser::Harmony(a) => {
                a.step(x, opts, budget);
                a.done()
            }
            Analyser::Drums(a) => {
                a.step(x, opts, budget);
                a.done()
            }
        };
        if done {
            let mut notes = match analyser {
                Analyser::Melody(a) => a.finish(x, opts),
                Analyser::Harmony(a) => a.finish(x, opts),
                Analyser::Drums(a) => a.finish(opts),
            };
            notes.retain(|n| n.start.is_finite() && n.duration.is_finite());
            notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
            self.notes = Some(notes);
            // Free the analysis buffers.
            self.analyser = None;
            self.prep.out = Vec::new();
            return 1.0;
        }
        self.progress()
    }

    /// Progress 0..=1 of the detection so far.
    pub fn progress(&self) -> f32 {
        if self.notes.is_some() {
            return 1.0;
        }
        let analysis = match &self.analyser {
            Some(Analyser::Melody(a)) => a.progress(),
            Some(Analyser::Harmony(a)) => a.progress(),
            Some(Analyser::Drums(a)) => a.progress(),
            None => 0.0,
        };
        (PREP_SHARE * self.prep.progress() + (1.0 - PREP_SHARE) * analysis).min(0.999)
    }

    /// The detected notes (sorted by start, then pitch) once [`Detector::step`] returned
    /// `1.0` (empty before).
    pub fn notes(&self) -> Vec<DetectedNote> {
        self.notes.clone().unwrap_or_default()
    }

    /// Run the whole detection at once (tests, tools).
    pub fn run(audio: Arc<DecodedAudio>, mode: Mode, options: Options) -> Vec<DetectedNote> {
        let mut d = Self::new(audio, mode, options);
        while d.step(1 << 16) < 1.0 {}
        d.notes()
    }
}

/// Frequency (Hz) of fractional MIDI key `m`.
pub(crate) fn hz_of(m: f32) -> f32 {
    440.0 * 2f32.powf((m - 69.0) / 12.0)
}

/// Fractional MIDI key of `hz`.
pub(crate) fn midi_of(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Velocity of a note whose attack peaks at `db`, the loudest note being at `max_db`: the
/// top 36 dB map linearly onto 0.2..=1.
pub(crate) fn velocity_of(db: f32, max_db: f32) -> f32 {
    (1.0 + (db - max_db) / 45.0).clamp(0.2, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(sr: u32, notes: &[(f64, f64, u8)], len: f64) -> Arc<DecodedAudio> {
        let n = (len * sr as f64) as usize;
        let mut x = vec![0.0f32; n];
        for &(s, d, p) in notes {
            let f = hz_of(p as f32) as f64;
            let (a, b) = (
                (s * sr as f64) as usize,
                (((s + d) * sr as f64) as usize).min(n),
            );
            for (i, v) in x.iter_mut().enumerate().take(b).skip(a) {
                let t = (i - a) as f64 / sr as f64;
                let ramp = (t / 0.003)
                    .min(1.0)
                    .min(((b - i) as f64 / sr as f64) / 0.003);
                *v += (0.4 * ramp * (2.0 * std::f64::consts::PI * f * t).sin()) as f32;
            }
        }
        Arc::new(DecodedAudio {
            sample_rate: sr,
            channels: vec![x],
        })
    }

    #[test]
    fn silence_gives_no_notes() {
        for mode in [Mode::Melody, Mode::Harmony, Mode::Drums] {
            let audio = Arc::new(DecodedAudio {
                sample_rate: 44_100,
                channels: vec![vec![0.0; 44_100]; 2],
            });
            assert!(Detector::run(audio, mode, Options::default()).is_empty());
        }
        let empty = Arc::new(DecodedAudio::default());
        assert!(Detector::run(empty, Mode::Melody, Options::default()).is_empty());
    }

    #[test]
    fn small_steps_make_steady_progress() {
        let audio = tone(44_100, &[(0.1, 0.4, 60), (0.6, 0.4, 64)], 1.2);
        let mut d = Detector::new(audio, Mode::Melody, Options::default());
        let mut last = 0.0;
        let mut steps = 0;
        loop {
            let p = d.step(512);
            assert!(p >= last, "{p} < {last}");
            last = p;
            steps += 1;
            if p >= 1.0 {
                break;
            }
            assert!(steps < 100_000);
        }
        assert!(steps > 50, "{steps}");
        let notes = d.notes();
        assert_eq!(notes.iter().map(|n| n.pitch).collect::<Vec<_>>(), [60, 64]);
    }
}
