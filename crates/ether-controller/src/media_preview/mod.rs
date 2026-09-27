//! Browser sample preview (owned by the `media-preview` node; see `docs/ROADMAP.md` and
//! CONTRACTS.md §11.15).
//!
//! `Media::Preview { source }` resolves the source (a library file, a file in the project's
//! `media/` folder, or project media), reads its bytes and probes the header in `handle`
//! (errors are replied), then decodes it with the controller's incremental decoder and
//! resamples it to the engine rate, a frame budget at a time: the first budget runs right in
//! the command (a typical one-shot sample starts before the reply), longer files continue
//! from [`EtherController::preview_tick`]. Previews are capped at [`MAX_PREVIEW_SECONDS`]
//! of source audio, so auditioning a long song starts quickly and holds bounded memory.
//! The result goes to `EngineBridge::preview` with a new monotonic preview id, and
//! `MediaEvent::PreviewStarted` is emitted. A small cache of recent decodes makes
//! auditioning the same sample twice instant.
//!
//! **Ids and events (frozen).** The controller owns the current preview id. A new
//! `Preview` or a `StopPreview` ends the current preview *at once* with
//! `PreviewEnded { Replaced | Stopped }` (whether it was still decoding or already playing)
//! and the new preview becomes current before the engine hears about it. The engine reports
//! natural ends only (`EngineOutputs::preview_ended = Some(id)`), and `preview_tick` emits
//! `PreviewEnded { Finished }` only when that id is still the current, started preview: a
//! late end of a replaced or stopped preview, even one in the same poll as the replace, is
//! ignored. A failed decode or engine call emits `PreviewEnded { Failed }` plus an error
//! notification. So every accepted preview gets exactly one `PreviewEnded`.
//!
//! Hosts whose bridge doesn't support previews reply `Unsupported` to both commands (the
//! engine is asked to stop first, which is how support is probed).

use std::collections::VecDeque;
use std::sync::Arc;

use ether_core::protocol::media::{
    BrowseLocation, MediaCommand, MediaEvent, MediaSource, PreviewEndReason,
};
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::model::file::MEDIA_DIR;
use ether_core::protocol::{CommandError, ErrorCode, Event, NotificationLevel, ReplyValue};
use ether_media::{DecodedAudio, MediaError};

use crate::handlers::{event, no_project, notify, store_err};
use crate::media::{IncrementalDecoder, IncrementalResampler, extension_of};
use crate::store::{Library, ProjectStore, check_relative_path};
use crate::tx::{CmdResult, cmd_err, internal, invalid, not_found, unsupported};
use crate::{BridgeError, EngineBridge, EtherController, HostServices, MessageSink};

/// Longest preview, in seconds of source audio (the rest of the file is not decoded).
pub const MAX_PREVIEW_SECONDS: u64 = 30;
/// Linear gain of the preview voice.
pub const PREVIEW_GAIN: f32 = 1.0;
/// Recent decodes kept for instant re-previews.
const CACHE_ENTRIES: usize = 4;

/// Identity of a decoded preview: the source, the project for project-relative sources,
/// and the engine rate it was resampled to.
#[derive(Clone, PartialEq)]
struct CacheKey {
    source: MediaSource,
    project: Option<ProjectId>,
    rate: u32,
}

enum Stage {
    Decode(Box<IncrementalDecoder>),
    Resample(Box<IncrementalResampler>),
}

struct Job {
    stage: Stage,
    key: CacheKey,
    /// Source frames to decode at most.
    cap: u64,
}

struct Current {
    id: u64,
    source: MediaSource,
    name: String,
    /// Still decoding (`None` once handed to the engine).
    job: Option<Job>,
}

/// Preview runtime state (one field on `EtherController`).
#[derive(Default)]
pub(crate) struct PreviewState {
    /// Last id handed out (ids start at 1).
    last_id: u64,
    current: Option<Current>,
    cache: VecDeque<(CacheKey, Arc<DecodedAudio>)>,
}

impl PreviewState {
    fn cached(&mut self, key: &CacheKey) -> Option<Arc<DecodedAudio>> {
        let i = self.cache.iter().position(|(k, _)| k == key)?;
        let entry = self.cache.remove(i)?;
        let audio = entry.1.clone();
        self.cache.push_front(entry);
        Some(audio)
    }

    fn remember(&mut self, key: CacheKey, audio: Arc<DecodedAudio>) {
        self.cache.retain(|(k, _)| *k != key);
        self.cache.push_front((key, audio));
        self.cache.truncate(CACHE_ENTRIES);
    }
}

