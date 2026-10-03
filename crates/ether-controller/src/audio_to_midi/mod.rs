//! Audio to MIDI (v0.3, owned by the `audio-to-midi` node; protocol
//! `ether_protocol::audio_to_midi`, detection DSP in `ether_media::to_midi`; CONTRACTS.md
//! §13.5).
//!
//! - [`EtherController::audio_to_midi_command`]: `AudioToMidi::{Start, Cancel}` (dispatched
//!   from `handlers.rs`). `Start` checks the clip (audio, media present, not missing), the
//!   new ids and the options, creates the job and replies `Unit`. One job at a time
//!   (`InvalidState`). `Cancel` of the running job emits `Cancelled`; any other id is a
//!   no-op.
//! - [`EtherController::audio_to_midi_tick`]: runs the job on the controller thread (never
//!   the audio thread), in bounded slices: read the media bytes (the source file, at its
//!   own rate: detection is on the source), decode them incrementally, then
//!   `ether_media::to_midi::Detector::step`, a chunk at a time until the tick's time
//!   budget ([`TICK_BUDGET_MS`]) or unit cap ([`MAX_UNITS_PER_TICK`]) is spent. `Progress`
//!   is emitted when it moves. At the end the result is applied with `edit_with` (one undo
//!   step: track, clip, notes, optional instrument; ids from the command, [`place`]) and
//!   `Done` follows its patch. A read/decode error or a refused edit emits `Failed`; the
//!   job is `Cancelled` when its clip (or the project) goes away.

mod place;

use std::sync::Arc;

use ether_core::protocol::audio_to_midi::{
    AudioToMidiCommand, AudioToMidiEvent, AudioToMidiJobId, AudioToMidiMode, AudioToMidiOptions,
};
use ether_core::protocol::model::*;
use ether_core::protocol::{Event, ReplyValue};
use ether_media::to_midi::{Detector, Mode, Options};

use crate::handlers::event;
use crate::media::{IncrementalDecoder, extension_of, read_media_bytes};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Work per tick: at most this many units (one unit = [`UNIT_FRAMES`] frames decoded or
/// analysed)...
const MAX_UNITS_PER_TICK: usize = 64;
/// ...and no more than this long (when the host clock advances).
const TICK_BUDGET_MS: u64 = 8;
/// Frames per unit of work.
const UNIT_FRAMES: usize = 4096;
/// Share of the progress taken by reading + decoding.
const DECODE_SHARE: f32 = 0.25;

enum Stage {
    Read,
    Decode(Box<IncrementalDecoder>),
    Detect(Box<Detector>),
}

struct Job {
    id: AudioToMidiJobId,
    project: ProjectId,
    clip: ClipId,
    media: MediaRef,
    mode: AudioToMidiMode,
    options: Options,
    track: TrackId,
    new_clip: ClipId,
    seed_notes: NoteId,
    instrument: Option<DeviceId>,
    stage: Stage,
    /// Length of the decoded source (seconds), once decoded.
    seconds: f64,
}

impl Job {
    fn progress(&self) -> f32 {
        match &self.stage {
            Stage::Read => 0.0,
            Stage::Decode(d) => {
                let total = d.n_frames.unwrap_or(self.media.frames).max(1) as f32;
                DECODE_SHARE * (d.decoded_frames() as f32 / total).min(1.0)
            }
            Stage::Detect(d) => DECODE_SHARE + (1.0 - DECODE_SHARE) * d.progress(),
        }
        .min(0.999)
    }
}

/// The running conversion job, if any.
#[derive(Default)]
pub(crate) struct AudioToMidiState {
    running: Option<Box<Job>>,
    last_progress: Option<f32>,
}

impl std::fmt::Debug for AudioToMidiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioToMidiState")
            .field("running", &self.running.as_ref().map(|j| &j.id))
            .finish()
    }
}

/// What a tick's work ended with.
enum Outcome {
    Working,
    Detected(Vec<ether_media::to_midi::DetectedNote>),
    Failed(String),
}

fn a2m_event(out: &mut dyn MessageSink, e: AudioToMidiEvent) {
    event(out, Event::AudioToMidi { event: e });
}

fn mode_of(m: AudioToMidiMode) -> Mode {
    match m {
        AudioToMidiMode::Melody => Mode::Melody,
        AudioToMidiMode::Harmony => Mode::Harmony,
        AudioToMidiMode::Drums => Mode::Drums,
    }
}

