//! Freeze, flatten, bounce in place, consolidate (v0.2, owned by the `freeze-bounce` node;
//! protocol `ether_protocol::freeze`, model `Track::freeze`, CONTRACTS.md §12.3).
//!
//! - [`EtherController::freeze_command`] (dispatched from `handlers.rs`): render jobs
//!   ([`job::RenderJob`]) on an `ether_core::offline::OfflineRenderer` stepped from
//!   [`EtherController::freeze_tick`] in bounded units (the export job's helpers,
//!   `crate::export::offline`). Each pass renders one track **in isolation**
//!   ([`job::isolate`]): post-chain (or clips only), pre-fader, into a neutral master; the
//!   sources of its sidechains keep playing silently so they duck like live playback. The
//!   result is a 32-bit float WAV at the engine rate written to the project's `media/`
//!   through `ProjectStore::write`; the document changes once, at `Done` (media insert +
//!   the edit, one undo step, [`edit`]). One job at a time (independent of export).
//! - Freeze: beat 0 to the end of the track's last clip, plus the tail until it falls below
//!   -90 dBFS (checked every 0.5 s, at most 30 s; trailing silence trimmed). At `Done` the
//!   live state of the track's plugins is kept in the document (their nodes are destroyed
//!   while frozen; unfreezing re-creates them from it).
//! - Rendered clips (flatten, bounce, consolidate) are warped to the tempo map
//!   ([`edit::place_render`]: markers derived from it, rate 1), so they play the render
//!   exactly at song time.
//! - [`frozen_desc`]: `compile.rs` calls it per track; `Some` = the track compiles to
//!   `TrackDesc::frozen` with no clips, chain, racks or modulation; `engine.rs::sync_nodes`
//!   creates no nodes for its devices. Engine side: `ether_core::freeze::render_frozen`.
//! - [`check_editable`]: called before every command (`handlers.rs`), see [`editable`].

mod edit;
mod editable;
mod job;

use std::collections::BTreeSet;

use ether_core::freeze::FrozenDesc;
use ether_core::protocol::freeze::{BounceTarget, FreezeCommand, FreezeEvent};
use ether_core::protocol::model::*;
use ether_core::protocol::{Event, NotificationLevel, ReplyValue};

use crate::doc::clip_start;
use crate::handlers::{event, notify};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

pub(crate) use editable::check_editable;
use job::{PassSpec, RenderJob, Step};

/// Work per tick: at most this many units of work...
const MAX_UNITS_PER_TICK: usize = 256;
/// ...and no more than this long (when the host clock advances).
const TICK_BUDGET_MS: u64 = 12;

/// What a job does with its renders at `Done`.
#[derive(Clone, Debug)]
enum Finish {
    Freeze {
        track: TrackId,
    },
    Bounce {
        track: TrackId,
        start: Beats,
        target: BounceTarget,
    },
    Consolidate {
        /// Every listed track with its index (derived ids), MIDI ones included.
        tracks: Vec<(u32, TrackId)>,
        start: Beats,
        end: Beats,
        seed_clips: ClipId,
        seed_notes: ClipId,
        seed_media: MediaId,
    },
}

struct Running {
    job: Box<RenderJob>,
    finish: Finish,
}

/// The running render job (hosted by `ExportState::render`).
#[derive(Default)]
pub(crate) struct RenderState {
    running: Option<Running>,
    last_progress: Option<f32>,
}

fn freeze_event(out: &mut dyn MessageSink, e: FreezeEvent) {
    event(out, Event::Freeze { event: e });
}

/// Audio/MIDI track `id` (the tracks freeze, bounce and consolidate work on).
fn content_track(p: &Project, id: TrackId) -> CmdResult<&Track> {
    let t = p
        .tracks
        .get(&id)
        .ok_or_else(|| not_found(format!("track {id}")))?;
    if !matches!(t.kind, TrackKind::Audio | TrackKind::Midi) {
        return Err(invalid("only audio and MIDI tracks can be rendered"));
    }
    Ok(t)
}

