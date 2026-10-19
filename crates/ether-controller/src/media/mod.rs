//! Media pipeline: import (a reference in place or a copy into the project), then — off the hot path, stepped from
//! `tick()` a frame budget at a time — decode, peaks (from the *source-rate* audio, since
//! `PeakRequest` frames are source frames), resample to the engine rate, and hand the
//! result to the engine (`EngineBridge::load_media`).
//!
//! Peak mipmaps are cached in the project's `cache/` folder, keyed by `MediaRef.hash`.
//!
//! **Resolution** (`media-references`, CONTRACTS.md §12.9; [`read_media_bytes`]): an
//! `External` path whose content hash matches → the project copy at `MediaRef::file` →
//! missing (`MediaEvent::Missing`, silence until relinked). The pipeline remembers what it
//! loaded (by hash) and what was missing (by location and hash), so a relink, an undo of
//! one, or a collect reloads exactly the media that changed.

mod decode;
pub mod hash;
mod resample;

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use ether_core::protocol::Event;
use ether_core::protocol::NotificationLevel;
use ether_core::protocol::media::MediaEvent;
use ether_core::protocol::media_refs::MediaRefEvent;
use ether_core::protocol::model::file::CACHE_DIR;
use ether_core::protocol::model::{MediaId, MediaLocation, MediaRef, Project, ProjectId};
use ether_media::{DecodedAudio, MAX_PAD_FRAMES, MediaError, PeakMipmap};

pub use decode::IncrementalDecoder;
pub use decode::is_chained_ogg;
pub use resample::{IncrementalResampler, StreamResampler};

use crate::EngineBridge;
use crate::media_refs::RefsState;
use crate::store::{Library, ProjectStore, StoreError};

/// The audio bytes of `m` on this site (see the module docs for the resolution order). An
/// external file whose content no longer matches `m.hash` is not used (it may be another
/// take saved over the original); `NotFound` = missing.
pub fn read_media_bytes<S: ProjectStore, L: Library>(
    store: &mut S,
    library: &mut L,
    project: ProjectId,
    m: &MediaRef,
) -> Result<Vec<u8>, StoreError> {
    resolve_media_bytes(store, library, project, m).map(|(bytes, _)| bytes)
}

/// [`read_media_bytes`], plus the external path the bytes came from (`None` = the project
/// copy at `MediaRef::file`).
pub fn resolve_media_bytes<S: ProjectStore, L: Library>(
    store: &mut S,
    library: &mut L,
    project: ProjectId,
    m: &MediaRef,
) -> Result<(Vec<u8>, Option<String>), StoreError> {
    if let MediaLocation::External { path } = &m.location {
        if let Ok(bytes) = library.read_external(path)
            && m.hash
                .as_deref()
                .is_none_or(|h| h == hash::content_hash(&bytes))
        {
            return Ok((bytes, Some(path.clone())));
        }
        return store
            .read(project, &m.file)
            .map(|b| (b, None))
            .map_err(|_| StoreError::NotFound(path.clone()));
    }
    store.read(project, &m.file).map(|b| (b, None))
}

pub fn peaks_cache_path(hash: &str) -> String {
    format!("{CACHE_DIR}/{hash}.peaks")
}

pub fn extension_of(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rfind('.').filter(|i| *i > 0).map(|i| &name[i + 1..])
}

pub fn chained_ogg_warning(name: &str) -> crate::text::Msg {
    crate::text::msg("notice.media.chained_ogg")
        .arg("name", name)
        .into()
}

/// A notification event.
fn notification(level: NotificationLevel, m: impl Into<crate::text::Msg>) -> Event {
    let m = m.into();
    Event::Notification {
        level,
        message: m.text,
        key: m.key,
    }
}

enum Stage {
    /// Read the file from the store (and the peaks cache).
    Read,
    Decode(Box<IncrementalDecoder>),
    Resample(Box<IncrementalResampler>),
}

