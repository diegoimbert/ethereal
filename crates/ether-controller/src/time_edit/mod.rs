//! Time-selection edits across tracks (v0.2, owned by the `time-edits` node; protocol
//! `ether_protocol::time_edit`, CONTRACTS.md §12.3).
//!
//! [`EtherController::time_edit_command`] (dispatched from `handlers.rs`): `Copy` fills the
//! controller's time clipboard ([`TimeEditState`], runtime state); every other command is
//! one document transaction through `edit_with` (one undo step), with new ids from
//! `derive_id(seed, i)` (see [`edit::Ids`]).
//!
//! # Scope
//! A selection's tracks: the listed ones plus the descendants of listed groups, in display
//! order; empty = every audio, MIDI and group track. On each, a time edit moves clips (main
//! and take lanes), comp regions and the track's arrangement automation. `global` (only
//! when the selection covers every audio/MIDI/group track, i.e. the whole song) also moves
//! markers, tempo points, time signatures and the loop region, and the automation of the
//! master, return and VCA tracks. Without `global` those stay put, so a time edit on some
//! tracks never shifts the song's markers or tempo under the others.
//!
//! # Frozen tracks
//! A time edit touching a frozen track is rejected (`InvalidState`; the `freeze-bounce`
//! node's `check_editable` enforces the same rule for every content command).
//!
//! # Semantics
//! - `Split`: every clip crossing `at` is cut in two (fades at the cut dropped).
//! - `DeleteTime`: clips crossing an edge are split, the inside goes, later material moves
//!   left; comp regions are cut and shifted; automation keeps its values on both sides (a
//!   jump at the join if they differ).
//! - `InsertSilence`: clips crossing `at` are split, material from `at` moves right; a comp
//!   region crossing `at` is split; automation holds its value at `at` through the gap.
//! - `Copy`/`Cut`: clip pieces (with notes, warp markers, clip envelopes; a non-looping MIDI
//!   piece keeps only the notes starting inside it), take clips, comp
//!   regions and automation of the range, relative to its start.
//! - `Paste`: onto `tracks` (clipboard track `i` → `tracks[i]`, kind mismatches skipped;
//!   empty = the original tracks). `insert` = insert the clipboard length first, else the
//!   range is overwritten. Take clips and comp regions only paste back onto their own track
//!   (other tracks don't have their lane). Automation targets map like clips moved between
//!   tracks (volume/pan/sends follow; device params only if the device is on the target).
//! - `DuplicateTime`: insert the selection's length after it, then paste a copy there (the
//!   clipboard is not touched).
//!
//! A command whose first derived id already exists in the project is a replay of an applied
//! command and does nothing (same idempotency rule as the client-id creates).

mod clipboard;
mod edit;
mod points;

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::*;
use ether_core::protocol::time_edit::{TimeEditCommand, TimeSelection};

use crate::doc::DocCtx;
use crate::handlers::no_project;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

use clipboard::TimeClipboard;
use edit::{EPS, Ids};

/// Time-edit runtime state: the time clipboard.
#[derive(Default)]
pub(crate) struct TimeEditState {
    clipboard: Option<TimeClipboard>,
}

/// Every track in display order (depth first).
fn display_order(p: &Project) -> Vec<TrackId> {
    fn walk(p: &Project, t: &Track, out: &mut Vec<TrackId>) {
        out.push(t.id);
        for c in p.child_tracks(t.id) {
            walk(p, c, out);
        }
    }
    let mut out = Vec::new();
    for t in p.tracks_ordered() {
        walk(p, t, &mut out);
    }
    out
}

fn content_kind(k: TrackKind) -> bool {
    matches!(k, TrackKind::Audio | TrackKind::Midi | TrackKind::Group)
}