fn bridge_err(e: BridgeError) -> CommandError {
    match e {
        BridgeError::Unsupported(m) => unsupported(m),
        other => internal(other.to_string()),
    }
}

fn ended(source: MediaSource, reason: PreviewEndReason) -> Event {
    Event::Media {
        event: MediaEvent::PreviewEnded { source, reason },
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Keep at most `frames` frames.
fn truncate(mut audio: DecodedAudio, frames: u64) -> DecodedAudio {
    for ch in &mut audio.channels {
        ch.truncate(frames as usize);
    }
    audio
}

enum Progress {
    Pending(Job),
    Done(CacheKey, Arc<DecodedAudio>),
}

/// Run `job` for about `budget` frames of work (decode, then resample to `rate`).
fn advance(mut job: Job, budget: usize, rate: u32) -> Result<Progress, MediaError> {
    let mut budget = budget.max(1) as isize;
    while budget > 0 {
        job.stage = match job.stage {
            Stage::Decode(mut dec) => {
                let before = dec.decoded_frames();
                let left = job.cap.saturating_sub(before as u64).max(1) as usize;
                let finished = dec.step((budget as usize).min(left))?;
                budget -= (dec.decoded_frames() - before).max(1) as isize;
                if !finished && (dec.decoded_frames() as u64) < job.cap {
                    Stage::Decode(dec)
                } else {
                    let audio = truncate(dec.finish()?, job.cap);
                    budget -= (audio.frames() / 16) as isize;
                    Stage::Resample(Box::new(IncrementalResampler::new(Arc::new(audio), rate)?))
                }
            }
            Stage::Resample(mut r) => {
                let done = r.step(budget as usize)?;
                budget = 0;
                if done {
                    return Ok(Progress::Done(job.key, Arc::new(r.finish())));
                }
                Stage::Resample(r)
            }
        };
    }
    Ok(Progress::Pending(job))
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Media::Preview` / `Media::StopPreview`.
    pub(crate) fn preview_command(
        &mut self,
        c: &MediaCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            MediaCommand::Preview { source } => self.start_preview(source, out)?,
            _ => {
                let id = self.preview.current.as_ref().map_or(0, |c| c.id);
                self.bridge.preview(id, None, 0.0).map_err(bridge_err)?;
                self.end_preview(PreviewEndReason::Stopped, out);
            }
        }
        Ok(ReplyValue::Unit)
    }

    /// Called every tick (after the engine outputs were polled).
    pub(crate) fn preview_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = now;
        if let Some(id) = self.outputs.preview_ended.take()
            && self
                .preview
                .current
                .as_ref()
                .is_some_and(|c| c.id == id && c.job.is_none())
        {
            self.end_preview(PreviewEndReason::Finished, out);
        }
        self.step_preview(out);
    }

    fn start_preview(&mut self, source: &MediaSource, out: &mut dyn MessageSink) -> CmdResult<()> {
        let rate = self.config.engine_sample_rate;
        let project = match source {
            MediaSource::Location {
                location: BrowseLocation::Library { .. },
                ..
            }
            | MediaSource::Upload { .. } => None,
            _ => Some(self.doc.as_ref().ok_or_else(no_project)?.project.id),
        };
        let key = CacheKey {
            source: source.clone(),
            project,
            rate,
        };
        let (name, job) = match self.preview.cached(&key) {
            Some(audio) => (self.preview_name(source), Ok(audio)),
            None => {
                let (bytes, name) = self.read_preview_source(source)?;
                let mut decoder = IncrementalDecoder::new(bytes.into(), extension_of(&name))
                    .map_err(|e| cmd_err(ErrorCode::Decode, e.to_string()))?;
                if decoder.sample_rate == 0 || decoder.channels == 0 {
                    decoder
                        .step(1)
                        .map_err(|e| cmd_err(ErrorCode::Decode, e.to_string()))?;
                }
                if decoder.sample_rate == 0 {
                    return Err(cmd_err(ErrorCode::Decode, "no decodable audio"));
                }
                let cap = MAX_PREVIEW_SECONDS * decoder.sample_rate as u64;
                // Stop the playing preview now (it is replaced), which also tells whether
                // this host can preview at all.
                self.bridge.preview(0, None, 0.0).map_err(bridge_err)?;
                let job = Job {
                    stage: Stage::Decode(Box::new(decoder)),
                    key,
                    cap,
                };
                (name, Err(job))
            }
        };
        self.preview.last_id += 1;
        let id = self.preview.last_id;
        match job {
            Ok(audio) => {
                // Cached: straight to the engine (this replaces the playing one, with a
                // crossfade).
                self.bridge
                    .preview(id, Some(audio), PREVIEW_GAIN)
                    .map_err(bridge_err)?;
                self.end_preview(PreviewEndReason::Replaced, out);
                self.preview.current = Some(Current {
                    id,
                    source: source.clone(),
                    name,
                    job: None,
                });
                event(
                    out,
                    Event::Media {
                        event: MediaEvent::PreviewStarted {
                            source: source.clone(),
                        },
                    },
                );
            }
            Err(job) => {
                self.end_preview(PreviewEndReason::Replaced, out);
                self.preview.current = Some(Current {
                    id,
                    source: source.clone(),
                    name,
                    job: Some(job),
                });
                self.step_preview(out);
            }
        }
        Ok(())
    }

    /// End the current preview (if any) with `reason`.
    fn end_preview(&mut self, reason: PreviewEndReason, out: &mut dyn MessageSink) {
        if let Some(c) = self.preview.current.take() {
            event(out, ended(c.source, reason));
        }
    }

    fn preview_name(&self, source: &MediaSource) -> String {
        match source {
            MediaSource::Location { path, .. } => basename(path).to_string(),
            MediaSource::Project { media } => self
                .doc
                .as_ref()
                .and_then(|d| d.project.media.get(media))
                .map_or_else(|| media.to_string(), |m| m.name.clone()),
            MediaSource::Upload { upload } => upload.clone(),
        }
    }

    /// Bytes and file name of a preview source.
    fn read_preview_source(&mut self, source: &MediaSource) -> CmdResult<(Vec<u8>, String)> {
        match source {
            MediaSource::Location { location, path } => {
                check_relative_path(path).map_err(store_err)?;
                if path.is_empty() {
                    return Err(invalid("path must name a file"));
                }
                let bytes = match location {
                    BrowseLocation::Library { id } => {
                        self.library.read(id, path).map_err(store_err)?
                    }
                    BrowseLocation::ProjectMedia => {
                        let pid = self.doc.as_ref().ok_or_else(no_project)?.project.id;
                        self.store
                            .read(pid, &format!("{MEDIA_DIR}/{path}"))
                            .map_err(store_err)?
                    }
                };
                Ok((bytes, basename(path).to_string()))
            }
            MediaSource::Project { media } => {
                let doc = self.doc.as_ref().ok_or_else(no_project)?;
                let m = doc
                    .project
                    .media
                    .get(media)
                    .ok_or_else(|| not_found(format!("media {media}")))?;
                let (pid, file, name) = (doc.project.id, m.file.clone(), m.name.clone());
                let bytes = self.store.read(pid, &file).map_err(store_err)?;
                Ok((bytes, name))
            }
            MediaSource::Upload { .. } => Err(unsupported("previewing uploads is not supported")),
        }
    }

    /// Advance the current preview's decode by the per-tick media budget; hand it to the
    /// engine when done.
    fn step_preview(&mut self, out: &mut dyn MessageSink) {
        let Some(current) = self.preview.current.as_mut() else {
            return;
        };
        let Some(job) = current.job.take() else {
            return;
        };
        let (id, name, source) = (current.id, current.name.clone(), current.source.clone());
        let budget = self.config.media_frames_per_tick;
        match advance(job, budget, self.config.engine_sample_rate) {
            Ok(Progress::Pending(job)) => current.job = Some(job),
            Ok(Progress::Done(key, audio)) => {
                match self.bridge.preview(id, Some(audio.clone()), PREVIEW_GAIN) {
                    Ok(()) => {
                        self.preview.remember(key, audio);
                        event(
                            out,
                            Event::Media {
                                event: MediaEvent::PreviewStarted { source },
                            },
                        );
                    }
                    Err(e) => self.fail_preview(&name, &e.to_string(), out),
                }
            }
            Err(e) => self.fail_preview(&name, &e.to_string(), out),
        }
    }

    fn fail_preview(&mut self, name: &str, error: &str, out: &mut dyn MessageSink) {
        notify(
            out,
            NotificationLevel::Error,
            format!("could not preview \"{name}\": {error}"),
        );
        self.end_preview(PreviewEndReason::Failed, out);
    }
}
