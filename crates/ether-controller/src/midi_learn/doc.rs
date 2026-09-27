//! Document part of MIDI mapping: `Map`, `Edit`, `Unmap` (undoable) and the learn commit.
//! The model validates ranges, targets and the one-mapping-per-source invariant.

use ether_core::protocol::midi_map::MidiMapCommand;
use ether_core::protocol::model::*;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, internal, invalid, not_found};

pub(crate) fn apply(ctx: &mut DocCtx, c: &MidiMapCommand) -> CmdResult<()> {
    match c {
        MidiMapCommand::Map { mapping } => map(ctx, mapping.clone()),
        MidiMapCommand::Edit { id, min, max, mode } => {
            let m = ctx
                .p()
                .midi_mappings
                .get(id)
                .cloned()
                .ok_or_else(|| not_found(format!("MIDI mapping {id}")))?;
            for v in [min, max].into_iter().flatten() {
                check_unit(*v)?;
            }
            let mut changes = Vec::new();
            if let Some(v) = min.filter(|v| *v != m.min) {
                changes.push(MidiMappingChange::Min(v));
            }
            if let Some(v) = max.filter(|v| *v != m.max) {
                changes.push(MidiMappingChange::Max(v));
            }
            if let Some(v) = mode.filter(|v| *v != m.mode) {
                changes.push(MidiMappingChange::Mode(v));
            }
            for change in changes {
                ctx.tx
                    .update(EntityUpdate::MidiMapping { id: *id, change })?;
            }
            Ok(())
        }
        MidiMapCommand::Unmap { ids } => {
            let mut seen = std::collections::BTreeSet::new();
            for id in ids {
                if !seen.insert(*id) {
                    continue;
                }
                if !ctx.p().midi_mappings.contains_key(id) {
                    return Err(not_found(format!("MIDI mapping {id}")));
                }
                ctx.tx.remove(EntityKey::MidiMapping(*id))?;
            }
            Ok(())
        }
        MidiMapCommand::Learn { .. } | MidiMapCommand::List => {
            Err(internal("Learn/List are not document commands"))
        }
    }
}

fn check_unit(v: f64) -> CmdResult<()> {
    if (0.0..=1.0).contains(&v) {
        Ok(())
    } else {
        Err(invalid("mapping min/max must be within 0..=1"))
    }
}

/// `Map`: create (or replace, same id) a mapping; a mapping with the same source is replaced
/// in the same step. Re-sending an identical mapping is a no-op.
fn map(ctx: &mut DocCtx, mapping: MidiMapping) -> CmdResult<()> {
    check_unit(mapping.min)?;
    check_unit(mapping.max)?;
    if let Some(existing) = ctx.p().midi_mappings.get(&mapping.id) {
        if *existing == mapping {
            return Ok(());
        }
        ctx.tx.remove(EntityKey::MidiMapping(mapping.id))?;
    }
    remove_where(ctx, |m| m.id != mapping.id && m.source == mapping.source)?;
    ctx.tx.insert(Entity::MidiMapping(mapping))
}

/// Learn: the new mapping replaces every mapping of its target and of its source.
pub(super) fn learn(ctx: &mut DocCtx, mapping: MidiMapping) -> CmdResult<()> {
    remove_where(ctx, |m| m.target == mapping.target)?;
    map(ctx, mapping)
}

fn remove_where(ctx: &mut DocCtx, pred: impl Fn(&MidiMapping) -> bool) -> CmdResult<()> {
    let ids: Vec<MidiMappingId> = ctx
        .p()
        .midi_mappings
        .values()
        .filter(|m| pred(m))
        .map(|m| m.id)
        .collect();
    for id in ids {
        ctx.tx.remove(EntityKey::MidiMapping(id))?;
    }
    Ok(())
}
