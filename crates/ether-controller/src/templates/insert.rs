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
//!   (`before: None` at the top level: like `Track::Create`, after the last track of
//!   the same rank: regular tracks after the regular tracks, returns after the returns).
//! - Idempotent: if `derive_id(seed, 0)` is already a track, nothing changes.

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::model::*;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, internal, invalid};

/// A loaded track template, ready to insert.
#[derive(Clone, Debug)]
pub struct Prepared {
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
pub fn remap(prepared: &Prepared, seed: TrackId) -> CmdResult<Vec<Entity>> {
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

/// Which run of keys a top-level template track takes: all of them before `before`, else
/// one run per display rank (regular tracks, returns), as `Track::Create` places them.
fn key_rank(kind: TrackKind, before: Option<TrackId>) -> u8 {
    match (before, kind) {
        (None, TrackKind::Return) => 1,
        _ => 0,
    }
}

/// The order keys of the template's top-level tracks, in entity order per run (see
/// [`key_rank`]): each run is spread over one gap of the project, worked out once (placing
/// the tracks one by one sorted the project's tracks once per track, and each key went
/// between the last one and its neighbour).
fn top_level_keys(
    p: &Project,
    entities: &[Entity],
    tracks: &BTreeSet<TrackId>,
    parent: Option<TrackId>,
    before: Option<TrackId>,
) -> CmdResult<BTreeMap<u8, std::vec::IntoIter<OrderKey>>> {
    let mut counts: BTreeMap<u8, (TrackKind, usize)> = BTreeMap::new();
    for e in entities {
        if let Entity::Track(t) = e
            && t.parent.is_none_or(|g| !tracks.contains(&g))
        {
            counts
                .entry(key_rank(t.kind, before))
                .or_insert((t.kind, 0))
                .1 += 1;
        }
    }
    let mut runs: Vec<(u8, usize, Gap)> = Vec::new();
    for (rank, (kind, n)) in counts {
        // `before: None`: as `Track::Create` (after the last track of its rank).
        let gap = match before {
            Some(b) => {
                let sibs = siblings(p, parent);
                let i = sibs
                    .iter()
                    .position(|s| s.1 == b)
                    .ok_or_else(|| invalid(format!("{b} is not a sibling")))?;
                (
                    i.checked_sub(1).map(|j| sibs[j].0.clone()),
                    Some(sibs[i].0.clone()),
                )
            }
            None => crate::doc::end_gap(p, kind, parent, None),
        };
        runs.push((rank, n, gap));
    }
    // Runs that land in the same gap (no regular track and no return yet: both go before
    // master) share it, in rank order, as placing them one by one did.
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < runs.len() {
        let gap = runs[i].2.clone();
        let same = runs[i..].iter().take_while(|r| r.2 == gap).count();
        let total: usize = runs[i..i + same].iter().map(|r| r.1).sum();
        let (lo, hi) = (gap.0.as_ref(), gap.1.as_ref());
        // Checked first: `n_between` panics on malformed keys.
        OrderKey::try_between(lo, hi).map_err(invalid)?;
        let mut keys = OrderKey::n_between(lo, hi, total).into_iter();
        for (rank, n, _) in &runs[i..i + same] {
            let run: Vec<OrderKey> = keys.by_ref().take(*n).collect();
            out.insert(*rank, run.into_iter());
        }
        i += same;
    }
    Ok(out)
}

/// The keys a run of tracks goes between: `(after, before)`, `None` = open end.
type Gap = (Option<OrderKey>, Option<OrderKey>);

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
pub fn insert(
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
    let mut keys = top_level_keys(ctx.p(), &entities, &tracks, parent, before)?;
    // The template's devices and their tracks (lanes on them are kept).
    let device_tracks: BTreeMap<DeviceId, TrackId> = entities
        .iter()
        .filter_map(|e| match e {
            Entity::Device(d) => Some((d.id, d.track)),
            _ => None,
        })
        .collect();
    // Applied together at the end: the document-wide checks run once, not once per entity.
    let mut ops: Vec<Op> = Vec::with_capacity(entities.len());
    let mut add = |e: Entity| ops.push(Op::Insert { entity: e });
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
                    t.order = keys
                        .get_mut(&key_rank(t.kind, before))
                        .and_then(|k| k.next())
                        .ok_or_else(|| internal("template track without an order key"))?;
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
                add(Entity::Track(t));
            }
            Entity::Device(mut d) => {
                if d.sidechain.is_some_and(|s| !tracks.contains(&s)) {
                    d.sidechain = None;
                }
                if d.midi_sidechain.is_some_and(|m| !tracks.contains(&m.track)) {
                    d.midi_sidechain = None;
                }
                if let DeviceKind::Builtin { device } = &mut d.kind {
                    clear_missing_media(device, &ctx.p().media);
                }
                add(Entity::Device(d));
            }
            Entity::Modulator(mut m) => {
                if m.sidechain.is_some_and(|s| !tracks.contains(&s)) {
                    m.sidechain = None;
                }
                add(Entity::Modulator(m));
            }
            Entity::Send(s) => {
                if tracks.contains(&s.from) && tracks.contains(&s.to) {
                    sends.insert(s.id);
                    add(Entity::Send(s));
                }
            }
            Entity::AutomationLane(l) => {
                let owner_in =
                    matches!(l.owner, AutomationOwner::Track { track } if tracks.contains(&track));
                let target_in = match l.target {
                    AutomationTarget::TrackVolume { track }
                    | AutomationTarget::TrackPan { track } => tracks.contains(&track),
                    AutomationTarget::SendLevel { send } => sends.contains(&send),
                    AutomationTarget::DeviceParam { device, .. }
                    | AutomationTarget::DeviceEnabled { device } => device_tracks
                        .get(&device)
                        .is_some_and(|t| tracks.contains(t)),
                    AutomationTarget::ClipGain | AutomationTarget::ClipTranspose => false,
                };
                if owner_in && target_in {
                    lanes.insert(l.id);
                    add(Entity::AutomationLane(l));
                }
            }
            Entity::AutomationPoint(pt) if lanes.contains(&pt.lane) => {
                add(Entity::AutomationPoint(pt));
            }
            e @ (Entity::DrumPad(_) | Entity::RackChain(_) | Entity::ModMapping(_)) => {
                add(e);
            }
            // Not carried by track templates.
            _ => {}
        }
    }
    ctx.tx.apply_all(ops)
}