fn check_range(start: Beats, end: Beats) -> CmdResult<()> {
    if !(start.0.is_finite() && end.0.is_finite() && start.0 >= 0.0) {
        return Err(invalid("invalid range"));
    }
    if end.0 <= start.0 + 1e-9 {
        return Err(invalid("the range is empty"));
    }
    Ok(())
}

fn not_frozen(t: &Track) -> CmdResult<()> {
    if t.freeze.is_some() {
        return Err(invalid_state(format!(
            "track \"{}\" is frozen: unfreeze it first",
            t.name
        )));
    }
    Ok(())
}

fn new_media(p: &Project, id: MediaId) -> CmdResult<()> {
    if p.media.contains_key(&id) {
        return Err(invalid(format!("media {id} already exists")));
    }
    Ok(())
}

/// End of the track's last clip (all lanes), in beats.
fn content_end(p: &Project, track: TrackId) -> f64 {
    p.clips
        .values()
        .filter(|c| c.track == track)
        .map(|c| clip_start(c).0 + c.length.0)
        .fold(0.0, f64::max)
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn freeze_command(
        &mut self,
        command: &FreezeCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let rate = self.config.engine_sample_rate;
        match command {
            FreezeCommand::Cancel { job } => {
                let state = &mut self.export.render;
                if state.running.as_ref().is_some_and(|r| &r.job.id == job) {
                    state.running = None;
                    freeze_event(out, FreezeEvent::Cancelled { job: job.clone() });
                }
                Ok(ReplyValue::Unit)
            }
            FreezeCommand::Unfreeze { track } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                if content_track(p, *track)?.freeze.is_none() {
                    return Ok(ReplyValue::Unit);
                }
                self.edit_with("Unfreeze", None, now, out, |ctx| {
                    ctx.set_track(*track, TrackChange::Freeze(None))
                })?;
                Ok(ReplyValue::Unit)
            }
            FreezeCommand::Flatten {
                track,
                clip,
                new_track,
            } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                content_track(p, *track)?;
                if p.clips.contains_key(clip) {
                    return Err(invalid(format!("clip {clip} already exists")));
                }
                self.edit_with("Flatten", None, now, out, |ctx| {
                    edit::flatten(ctx, *track, *clip, *new_track)
                })?;
                Ok(ReplyValue::Unit)
            }
            FreezeCommand::Freeze { job, track, media } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                self.check_can_start(job)?;
                let t = content_track(p, *track)?;
                not_frozen(t)?;
                new_media(p, *media)?;
                let end = content_end(p, *track);
                if end <= 1e-9 {
                    return Err(invalid(format!(
                        "track \"{}\" has no clips to freeze",
                        t.name
                    )));
                }
                let spec = PassSpec {
                    track: *track,
                    start: Beats::ZERO,
                    frames: RenderJob::frames_of(p, Beats::ZERO, Beats(end), rate),
                    tail: true,
                    chain: true,
                    media: *media,
                    name: format!("{} Freeze", t.name),
                };
                self.start_job(job, vec![spec], Finish::Freeze { track: *track })
            }
            FreezeCommand::Bounce {
                job,
                track,
                start,
                end,
                include_chain,
                media,
                target,
            } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                self.check_can_start(job)?;
                let t = content_track(p, *track)?;
                check_range(*start, *end)?;
                new_media(p, *media)?;
                if !include_chain && t.kind == TrackKind::Midi {
                    return Err(invalid(
                        "a MIDI track has no audio before its devices: bounce it with its chain",
                    ));
                }
                if t.freeze.is_some()
                    && (!include_chain || matches!(target, BounceTarget::InPlace { .. }))
                {
                    not_frozen(t)?;
                }
                match target {
                    BounceTarget::NewTrack { track: nt, clip } => {
                        if p.tracks.contains_key(nt) {
                            return Err(invalid(format!("track {nt} already exists")));
                        }
                        if p.clips.contains_key(clip) {
                            return Err(invalid(format!("clip {clip} already exists")));
                        }
                    }
                    BounceTarget::InPlace { clip } => {
                        if t.kind != TrackKind::Audio || *include_chain {
                            return Err(invalid(
                                "bounce in place is for audio tracks, without the device chain",
                            ));
                        }
                        if p.clips.contains_key(clip) {
                            return Err(invalid(format!("clip {clip} already exists")));
                        }
                    }
                }
                let spec = PassSpec {
                    track: *track,
                    start: *start,
                    frames: RenderJob::frames_of(p, *start, *end, rate),
                    tail: false,
                    chain: *include_chain,
                    media: *media,
                    name: format!("{} Bounce", t.name),
                };
                self.start_job(
                    job,
                    vec![spec],
                    Finish::Bounce {
                        track: *track,
                        start: *start,
                        target: target.clone(),
                    },
                )
            }
            FreezeCommand::Consolidate {
                job,
                tracks,
                start,
                end,
                seed_clips,
                seed_notes,
                seed_media,
            } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                check_range(*start, *end)?;
                if tracks.is_empty() {
                    return Err(invalid("no tracks to consolidate"));
                }
                let mut seen = BTreeSet::new();
                let mut listed = Vec::new();
                let mut specs = Vec::new();
                for (i, id) in tracks.iter().enumerate() {
                    let t = content_track(p, *id)?;
                    not_frozen(t)?;
                    if !seen.insert(*id) {
                        continue;
                    }
                    let i = i as u32;
                    listed.push((i, *id));
                    let has_clips = p.clips.values().any(|c| {
                        c.track == *id
                            && c.lane.is_none()
                            && clip_start(c).0 < end.0
                            && clip_start(c).0 + c.length.0 > start.0
                    });
                    if t.kind == TrackKind::Audio && has_clips {
                        let media: MediaId = ether_model::derive_id(*seed_media, i);
                        new_media(p, media)?;
                        specs.push(PassSpec {
                            track: *id,
                            start: *start,
                            frames: RenderJob::frames_of(p, *start, *end, rate),
                            tail: false,
                            chain: false,
                            media,
                            name: format!("{} Consolidated", t.name),
                        });
                    }
                }
                let finish = Finish::Consolidate {
                    tracks: listed,
                    start: *start,
                    end: *end,
                    seed_clips: *seed_clips,
                    seed_notes: *seed_notes,
                    seed_media: *seed_media,
                };
                if specs.is_empty() {
                    // MIDI only: instantaneous.
                    self.edit_with("Consolidate", None, now, out, |ctx| {
                        consolidate(ctx, &finish, &[])
                    })?;
                    return Ok(ReplyValue::Unit);
                }
                self.check_can_start(job)?;
                self.start_job(job, specs, finish)
            }
        }
    }

    fn check_can_start(&self, job: &str) -> CmdResult<()> {
        if job.is_empty() {
            return Err(invalid("empty job id"));
        }
        if self.export.render.running.is_some() {
            return Err(invalid_state("a freeze or bounce is already running"));
        }
        Ok(())
    }

    fn start_job(
        &mut self,
        job: &str,
        passes: Vec<PassSpec>,
        finish: Finish,
    ) -> CmdResult<ReplyValue> {
        self.check_can_start(job)?;
        let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
        let render = RenderJob::new(job.to_string(), p, self.config.engine_sample_rate, passes);
        self.export.render.running = Some(Running {
            job: Box::new(render),
            finish,
        });
        self.export.render.last_progress = None;
        Ok(ReplyValue::RenderStarted {
            job: job.to_string(),
        })
    }

    /// Called every tick: advance the running render job.
    pub(crate) fn freeze_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(mut running) = self.export.render.running.take() else {
            return;
        };
        let current = self.doc.as_ref().map(|d| d.project.id);
        let id = running.job.id.clone();
        if Some(running.job.project_id) != current {
            freeze_event(
                out,
                FreezeEvent::Failed {
                    job: id,
                    message: "the project was closed".into(),
                },
            );
            return;
        }
        let started = self.host.now_ms().max(now);
        for _ in 0..MAX_UNITS_PER_TICK {
            let step = running
                .job
                .step(&mut self.bridge, &self.engine, &mut self.store);
            for message in running.job.warnings.drain(..) {
                notify(out, NotificationLevel::Warning, message);
            }
            match step {
                Ok(Step::Working) => {}
                Ok(Step::Done) => {
                    self.finish_render(&running, now, out);
                    return;
                }
                Err(message) => {
                    freeze_event(out, FreezeEvent::Failed { job: id, message });
                    return;
                }
            }
            if self.host.now_ms().saturating_sub(started) >= TICK_BUDGET_MS {
                break;
            }
        }
        let progress = running.job.progress();
        let state = &mut self.export.render;
        if state
            .last_progress
            .is_none_or(|p| (progress - p).abs() >= 0.001)
        {
            state.last_progress = Some(progress);
            freeze_event(out, FreezeEvent::Progress { job: id, progress });
        }
        state.running = Some(running);
    }

    /// Apply the job's document edit (one undo step), then report `Done` (or `Failed`).
    fn finish_render(&mut self, running: &Running, now: u64, out: &mut dyn MessageSink) {
        let job = running.job.id.clone();
        let media: Vec<MediaRef> = running
            .job
            .results
            .iter()
            .map(|r| r.media.clone())
            .collect();
        let (label, result) = match &running.finish {
            Finish::Freeze { track } => (
                "Freeze",
                self.edit_with("Freeze", None, now, out, |ctx| {
                    edit::finish_freeze(ctx, *track, media[0].clone())
                }),
            ),
            Finish::Bounce {
                track,
                start,
                target,
            } => (
                "Bounce",
                self.edit_with("Bounce", None, now, out, |ctx| {
                    edit::finish_bounce(ctx, *track, *start, target, media[0].clone())
                }),
            ),
            f @ Finish::Consolidate { .. } => (
                "Consolidate",
                self.edit_with("Consolidate", None, now, out, |ctx| {
                    consolidate(ctx, f, &media)
                }),
            ),
        };
        match result {
            Ok(()) => {
                freeze_event(
                    out,
                    FreezeEvent::Progress {
                        job: job.clone(),
                        progress: 1.0,
                    },
                );
                freeze_event(out, FreezeEvent::Done { job });
            }
            Err(e) => freeze_event(
                out,
                FreezeEvent::Failed {
                    job,
                    message: format!("{label}: {}", e.message),
                },
            ),
        }
    }
}