fn options_of(o: &AudioToMidiOptions) -> CmdResult<Options> {
    if !(o.sensitivity.is_finite() && (0.0..=1.0).contains(&o.sensitivity)) {
        return Err(invalid("sensitivity must be in 0..=1"));
    }
    if !(o.min_duration.0.is_finite() && (0.0..=10.0).contains(&o.min_duration.0)) {
        return Err(invalid("min_duration must be in 0..=10 seconds"));
    }
    if o.min_pitch > o.max_pitch || o.max_pitch > 127 {
        return Err(invalid("the pitch range must satisfy min <= max <= 127"));
    }
    if [o.kick_key, o.snare_key, o.hihat_key]
        .iter()
        .any(|k| *k > 127)
    {
        return Err(invalid("drum keys must be MIDI keys (0..=127)"));
    }
    Ok(Options {
        sensitivity: o.sensitivity,
        min_duration: o.min_duration.0,
        min_pitch: o.min_pitch,
        max_pitch: o.max_pitch,
        kick_key: o.kick_key,
        snare_key: o.snare_key,
        hihat_key: o.hihat_key,
    })
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn audio_to_midi_command(
        &mut self,
        command: &AudioToMidiCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = now;
        match command {
            AudioToMidiCommand::Cancel { job } => {
                let state = &mut self.audio_to_midi;
                if state.running.as_ref().is_some_and(|r| &r.id == job) {
                    state.running = None;
                    a2m_event(out, AudioToMidiEvent::Cancelled { job: job.clone() });
                }
                Ok(ReplyValue::Unit)
            }
            AudioToMidiCommand::Start {
                job,
                clip,
                mode,
                options,
                track,
                new_clip,
                seed_notes,
                instrument,
            } => {
                if job.is_empty() {
                    return Err(invalid("empty job id"));
                }
                if self.audio_to_midi.running.is_some() {
                    return Err(invalid_state(
                        "an audio to MIDI conversion is already running",
                    ));
                }
                let options = options_of(options)?;
                let p = &self
                    .doc
                    .as_ref()
                    .ok_or_else(|| invalid_state("no project is open"))?
                    .project;
                let c = p
                    .clips
                    .get(clip)
                    .ok_or_else(|| not_found(format!("clip {clip}")))?;
                let ClipContent::Audio(a) = &c.content else {
                    return Err(invalid(format!("clip {clip} is not an audio clip")));
                };
                let media = p
                    .media
                    .get(&a.media)
                    .ok_or_else(|| not_found(format!("media {}", a.media)))?
                    .clone();
                if self.media.is_missing(media.id) {
                    return Err(invalid_state(format!(
                        "\"{}\" is missing: relink it first",
                        media.name
                    )));
                }
                if p.tracks.contains_key(track) {
                    return Err(invalid(format!("track {track} already exists")));
                }
                if p.clips.contains_key(new_clip) {
                    return Err(invalid(format!("clip {new_clip} already exists")));
                }
                if let Some(d) = instrument
                    && p.devices.contains_key(d)
                {
                    return Err(invalid(format!("device {d} already exists")));
                }
                self.audio_to_midi.running = Some(Box::new(Job {
                    id: job.clone(),
                    project: p.id,
                    clip: *clip,
                    media,
                    mode: *mode,
                    options,
                    track: *track,
                    new_clip: *new_clip,
                    seed_notes: *seed_notes,
                    instrument: *instrument,
                    stage: Stage::Read,
                    seconds: 0.0,
                }));
                self.audio_to_midi.last_progress = None;
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// Called every tick: advance the running job (see the module docs).
    pub(crate) fn audio_to_midi_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(mut job) = self.audio_to_midi.running.take() else {
            return;
        };
        // Cancelled when its project or clip goes away (or stops being that audio clip).
        let alive = self.doc.as_ref().is_some_and(|d| {
            d.project.id == job.project
                && d.project.clips.get(&job.clip).is_some_and(
                    |c| matches!(&c.content, ClipContent::Audio(a) if a.media == job.media.id),
                )
        });
        if !alive {
            a2m_event(out, AudioToMidiEvent::Cancelled { job: job.id });
            return;
        }
        let started = self.host.now_ms().max(now);
        let mut outcome = Outcome::Working;
        for _ in 0..MAX_UNITS_PER_TICK {
            outcome = self.advance(&mut job);
            if !matches!(outcome, Outcome::Working) {
                break;
            }
            if self.host.now_ms().saturating_sub(started) >= TICK_BUDGET_MS {
                break;
            }
        }
        match outcome {
            Outcome::Working => {
                let progress = job.progress();
                let state = &mut self.audio_to_midi;
                if state
                    .last_progress
                    .is_none_or(|p| (progress - p).abs() >= 0.005)
                {
                    state.last_progress = Some(progress);
                    a2m_event(
                        out,
                        AudioToMidiEvent::Progress {
                            job: job.id.clone(),
                            progress,
                        },
                    );
                }
                self.audio_to_midi.running = Some(job);
            }
            Outcome::Failed(message) => {
                a2m_event(
                    out,
                    AudioToMidiEvent::Failed {
                        job: job.id,
                        message,
                    },
                );
            }
            Outcome::Detected(notes) => self.finish(&job, &notes, now, out),
        }
    }

    /// One unit of work.
    fn advance(&mut self, job: &mut Job) -> Outcome {
        match &mut job.stage {
            Stage::Read => {
                let bytes = match read_media_bytes(
                    &mut self.store,
                    &mut self.library,
                    job.project,
                    &job.media,
                ) {
                    Ok(b) => b,
                    Err(e) => {
                        return Outcome::Failed(format!(
                            "\"{}\" could not be read: {e}",
                            job.media.name
                        ));
                    }
                };
                match IncrementalDecoder::new(bytes.into(), extension_of(&job.media.file)) {
                    Ok(d) => job.stage = Stage::Decode(Box::new(d)),
                    Err(e) => {
                        return Outcome::Failed(format!(
                            "\"{}\" could not be decoded: {e}",
                            job.media.name
                        ));
                    }
                }
                Outcome::Working
            }
            Stage::Decode(dec) => match dec.step(UNIT_FRAMES) {
                Ok(false) => Outcome::Working,
                Ok(true) => {
                    let Stage::Decode(dec) = std::mem::replace(&mut job.stage, Stage::Read) else {
                        unreachable!()
                    };
                    match dec.finish() {
                        Ok(audio) => {
                            job.seconds = audio.frames() as f64 / audio.sample_rate.max(1) as f64;
                            job.stage = Stage::Detect(Box::new(Detector::new(
                                Arc::new(audio),
                                mode_of(job.mode),
                                job.options,
                            )));
                            Outcome::Working
                        }
                        Err(e) => Outcome::Failed(format!(
                            "\"{}\" could not be decoded: {e}",
                            job.media.name
                        )),
                    }
                }
                Err(e) => {
                    Outcome::Failed(format!("\"{}\" could not be decoded: {e}", job.media.name))
                }
            },
            Stage::Detect(det) => {
                if det.step(UNIT_FRAMES) >= 1.0 {
                    Outcome::Detected(det.notes())
                } else {
                    Outcome::Working
                }
            }
        }
    }

    /// Apply the result (one undo step), then report `Done` (or `Failed`).
    fn finish(
        &mut self,
        job: &Job,
        notes: &[ether_media::to_midi::DetectedNote],
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let result = place::Result {
            clip: job.clip,
            mode: job.mode,
            track: job.track,
            new_clip: job.new_clip,
            seed_notes: job.seed_notes,
            instrument: job.instrument,
            notes,
            media_seconds: job.seconds,
        };
        let mut count = 0;
        let applied = self.edit_with("Convert to MIDI", None, now, out, |ctx| {
            count = place::apply(ctx, &result)?;
            Ok(())
        });
        match applied {
            Ok(()) => {
                a2m_event(
                    out,
                    AudioToMidiEvent::Progress {
                        job: job.id.clone(),
                        progress: 1.0,
                    },
                );
                a2m_event(
                    out,
                    AudioToMidiEvent::Done {
                        job: job.id.clone(),
                        track: job.track,
                        clip: job.new_clip,
                        notes: count,
                    },
                );
            }
            Err(e) => a2m_event(
                out,
                AudioToMidiEvent::Failed {
                    job: job.id.clone(),
                    message: format!("Convert to MIDI: {}", e.message),
                },
            ),
        }
    }
}
