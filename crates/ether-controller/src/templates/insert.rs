//! `Template::Insert` (document command): the template's entities with derived ids,
//! placed under `parent` before `before`.
//!
//! - Entity `i` of the template gets `derive_id(seed, i)`; every reference to a template
//!   entity is rewritten to its new id, and every template media id to the project media it
//!   was imported as (done on the JSON form of each entity: ids are ULID strings, so one map
//!   covers every reference field, including map keys).
//! - References out of the template are cut: sends to/from outside tracks are dropped (with
//!   their automation), outputs reset to `Default`, inputs/VCAs/sidechains to none, media
//!   that could not be imported are cleared. Entity kinds a track template doesn't carry
//!   (clips, notes, media, markers, ...) are ignored.
//! - Top-level template tracks go under `parent` before `before` in template order
//!   (`before: None` at the top level: like `Track::Create`, regular tracks before the
//!   first return/master track, returns before master).
//! - Idempotent: if `derive_id(seed, 0)` is already a track, nothing changes.

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::model::*;

use crate::doc::{DocCtx, order_before};
use crate::tx::{CmdResult, internal, invalid};

/// A loaded track template, ready to insert.
#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    pub entities: Vec<Entity>,
    /// Template media id → project media id (imported or matched by hash).
    pub media: BTreeMap<MediaId, MediaId>,
}

fn ulid_of(key: EntityKey) -> Ulid {
    match key {
        EntityKey::Track(i) => i.0,
        EntityKey::Clip(i) => i.0,
        EntityKey::Note(i) => i.0,
        EntityKey::Device(i) => i.0,
        EntityKey::Send(i) => i.0,
        EntityKey::AutomationLane(i) => i.0,
        EntityKey::AutomationPoint(i) => i.0,
        EntityKey::TempoPoint(i) => i.0,
        EntityKey::TimeSignature(i) => i.0,
        EntityKey::WarpMarker(i) => i.0,
        EntityKey::Media(i) => i.0,
        EntityKey::Marker(i) => i.0,
        EntityKey::MidiMapping(i) => i.0,
        EntityKey::DrumPad(i) => i.0,
        EntityKey::TakeLane(i) => i.0,
        EntityKey::CompRegion(i) => i.0,
        EntityKey::RackChain(i) => i.0,
        EntityKey::Modulator(i) => i.0,
        EntityKey::ModMapping(i) => i.0,
        EntityKey::ChatMessage(i) => i.0,
        EntityKey::PinnedNote(i) => i.0,
        EntityKey::ExpressionLane(i) => i.0,
        EntityKey::NoteExpression(i) => i.0,
    }
}

