//! `TrackCommand`s.

use std::collections::BTreeMap;

use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;

use super::{DocCtx, order_after, order_before};
use crate::tx::{CmdResult, invalid};

/// Default track colors (same palette as the UI mock).
pub(crate) const TRACK_COLORS: [u32; 10] = [
    0xff764d, 0xffa53f, 0xf0d03f, 0x99d44a, 0x3fc98c, 0x3fc2d9, 0x5c9dff, 0x9b7bff, 0xe06adf,
    0xff6b8b,
];

fn default_name(kind: TrackKind, n: usize) -> String {
    match kind {
        TrackKind::Audio => format!("{n} Audio"),
        TrackKind::Midi => format!("{n} MIDI"),
        TrackKind::Group => format!("{n} Group"),
        TrackKind::Return => {
            let letter = char::from(b'A' + ((n.max(1) - 1) % 26) as u8);
            format!("{letter} Return")
        }
        TrackKind::Master => "Master".into(),
    }
}

fn default_input(kind: TrackKind) -> TrackInput {
    match kind {
        TrackKind::Audio => TrackInput::Audio { first: 0, count: 2 },
        TrackKind::Midi => TrackInput::Midi {
            port: None,
            channel: None,
        },
        _ => TrackInput::None,
    }
}

/// Sorted `(order, id)` of the children of `parent` (`None` = top level), minus `except`.
pub(crate) fn siblings(
    p: &Project,
    parent: Option<TrackId>,
    except: Option<TrackId>,
) -> Vec<(OrderKey, TrackId)> {
    let tracks = match parent {
        None => p.tracks_ordered(),
        Some(g) => p.child_tracks(g),
    };
    tracks
        .into_iter()
        .filter(|t| Some(t.id) != except)
        .map(|t| (t.order.clone(), t.id))
        .collect()
}

/// Ableton layout for `before: None` at the top level: regular tracks go before the first
/// return/master track, returns before master.
fn default_before(
    p: &Project,
    kind: TrackKind,
    parent: Option<TrackId>,
    except: Option<TrackId>,
) -> Option<TrackId> {
    if parent.is_some() || kind == TrackKind::Master {
        return None;
    }
    p.tracks_ordered()
        .into_iter()
        .filter(|t| Some(t.id) != except)
        .find(|t| {
            t.kind == TrackKind::Master
                || (kind != TrackKind::Return && t.kind == TrackKind::Return)
        })
        .map(|t| t.id)
}

fn validate_parent(
    ctx: &DocCtx,
    kind: TrackKind,
    parent: Option<TrackId>,
    this: Option<TrackId>,
) -> CmdResult<()> {
    let Some(parent) = parent else { return Ok(()) };
    if matches!(kind, TrackKind::Return | TrackKind::Master) {
        return Err(invalid(format!("{kind:?} tracks must be top-level")));
    }
    if ctx.track(parent)?.kind != TrackKind::Group {
        return Err(invalid(format!("parent {parent} is not a group track")));
    }
    if let Some(this) = this {
        let mut cur = Some(parent);
        while let Some(c) = cur {
            if c == this {
                return Err(invalid("a group cannot be moved into itself"));
            }
            cur = ctx.p().tracks.get(&c).and_then(|t| t.parent);
        }
    }
    Ok(())
}

/// All descendants of a group, parents before children.
fn descendants(p: &Project, id: TrackId) -> Vec<TrackId> {
    let mut out = Vec::new();
    for c in p.child_tracks(id) {
        out.push(c.id);
        out.extend(descendants(p, c.id));
    }
    out
}

pub(super) fn apply(ctx: &mut DocCtx, c: &TrackCommand) -> CmdResult<()> {
    match c {
        TrackCommand::SetScale { id, scale } => {
            if ctx.track(*id)?.kind != TrackKind::Midi {
                return Err(invalid("track scales are only available on MIDI tracks"));
            }
            ctx.set_track(*id, TrackChange::Scale(*scale))
        }
        TrackCommand::Create {
            id,
            kind,
            name,
            color,
            parent,
            before,
        } => {
            if ctx.p().tracks.contains_key(id) {
                return Ok(());
            }
            if *kind == TrackKind::Master {
                return Err(invalid("there is exactly one master track"));
            }
            validate_parent(ctx, *kind, *parent, None)?;
            let before = before.or_else(|| default_before(ctx.p(), *kind, *parent, None));
            let order = order_before(&siblings(ctx.p(), *parent, None), before)?;
            let same_kind = ctx.p().tracks.values().filter(|t| t.kind == *kind).count();
            let n_tracks = ctx.p().tracks.len();
            let track = Track {
                id: *id,
                kind: *kind,
                name: name
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| default_name(*kind, same_kind + 1)),
                color: color.unwrap_or(Color(TRACK_COLORS[n_tracks % TRACK_COLORS.len()])),
                order,
                parent: *parent,
                mixer: TrackMixer::default(),
                input: default_input(*kind),
                output: TrackOutput::Default,
                monitor: MonitorMode::default(),
                scale: Default::default(),
            };
            ctx.tx.insert(Entity::Track(track))
        }
        TrackCommand::Delete { id } => {
            let t = ctx.track(*id)?;
            if t.kind == TrackKind::Master {
                return Err(invalid("the master track cannot be deleted"));
            }
            for d in descendants(ctx.p(), t.id).into_iter().rev() {
                ctx.delete_track_only(d)?;
            }
            ctx.delete_track_only(t.id)
        }
        TrackCommand::Duplicate { id, new_id } => {
            if ctx.p().tracks.contains_key(new_id) {
                return Ok(());
            }
            let t = ctx.track(*id)?;
            if t.kind == TrackKind::Master {
                return Err(invalid("the master track cannot be duplicated"));
            }
            let order = order_after(&siblings(ctx.p(), t.parent, None), t.id)?;
            duplicate(ctx, &t, *new_id, order, t.parent)
        }
        TrackCommand::Rename { id, name } => {
            ctx.track(*id)?;
            ctx.set_track(*id, TrackChange::Name(name.clone()))
        }
        TrackCommand::SetColor { id, color } => {
            ctx.track(*id)?;
            ctx.set_track(*id, TrackChange::Color(*color))
        }
        TrackCommand::Move { id, parent, before } => {
            let t = ctx.track(*id)?;
            if t.kind == TrackKind::Master && parent.is_some() {
                return Err(invalid("the master track must be top-level"));
            }
            validate_parent(ctx, t.kind, *parent, Some(t.id))?;
            if *before == Some(t.id) {
                return Ok(());
            }
            let before = before.or_else(|| default_before(ctx.p(), t.kind, *parent, Some(t.id)));
            let order = order_before(&siblings(ctx.p(), *parent, Some(t.id)), before)?;
            if t.parent != *parent {
                ctx.set_track(t.id, TrackChange::Parent(*parent))?;
            }
            ctx.set_track(t.id, TrackChange::Order(order))
        }
    }
}