struct Job {
    media: MediaRef,
    stage: Stage,
    /// Emit `ImportProgress` (user-initiated imports).
    report: bool,
    /// The chained-Ogg warning was already given (at import).
    warned: bool,
    /// `audio-streaming`: the host streams this media from disk; the job only builds the
    /// peaks (the decoder runs in peaks-only mode).
    streamed: bool,
    /// Streaming was considered (at the start of the decode stage).
    stream_checked: bool,
    /// The external file the bytes were read from (`None` = the project copy).
    external_path: Option<String>,
    /// #529: decoded again only for the bridge to build a convolution IR from
    /// (`EngineBridge::ir_media`): the first `MAX_IR_SECONDS`, no peaks, not loaded.
    ir: bool,
}

#[derive(Default)]
pub struct MediaState {
    peaks: BTreeMap<MediaId, PeakMipmap>,
    /// Media loaded into the engine, with the content hash they were loaded with.
    loaded: BTreeMap<MediaId, Option<String>>,
    jobs: VecDeque<Job>,
    /// Lengths learned by decoding media whose header had none (`MediaRef.frames == 0`).
    frame_fixups: Vec<(MediaId, u64)>,
    /// Unresolved media (`media-references`), with the location and hash that failed: a
    /// change of either (relink, undo, collect) retries.
    missing: BTreeMap<MediaId, (MediaLocation, Option<String>)>,
    /// Media reported missing since the project opened: loading one again reports
    /// `MediaRefEvent::Resolved`.
    reported: BTreeSet<MediaId>,
    /// The next [`Self::sync`] retries the missing media (they stay missing until loaded).
    recheck: bool,
    /// Background search for missing media (`media_refs`).
    pub refs: RefsState,
    /// Media handed to the engine during the current [`Self::step`] (streamed media are
    /// available before their job ends: the job still builds the peaks).
    just_loaded: Vec<MediaId>,
    /// `audio-streaming`: media the host streams from disk (vs decoded into memory).
    streamed: BTreeSet<MediaId>,
    /// `audio-streaming`: engine underruns not reported yet, and the last report (ms).
    pub underruns_pending: u32,
    pub underruns_at: u64,
    /// "No engine" (docs/COLLAB.md §15): nothing is decoded or handed to the engine.
    engine_off: bool,
    /// Project files still on their way (an extension fills them in later). Their media are
    /// not read (nor reported missing) until [`Self::reload_file`].
    incoming: BTreeSet<String>,
}

/// Decoded audio sized to the document's frame count (headers and decoders can disagree
/// by a few frames; the document is the reference, padded by at most `MAX_PAD_FRAMES`: it
/// comes from a project file or a peer and may claim far more than the file holds).
pub fn fit_frames(mut audio: DecodedAudio, frames: u64) -> DecodedAudio {
    let frames = frames.min((audio.frames() as u64).saturating_add(MAX_PAD_FRAMES)) as usize;
    for ch in &mut audio.channels {
        ch.resize(frames, 0.0);
    }
    audio
}

impl MediaState {
    pub fn peaks(&self, media: MediaId) -> Option<&PeakMipmap> {
        self.peaks.get(&media)
    }

    pub fn is_pending(&self, media: MediaId) -> bool {
        self.jobs.iter().any(|j| j.media.id == media && !j.ir)
    }

    /// #529: queue a decode of the media the bridge asked for to build convolution IRs
    /// (`EngineBridge::take_ir_media_requests`); handed over with `EngineBridge::ir_media`.
    /// The audio isn't kept here: a later need decodes it again.
    pub fn queue_ir_decodes<B: EngineBridge + ?Sized>(
        &mut self,
        bridge: &mut B,
        project: &Project,
    ) {
        for id in bridge.take_ir_media_requests() {
            let Some(m) = project.media.get(&id) else {
                continue;
            };
            if self.engine_off || self.jobs.iter().any(|j| j.ir && j.media.id == id) {
                continue;
            }
            self.jobs.push_back(Job {
                media: m.clone(),
                stage: Stage::Read,
                report: false,
                warned: true,
                streamed: false,
                stream_checked: true,
                external_path: None,
                ir: true,
            });
        }
    }

    /// Media lengths learned by decoding (see `MediaRef.frames == 0`).
    pub fn take_frame_fixups(&mut self) -> Vec<(MediaId, u64)> {
        std::mem::take(&mut self.frame_fixups)
    }

