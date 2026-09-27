//! Deterministic application of ops from other sites (docs/COLLAB.md §2, §3).
//!
//! Everything here is a pure function of (project, ops): the same on every site, and every
//! op goes through `Project::apply`, so every intermediate state validates.

use std::collections::BTreeMap;

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
/// Mute and solo are per-user (each site mixes for itself), like the settings below.
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
                    change: TrackChange::Mute(_) | TrackChange::Solo(_),
                    ..
                }
                | EntityUpdate::DrumPad {
                    change: DrumPadChange::Mute(_),
                    ..
                },
        } => true,
        _ => false,
    }
}

/// `op` as sent to the relay: `None` if it is local-only; an `Insert` of a track or drum pad
/// carries no mute/solo (the receiver's new entity starts unmuted and unsoloed).
pub(crate) fn outgoing(op: &Op) -> Option<Op> {
    if is_local_only(op) {
        return None;
    }
    Some(match op {
        Op::Insert {
            entity: Entity::Track(t),
        } => {
            let mut t = t.clone();
            t.mixer.mute = false;
            t.mixer.solo = false;
            Op::Insert {
                entity: Entity::Track(t),
            }
        }
        Op::Insert {
            entity: Entity::DrumPad(d),
        } => {
            let mut d = d.clone();
            d.mute = false;
            Op::Insert {
                entity: Entity::DrumPad(d),
            }
        }
        op => op.clone(),
    })
}

/// The ops of an applied transaction to send ([`outgoing`]) and the inverses of the ones
/// kept (`inverse[i]` reverts `ops[len - 1 - i]`).
pub(crate) fn split_shared(ops: &[Op], inverse: &[Op]) -> (Vec<Op>, Vec<Op>) {
    let n = ops.len();
    let sent: Vec<Option<Op>> = ops.iter().map(outgoing).collect();
    let kept_inv = if inverse.len() == n {
        inverse
            .iter()
            .enumerate()
            .filter(|(i, _)| sent[n - 1 - i].is_some())
            .map(|(_, op)| op.clone())
            .collect()
    } else {
        inverse.to_vec()
    };
    (sent.into_iter().flatten().collect(), kept_inv)
}

/// This site's mix (docs/COLLAB.md §2.1): track mute/solo and drum pad mute.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LocalMix {
    tracks: BTreeMap<TrackId, (bool, bool)>,
    pads: BTreeMap<DrumPadId, bool>,
}

impl LocalMix {
    pub(crate) fn of(p: &Project) -> Self {
        Self {
            tracks: p
                .tracks
                .values()
                .map(|t| (t.id, (t.mixer.mute, t.mixer.solo)))
                .collect(),
            pads: p.drum_pads.values().map(|d| (d.id, d.mute)).collect(),
        }
    }

    /// This mix, plus `other`'s values for the tracks and pads it doesn't have.
    pub(crate) fn or(mut self, other: &LocalMix) -> Self {
        for (id, v) in &other.tracks {
            self.tracks.entry(*id).or_insert(*v);
        }
        for (id, v) in &other.pads {
            self.pads.entry(*id).or_insert(*v);
        }
        self
    }

    /// Put this mix on `p`: tracks and pads it doesn't know (created by others) end up
    /// unmuted and unsoloed. Returns the ops applied (for the patch and the engine).
    pub(crate) fn overlay(&self, p: &mut Project) -> Vec<Op> {
        let mut ops = Vec::new();
        for t in p.tracks.values() {
            let (mute, solo) = self.tracks.get(&t.id).copied().unwrap_or_default();
            if t.mixer.mute != mute {
                ops.push(Op::Update {
                    update: EntityUpdate::Track {
                        id: t.id,
                        change: TrackChange::Mute(mute),
                    },
                });
            }
            if t.mixer.solo != solo {
                ops.push(Op::Update {
                    update: EntityUpdate::Track {
                        id: t.id,
                        change: TrackChange::Solo(solo),
                    },
                });
            }
        }
        for d in p.drum_pads.values() {
            let mute = self.pads.get(&d.id).copied().unwrap_or_default();
            if d.mute != mute {
                ops.push(Op::Update {
                    update: EntityUpdate::DrumPad {
                        id: d.id,
                        change: DrumPadChange::Mute(mute),
                    },
                });
            }
        }
        ops.retain(|op| p.apply(op).is_ok());
        ops
    }
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

/// The part of `p` every site shares (site-local fields reset, opaque plugin states
/// dropped): what "the same document" means when comparing a stored copy with the session.
pub(crate) fn shared_part(p: &Project) -> Project {
    let mut p = p.clone();
    for op in local_settings(&ProjectSettings::default()) {
        if let Op::Settings { change } = op {
            let _ = p.apply(&Op::Settings { change });
        }
    }
    LocalMix::default().overlay(&mut p);
    for d in p.devices.values_mut() {
        if let DeviceKind::Plugin { plugin } = &mut d.kind {
            plugin.state = None;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_op(id: TrackId, change: TrackChange) -> Op {
        Op::Update {
            update: EntityUpdate::Track { id, change },
        }
    }

    #[test]
    fn mute_and_solo_are_stripped_from_what_is_sent() {
        let mut ids = IdGen::new(3);
        let p = Project::new(&mut ids, 0);
        let t = TrackId(ids.next_ulid(0));
        let mut track = p.tracks.values().next().unwrap().clone();
        track.id = t;
        track.mixer.mute = true;
        track.mixer.solo = true;
        let ops = vec![
            Op::Insert {
                entity: Entity::Track(track),
            },
            track_op(t, TrackChange::Name("lead".into())),
            track_op(t, TrackChange::Mute(true)),
            track_op(t, TrackChange::Solo(true)),
        ];
        // inverse[i] reverts ops[len - 1 - i].
        let inverse = vec![
            track_op(t, TrackChange::Solo(false)),
            track_op(t, TrackChange::Mute(false)),
            track_op(t, TrackChange::Name("old".into())),
            Op::Remove {
                key: EntityKey::Track(t),
            },
        ];
        let (sent, inv) = split_shared(&ops, &inverse);
        assert_eq!(sent.len(), 2);
        let Op::Insert {
            entity: Entity::Track(sent_track),
        } = &sent[0]
        else {
            panic!("{sent:?}")
        };
        assert!(!sent_track.mixer.mute && !sent_track.mixer.solo);
        assert_eq!(sent[1], ops[1]);
        assert_eq!(inv, inverse[2..].to_vec());
    }

    #[test]
    fn overlay_keeps_known_tracks_and_resets_unknown_ones() {
        let mut ids = IdGen::new(5);
        let mut p = Project::new(&mut ids, 0);
        let master = *p.tracks.keys().next().unwrap();
        // Another site's state of a track we know: ours is put back.
        let ours = LocalMix::of(&p);
        p.apply(&track_op(master, TrackChange::Solo(true))).unwrap();
        assert_eq!(
            ours.overlay(&mut p),
            vec![track_op(master, TrackChange::Solo(false))]
        );
        assert!(ours.overlay(&mut p).is_empty());
        // A track we don't know starts unmuted and unsoloed.
        p.apply(&track_op(master, TrackChange::Mute(true))).unwrap();
        let none = LocalMix::default();
        assert_eq!(
            none.overlay(&mut p),
            vec![track_op(master, TrackChange::Mute(false))]
        );
    }
}
