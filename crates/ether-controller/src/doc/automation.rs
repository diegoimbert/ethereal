//! `AutomationCommand`s.

use ether_core::protocol::automation::AutomationCommand;
use ether_core::protocol::model::*;

use super::DocCtx;
use crate::tx::{CmdResult, invalid, not_found};

fn check_target(ctx: &DocCtx, target: &AutomationTarget) -> CmdResult<()> {
    match target {
        AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track } => {
            ctx.track(*track).map(drop)
        }
        AutomationTarget::SendLevel { send } => ctx.send(*send).map(drop),
        AutomationTarget::DeviceParam { device, .. } => ctx.device(*device).map(drop),
    }
}

fn value(v: f64) -> CmdResult<f64> {
    if !v.is_finite() {
        return Err(invalid("automation values must be finite"));
    }
    Ok(v.clamp(0.0, 1.0))
}

fn time(t: Beats) -> CmdResult<Beats> {
    if !t.0.is_finite() {
        return Err(invalid("automation times must be finite"));
    }
    Ok(Beats(t.0.max(0.0)))
}

fn point(ctx: &DocCtx, id: AutomationPointId) -> CmdResult<AutomationPoint> {
    ctx.p()
        .automation_points
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("automation point {id}")))
}

fn set(ctx: &mut DocCtx, id: AutomationPointId, change: AutomationPointChange) -> CmdResult<()> {
    ctx.tx.update(EntityUpdate::AutomationPoint { id, change })
}

pub(super) fn apply(ctx: &mut DocCtx, c: &AutomationCommand) -> CmdResult<()> {
    match c {
        AutomationCommand::CreateLane { id, owner, target } => {
            if ctx.p().automation_lanes.contains_key(id) {
                return Ok(());
            }
            match owner {
                AutomationOwner::Track { track } => ctx.track(*track).map(drop)?,
                AutomationOwner::Clip { clip } => ctx.clip(*clip).map(drop)?,
            }
            check_target(ctx, target)?;
            if ctx
                .p()
                .automation_lanes
                .values()
                .any(|l| l.owner == *owner && l.target == *target)
            {
                return Err(invalid("a lane for this owner/target already exists"));
            }
            ctx.tx.insert(Entity::AutomationLane(AutomationLane {
                id: *id,
                owner: *owner,
                target: *target,
                enabled: true,
            }))
        }
        AutomationCommand::DeleteLane { id } => {
            ctx.lane(*id)?;
            ctx.delete_lane(*id)
        }
        AutomationCommand::SetLaneEnabled { id, enabled } => {
            ctx.lane(*id)?;
            ctx.tx.update(EntityUpdate::AutomationLane {
                id: *id,
                change: AutomationLaneChange::Enabled(*enabled),
            })
        }
        AutomationCommand::AddPoints { lane, points } => {
            ctx.lane(*lane)?;
            for p in points {
                if ctx.p().automation_points.contains_key(&p.id) {
                    continue;
                }
                ctx.tx.insert(Entity::AutomationPoint(AutomationPoint {
                    id: p.id,
                    lane: *lane,
                    time: time(p.time)?,
                    value: value(p.value)?,
                    curve: p.curve,
                }))?;
            }
            Ok(())
        }
        AutomationCommand::RemovePoints { ids } => {
            for id in ids {
                point(ctx, *id)?;
                ctx.tx.remove(EntityKey::AutomationPoint(*id))?;
            }
            Ok(())
        }
        AutomationCommand::EditPoints { edits } => {
            for e in edits {
                let p = point(ctx, e.id)?;
                if let Some(t) = e.time {
                    set(ctx, p.id, AutomationPointChange::Time(time(t)?))?;
                }
                if let Some(v) = e.value {
                    set(ctx, p.id, AutomationPointChange::Value(value(v)?))?;
                }
                if let Some(c) = e.curve {
                    set(ctx, p.id, AutomationPointChange::Curve(c))?;
                }
            }
            Ok(())
        }
        AutomationCommand::ClearRange { lane, start, end } => {
            ctx.lane(*lane)?;
            let ids: Vec<AutomationPointId> = ctx
                .p()
                .points_of(*lane)
                .into_iter()
                .filter(|p| p.time.0 >= start.0 && p.time.0 < end.0)
                .map(|p| p.id)
                .collect();
            for id in ids {
                ctx.tx.remove(EntityKey::AutomationPoint(id))?;
            }
            Ok(())
        }
    }
}
