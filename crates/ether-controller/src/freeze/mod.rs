//! Freeze, flatten, bounce in place, consolidate (v0.2;
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
//! - Partial freeze ("freeze up to here", `TrackFreeze::up_to`): the render stops at the
//!   freeze point (`job::PassSpec::up_to`: the devices after it get no node in the render).
//!   Only the frozen devices (`Project::device_frozen`) lose their nodes and are locked; the
//!   live tail compiles into the chain after the render and stays editable. Flatten keeps
//!   the live tail.
//! - [`check_editable`]: called before every command (`handlers.rs`), see [`editable`].
//! - An extension may run a render elsewhere (`ControllerExt::offload_render`: this site
//!   validates and keeps the [`Finish`], the result comes back through
//!   [`EtherController::finish_render`]) or run one here for another site
//!   ([`EtherController::freeze_for`]: its progress and result go to the extension,
//!   `ControllerExt::render_update`, and this site's document does not change).

mod edit;
mod editable;
mod job;
mod sample;

use std::collections::BTreeSet;

use ether_core::freeze::FrozenDesc;
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::freeze::{BounceTarget, FreezeCommand, FreezeEvent};
use ether_core::protocol::model::*;
use ether_core::protocol::{Event, NotificationLevel, ReplyValue};

use ether_core::protocol::model::SiteId;
use ether_core::protocol::model::text::TextKey;

use crate::doc::clip_start;
use crate::handlers::{event, notify};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

pub use editable::check_editable;
use job::{PassSpec, RenderJob, Step};

/// Work per tick: at most this many units of work...
const MAX_UNITS_PER_TICK: usize = 256;
/// ...and no more than this long (when the host clock advances).
const TICK_BUDGET_MS: u64 = 12;

