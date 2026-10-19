//! Offline export (roadmap v2; see CONTRACTS.md §11.1 and `ether_protocol::export`).
//!
//! A job snapshots the document and builds an independent
//! `ether_core::offline::OfflineRenderer` per file at the engine rate: fresh device nodes
//! (`ether_devices::create` for built-ins, `EngineBridge::create_offline_plugin` for
//! plugins, with their current state; a plugin that can't be instantiated offline fails the
//! job with a clear error, it is never skipped silently), the project's media decoded and
//! resampled again with the live pipeline's code (bit-identical sources), and a graph from
//! `compile::compile_graph_with` with the click, looping and inputs off. `latency()`
//! leading frames are dropped, then each file is resampled (optional), normalized
//! (optional), dithered (16-bit) and encoded (WAV via `hound`, FLAC via `flacenc`, MP3 via
//! `rusty_mp3`), once per requested format (`ExportRequest::extra_formats`). Native
//! hosts write `exports/<name>` through the `ProjectStore` (never overwriting: a taken name
//! gets a ` (2)` suffix); hosts whose store says `Unsupported` (web/remote) keep the bytes
//! for `Export::ReadChunk`.
//!
//! # Streaming (#434)
//! The render is never held whole: each engine block goes straight through every file of
//! the render (a `StreamResampler` per output rate, then each format's encoder), and the
//! encoders write into `ProjectStore::create_export` files as they go (native: a temp file
//! renamed on commit), so peak memory is the offline engine and a few blocks whatever the
//! length. Where the store can't stream (web OPFS says `Unsupported`; `in_memory` requests
//! such as static-share publishes) the *encoded* bytes are kept in memory, never the `f32`
//! render. The output is byte-identical to encoding the whole render at once
//! (`StreamResampler` and the encoders don't depend on the block size). Exception:
//! **normalize** needs each file's peak before its first sample, so a normalized export
//! holds its signal (at each output rate) until the render is done, then encodes it at its
//! gain; a two-pass render would avoid that, but plugins don't promise the same samples
//! twice.
//!
//! # Bounded ticks
//! Everything runs from [`EtherController::export_tick`] as small units of work
//! ([`job::UNIT_FRAMES`] frames of media decoding, one `OFFLINE_MAX_BLOCK` render block,
//! that block through one file's resampler and encoders, [`job::UNIT_FRAMES`] frames of a
//! normalized signal encoded, [`job::UNIT_BUILTINS`] built-in nodes or one plugin instance
//! per unit); a tick runs units until
//! [`TICK_BUDGET_MS`] elapsed (at most [`MAX_UNITS_PER_TICK`]), so the controller stays
//! responsive whatever the export length. A few steps still take time proportional to a
//! file's length rather than to a unit: reading a media file from the store (one
//! `ProjectStore::read`), the media resamplers' final step (`IncrementalResampler::finish`
//! trims and copies the output: a memory copy), and committing each export file (native:
//! fsync and rename; a store that can't stream: one write of the whole file).
//!
//! # Stem semantics
//! A stem is what the track would be if it were the only source: its post-fader output
//! (device chain, fader, pan, VCA) goes straight into master. Two ways to render it, with
//! the same result (`job::capturable` says when; `tests/freeze.rs`
//! `one_pass_stems_equal_per_track_passes` renders both and compares sample for sample):
//! - **Shared render.** `ExportMode::MixAndStems`, or `Stems` with two or more eligible
//!   tracks: one offline render of the whole project, in which the engine also copies each
//!   listed track's post-fader signal out ([`ether_core::offline::OfflineRenderer::capture_tracks`];
//!   the tap sits before the mute/solo gate, so solo is ignored and a muted track is
//!   unmuted for its own stem) with the track's own latency dropped instead of the graph's.
//!   Each captured signal streams to its files like the main one (no second render). A
//!   render writes at most `job::MAX_RUN_FILES` files at once and holds at most
//!   `job::MAX_CAPTURE_BYTES` of normalized signals; more stems get a pass each.
//! - **Pass per stem** ([`job::stem_graph`]) for the tracks the shared render can't
//!   represent: the graph is rewritten so only that track sounds.
//!
//! Properties, for both:
//! - Solo is ignored in stems, and a muted track is unmuted for its own stem (muted tracks
//!   nested in a group stem stay muted).
//! - A child of a group bypasses the group's processing (chain, fader) in its own stem; a
//!   group's stem contains its children through the group's processing.
//! - A return's stem is everything sent to it (through the return).
//! - Stems overlap when a track is selected together with a group containing it, or a
//!   return it sends to: that audio is in both files.
//! - The master chain is excluded, the master fader and pan are kept.
//!
//! A track needs its own pass when the shared render would differ from that: the project
//! has a solo or a muted VCA, the master is not neutral (muted, not unity, off center or
//! automated), or the track (or something nested in it) has sends (a stem includes the
//! returns' share, which only a render with the other sources silent isolates) or takes
//! another track's output as input (silent in a stem).
//!
//! One difference remains in the shared render: a device sidechained from another track
//! is keyed by it as in the mix (the stem pass leaves other tracks silent, so it didn't
//! duck), which makes such stems closer to the mix.
//! - So stems sum to the mix only with neutral master devices, neutral groups (unity, no
//!   devices) and linear returns, and without solo or sidechains.

