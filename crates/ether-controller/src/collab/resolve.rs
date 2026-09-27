//! Deterministic application of ops from other sites (docs/COLLAB.md §2, §3).
//!
//! Everything here is a pure function of (project, ops): the same on every site, and every
//! op goes through `Project::apply`, so every intermediate state validates.

use ether_core::protocol::Command;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;

use crate::doc::{self, DocCtx, DocHost};
use crate::tx::{CmdResult, Tx};

/// A host with no engine: cascades must not depend on site-local state (plugin state).
struct NullHost;

impl DocHost for NullHost {
    fn descriptor(&mut self, _: DeviceId, _: &DeviceKind) -> Option<DeviceDescriptor> {
        None
    }
    fn plugin_state(&mut self, _: DeviceId) -> Option<Base64Bytes> {
        None
    }
    fn instantiate_plugin(
        &mut self,
        _: DeviceId,
        _: &PluginInstance,
    ) -> CmdResult<Option<DeviceDescriptor>> {
        Ok(None)
    }
}

/// The ops that delete `key` with everything that depends on it (the controller's own
/// cascades: children removed, references detached), ending with its `Remove`. `None` if
/// there is no cascade for that kind of entity (the remove is then skipped).
fn cascade(project: &mut Project, key: EntityKey) -> Option<Vec<Op>> {
    let mut ids = IdGen::new(0);
    let mut host = NullHost;
    let mut ctx = DocCtx {
        tx: Tx::new(project),
        ids: &mut ids,
        now: 0,
        position: Beats::ZERO,
        host: &mut host,
        warnings: Vec::new(),
    };
    let result = match key {
        EntityKey::Track(id) => {
            doc::apply(&mut ctx, &Command::Track(TrackCommand::Delete { id })).map(drop)
        }
        EntityKey::Clip(id) => ctx.delete_clip(id),
        EntityKey::Device(id) => ctx.delete_device(id),
        EntityKey::Send(id) => ctx.delete_send(id),
        EntityKey::AutomationLane(id) => ctx.delete_lane(id),
        EntityKey::DrumPad(id) => ctx.delete_pad(id),
        // Media still used by a clip/sampler stays (the use wins).
        _ => return None,
    };
    match result {
        Ok(()) => Some(ctx.tx.finish()),
        Err(_) => {
            ctx.tx.rollback();
            None
        }
    }
}

/// Apply one op with the collab rules, appending what was applied and its inverses:
/// - applies → kept;
/// - `Remove` refused because of dependents → delete wins (cascade, then the remove);
/// - anything else refused (missing entity, existing id, dangling reference, cycle,
///   invalid value) → skipped.
fn resolve_one(project: &mut Project, op: &Op, applied: &mut Vec<Op>, inverses: &mut Vec<Op>) {
    match project.apply(op) {
        Ok(inv) => {
            applied.push(op.clone());
            inverses.push(inv);
        }
        Err(ModelError::HasChildren(key)) => {
            if let Some(ops) = cascade(project, key) {
                for c in ops {
                    if let Ok(inv) = project.apply(&c) {
                        applied.push(c);
                        inverses.push(inv);
                    }
                }
            }
        }
        Err(_) => {}
    }
}

/// Apply `ops` in order with the collab rules. Returns `(applied, inverse)`: the ops that
/// were applied (cascades included), and the ops reverting them, ready to apply in order.
pub(crate) fn resolve_all(project: &mut Project, ops: &[Op]) -> (Vec<Op>, Vec<Op>) {
    let mut applied = Vec::with_capacity(ops.len());
    let mut inverses = Vec::with_capacity(ops.len());
    for op in ops {
        resolve_one(project, op, &mut applied, &mut inverses);
    }
    inverses.reverse();
    (applied, inverses)
}