    pub fn has_jobs(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// `audio-streaming`: are any media of the project streamed?
    pub fn any_streamed(&self) -> bool {
        !self.streamed.is_empty()
    }

    /// The currently unresolved media (`MediaRef::ListMissing`).
    pub fn missing(&self) -> Vec<MediaId> {
        self.missing.keys().copied().collect()
    }

    pub fn is_missing(&self, media: MediaId) -> bool {
        self.missing.contains_key(&media)
    }

    /// The project file `file` was replaced (collab: a peer's relink to other content):
    /// reload the media stored there.
    /// The next [`Self::sync`] queues them again.
    pub fn reload_file<B: EngineBridge + ?Sized>(
        &mut self,
        bridge: &mut B,
        project: &Project,
        file: &str,
    ) {
        self.incoming.remove(file);
        for m in project.media.values().filter(|m| m.file == file) {
            self.streamed.remove(&m.id);
            if self.loaded.remove(&m.id).is_some() {
                let _ = bridge.unload_media(m.id);
            }
            self.peaks.remove(&m.id);
            self.missing.remove(&m.id);
            self.jobs.retain(|j| j.media.id != m.id);
        }
    }

    /// These project files are on their way: their media wait for [`Self::reload_file`] (no
    /// read, no "missing" report).
    pub fn set_incoming(&mut self, project: &Project, files: BTreeSet<String>) {
        let ids: BTreeSet<MediaId> = project
            .media
            .values()
            .filter(|m| files.contains(&m.file))
            .map(|m| m.id)
            .collect();
        self.jobs.retain(|j| !ids.contains(&j.media.id));
        for id in &ids {
            self.missing.remove(id);
        }
        self.incoming = files;
    }

    /// No file is on its way any more.
    pub fn clear_incoming(&mut self) {
        self.incoming.clear();
        self.recheck = true;
    }

    /// Check the missing media again (a file may have been restored): they are retried by
    /// the next [`Self::sync`].
    pub fn recheck_missing(&mut self) {
        self.recheck = true;
    }

    /// Queue an import whose decoder was already created (header probed in `handle`).
    pub fn queue_import(&mut self, media: MediaRef, decoder: IncrementalDecoder, warned: bool) {
        self.jobs.retain(|j| j.media.id != media.id);
        self.missing.remove(&media.id);
        self.jobs.push_back(Job {
            media,
            stage: Stage::Decode(Box::new(decoder)),
            report: true,
            warned,
            streamed: false,
            stream_checked: false,
            external_path: None,
            ir: false,
        });
    }

    /// Forget everything (project switch). Engine sources are replaced/unloaded.
    pub fn reset<B: EngineBridge + ?Sized>(&mut self, bridge: &mut B) {
        for m in std::mem::take(&mut self.loaded).into_keys() {
            let _ = bridge.unload_media(m);
        }
        self.streamed.clear();
        self.peaks.clear();
        self.jobs.clear();
        self.missing.clear();
        self.reported.clear();
        self.incoming.clear();
        self.refs = RefsState::default();
    }

    /// Reload every media (engine sample rate changed).
    pub fn reload_all(&mut self) {
        self.loaded.clear();
        self.streamed.clear();
        self.jobs.clear();
        self.missing.clear();
    }

    /// Turn the engine off (unload what the engine holds, queue nothing) or on (the next
    /// [`Self::sync`] queues every media again).
    pub fn set_engine_off<B: EngineBridge + ?Sized>(&mut self, bridge: &mut B, off: bool) {
        self.engine_off = off;
        if off {
            for m in std::mem::take(&mut self.loaded).into_keys() {
                let _ = bridge.unload_media(m);
            }
            self.streamed.clear();
            self.jobs.clear();
            self.missing.clear();
            self.just_loaded.clear();
        }
    }

    /// Match the document: queue loads for new media, unload removed media.
    pub fn sync<B: EngineBridge + ?Sized>(&mut self, bridge: &mut B, project: Option<&Project>) {
        let empty = BTreeMap::new();
        let media = project.map_or(&empty, |p| &p.media);
        // Removed media, and media whose content changed (a relink to other content, or its
        // undo): unloaded, and the latter reloaded below.
        let gone: Vec<MediaId> = self
            .loaded
            .iter()
            .filter(|(m, hash)| media.get(m).is_none_or(|doc| doc.hash != **hash))
            .map(|(m, _)| *m)
            .collect();
        for m in gone {
            self.loaded.remove(&m);
            self.streamed.remove(&m);
            self.peaks.remove(&m);
            let _ = bridge.unload_media(m);
        }
        self.peaks.retain(|m, _| media.contains_key(m));
        self.jobs.retain(|j| {
            media
                .get(&j.media.id)
                .is_some_and(|m| m.hash == j.media.hash && m.location == j.media.location)
        });
        self.missing.retain(|id, (location, hash)| {
            media
                .get(id)
                .is_some_and(|m| m.location == *location && m.hash == *hash)
        });
        self.reported.retain(|m| media.contains_key(m));
        if self.engine_off {
            return;
        }
        for m in media.values() {
            if !self.loaded.contains_key(&m.id)
                && !self.is_pending(m.id)
                && !self.incoming.contains(&m.file)
                && (self.recheck || !self.missing.contains_key(&m.id))
            {
                self.jobs.push_back(Job {
                    media: m.clone(),
                    stage: Stage::Read,
                    report: false,
                    warned: false,
                    streamed: false,
                    stream_checked: false,
                    external_path: None,
                    ir: false,
                });
            }
        }
        self.recheck = false;
    }

    /// Advance jobs by about `budget` frames of work. Returns the media that became
    /// available to the engine.
    #[allow(clippy::too_many_arguments)]
    pub fn step<S: ProjectStore, L: Library, B: EngineBridge + ?Sized>(
        &mut self,
        project: ProjectId,
        store: &mut S,
        library: &mut L,
        bridge: &mut B,
        engine_rate: u32,
        budget: usize,
        events: &mut Vec<Event>,
    ) -> Vec<MediaId> {
        let mut loaded = Vec::new();
        let mut budget = budget.max(1) as isize;
        let mut reported = BTreeSet::new();
        while budget > 0 {
            let Some(mut job) = self.jobs.pop_front() else {
                break;
            };
            let res = self.advance(
                &mut job,
                project,
                store,
                library,
                bridge,
                engine_rate,
                &mut budget,
                events,
            );
            loaded.append(&mut self.just_loaded);
            match res {
                // #529: a decode for an IR only; failures leave the IR as it was.
                Ok(true) | Err(_) if job.ir => {}
                Ok(false) if job.ir => self.jobs.push_front(job),
                Ok(true) => {
                    self.missing.remove(&job.media.id);
                    if self.reported.remove(&job.media.id) {
                        events.push(Event::MediaRef {
                            event: MediaRefEvent::Resolved {
                                media: job.media.id,
                            },
                        });
                    }
                    if job.report {
                        events.push(Event::Media {
                            event: MediaEvent::ImportProgress {
                                media: job.media.id,
                                progress: 1.0,
                            },
                        });
                    }
                }
                Ok(false) => {
                    if job.report && reported.insert(job.media.id) {
                        events.push(Event::Media {
                            event: MediaEvent::ImportProgress {
                                media: job.media.id,
                                progress: job_progress(&job),
                            },
                        });
                    }
                    self.jobs.push_front(job);
                }
                // Missing: reported by `advance` (the clips show it; no toast per file).
                Err(_) if self.missing.contains_key(&job.media.id) => {}
                Err(e) => {
                    events.push(notification(
                        NotificationLevel::Error,
                        crate::text::msg("notice.media.load_failed")
                            .arg("name", job.media.name.as_str())
                            .arg("error", e.to_string()),
                    ));
                }
            }
        }
        loaded
    }

    /// Run one job until done or out of budget. `Ok(true)` = done (the media was handed to
    /// the engine: listed in `just_loaded`, possibly by an earlier call for streamed media).
    #[allow(clippy::too_many_arguments)]
    fn advance<S: ProjectStore, L: Library, B: EngineBridge + ?Sized>(
        &mut self,
        job: &mut Job,
        project: ProjectId,
        store: &mut S,
        library: &mut L,
        bridge: &mut B,
        engine_rate: u32,
        budget: &mut isize,
        events: &mut Vec<Event>,
    ) -> Result<bool, MediaError> {
        loop {
            match &mut job.stage {
                Stage::Read => {
                    let (bytes, external_path) =
                        match resolve_media_bytes(store, library, project, &job.media) {
                            Ok(b) => b,
                            Err(e) if job.ir => return Err(MediaError::Decode(e.to_string())),
                            Err(e) => {
                                self.missing.insert(
                                    job.media.id,
                                    (job.media.location.clone(), job.media.hash.clone()),
                                );
                                self.reported.insert(job.media.id);
                                events.push(Event::Media {
                                    event: MediaEvent::Missing {
                                        media: job.media.id,
                                    },
                                });
                                return Err(MediaError::Decode(e.to_string()));
                            }
                        };
                    *budget -= (bytes.len() / 64) as isize;
                    if !job.ir
                        && !self.peaks.contains_key(&job.media.id)
                        && let Some(hash) = &job.media.hash
                        && let Ok(cached) = store.read(project, &peaks_cache_path(hash))
                        && let Ok(peaks) = PeakMipmap::from_bytes(&cached)
                    {
                        self.peaks.insert(job.media.id, peaks);
                        events.push(Event::Media {
                            event: MediaEvent::PeaksReady {
                                media: job.media.id,
                            },
                        });
                    }
                    let chained = is_chained_ogg(&bytes);
                    let mut decoder =
                        IncrementalDecoder::new(bytes.into(), extension_of(&job.media.file))?;
                    if job.ir {
                        // A little over the longest IR kept: the resampler's edge.
                        let rate = match decoder.sample_rate {
                            0 => job.media.sample_rate,
                            r => r,
                        };
                        if rate > 0 {
                            let secs = ether_devices::fx_space::ir::MAX_IR_SECONDS + 0.1;
                            decoder.limit((secs * rate as f64).ceil() as usize);
                        }
                    }
                    if chained && !job.warned {
                        job.warned = true;
                        events.push(notification(
                            NotificationLevel::Warning,
                            chained_ogg_warning(&job.media.name),
                        ));
                    }
                    job.external_path = external_path;
                    job.stage = Stage::Decode(Box::new(decoder));
                }
                Stage::Decode(dec) => {
                    // `audio-streaming`: long media stream from disk when the host can (once
                    // per job, before decoding: imports arrive here directly).
                    if !job.stream_checked {
                        job.stream_checked = true;
                        if crate::media_stream::should_stream(&job.media, engine_rate) {
                            let external_path = match &job.media.location {
                                // An import references the file it just read.
                                MediaLocation::External { path } if job.report => {
                                    Some(path.clone())
                                }
                                _ => job.external_path.take(),
                            };
                            let source = crate::media_stream::StreamSource {
                                project,
                                media: job.media.clone(),
                                external_path,
                                engine_sample_rate: engine_rate,
                            };
                            match bridge.stream_media(&source) {
                                Ok(true) => {
                                    job.streamed = true;
                                    self.loaded.insert(job.media.id, job.media.hash.clone());
                                    self.streamed.insert(job.media.id);
                                    self.just_loaded.push(job.media.id);
                                    if self.peaks.contains_key(&job.media.id) {
                                        return Ok(true);
                                    }
                                    dec.peaks_only(job.media.frames);
                                }
                                Ok(false) => {}
                                // The host could not stream it (file unreadable by path,
                                // ...): decode it whole as before.
                                Err(_) => {}
                            }
                        }
                    }
                    let before = dec.decoded_frames();
                    let done = dec.step((*budget).max(1) as usize)?;
                    *budget -= (dec.decoded_frames() - before) as isize;
                    if !done {
                        return Ok(false);
                    }
                    let Stage::Decode(dec) = std::mem::replace(&mut job.stage, Stage::Read) else {
                        unreachable!()
                    };
                    let truncated = dec.truncated;
                    if job.ir {
                        let audio = dec.finish()?;
                        *budget -= (audio.frames() / 16) as isize;
                        job.stage = Stage::Resample(Box::new(IncrementalResampler::new(
                            Arc::new(audio),
                            engine_rate,
                        )?));
                        continue;
                    }
                    if job.streamed {
                        // Peaks-only pass of a streamed media: the engine already has it.
                        if truncated && !job.warned {
                            job.warned = true;
                            events.push(notification(
                                NotificationLevel::Warning,
                                chained_ogg_warning(&job.media.name),
                            ));
                        }
                        let peaks = dec.finish_peaks()?;
                        if let Some(hash) = &job.media.hash {
                            let _ =
                                store.write(project, &peaks_cache_path(hash), &peaks.to_bytes());
                        }
                        self.peaks.insert(job.media.id, peaks);
                        events.push(Event::Media {
                            event: MediaEvent::PeaksReady {
                                media: job.media.id,
                            },
                        });
                        return Ok(true);
                    }
                    let audio = dec.finish()?;
                    let audio = if job.media.frames == 0 {
                        let frames = audio.frames() as u64;
                        job.media.frames = frames;
                        self.frame_fixups.push((job.media.id, frames));
                        audio
                    } else {
                        fit_frames(audio, job.media.frames)
                    };
                    if truncated && !job.warned {
                        job.warned = true;
                        events.push(notification(
                            NotificationLevel::Warning,
                            chained_ogg_warning(&job.media.name),
                        ));
                    }
                    if let Entry::Vacant(slot) = self.peaks.entry(job.media.id) {
                        let peaks = PeakMipmap::build(&audio);
                        if let Some(hash) = &job.media.hash {
                            let _ =
                                store.write(project, &peaks_cache_path(hash), &peaks.to_bytes());
                        }
                        slot.insert(peaks);
                        events.push(Event::Media {
                            event: MediaEvent::PeaksReady {
                                media: job.media.id,
                            },
                        });
                    }
                    *budget -= (audio.frames() / 16) as isize;
                    job.stage = Stage::Resample(Box::new(IncrementalResampler::new(
                        Arc::new(audio),
                        engine_rate,
                    )?));
                }
                Stage::Resample(r) => {
                    let done = r.step((*budget).max(1) as usize)?;
                    *budget -= (*budget).max(1);
                    if !done {
                        return Ok(false);
                    }
                    let Stage::Resample(r) = std::mem::replace(&mut job.stage, Stage::Read) else {
                        unreachable!()
                    };
                    let audio = Arc::new(r.finish());
                    if job.ir {
                        bridge
                            .ir_media(job.media.id, audio)
                            .map_err(|e| MediaError::Decode(format!("engine: {e}")))?;
                        return Ok(true);
                    }
                    bridge
                        .load_media(&job.media, audio)
                        .map_err(|e| MediaError::Decode(format!("engine: {e}")))?;
                    self.loaded.insert(job.media.id, job.media.hash.clone());
                    self.just_loaded.push(job.media.id);
                    return Ok(true);
                }
            }
        }
    }
}

fn job_progress(job: &Job) -> f32 {
    match &job.stage {
        Stage::Read => 0.0,
        Stage::Decode(d) => {
            let total = job.media.frames.max(1) as f32;
            let done = (d.decoded_frames() as f32 / total).min(1.0);
            // A streamed media's job is only the peaks pass.
            if job.streamed { done } else { 0.5 * done }
        }
        Stage::Resample(r) => 0.5 + 0.5 * r.progress(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_frames_pads_a_bounded_amount() {
        let audio = || DecodedAudio {
            sample_rate: 48_000,
            channels: vec![vec![0.5; 100]],
        };
        assert_eq!(fit_frames(audio(), 150).frames(), 150);
        assert_eq!(fit_frames(audio(), 70).frames(), 70);
        // A document claiming far more than the file holds does not allocate it.
        let fitted = fit_frames(audio(), u64::MAX);
        assert_eq!(fitted.frames() as u64, 100 + MAX_PAD_FRAMES);
    }
}