/// The tracks a time edit applies to (display order), frozen tracks rejected.
fn scope(p: &Project, tracks: &[TrackId], global: bool) -> CmdResult<Vec<TrackId>> {
    let order = display_order(p);
    let mut wanted = std::collections::BTreeSet::new();
    if tracks.is_empty() {
        wanted.extend(
            order
                .iter()
                .copied()
                .filter(|t| content_kind(p.tracks[t].kind)),
        );
    } else {
        for &t in tracks {
            if !p.tracks.contains_key(&t) {
                return Err(not_found(format!("track {t}")));
            }
            // A group brings its descendants.
            let mut stack = vec![t];
            while let Some(t) = stack.pop() {
                if wanted.insert(t) {
                    stack.extend(p.child_tracks(t).iter().map(|c| c.id));
                }
            }
        }
    }
    if global {
        let all = order
            .iter()
            .filter(|t| content_kind(p.tracks[t].kind))
            .all(|t| wanted.contains(t));
        if !all {
            return Err(invalid(
                "a global time edit must cover every track (the whole song)",
            ));
        }
        wanted.extend(order.iter().copied());
    }
    if let Some(t) = order
        .iter()
        .map(|t| &p.tracks[t])
        .find(|t| wanted.contains(&t.id) && t.freeze.is_some())
    {
        return Err(invalid_state(format!(
            "track \"{}\" is frozen: unfreeze it to edit its time line",
            t.name
        )));
    }
    Ok(order.into_iter().filter(|t| wanted.contains(t)).collect())
}

/// The content tracks of a scope (what the clipboard copies).
fn content_tracks(p: &Project, scope: &[TrackId]) -> Vec<TrackId> {
    scope
        .iter()
        .copied()
        .filter(|t| content_kind(p.tracks[t].kind))
        .collect()
}

fn check_time(t: Beats, what: &str) -> CmdResult<f64> {
    if !(t.0.is_finite() && t.0 >= 0.0) {
        return Err(invalid(format!("{what} must be >= 0")));
    }
    Ok(t.0)
}

fn check_range(s: &TimeSelection) -> CmdResult<(f64, f64)> {
    let a = check_time(s.start, "the selection start")?;
    let b = check_time(s.end, "the selection end")?;
    if b - a <= EPS {
        return Err(invalid("the time selection is empty"));
    }
    Ok((a, b))
}

/// `true` if the command with this seed was already applied (its first id exists).
fn replayed(p: &Project, seed: ClipId) -> bool {
    let u = derive_id::<ClipId, ClipId>(seed, 0).ulid();
    p.clips.contains_key(&ClipId(u))
        || p.comp_regions.contains_key(&CompRegionId(u))
        || p.automation_points.contains_key(&AutomationPointId(u))
        || p.automation_lanes.contains_key(&AutomationLaneId(u))
        || p.notes.contains_key(&NoteId(u))
        || p.warp_markers.contains_key(&WarpMarkerId(u))
        || p.tempo_points.contains_key(&TempoPointId(u))
        || p.time_signatures.contains_key(&TimeSignatureId(u))
}

fn delete_time(ctx: &mut DocCtx, ids: &mut Ids, sel: &TimeSelection) -> CmdResult<()> {
    let (a, b) = check_range(sel)?;
    for t in scope(ctx.p(), &sel.tracks, sel.global)? {
        edit::delete_time(ctx, ids, t, a, b)?;
    }
    if sel.global {
        edit::global_delete(ctx, ids, a, b)?;
    }
    Ok(())
}

fn insert_silence(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    tracks: &[TrackId],
    at: f64,
    len: f64,
    global: bool,
) -> CmdResult<()> {
    for t in scope(ctx.p(), tracks, global)? {
        edit::insert_time(ctx, ids, t, at, len)?;
    }
    if global {
        edit::global_insert(ctx, at, len)?;
    }
    Ok(())
}