/// Does the field `guard` writes currently hold the value `guard` writes? (Applying it
/// would change nothing.) Always true for inserts/removes.
fn holds(project: &mut Project, guard: Option<&Op>) -> bool {
    match guard {
        Some(g @ (Op::Update { .. } | Op::Settings { .. })) => match project.apply(g) {
            Ok(inv) if &inv == g => true,
            Ok(inv) => {
                // Not the same value: put the current one back.
                let _ = project.apply(&inv);
                false
            }
            Err(_) => false,
        },
        _ => true,
    }
}

/// Per-site undo/redo (`History::undo_with`/`redo_with`): apply `ops` (a step's inverse, or
/// its forward ops for redo) where `ops[i]` reverts `guards[len - 1 - i]`. A field update is
/// only applied if the field still holds the value the guarded op wrote (else a peer or a
/// later edit changed it since, and theirs wins); inserts/removes go through the collab
/// rules. Returns `(applied, inverse)` like [`resolve_all`].
pub(crate) fn apply_guarded(
    project: &mut Project,
    ops: &[Op],
    guards: &[Op],
) -> (Vec<Op>, Vec<Op>) {
    let n = ops.len();
    let aligned = guards.len() == n;
    let mut applied = Vec::with_capacity(n);
    let mut inverses = Vec::with_capacity(n);
    for (i, op) in ops.iter().enumerate() {
        let guard = if aligned { guards.get(n - 1 - i) } else { None };
        if matches!(op, Op::Update { .. } | Op::Settings { .. }) && !holds(project, guard) {
            continue;
        }
        resolve_one(project, op, &mut applied, &mut inverses);
    }
    inverses.reverse();
    (applied, inverses)
}

/// Ops that are site-local (docs/COLLAB.md §2.1): never sent, ignored when received.
pub(crate) fn is_local_only(op: &Op) -> bool {
    match op {
        Op::Settings { change } => matches!(
            change,
            SettingsChange::LoopEnabled(_)
                | SettingsChange::LoopRegion(_)
                | SettingsChange::Metronome(_)
                | SettingsChange::CountInBars(_)
                | SettingsChange::MetronomeVolume(_)
                | SettingsChange::MetronomeAccent(_)
                | SettingsChange::MetronomeSound(_)
        ),
        Op::Update {
            update:
                EntityUpdate::Track {
                    change: TrackChange::Solo(_),
                    ..
                },
        } => true,
        _ => false,
    }
}

/// Keep the shared ops of an applied transaction and their inverses (`inverse[i]` reverts
/// `ops[len - 1 - i]`).
pub(crate) fn split_shared(ops: &[Op], inverse: &[Op]) -> (Vec<Op>, Vec<Op>) {
    let n = ops.len();
    let shared: Vec<bool> = ops.iter().map(|op| !is_local_only(op)).collect();
    let kept_ops = ops
        .iter()
        .zip(&shared)
        .filter(|(_, s)| **s)
        .map(|(op, _)| op.clone())
        .collect();
    let kept_inv = if inverse.len() == n {
        inverse
            .iter()
            .enumerate()
            .filter(|(i, _)| shared[n - 1 - i])
            .map(|(_, op)| op.clone())
            .collect()
    } else {
        inverse.to_vec()
    };
    (kept_ops, kept_inv)
}

/// The site-local settings of `from` as ops to apply to another project (a joiner keeps
/// its own).
pub(crate) fn local_settings(from: &ProjectSettings) -> Vec<Op> {
    [
        SettingsChange::LoopEnabled(from.loop_enabled),
        SettingsChange::LoopRegion(from.loop_region),
        SettingsChange::Metronome(from.metronome),
        SettingsChange::CountInBars(from.count_in_bars),
        SettingsChange::MetronomeVolume(from.metronome_volume),
        SettingsChange::MetronomeAccent(from.metronome_accent),
        SettingsChange::MetronomeSound(from.metronome_sound),
    ]
    .into_iter()
    .map(|change| Op::Settings { change })
    .collect()
}