/// `Consolidate`'s edit: MIDI tracks from their notes, audio tracks from `media` (the
/// renders, `derive_id(seed_media, i)`).
fn consolidate(ctx: &mut crate::doc::DocCtx, f: &Finish, media: &[MediaRef]) -> CmdResult<()> {
    let Finish::Consolidate {
        tracks,
        start,
        end,
        seed_clips,
        seed_notes,
        seed_media,
    } = f
    else {
        return Ok(());
    };
    let mut next_note = 0;
    for &(i, track) in tracks {
        let Some(t) = ctx.p().tracks.get(&track).cloned() else {
            continue;
        };
        let clip: ClipId = ether_model::derive_id(*seed_clips, i);
        match t.kind {
            TrackKind::Midi => {
                edit::consolidate_midi(ctx, track, *start, *end, clip, *seed_notes, &mut next_note)?
            }
            TrackKind::Audio => {
                let id: MediaId = ether_model::derive_id(*seed_media, i);
                let Some(m) = media.iter().find(|m| m.id == id) else {
                    continue;
                };
                ctx.tx.insert(Entity::Media(m.clone()))?;
                edit::place_render(ctx, clip, track, *start, m)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn no_project() -> ether_core::protocol::CommandError {
    invalid_state("no project is open")
}

/// The frozen render of `track` for the engine (see the module docs).
pub(crate) fn frozen_desc(p: &Project, track: &Track) -> Option<FrozenDesc> {
    let _ = p;
    let f = track.freeze.as_ref()?;
    Some(FrozenDesc {
        media: f.media,
        start_seconds: f.start.0,
    })
}