/// What a job does with its renders at `Done`.
#[derive(Clone, Debug)]
pub enum Finish {
    Freeze {
        track: TrackId,
        /// The freeze point (`TrackFreeze::up_to`; `None` = whole chain).
        up_to: Option<DeviceId>,
    },
    Bounce {
        track: TrackId,
        start: Beats,
        target: BounceTarget,
    },
    /// `BounceRange`: per bounced source track, its new track, clip and media.
    BounceRange {
        bounces: Vec<(TrackId, TrackId, ClipId, MediaId)>,
        start: Beats,
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
    /// `ConvertToSampler`: replace `device` with a multisampler playing `zones` (whose media
    /// are the renders, inserted with them).
    ConvertToSampler {
        device: DeviceId,
        new_device: DeviceId,
        zones: Vec<SampleZone>,
    },
}

struct Running {
    job: Box<RenderJob>,
    finish: Finish,
    /// Rendered for another site (`freeze_for`): progress, warnings and the result go to it
    /// through the extensions, and this site's document does not change.
    requester: Option<SiteId>,
    /// Last progress sent to the requester, and when (ms; the keepalive).
    sent: (f32, u64),
}

/// The running render job (hosted by `ExportState::render`).
#[derive(Default)]
pub struct RenderState {
    running: Option<Running>,
    last_progress: Option<f32>,
    /// Set while another site's request is validated and started: the job runs here for it.
    serving: Option<SiteId>,
}

/// The renderer for another site sends a progress at most this often...
pub const PROGRESS_MS: u64 = 250;
/// ...and at least this often while it renders.
pub const KEEPALIVE_MS: u64 = 2_000;

/// What a render for another site reports (`ControllerExt::render_update`).
#[derive(Clone, Debug, PartialEq)]
pub enum RenderUpdate {
    Progress {
        progress: f32,
    },
    Warning {
        message: String,
        key: Option<TextKey>,
    },
    Failed {
        message: String,
    },
    /// The rendered media, in this site's store.
    Done {
        media: Vec<MediaRef>,
    },
}

pub fn freeze_event(out: &mut dyn MessageSink, e: FreezeEvent) {
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

/// Audio, MIDI or group track `id` (the tracks that can be frozen and flattened).
fn freezable_track(p: &Project, id: TrackId) -> CmdResult<&Track> {
    let t = p
        .tracks
        .get(&id)
        .ok_or_else(|| not_found(format!("track {id}")))?;
    if !matches!(
        t.kind,
        TrackKind::Audio | TrackKind::Midi | TrackKind::Group
    ) {
        return Err(invalid("only audio, MIDI and group tracks can be frozen"));
    }
    Ok(t)
}

/// `t` is not inside a frozen group.
fn not_inert(p: &Project, t: &Track) -> CmdResult<()> {
    if let Some(g) = p.frozen_ancestor(t.id).and_then(|g| p.tracks.get(&g)) {
        return Err(invalid_state(format!(
            "track \"{}\" is inside frozen group \"{}\": unfreeze the group first",
            t.name, g.name
        )));
    }
    Ok(())
}

/// Freeze Group preconditions (the group is not frozen). Every descendant's output must stay
/// inside the group (a track routed to a bus or return outside it would go silent), and a
/// warning lists what a frozen group stops (see [`apply_inert`]): the sends of its
/// descendants, and external sidechains they feed.
fn check_group_freezable(p: &Project, group: &Track, out: &mut dyn MessageSink) -> CmdResult<()> {
    let subtree: BTreeSet<TrackId> = p.descendants(group.id).into_iter().collect();
    if subtree.is_empty() {
        return Err(invalid(format!("group \"{}\" is empty", group.name)));
    }
    for id in &subtree {
        let t = &p.tracks[id];
        if let Some(dest) = crate::compile::resolve_output(p, t)
            && dest != group.id
            && !subtree.contains(&dest)
        {
            return Err(invalid_state(format!(
                "track \"{}\" in group \"{}\" is routed outside the group: \
                 route it into the group (or move it out) before freezing",
                t.name, group.name
            )));
        }
    }
    let sends = p
        .sends
        .values()
        .filter(|s| subtree.contains(&s.from))
        .count();
    let sidechains = p
        .devices
        .values()
        .filter(|d| !subtree.contains(&d.track))
        .filter(|d| {
            d.sidechain.is_some_and(|s| subtree.contains(&s))
                || d.midi_sidechain.is_some_and(|m| subtree.contains(&m.track))
        })
        .count();
    if sends + sidechains > 0 {
        notify(
            out,
            NotificationLevel::Warning,
            crate::text::msg("notice.freeze.silenced")
                .arg("group", group.name.as_str())
                .arg("sends", sends)
                .arg("sidechains", sidechains),
        );
    }
    Ok(())
}

/// Validate a "freeze up to here" device of `t` (see `FreezeCommand::Freeze`): a device of the
/// track's own chain, not a MIDI effect, not before a MIDI track's instrument. Returns the
/// freeze point to store: `None` when it is the last device (the whole chain).
fn freeze_point(
    p: &Project,
    t: &Track,
    up_to: DeviceId,
    descriptor: &dyn Fn(&Device) -> Option<ether_core::protocol::devices::DeviceDescriptor>,
) -> CmdResult<Option<DeviceId>> {
    use ether_core::protocol::devices::DeviceCategory as C;
    let chain = p.devices_of(t.id);
    let Some(i) = chain.iter().position(|d| d.id == up_to) else {
        return Err(invalid(format!(
            "device {up_to} is not on the chain of track \"{}\"",
            t.name
        )));
    };
    let category = |d: &Device| descriptor(d).map(|desc| desc.category);
    let d = chain[i];
    if category(d) == Some(C::NoteEffect) {
        return Err(invalid(format!(
            "\"{}\" is a MIDI effect: freeze up to the instrument or an audio effect",
            d.name
        )));
    }
    if t.kind == TrackKind::Midi
        && let Some(first) = chain
            .iter()
            .position(|d| category(d) == Some(C::Instrument))
        && i < first
    {
        return Err(invalid(format!(
            "\"{}\" comes before the instrument: freeze up to the instrument or an audio effect",
            d.name
        )));
    }
    Ok((i + 1 < chain.len()).then_some(up_to))
}

/// Frames of `[start, end)` at `engine_rate` (song time), as a render counts them.
pub(crate) fn render_frames(p: &Project, start: Beats, end: Beats, engine_rate: u32) -> usize {
    RenderJob::frames_of(p, start, end, engine_rate)
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

fn not_frozen(p: &Project, t: &Track) -> CmdResult<()> {
    not_inert(p, t)?;
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

/// End of the track's last clip (all lanes), in beats; for a group, of its descendants'.
fn content_end(p: &Project, track: TrackId) -> f64 {
    let mut tracks = p.descendants(track);
    tracks.push(track);
    p.clips
        .values()
        .filter(|c| tracks.contains(&c.track))
        .map(|c| clip_start(c).0 + c.length.0)
        .fold(0.0, f64::max)
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub fn freeze_command(
        &mut self,
        command: &FreezeCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let rate = self.config.engine_sample_rate;
        match command {
            FreezeCommand::ConvertToSampler {
                job,
                device,
                new_device,
                seed_media,
                keys,
                velocities,
                note_seconds,
                tail_seconds,
            } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                self.check_can_start(job)?;
                let d = p
                    .devices
                    .get(device)
                    .ok_or_else(|| not_found(format!("device {device}")))?;
                if d.pad.is_some() || d.chain.is_some() {
                    return Err(invalid(
                        "only an instrument on a track's own chain can be converted",
                    ));
                }
                let t = p
                    .tracks
                    .get(&d.track)
                    .ok_or_else(|| not_found(format!("track {}", d.track)))?;
                if t.kind != TrackKind::Midi {
                    return Err(invalid("only instruments on MIDI tracks can be converted"));
                }
                not_inert(p, t)?;
                if t.freeze.is_some() {
                    return Err(invalid_state(format!(
                        "track \"{}\" is frozen: unfreeze it first",
                        t.name
                    )));
                }
                let descriptor = self.engine.descriptor(d);
                if descriptor.map(|d| d.category) != Some(DeviceCategory::Instrument) {
                    return Err(invalid(format!("\"{}\" is not an instrument", d.name)));
                }
                if !p.devices_of(t.id).iter().any(|x| x.id == *device) {
                    return Err(invalid(format!(
                        "device {device} is not on the track's chain"
                    )));
                }
                if p.devices.contains_key(new_device) {
                    return Err(invalid(format!("device {new_device} already exists")));
                }
                if !(note_seconds.0.is_finite() && note_seconds.0 > 0.0) {
                    return Err(invalid("the note length must be finite and above 0"));
                }
                if !(tail_seconds.0.is_finite() && tail_seconds.0 >= 0.0) {
                    return Err(invalid("the tail must be finite and >= 0"));
                }
                let plan = sample::plan(keys, velocities, *seed_media)?;
                for point in &plan.points {
                    new_media(p, point.media)?;
                }
                let duration = p.tempo_map().seconds_to_beats(*note_seconds);
                let frames = RenderJob::frames_of(p, Beats::ZERO, duration, rate);
                if frames == 0 {
                    return Err(invalid("the note length is too short to render"));
                }
                let specs = plan
                    .points
                    .iter()
                    .map(|point| {
                        let clip: ClipId = ether_model::derive_id(point.media, 0);
                        let note: NoteId = ether_model::derive_id(point.media, 1);
                        PassSpec {
                            track: d.track,
                            start: Beats::ZERO,
                            frames,
                            tail: true,
                            cut: None,
                            chain: true,
                            up_to: Some(*device),
                            sample: Some(job::SampleNote {
                                key: point.key,
                                velocity: f32::from(point.velocity) / 127.0,
                                duration,
                                clip,
                                note,
                            }),
                            max_tail_seconds: Some(tail_seconds.0),
                            media: point.media,
                            name: format!(
                                "{} {} v{}",
                                d.name,
                                crate::drum_rack::note_name(point.key),
                                point.velocity
                            ),
                        }
                    })
                    .collect();
                self.start_job(
                    command,
                    job,
                    specs,
                    Finish::ConvertToSampler {
                        device: *device,
                        new_device: *new_device,
                        zones: plan.zones,
                    },
                    out,
                )
            }
            FreezeCommand::Cancel { job } => {
                let state = &mut self.export.render;
                if state
                    .running
                    .as_ref()
                    .is_some_and(|r| &r.job.id == job && r.requester.is_none())
                {
                    state.running = None;
                    freeze_event(out, FreezeEvent::Cancelled { job: job.clone() });
                } else if self
                    .with_extensions(|e, ctl| e.cancel_render(ctl, job).then_some(()))
                    .is_some()
                {
                    freeze_event(out, FreezeEvent::Cancelled { job: job.clone() });
                }
                Ok(ReplyValue::Unit)
            }
            FreezeCommand::Unfreeze { track } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                let t = freezable_track(p, *track)?;
                not_inert(p, t)?;
                if t.freeze.is_none() {
                    return Ok(ReplyValue::Unit);
                }
                self.edit_with(
                    crate::doc::undo("undo.freeze.unfreeze"),
                    None,
                    now,
                    out,
                    |ctx| ctx.set_track(*track, TrackChange::Freeze(None)),
                )?;
                Ok(ReplyValue::Unit)
            }
            FreezeCommand::Flatten {
                track,
                clip,
                new_track,
            } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                not_inert(p, freezable_track(p, *track)?)?;
                if p.clips.contains_key(clip) {
                    return Err(invalid(format!("clip {clip} already exists")));
                }
                self.edit_with(
                    crate::doc::undo("undo.freeze.flatten"),
                    None,
                    now,
                    out,
                    |ctx| edit::flatten(ctx, *track, *clip, *new_track),
                )?;
                Ok(ReplyValue::Unit)
            }
            FreezeCommand::Freeze {
                job,
                track,
                media,
                up_to,
            } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                self.check_can_start(job)?;
                let t = freezable_track(p, *track)?;
                not_frozen(p, t)?;
                new_media(p, *media)?;
                let up_to = match up_to {
                    Some(d) => freeze_point(p, t, *d, &|d| self.engine.descriptor(d))?,
                    None => None,
                };
                let group = t.kind == TrackKind::Group;
                if group {
                    check_group_freezable(p, t, out)?;
                }
                let end = content_end(p, *track);
                if end <= 1e-9 {
                    return Err(invalid(format!(
                        "{} \"{}\" has no clips to freeze",
                        if group { "group" } else { "track" },
                        t.name
                    )));
                }
                let spec = PassSpec {
                    track: *track,
                    start: Beats::ZERO,
                    frames: RenderJob::frames_of(p, Beats::ZERO, Beats(end), rate),
                    tail: true,
                    cut: None,
                    chain: true,
                    up_to,
                    sample: None,
                    max_tail_seconds: None,
                    media: *media,
                    name: format!("{} Freeze", t.name),
                };
                self.start_job(
                    command,
                    job,
                    vec![spec],
                    Finish::Freeze {
                        track: *track,
                        up_to,
                    },
                    out,
                )
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
                not_inert(p, t)?;
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
                    not_frozen(p, t)?;
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
                    // A new track has room for the decay of the chain (reverb, release...);
                    // in place, the render replaces exactly the range.
                    tail: *include_chain && matches!(target, BounceTarget::NewTrack { .. }),
                    cut: Some(*end),
                    chain: *include_chain,
                    up_to: None,
                    sample: None,
                    max_tail_seconds: None,
                    media: *media,
                    name: format!("{} Bounce", t.name),
                };
                self.start_job(
                    command,
                    job,
                    vec![spec],
                    Finish::Bounce {
                        track: *track,
                        start: *start,
                        target: target.clone(),
                    },
                    out,
                )
            }
            FreezeCommand::BounceRange {
                job,
                tracks,
                start,
                end,
                seed_tracks,
                seed_clips,
                seed_media,
            } => {
                let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
                self.check_can_start(job)?;
                check_range(*start, *end)?;
                if tracks.is_empty() {
                    return Err(invalid("no tracks to bounce"));
                }
                let mut seen = BTreeSet::new();
                let mut bounces = Vec::new();
                let mut specs = Vec::new();
                for (i, id) in tracks.iter().enumerate() {
                    let t = content_track(p, *id)?;
                    not_inert(p, t)?;
                    if !seen.insert(*id) {
                        continue;
                    }
                    let has_clips = p.clips.values().any(|c| {
                        c.track == *id
                            && c.lane.is_none()
                            && !c.muted
                            && clip_start(c).0 < end.0
                            && clip_start(c).0 + c.length.0 > start.0
                    });
                    if !has_clips {
                        continue;
                    }
                    let i = i as u32;
                    let (nt, clip, media): (TrackId, ClipId, MediaId) = (
                        ether_model::derive_id(*seed_tracks, i),
                        ether_model::derive_id(*seed_clips, i),
                        ether_model::derive_id(*seed_media, i),
                    );
                    if p.tracks.contains_key(&nt) {
                        return Err(invalid(format!("track {nt} already exists")));
                    }
                    if p.clips.contains_key(&clip) {
                        return Err(invalid(format!("clip {clip} already exists")));
                    }
                    new_media(p, media)?;
                    specs.push(PassSpec {
                        track: *id,
                        start: *start,
                        frames: RenderJob::frames_of(p, *start, *end, rate),
                        tail: true,
                        cut: Some(*end),
                        chain: true,
                        up_to: None,
                        sample: None,
                        max_tail_seconds: None,
                        media,
                        name: format!("{} Bounce", t.name),
                    });
                    bounces.push((*id, nt, clip, media));
                }
                if specs.is_empty() {
                    return Err(invalid("nothing to bounce: no clips in the selection"));
                }
                self.start_job(
                    command,
                    job,
                    specs,
                    Finish::BounceRange {
                        bounces,
                        start: *start,
                    },
                    out,
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
                    not_frozen(p, t)?;
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
                            cut: None,
                            chain: false,
                            up_to: None,
                            sample: None,
                            max_tail_seconds: None,
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
                    if self.export.render.serving.is_some() {
                        // A peer asked for a render: nothing to render here is a mismatch.
                        return Err(invalid("nothing to render"));
                    }
                    // MIDI only: instantaneous.
                    self.edit_with(
                        crate::doc::undo("undo.freeze.consolidate"),
                        None,
                        now,
                        out,
                        |ctx| consolidate(ctx, &finish, &[]),
                    )?;
                    return Ok(ReplyValue::Unit);
                }
                self.check_can_start(job)?;
                self.start_job(command, job, specs, finish, out)
            }
        }
    }

    fn check_can_start(&self, job: &str) -> CmdResult<()> {
        if job.is_empty() {
            return Err(invalid("empty job id"));
        }
        let busy_elsewhere = self
            .ext_ask(|e, ctl| e.render_busy(ctl).then_some(()))
            .is_some();
        if self.export.render.running.is_some() || busy_elsewhere {
            return Err(crate::tx::keyed_err(
                ether_core::protocol::ErrorCode::InvalidState,
                crate::text::msg("error.freeze.already_running"),
            ));
        }
        Ok(())
    }

    /// Start the render here, or let an extension run it elsewhere.
    fn start_job(
        &mut self,
        command: &FreezeCommand,
        job: &str,
        passes: Vec<PassSpec>,
        finish: Finish,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        self.check_can_start(job)?;
        for pass in &passes {
            crate::export::check_render_frames(pass.frames, self.config.engine_sample_rate)?;
        }
        let requester = self.export.render.serving;
        if requester.is_none()
            && let Some(reply) =
                self.with_extensions(|e, ctl| e.offload_render(ctl, command, job, &finish, out))
        {
            return Ok(reply);
        }
        let p = &self.doc.as_ref().ok_or_else(no_project)?.project;
        let render = RenderJob::new(job.to_string(), p, self.config.engine_sample_rate, passes);
        self.export.render.running = Some(Running {
            job: Box::new(render),
            finish,
            requester,
            sent: (0.0, self.host.now_ms()),
        });
        self.export.render.last_progress = None;
        Ok(ReplyValue::RenderStarted {
            job: job.to_string(),
        })
    }

    /// Called every tick: advance the running render job.
    /// Run `command` (a render) here for another site `requester` (an extension asked):
    /// validated against this site's document, its progress and result go to the extensions
    /// (`ControllerExt::render_update`), and this site's document does not change.
    pub fn freeze_for(
        &mut self,
        requester: SiteId,
        command: &FreezeCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        self.export.render.serving = Some(requester);
        let result = self.freeze_command(command, now, out);
        self.export.render.serving = None;
        result
    }

    /// Drop the render running here for `requester` (`job`: only that one).
    pub fn cancel_render_for(&mut self, requester: SiteId, job: Option<&str>) {
        let state = &mut self.export.render;
        if state
            .running
            .as_ref()
            .is_some_and(|r| r.requester == Some(requester) && job.is_none_or(|j| r.job.id == j))
        {
            state.running = None;
        }
    }

    /// Tell the extensions how a render for another site goes.
    fn render_update(&mut self, to: SiteId, job: &str, update: RenderUpdate) {
        self.ext_each(|e, ctl| e.render_update(ctl, to, job, update.clone()));
    }

    pub fn freeze_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(mut running) = self.export.render.running.take() else {
            return;
        };
        let current = self.doc.as_ref().map(|d| d.project.id);
        let id = running.job.id.clone();
        // Rendered for a peer: dropped when it is no longer in the session with us.
        if running.requester.is_some_and(|to| {
            self.ext_ask(|e, ctl| e.render_requester_present(ctl, to).then_some(()))
                .is_none()
        }) {
            return;
        }
        if Some(running.job.project_id) != current {
            let message = "the project was closed".to_string();
            match running.requester {
                Some(to) => self.render_update(to, &id, RenderUpdate::Failed { message }),
                None => freeze_event(out, FreezeEvent::Failed { job: id, message }),
            }
            return;
        }
        let started = self.host.now_ms().max(now);
        for _ in 0..MAX_UNITS_PER_TICK {
            let step = running.job.step(
                &mut self.bridge,
                &self.engine,
                &mut self.store,
                &mut self.library,
            );
            for message in std::mem::take(&mut running.job.warnings) {
                match running.requester {
                    Some(to) => self.render_update(
                        to,
                        &id,
                        RenderUpdate::Warning {
                            message: message.text,
                            key: message.key,
                        },
                    ),
                    None => notify(out, NotificationLevel::Warning, message),
                }
            }
            match (step, running.requester) {
                (Ok(Step::Working), _) => {}
                (Ok(Step::Done), None) => {
                    let media = running
                        .job
                        .results
                        .iter()
                        .map(|r| r.media.clone())
                        .collect();
                    self.finish_render(id, &running.finish, media, None, now, out);
                    return;
                }
                (Ok(Step::Done), Some(to)) => {
                    let media = running
                        .job
                        .results
                        .iter()
                        .map(|r| r.media.clone())
                        .collect();
                    self.render_update(to, &id, RenderUpdate::Done { media });
                    return;
                }
                (Err(message), None) => {
                    freeze_event(out, FreezeEvent::Failed { job: id, message });
                    return;
                }
                (Err(message), Some(to)) => {
                    self.render_update(to, &id, RenderUpdate::Failed { message });
                    return;
                }
            }
            if self.host.now_ms().saturating_sub(started) >= TICK_BUDGET_MS {
                break;
            }
        }
        let progress = running.job.progress();
        match running.requester {
            // A peer's: whole percents, and a keepalive while a long step runs.
            Some(to) => {
                let (sent, at) = running.sent;
                let since = now.saturating_sub(at);
                if (progress - sent >= 0.01 && since >= PROGRESS_MS) || since >= KEEPALIVE_MS {
                    running.sent = (progress, now);
                    self.render_update(to, &id, RenderUpdate::Progress { progress });
                }
            }
            None => {
                let state = &mut self.export.render;
                if state
                    .last_progress
                    .is_none_or(|p| (progress - p).abs() >= 0.001)
                {
                    state.last_progress = Some(progress);
                    freeze_event(
                        out,
                        FreezeEvent::Progress {
                            job: id,
                            progress,
                            host: None,
                        },
                    );
                }
            }
        }
        self.export.render.running = Some(running);
    }

    /// Apply the job's document edit (one undo step), then report `Done` (or `Failed`).
    /// `host`: the peer that rendered it (its name, for the last progress event).
    pub fn finish_render(
        &mut self,
        job: String,
        finish: &Finish,
        media: Vec<MediaRef>,
        host: Option<String>,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let (label, result) = match finish {
            Finish::Freeze { track, up_to } => (
                "Freeze",
                self.edit_with(
                    crate::doc::undo("undo.freeze.freeze"),
                    None,
                    now,
                    out,
                    |ctx| edit::finish_freeze(ctx, *track, *up_to, media[0].clone()),
                ),
            ),
            Finish::Bounce {
                track,
                start,
                target,
            } => (
                "Bounce",
                self.edit_with(
                    crate::doc::undo("undo.freeze.bounce"),
                    None,
                    now,
                    out,
                    |ctx| edit::finish_bounce(ctx, *track, *start, target, media[0].clone()),
                ),
            ),
            Finish::BounceRange { bounces, start } => (
                "Bounce",
                self.edit_with(
                    crate::doc::undo("undo.freeze.bounce"),
                    None,
                    now,
                    out,
                    |ctx| {
                        for &(track, new_track, clip, id) in bounces {
                            let Some(m) = media.iter().find(|m| m.id == id) else {
                                continue;
                            };
                            let target = BounceTarget::NewTrack {
                                track: new_track,
                                clip,
                            };
                            edit::finish_bounce(ctx, track, *start, &target, m.clone())?;
                        }
                        Ok(())
                    },
                ),
            ),
            Finish::ConvertToSampler {
                device,
                new_device,
                zones,
            } => (
                "Convert to sampler",
                self.edit_with(
                    crate::doc::undo("undo.freeze.convert_sampler"),
                    None,
                    now,
                    out,
                    |ctx| edit::finish_convert_to_sampler(ctx, *device, *new_device, zones, &media),
                ),
            ),
            f @ Finish::Consolidate { .. } => (
                "Consolidate",
                self.edit_with(
                    crate::doc::undo("undo.freeze.consolidate"),
                    None,
                    now,
                    out,
                    |ctx| consolidate(ctx, f, &media),
                ),
            ),
        };
        match result {
            Ok(()) => {
                freeze_event(
                    out,
                    FreezeEvent::Progress {
                        job: job.clone(),
                        progress: 1.0,
                        host,
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
    crate::tx::keyed_err(
        ether_core::protocol::ErrorCode::InvalidState,
        crate::text::msg("error.project.none_open"),
    )
}

/// The frozen render of `track` for the engine (see the module docs).
pub fn frozen_desc(p: &Project, track: &Track) -> Option<FrozenDesc> {
    let _ = p;
    let f = track.freeze.as_ref()?;
    Some(FrozenDesc {
        media: f.media,
        start_seconds: f.start.0,
    })
}

/// Group freeze: compile every descendant of a frozen group as an inert track. It keeps its
/// place in the graph (ids, `group` for the mute/solo tree) but carries nothing: no clips,
/// chain, racks, modulation, taps or sends, no solo/mute, no input, no routing (it is
/// silent and costs nothing; its devices have no nodes, `engine.rs::sync_nodes`). Taps of
/// tracks that are not inside a frozen group are untouched, so the "taps are unaffected by
/// mute/solo" rule holds for them; a sidechain fed by an inert track hears
/// silence.
pub fn apply_inert(p: &Project, tracks: &mut [ether_core::graph::TrackDesc]) {
    if !p
        .tracks
        .values()
        .any(|t| t.freeze.is_some() && t.kind == TrackKind::Group)
    {
        return;
    }
    for td in tracks.iter_mut() {
        if p.frozen_ancestor(td.id).is_none() {
            continue;
        }
        td.chain.clear();
        td.output = None;
        td.sends.clear();
        td.mute = false;
        td.solo = false;
        td.audio_input = None;
        td.monitor = false;
        td.armed = false;
        td.clips.clear();
        td.automation.clear();
        td.racks.clear();
        td.chain_racks.clear();
        td.modulation = Default::default();
        td.input_tap = None;
        td.vca = None;
        td.expression = Default::default();
        td.hw_io.clear();
        td.sidechain_taps.clear();
        td.midi_sidechains.clear();
        td.frozen = None;
    }
}