pub(crate) mod encode;
mod job;
pub(crate) mod offline;

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::export::{
    AudioContainer, BitDepth, ByteChunk, ExportCommand, ExportEvent, ExportFormat, ExportMode,
    ExportRequest, ExportResult, MAX_EXTRA_FORMATS,
};
use ether_core::protocol::model::{Base64Bytes, ProjectId, TrackId};
use ether_core::protocol::{Event, NotificationLevel, ReplyValue};

use crate::handlers::{event, notify};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

use job::{Delivered, Job, Step};

/// Most bytes one `ReadChunk` serves (the protocol promises at least 256 KiB).
pub(crate) const MAX_CHUNK_BYTES: usize = 1 << 20;
/// Work per tick: at most this many units of work (see "Bounded ticks")...
const MAX_UNITS_PER_TICK: usize = 256;
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
    /// Unit sizes of the last job (tests/diagnostics).
    last_stats: job::UnitStats,
    /// v0.2 (`freeze-bounce`): the running freeze/bounce/consolidate render, independent of
    /// the export job (kept here so the controller struct needs no new field).
    pub(crate) render: crate::freeze::RenderState,
}

pub use job::UnitStats as ExportUnitStats;

impl ExportState {
    /// Keep `bytes` engine-side as a download for `Export::ReadChunk` (base-114 bundles).
    /// Dropped with the others when `project` is no longer open.
    pub(crate) fn add_download(&mut self, token: String, project: ProjectId, bytes: Vec<u8>) {
        self.downloads.insert(token, Download { project, bytes });
    }
}

fn export_event(out: &mut dyn MessageSink, e: ExportEvent) {
    event(out, Event::Export { event: e });
}

/// The end of the arrangement in beats (the `Project` export range's end; 0 when empty).
/// (Also the private overlay's static-share export; unused in the OSS build.)
#[allow(dead_code)]
pub(crate) fn project_length(p: &ether_core::protocol::model::Project) -> f64 {
    job::range_of(p, &ether_core::protocol::export::ExportRange::Project)
        .map_or(0.0, |(_, end)| end.0)
}

fn validate_format(f: &ExportFormat) -> CmdResult<()> {
    if f.container == AudioContainer::Flac && f.bit_depth == BitDepth::Float32 {
        return Err(invalid("FLAC supports 16 or 24 bits only"));
    }
    if let Some(rate) = f.sample_rate {
        if !(8_000..=384_000).contains(&rate) {
            return Err(invalid(format!("unsupported sample rate {rate} Hz")));
        }
        if f.container == AudioContainer::Mp3 && !encode::MP3_RATES.contains(&rate) {
            return Err(invalid("MP3 supports 32, 44.1 or 48 kHz only"));
        }
    }
    Ok(())
}

/// Validate a request against the project: the stem tracks (deduplicated, in order).
fn validate(
    p: &ether_core::protocol::model::Project,
    r: &ExportRequest,
) -> CmdResult<Vec<TrackId>> {
    let formats = job::formats(r);
    if formats.len() > 1 + MAX_EXTRA_FORMATS {
        return Err(invalid(format!(
            "at most {MAX_EXTRA_FORMATS} extra formats"
        )));
    }
    for (i, f) in formats.iter().enumerate() {
        validate_format(f)?;
        if formats[..i].iter().any(|g| g.container == f.container) {
            return Err(invalid("each format of an export needs its own container"));
        }
    }
    if !(r.tail_seconds.is_finite() && r.tail_seconds >= 0.0) {
        return Err(invalid("tail must be >= 0 seconds"));
    }
    job::range_of(p, &r.range).map_err(invalid)?;
    match &r.mode {
        ExportMode::Mix => Ok(Vec::new()),
        ExportMode::Stems { tracks } | ExportMode::MixAndStems { tracks } => {
            if tracks.is_empty() && matches!(r.mode, ExportMode::Stems { .. }) {
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
                    return Err(crate::tx::keyed_err(
                        ether_core::protocol::ErrorCode::InvalidState,
                        crate::text::msg("error.project.none_open"),
                    ));
                };
                if self.export.job.is_some() {
                    return Err(crate::tx::keyed_err(
                        ether_core::protocol::ErrorCode::InvalidState,
                        crate::text::msg("error.export.already_running"),
                    ));
                }
                if job.is_empty() {
                    return Err(invalid("empty job id"));
                }
                let p = &doc.project;
                let stems = validate(p, request)?;
                let (start, end) = job::range_of(p, &request.range).map_err(invalid)?;
                // Names already in `exports/` (an export never overwrites a file).
                let existing: BTreeSet<String> = self
                    .store
                    .list_dir(p.id, ether_core::protocol::model::file::EXPORTS_DIR)
                    .map(|l| {
                        l.entries
                            .into_iter()
                            .map(|e| e.name.to_lowercase())
                            .collect()
                    })
                    .unwrap_or_default();
                let passes = job::plan_passes(p, request, &stems, &existing);
                // A new render drops earlier downloads.
                self.export.downloads.clear();
                self.export.last_progress = None;
                self.export.last_stats = Default::default();
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

    /// Largest units of work of the running (or last) export job (tests/diagnostics).
    #[doc(hidden)]
    pub fn export_unit_stats(&self) -> ExportUnitStats {
        self.export.last_stats
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
            let step = job.step(
                &mut self.bridge,
                &self.engine,
                &mut self.store,
                &mut self.library,
            );
            for message in job.warnings.drain(..) {
                notify(out, NotificationLevel::Warning, message);
            }
            self.export.last_stats = job.stats;
            match step {
                Ok(Step::Working) => {}
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
