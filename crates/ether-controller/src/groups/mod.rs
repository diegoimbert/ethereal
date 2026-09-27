//! Groups, buses, track input taps and VCAs (v0.2, owned by the `groups-buses` node,
//! priority 1; CONTRACTS.md §12.10).
//!
//! - [`track_command`]: `Track::{GroupSelected, Ungroup, SetVca}` (document commands,
//!   dispatched from `doc/tracks.rs`). Moving tracks in/out of groups is `Track::Move`.
//! - [`vca_descs`]: `RenderGraphDesc::vcas` (`compile.rs` keeps VCA tracks out of `tracks`
//!   and copies `Track::vca` / `TrackInput::Track` into `TrackDesc::{vca, input_tap}`).
//!   Engine: `ether_core::{vca, bus_tap}`.
//! - Solo rules (CONTRACTS.md §12.10) are compiled in `ether-core/src/graph.rs` (solo_ok);
//!   [`fold_vca_solo`] folds VCA solo into the assigned tracks' `TrackDesc::solo`.
//! - Mute and solo of groups and VCAs are the tracks' own `TrackMixer::{mute, solo}`, so in
//!   collab they are per-user like every other track's (site-local, COLLAB.md §2.1).
//! - Deleting a VCA unassigns its tracks (`doc/mod.rs`).

use std::collections::BTreeSet;

use ether_core::graph::{AutomationDesc, ResolvedTarget, TrackDesc};
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::vca::VcaDesc;

use crate::compile::{CompileContext, TRACK_VOLUME_MAPPING};
use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid, invalid_state};

/// Sorted `(order, id)` of the children of `parent` (`None` = top level).
fn siblings(p: &Project, parent: Option<TrackId>) -> Vec<(OrderKey, TrackId)> {
    let tracks = match parent {
        None => p.tracks_ordered(),
        Some(g) => p.child_tracks(g),
    };
    tracks
        .into_iter()
        .map(|t| (t.order.clone(), t.id))
        .collect()
}

/// Name of a group created by `GroupSelected { name: None }`.
pub const DEFAULT_GROUP_NAME: &str = "Group";

pub(crate) fn track_command(ctx: &mut DocCtx, command: &TrackCommand) -> CmdResult<()> {
    match command {
        TrackCommand::GroupSelected { ids, group, name } => {
            group_selected(ctx, ids, *group, name.as_deref())
        }
        TrackCommand::Ungroup { group, force } => ungroup(ctx, *group, *force),
        TrackCommand::SetVca { id, vca } => set_vca(ctx, *id, *vca),
        _ => Err(invalid("not a groups command")),
    }
}