/// Clipboard → destination pairs for a paste (see the module docs).
fn paste_pairs(
    p: &Project,
    cb: &TimeClipboard,
    tracks: &[TrackId],
) -> CmdResult<Vec<(usize, TrackId)>> {
    let mut pairs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (i, src) in cb.tracks.iter().enumerate() {
        let dest = if tracks.is_empty() {
            src.track
        } else if let Some(&t) = tracks.get(i) {
            if !p.tracks.contains_key(&t) {
                return Err(not_found(format!("track {t}")));
            }
            t
        } else {
            break;
        };
        match p.tracks.get(&dest) {
            Some(t) if t.kind == src.kind && seen.insert(dest) => pairs.push((i, dest)),
            _ => {}
        }
    }
    let dests: Vec<TrackId> = pairs.iter().map(|p| p.1).collect();
    // Frozen check on the destinations (no group expansion: pairs are explicit).
    if let Some(t) = dests
        .iter()
        .filter_map(|t| p.tracks.get(t))
        .find(|t| t.freeze.is_some())
    {
        return Err(invalid_state(format!(
            "track \"{}\" is frozen: unfreeze it to edit its time line",
            t.name
        )));
    }
    Ok(pairs)
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn time_edit_command(
        &mut self,
        command: &TimeEditCommand,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let project = &self.doc.as_ref().ok_or_else(no_project)?.project;
        let seed = match command {
            TimeEditCommand::Copy { selection } => {
                let (a, b) = check_range(selection)?;
                let tracks = scope(project, &selection.tracks, false)?;
                let cb = clipboard::copy(project, &content_tracks(project, &tracks), a, b);
                self.time_edit.clipboard = Some(cb);
                return Ok(ReplyValue::Unit);
            }
            TimeEditCommand::Split { seed, .. }
            | TimeEditCommand::Cut { seed, .. }
            | TimeEditCommand::Paste { seed, .. }
            | TimeEditCommand::DeleteTime { seed, .. }
            | TimeEditCommand::InsertSilence { seed, .. }
            | TimeEditCommand::DuplicateTime { seed, .. } => *seed,
        };
        if replayed(project, seed) {
            return Ok(ReplyValue::Unit);
        }
        let mut ids = Ids::new(seed);
        match command {
            TimeEditCommand::Copy { .. } => unreachable!("handled above"),
            TimeEditCommand::Split { tracks, at, .. } => {
                let at = check_time(*at, "the split position")?;
                self.edit_with("Split", gesture, now, out, |ctx| {
                    for t in scope(ctx.p(), tracks, false)? {
                        edit::split(ctx, &mut ids, t, at)?;
                    }
                    Ok(())
                })?;
            }
            TimeEditCommand::Cut { selection, .. } => {
                let (a, b) = check_range(selection)?;
                let tracks = scope(project, &selection.tracks, selection.global)?;
                let cb = clipboard::copy(project, &content_tracks(project, &tracks), a, b);
                self.edit_with("Cut Time", gesture, now, out, |ctx| {
                    delete_time(ctx, &mut ids, selection)
                })?;
                self.time_edit.clipboard = Some(cb);
            }
            TimeEditCommand::DeleteTime { selection, .. } => {
                self.edit_with("Delete Time", gesture, now, out, |ctx| {
                    delete_time(ctx, &mut ids, selection)
                })?;
            }
            TimeEditCommand::InsertSilence {
                tracks,
                at,
                length,
                global,
                ..
            } => {
                let at = check_time(*at, "the insert position")?;
                let len = length.0;
                if !(len.is_finite() && len > EPS) {
                    return Err(invalid("the inserted length must be > 0"));
                }
                self.edit_with("Insert Silence", gesture, now, out, |ctx| {
                    insert_silence(ctx, &mut ids, tracks, at, len, *global)
                })?;
            }
            TimeEditCommand::Paste {
                at, tracks, insert, ..
            } => {
                let at = check_time(*at, "the paste position")?;
                let cb = self
                    .time_edit
                    .clipboard
                    .clone()
                    .ok_or_else(|| invalid_state("the time clipboard is empty"))?;
                let pairs = paste_pairs(project, &cb, tracks)?;
                if pairs.is_empty() {
                    return Err(invalid(
                        "none of the target tracks can take the copied material",
                    ));
                }
                let label = if *insert { "Paste Time" } else { "Paste" };
                self.edit_with(label, gesture, now, out, |ctx| {
                    clipboard::paste(ctx, &mut ids, &cb, at, &pairs, *insert)
                })?;
            }
            TimeEditCommand::DuplicateTime { selection, .. } => {
                let (a, b) = check_range(selection)?;
                let tracks = scope(project, &selection.tracks, selection.global)?;
                let content = content_tracks(project, &tracks);
                let cb = clipboard::copy(project, &content, a, b);
                let pairs: Vec<(usize, TrackId)> = content.into_iter().enumerate().collect();
                self.edit_with("Duplicate Time", gesture, now, out, |ctx| {
                    insert_silence(ctx, &mut ids, &selection.tracks, b, b - a, selection.global)?;
                    clipboard::paste(ctx, &mut ids, &cb, b, &pairs, false)
                })?;
            }
        }
        Ok(ReplyValue::Unit)
    }
}
