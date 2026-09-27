//! Offline export (roadmap v2, owned by the `export` node; see `docs/ROADMAP.md` and
//! `ether_protocol::export`).
//!
//! A job snapshots the document and builds an independent
//! `ether_core::offline::OfflineRenderer` per file at the engine rate: fresh device nodes
//! (`ether_devices::create` for built-ins, `EngineBridge::create_offline_plugin` for
//! plugins, with their current state; a plugin that can't be instantiated offline fails the
//! job with a clear error, it is never skipped silently), the project's media decoded and
//! resampled again with the live pipeline's code (bit-identical sources), and a graph from
//! `compile::compile_graph_with` with the click, looping and inputs off. Stems are one pass
//! per track ([`job::stem_graph`]). Rendering is stepped from
//! [`EtherController::export_tick`] a bounded amount per tick (so the controller stays
//! responsive), `latency()` leading frames are dropped, then each file is resampled
//! (optional), normalized (optional), dithered (16-bit) and encoded (WAV via `hound`, FLAC
//! via `flacenc`). Native hosts write `exports/<name>` through the `ProjectStore`; hosts
//! whose store says `Unsupported` (web/remote) keep the bytes for `Export::ReadChunk`.

mod encode;
mod job;

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::export::{
    AudioContainer, BitDepth, ByteChunk, ExportCommand, ExportEvent, ExportMode, ExportRequest,
    ExportResult,
};
use ether_core::protocol::model::{Base64Bytes, ProjectId, TrackId};
use ether_core::protocol::{Event, NotificationLevel, ReplyValue};

use crate::handlers::{event, notify};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

use job::{Delivered, Job, Step};

/// Most bytes one `ReadChunk` serves (the protocol promises at least 256 KiB).
pub(crate) const MAX_CHUNK_BYTES: usize = 1 << 20;
/// Work per tick: at most this many units of [`job::CHUNK`] frames...
const MAX_UNITS_PER_TICK: usize = 64;
/// ...and no more than this long (when the host clock advances).
const TICK_BUDGET_MS: u64 = 12;

struct Download {
    project: ProjectId,
    bytes: Vec<u8>,
}

/// The running job and finished downloads.
#[derive(Default)]
pub(crate) struct ExportState {
    job: Option<Box<Job>>,
    downloads: BTreeMap<String, Download>,
    last_progress: Option<f32>,
}

fn export_event(out: &mut dyn MessageSink, e: ExportEvent) {
    event(out, Event::Export { event: e });
}