/// Cmd+G (see [`TrackCommand::GroupSelected`]).
fn group_selected(
    ctx: &mut DocCtx,
    ids: &[TrackId],
    group: TrackId,
    name: Option<&str>,
) -> CmdResult<()> {
    if ctx.p().tracks.contains_key(&group) {
        // Idempotent retry of the same command (like `Track::Create`).
        let existing = ctx.track(group)?;
        if existing.kind == TrackKind::Group
            && ids.iter().all(|id| {
                ctx.p()
                    .tracks
                    .get(id)
                    .is_some_and(|t| t.parent == Some(group))
            })
        {
            return Ok(());
        }
        return Err(invalid(format!("track {group} already exists")));
    }
    if ids.is_empty() {
        return Err(invalid("select at least one track to group"));
    }
    let unique: BTreeSet<TrackId> = ids.iter().copied().collect();
    if unique.len() != ids.len() {
        return Err(invalid("duplicate track in the selection"));
    }
    let mut tracks = Vec::with_capacity(ids.len());
    for id in ids {
        let t = ctx.track(*id)?;
        if matches!(
            t.kind,
            TrackKind::Master | TrackKind::Return | TrackKind::Vca
        ) {
            return Err(invalid(format!("{:?} tracks cannot be grouped", t.kind)));
        }
        tracks.push(t);
    }
    let parent = tracks[0].parent;
    if tracks.iter().any(|t| t.parent != parent) {
        return Err(invalid(
            "grouped tracks must share the same parent (select siblings)",
        ));
    }
    // Siblings in track order; the group takes the place of the first selected one.
    let sibs = siblings(ctx.p(), parent);
    let first = sibs
        .iter()
        .position(|(_, id)| unique.contains(id))
        .ok_or_else(|| invalid("selection not found among its siblings"))?;
    let lo = first.checked_sub(1).map(|i| &sibs[i].0);
    let order = OrderKey::try_between(lo, Some(&sibs[first].0)).map_err(invalid)?;
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(DEFAULT_GROUP_NAME)
        .to_string();
    ctx.tx.insert(Entity::Track(Track {
        id: group,
        kind: TrackKind::Group,
        name,
        // The group takes the colour of its first track.
        color: ctx.track(sibs[first].1)?.color,
        order,
        parent,
        mixer: TrackMixer::default(),
        input: TrackInput::None,
        output: TrackOutput::Default,
        monitor: MonitorMode::default(),
        scale: Default::default(),
        freeze: None,
        vca: None,
    }))?;
    // Children keep their relative order (their order keys are already sorted and unique
    // among the old siblings, so they stay valid inside the new group).
    for (_, id) in sibs.iter().filter(|(_, id)| unique.contains(id)) {
        ctx.set_track(*id, TrackChange::Parent(Some(group)))?;
    }
    Ok(())
}

/// What ungrouping `group` would lose (for the `force` check), as a human list.
pub(crate) fn ungroup_losses(p: &Project, group: TrackId) -> Vec<String> {
    let mut lost = Vec::new();
    let devices = p.devices_of(group).len();
    if devices > 0 {
        lost.push(format!("{devices} device(s)"));
    }
    let lanes = p
        .automation_lanes
        .values()
        .filter(|l| {
            matches!(l.owner, AutomationOwner::Track { track } if track == group)
                || matches!(
                    l.target,
                    AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track }
                        if track == group
                )
        })
        .count();
    if lanes > 0 {
        lost.push(format!("{lanes} automation lane(s)"));
    }
    let sends = p
        .sends
        .values()
        .filter(|s| s.from == group || s.to == group)
        .count();
    if sends > 0 {
        lost.push(format!("{sends} send(s)"));
    }
    let routed = p
        .tracks
        .values()
        .filter(|t| {
            matches!(t.output, TrackOutput::Track { track } if track == group)
                || matches!(t.input, TrackInput::Track { track, .. } if track == group)
        })
        .count();
    if routed > 0 {
        lost.push(format!("{routed} routing(s) to it"));
    }
    lost
}

fn ungroup(ctx: &mut DocCtx, group: TrackId, force: bool) -> CmdResult<()> {
    let g = ctx.track(group)?;
    if g.kind != TrackKind::Group {
        return Err(invalid(format!("track {group} is not a group")));
    }
    let lost = ungroup_losses(ctx.p(), group);
    if !lost.is_empty() && !force {
        return Err(invalid_state(format!(
            "ungrouping {} loses its {}",
            g.name,
            lost.join(", ")
        )));
    }
    // Children go to the group's parent, at the group's place, in their order.
    let sibs = siblings(ctx.p(), g.parent);
    let at = sibs
        .iter()
        .position(|(_, id)| *id == group)
        .ok_or_else(|| invalid("group not found among its siblings"))?;
    let hi = sibs.get(at + 1).map(|s| s.0.clone());
    let mut lo = at.checked_sub(1).map(|i| sibs[i].0.clone());
    let kids: Vec<TrackId> = ctx.p().child_tracks(group).iter().map(|t| t.id).collect();
    for k in kids {
        let order = OrderKey::try_between(lo.as_ref(), hi.as_ref()).map_err(invalid)?;
        ctx.set_track(k, TrackChange::Parent(g.parent))?;
        ctx.set_track(k, TrackChange::Order(order.clone()))?;
        lo = Some(order);
    }
    // Devices, lanes, sends and routings go with the group (outputs reset to `Default`).
    ctx.delete_track_only(group)
}