/// Rewrite every string (and object key) found in `map`.
fn rewrite(v: &mut serde_json::Value, map: &BTreeMap<String, String>) {
    match v {
        serde_json::Value::String(s) => {
            if let Some(n) = map.get(s.as_str()) {
                *s = n.clone();
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(|x| rewrite(x, map)),
        serde_json::Value::Object(o) => {
            let old = std::mem::take(o);
            for (k, mut x) in old {
                rewrite(&mut x, map);
                let k = map.get(&k).cloned().unwrap_or(k);
                o.insert(k, x);
            }
        }
        _ => {}
    }
}

/// The template's entities with their new ids and remapped references (no placement yet).
pub(crate) fn remap(prepared: &Prepared, seed: TrackId) -> CmdResult<Vec<Entity>> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (i, e) in prepared.entities.iter().enumerate() {
        let i = u32::try_from(i).map_err(|_| invalid("template too large"))?;
        let new: TrackId = derive_id(seed, i);
        map.insert(ulid_of(e.key()).to_string(), new.0.to_string());
    }
    for (old, new) in &prepared.media {
        map.insert(old.0.to_string(), new.0.to_string());
    }
    prepared
        .entities
        .iter()
        .map(|e| {
            let mut v = serde_json::to_value(e).map_err(|e| internal(e.to_string()))?;
            rewrite(&mut v, &map);
            serde_json::from_value(v).map_err(|e| internal(format!("template entity: {e}")))
        })
        .collect()
}

/// Ableton layout for `before: None` at the top level (as `Track::Create`).
fn default_before(p: &Project, kind: TrackKind) -> Option<TrackId> {
    p.tracks_ordered()
        .into_iter()
        .find(|t| {
            t.kind == TrackKind::Master
                || (kind != TrackKind::Return && t.kind == TrackKind::Return)
        })
        .map(|t| t.id)
}

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

/// Media a built-in device kind references, cleared when the project doesn't have them.
fn clear_missing_media(kind: &mut BuiltinDevice, media: &BTreeMap<MediaId, MediaRef>) {
    match kind {
        BuiltinDevice::Sampler { sample, .. } => {
            if sample.is_some_and(|m| !media.contains_key(&m)) {
                *sample = None;
            }
        }
        BuiltinDevice::MultiSampler { zones } => {
            for z in zones {
                if z.media.is_some_and(|m| !media.contains_key(&m)) {
                    z.media = None;
                }
            }
        }
        BuiltinDevice::ConvolutionReverb { ir } => {
            if matches!(ir, Some(IrSource::Media { media: m }) if !media.contains_key(m)) {
                *ir = None;
            }
        }
        _ => {}
    }
}

/// `Template::Insert` with an already loaded template.
pub(crate) fn insert(
    ctx: &mut DocCtx,
    prepared: &Prepared,
    seed: TrackId,
    parent: Option<TrackId>,
    before: Option<TrackId>,
) -> CmdResult<()> {
    let first: TrackId = derive_id(seed, 0);
    if ctx.p().tracks.contains_key(&first) {
        return Ok(());
    }
    if let Some(g) = parent
        && ctx.track(g)?.kind != TrackKind::Group
    {
        return Err(invalid(format!("parent {g} is not a group track")));
    }
    if let Some(b) = before {
        let b = ctx.track(b)?;
        if b.parent != parent {
            return Err(invalid(format!("{} is not a sibling", b.id)));
        }
    }
    let entities = remap(prepared, seed)?;
    let tracks: BTreeSet<TrackId> = entities
        .iter()
        .filter_map(|e| match e {
            Entity::Track(t) => Some(t.id),
            _ => None,
        })
        .collect();
    if !matches!(entities.first(), Some(Entity::Track(_))) {
        return Err(invalid("a track template starts with a track"));
    }
    let mut sends: BTreeSet<SendId> = BTreeSet::new();
    let mut lanes: BTreeSet<AutomationLaneId> = BTreeSet::new();
    for e in entities {
        match e {
            Entity::Track(mut t) => {
                if t.kind == TrackKind::Master {
                    return Err(invalid("a track template cannot contain the master track"));
                }
                if t.parent.is_none_or(|g| !tracks.contains(&g)) {
                    if parent.is_some() && t.kind == TrackKind::Return {
                        return Err(invalid("return tracks must be top-level"));
                    }
                    t.parent = parent;
                    let before = before.or_else(|| {
                        parent
                            .is_none()
                            .then(|| default_before(ctx.p(), t.kind))
                            .flatten()
                    });
                    t.order = order_before(&siblings(ctx.p(), parent), before)?;
                }
                if matches!(t.output, TrackOutput::Track { track } if !tracks.contains(&track)) {
                    t.output = TrackOutput::Default;
                }
                if matches!(t.input, TrackInput::Track { track, .. } if !tracks.contains(&track)) {
                    t.input = TrackInput::None;
                }
                if t.vca.is_some_and(|v| !tracks.contains(&v)) {
                    t.vca = None;
                }
                t.freeze = None;
                ctx.tx.insert(Entity::Track(t))?;
            }
            Entity::Device(mut d) => {
                if d.sidechain.is_some_and(|s| !tracks.contains(&s)) {
                    d.sidechain = None;
                }
                if let DeviceKind::Builtin { device } = &mut d.kind {
                    clear_missing_media(device, &ctx.p().media);
                }
                ctx.tx.insert(Entity::Device(d))?;
            }
            Entity::Modulator(mut m) => {
                if m.sidechain.is_some_and(|s| !tracks.contains(&s)) {
                    m.sidechain = None;
                }
                ctx.tx.insert(Entity::Modulator(m))?;
            }
            Entity::Send(s) => {
                if tracks.contains(&s.from) && tracks.contains(&s.to) {
                    sends.insert(s.id);
                    ctx.tx.insert(Entity::Send(s))?;
                }
            }
            Entity::AutomationLane(l) => {
                let owner_in =
                    matches!(l.owner, AutomationOwner::Track { track } if tracks.contains(&track));
                let target_in = match l.target {
                    AutomationTarget::TrackVolume { track }
                    | AutomationTarget::TrackPan { track } => tracks.contains(&track),
                    AutomationTarget::SendLevel { send } => sends.contains(&send),
                    AutomationTarget::DeviceParam { device, .. } => ctx
                        .p()
                        .devices
                        .get(&device)
                        .is_some_and(|d| tracks.contains(&d.track)),
                };
                if owner_in && target_in {
                    lanes.insert(l.id);
                    ctx.tx.insert(Entity::AutomationLane(l))?;
                }
            }
            Entity::AutomationPoint(pt) if lanes.contains(&pt.lane) => {
                ctx.tx.insert(Entity::AutomationPoint(pt))?;
            }
            e @ (Entity::DrumPad(_) | Entity::RackChain(_) | Entity::ModMapping(_)) => {
                ctx.tx.insert(e)?;
            }
            // Not carried by track templates.
            _ => {}
        }
    }
    Ok(())
}