/// Validate a request against the project: the stem tracks (deduplicated, in order).
fn validate(
    p: &ether_core::protocol::model::Project,
    r: &ExportRequest,
) -> CmdResult<Vec<TrackId>> {
    if r.format.container == AudioContainer::Flac && r.format.bit_depth == BitDepth::Float32 {
        return Err(invalid("FLAC supports 16 or 24 bits only"));
    }
    if let Some(rate) = r.format.sample_rate
        && !(8_000..=384_000).contains(&rate)
    {
        return Err(invalid(format!("unsupported sample rate {rate} Hz")));
    }
    if !(r.tail_seconds.is_finite() && r.tail_seconds >= 0.0) {
        return Err(invalid("tail must be >= 0 seconds"));
    }
    job::range_of(p, &r.range).map_err(invalid)?;
    match &r.mode {
        ExportMode::Mix => Ok(Vec::new()),
        ExportMode::Stems { tracks } => {
            if tracks.is_empty() {
                return Err(invalid("no tracks selected for stems"));
            }
            let mut seen = BTreeSet::new();
            let mut out = Vec::new();
            for t in tracks {
                if !p.tracks.contains_key(t) {
                    return Err(not_found(format!("track {t}")));
                }
                if seen.insert(*t) {
                    out.push(*t);
                }
            }
            Ok(out)
        }
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Command::Export`.
    pub(crate) fn export_command(
        &mut self,
        c: &ExportCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            ExportCommand::Render { job, request } => {
                let Some(doc) = self.doc.as_ref() else {
                    return Err(invalid_state("no project is open"));
                };
                if self.export.job.is_some() {
                    return Err(invalid_state("an export is already running"));
                }
                if job.is_empty() {
                    return Err(invalid("empty job id"));
                }
                let p = &doc.project;
                let stems = validate(p, request)?;
                let (start, end) = job::range_of(p, &request.range).map_err(invalid)?;
                let passes = job::plan_passes(p, request, &stems);
                // A new render drops earlier downloads.
                self.export.downloads.clear();
                self.export.last_progress = None;
                self.export.job = Some(Box::new(Job::new(
                    job.clone(),
                    p,
                    request.clone(),
                    self.config.engine_sample_rate,
                    start,
                    end,
                    passes,
                )));
                Ok(ReplyValue::ExportStarted { job: job.clone() })
            }
            ExportCommand::Cancel { job } => {
                if self.export.job.as_ref().is_some_and(|j| &j.id == job) {
                    self.export.job = None;
                    export_event(out, ExportEvent::Cancelled { job: job.clone() });
                }
                Ok(ReplyValue::Unit)
            }
            ExportCommand::ReadChunk {
                token,
                offset,
                length,
            } => {
                let d = self
                    .export
                    .downloads
                    .get(token)
                    .ok_or_else(|| not_found(format!("download {token}")))?;
                if !(offset.is_finite() && *offset >= 0.0 && offset.fract() == 0.0) {
                    return Err(invalid("offset must be a non-negative integer"));
                }
                let size = d.bytes.len();
                let start = (*offset as usize).min(size);
                let end = start + (*length as usize).min(MAX_CHUNK_BYTES).min(size - start);
                Ok(ReplyValue::Bytes {
                    chunk: ByteChunk {
                        offset: start as f64,
                        data: Base64Bytes(d.bytes[start..end].to_vec()),
                        eof: end >= size,
                    },
                })
            }
            ExportCommand::Release { token } => {
                self.export.downloads.remove(token);
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// Called from every tick: steps the running job, emits `Event::Export`.
    pub(crate) fn export_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let current = self.doc.as_ref().map(|d| d.project.id);
        // Closing/switching the project drops its downloads and stops its job.
        self.export
            .downloads
            .retain(|_, d| Some(d.project) == current);
        let Some(mut job) = self.export.job.take() else {
            return;
        };
        if Some(job.project_id) != current {
            export_event(
                out,
                ExportEvent::Failed {
                    job: job.id.clone(),
                    message: "the project was closed".into(),
                },
            );
            return;
        }
        let started = self.host.now_ms().max(now);
        for _ in 0..MAX_UNITS_PER_TICK {
            match job.step(&mut self.bridge, &self.engine, &mut self.store) {
                Ok(Step::Working) => {}
                Ok(Step::Warning(message)) => notify(out, NotificationLevel::Warning, message),
                Ok(Step::Done(files)) => {
                    self.finish_job(&job, files, out);
                    return;
                }
                Err(message) => {
                    export_event(
                        out,
                        ExportEvent::Failed {
                            job: job.id.clone(),
                            message,
                        },
                    );
                    return;
                }
            }
            if self.host.now_ms().saturating_sub(started) >= TICK_BUDGET_MS {
                break;
            }
        }
        let progress = job.progress();
        if self
            .export
            .last_progress
            .is_none_or(|p| (progress - p).abs() >= 0.001)
        {
            self.export.last_progress = Some(progress);
            export_event(
                out,
                ExportEvent::Progress {
                    job: job.id.clone(),
                    progress,
                },
            );
        }
        self.export.job = Some(job);
    }

    fn finish_job(&mut self, job: &Job, files: Vec<Delivered>, out: &mut dyn MessageSink) {
        let mut paths = Vec::new();
        let mut downloads = Vec::new();
        for f in files {
            match f {
                Delivered::File(path) => paths.push(path),
                Delivered::Download(d, bytes) => {
                    self.export.downloads.insert(
                        d.token.clone(),
                        Download {
                            project: job.project_id,
                            bytes,
                        },
                    );
                    downloads.push(d);
                }
            }
        }
        let result = if downloads.is_empty() {
            ExportResult::Files { files: paths }
        } else {
            ExportResult::Download { downloads }
        };
        export_event(
            out,
            ExportEvent::Progress {
                job: job.id.clone(),
                progress: 1.0,
            },
        );
        export_event(
            out,
            ExportEvent::Done {
                job: job.id.clone(),
                result,
            },
        );
    }
}