fn set_vca(ctx: &mut DocCtx, id: TrackId, vca: Option<TrackId>) -> CmdResult<()> {
    let t = ctx.track(id)?;
    if t.vca == vca {
        return Ok(());
    }
    if t.kind == TrackKind::Master {
        return Err(invalid("the master track cannot be assigned to a VCA"));
    }
    if let Some(v) = vca {
        if ctx.track(v)?.kind != TrackKind::Vca {
            return Err(invalid(format!("track {v} is not a VCA")));
        }
        // Cycles (a VCA assigned to itself through others).
        let mut cur = Some(v);
        let mut steps = 0;
        while let Some(c) = cur {
            if c == id || steps > ctx.p().tracks.len() {
                return Err(invalid("VCA assignment cycle"));
            }
            cur = ctx.p().tracks.get(&c).and_then(|t| t.vca);
            steps += 1;
        }
    }
    ctx.set_track(id, TrackChange::Vca(vca))
}

/// `RenderGraphDesc::vcas`: every VCA track with its fader, mute, parent VCA and volume
/// automation.
pub(crate) fn vca_descs(p: &Project, ctx: &CompileContext) -> Vec<VcaDesc> {
    let _ = ctx;
    let mut vcas: Vec<&Track> = p
        .tracks
        .values()
        .filter(|t| t.kind == TrackKind::Vca)
        .collect();
    vcas.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
    vcas.into_iter()
        .map(|t| VcaDesc {
            id: t.id,
            volume: t.mixer.volume.to_linear(),
            mute: t.mixer.mute,
            parent: t
                .vca
                .filter(|v| p.tracks.get(v).is_some_and(|v| v.kind == TrackKind::Vca)),
            automation: p
                .automation_lanes
                .values()
                .filter(|l| {
                    l.enabled
                        && matches!(l.owner, AutomationOwner::Track { .. })
                        && l.target == AutomationTarget::TrackVolume { track: t.id }
                })
                .filter_map(|l| {
                    let points: Vec<(f64, f64, CurveShape)> = p
                        .points_of(l.id)
                        .into_iter()
                        .map(|pt| (pt.time.0, pt.value, pt.curve))
                        .collect();
                    (!points.is_empty()).then_some(AutomationDesc {
                        target: l.target,
                        resolved: ResolvedTarget::TrackVolume,
                        points,
                        mapping: TRACK_VOLUME_MAPPING,
                    })
                })
                .collect(),
        })
        .collect()
}

/// VCAs soloed directly or through a soloed parent VCA.
fn soloed_vcas(p: &Project) -> BTreeSet<TrackId> {
    p.tracks
        .values()
        .filter(|t| t.kind == TrackKind::Vca)
        .filter(|t| {
            let mut cur = Some(t.id);
            let mut steps = 0;
            while let Some(c) = cur {
                let Some(v) = p.tracks.get(&c) else { break };
                if v.mixer.solo {
                    return true;
                }
                cur = v.vca;
                steps += 1;
                if steps > p.tracks.len() {
                    break;
                }
            }
            false
        })
        .map(|t| t.id)
        .collect()
}

/// Fold VCA solo into the assigned tracks' `TrackDesc::solo` (CONTRACTS.md §12.10: soloing
/// a VCA solos its tracks, nested VCAs included). Called by `compile_graph_with`.
pub(crate) fn fold_vca_solo(p: &Project, tracks: &mut [TrackDesc]) {
    let soloed = soloed_vcas(p);
    if soloed.is_empty() {
        return;
    }
    for td in tracks.iter_mut() {
        if td.vca.is_some_and(|v| soloed.contains(&v)) {
            td.solo = true;
        }
    }
}