/// Deep copy of a track: devices, sends, clips (with notes/markers/envelopes), its
/// automation lanes (retargeted to the copies) and, for groups, all children.
fn duplicate(
    ctx: &mut DocCtx,
    t: &Track,
    new_id: TrackId,
    order: OrderKey,
    parent: Option<TrackId>,
) -> CmdResult<()> {
    let mut copy = t.clone();
    copy.id = new_id;
    copy.order = order;
    copy.parent = parent;
    ctx.tx.insert(Entity::Track(copy))?;

    let mut device_ids = BTreeMap::new();
    let devices: Vec<Device> = ctx.p().devices_of(t.id).into_iter().cloned().collect();
    for d in devices {
        let mut nd = d.clone();
        nd.id = ctx.new_id();
        nd.track = new_id;
        if let DeviceKind::Plugin { plugin } = &mut nd.kind
            && let Some(state) = ctx.host.plugin_state(d.id)
        {
            plugin.state = Some(state);
        }
        device_ids.insert(d.id, nd.id);
        ctx.tx.insert(Entity::Device(nd))?;
    }
    let mut send_ids = BTreeMap::new();
    let sends: Vec<TrackSend> = ctx
        .p()
        .sends
        .values()
        .filter(|s| s.from == t.id)
        .cloned()
        .collect();
    for s in sends {
        let mut ns = s.clone();
        ns.id = ctx.new_id();
        ns.from = new_id;
        send_ids.insert(s.id, ns.id);
        ctx.tx.insert(Entity::Send(ns))?;
    }
    let clips: Vec<Clip> = ctx
        .p()
        .clips
        .values()
        .filter(|c| c.track == t.id)
        .cloned()
        .collect();
    let remap = |target: AutomationTarget| -> Option<AutomationTarget> {
        Some(match target {
            AutomationTarget::TrackVolume { track } if track == t.id => {
                AutomationTarget::TrackVolume { track: new_id }
            }
            AutomationTarget::TrackPan { track } if track == t.id => {
                AutomationTarget::TrackPan { track: new_id }
            }
            AutomationTarget::SendLevel { send } => AutomationTarget::SendLevel {
                send: *send_ids.get(&send)?,
            },
            AutomationTarget::DeviceParam { device, param } => AutomationTarget::DeviceParam {
                device: *device_ids.get(&device)?,
                param,
            },
            other => other,
        })
    };
    // Clip envelopes follow their clip to the copied track (retargeted like track lanes).
    for c in clips {
        let id = ctx.new_id();
        ctx.copy_clip(&c, id, |c| c.track = new_id, &|t| remap(*t))?;
    }
    let lanes: Vec<AutomationLane> = ctx
        .p()
        .automation_lanes
        .values()
        .filter(|l| matches!(l.owner, AutomationOwner::Track { track } if track == t.id))
        .cloned()
        .collect();
    for l in lanes {
        let Some(target) = remap(l.target) else {
            continue;
        };
        let lane = AutomationLane {
            id: ctx.new_id(),
            owner: AutomationOwner::Track { track: new_id },
            target,
            enabled: l.enabled,
        };
        ctx.copy_lane(l.id, lane)?;
    }
    let kids: Vec<Track> = ctx.p().child_tracks(t.id).into_iter().cloned().collect();
    let mut prev: Option<OrderKey> = None;
    for k in kids {
        let order = OrderKey::try_between(prev.as_ref(), None).map_err(invalid)?;
        prev = Some(order.clone());
        let id = ctx.new_id();
        duplicate(ctx, &k, id, order, Some(new_id))?;
    }
    Ok(())
}
