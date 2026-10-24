//! Model → [`RenderGraphDesc`]: a pure function of the document plus the engine node table.
//!
//! - Every track (groups, returns, master included) becomes a `TrackDesc` with its output
//!   fully resolved (`TrackOutput::Default` → parent group bus, else master; master → the
//!   hardware), its enclosing group, sends, device chain (devices without an engine node
//!   are skipped), fader state (linear gain), input/monitoring, arrangement clips and
//!   automation.
//! - Automation lanes are placed on the track that owns their *target* (a volume/pan lane
//!   on that track, a send lane on the send's source track, a device lane on the device's
//!   track) with a normalized→plain [`ParamMapping`]: track volume and send level use the
//!   `Fader` law from [`crate::SILENCE_DB`] to [`crate::MAX_VOLUME_DB`] /
//!   [`crate::MAX_SEND_DB`] (plain = dB, as `ether_core::automation::gain_from_plain`
//!   expects), pan is linear -1..=1, device params use the device descriptor. Disabled
//!   lanes, lanes without points and unresolvable targets are skipped.
//! - Clip envelopes of an audio clip's own gain / transposition (`AutomationTarget::
//!   {ClipGain, ClipTranspose}`) don't drive a track target: they go into the clip's
//!   content desc ([`clip_param_envelopes`]), which the audio renderer applies (both
//!   modulate the clip's static value).
//! - Audio clips reference their media by id even if it is not loaded yet (silence until
//!   the source arrives).

use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, ClipParamEnvelope, NoteDesc,
    ParamMapping, RateIntegral, ResolvedTarget, SendDesc, TrackDesc, TransposeEnvelope,
};
use ether_core::protocol::devices::{DeviceDescriptor, ParamInfo, ParamScale, ParamUnit};
use ether_core::protocol::model::*;
use ether_core::tempo::{TempoPointDesc, TimeSignatureDesc};
use ether_core::{NodeKey, RenderGraphDesc};

use std::collections::HashMap;
use std::sync::Arc;

use crate::doc::clip_start;
use crate::{MAX_SEND_DB, MAX_VOLUME_DB, SILENCE_DB};

/// Normalized → plain mapping of track volume automation (plain = dB).
pub const TRACK_VOLUME_MAPPING: ParamMapping = ParamMapping {
    min: SILENCE_DB as f64,
    max: MAX_VOLUME_DB as f64,
    scale: ParamScale::Fader,
    steps: None,
};

/// Normalized → plain mapping of send level automation (plain = dB).
pub const SEND_LEVEL_MAPPING: ParamMapping = ParamMapping {
    min: SILENCE_DB as f64,
    max: MAX_SEND_DB as f64,
    scale: ParamScale::Fader,
    steps: None,
};

/// Normalized → plain mapping of device on/off automation (two steps: off, on).
pub const DEVICE_ENABLED_MAPPING: ParamMapping = ParamMapping {
    min: 0.0,
    max: 1.0,
    scale: ParamScale::Linear,
    steps: Some(2),
};

/// Normalized → plain mapping of a clip gain envelope (plain = dB added to the clip gain;
/// the bottom is silence).
pub const CLIP_GAIN_MAPPING: ParamMapping = ParamMapping {
    min: CLIP_GAIN_MIN_DB,
    max: CLIP_GAIN_MAX_DB,
    scale: ParamScale::Fader,
    steps: None,
};

/// Normalized → plain mapping of a clip transposition envelope (plain = semitones added to
/// the clip transpose; continuous, like Live's).
pub const CLIP_TRANSPOSE_MAPPING: ParamMapping = ParamMapping {
    min: -CLIP_TRANSPOSE_RANGE,
    max: CLIP_TRANSPOSE_RANGE,
    scale: ParamScale::Linear,
    steps: None,
};

/// Normalized → plain mapping of pan automation.
pub const PAN_MAPPING: ParamMapping = ParamMapping {
    min: -1.0,
    max: 1.0,
    scale: ParamScale::Linear,
    steps: None,
};

/// Parameter metadata of the mixer targets of automation (`None` for device params, which
/// come from the device descriptor). The single source of truth for mixer automation:
/// track volume and send level use a `Fader` law over [`SILENCE_DB`]..=[`MAX_VOLUME_DB`]
/// (-144..=+6 dB, plain value in dB; the bottom is silence), pan is linear -1..=1. The UI
/// (mixer faders, automation lanes) uses the same mapping.
pub fn track_param_info(target: &AutomationTarget) -> Option<ParamInfo> {
    let (name, unit, m, default) = match target {
        AutomationTarget::TrackVolume { .. } => {
            ("Volume", ParamUnit::Decibels, TRACK_VOLUME_MAPPING, 0.0)
        }
        AutomationTarget::SendLevel { .. } => (
            "Send",
            ParamUnit::Decibels,
            SEND_LEVEL_MAPPING,
            SILENCE_DB as f64,
        ),
        AutomationTarget::TrackPan { .. } => ("Pan", ParamUnit::Pan, PAN_MAPPING, 0.0),
        AutomationTarget::ClipGain => ("Clip Gain", ParamUnit::Decibels, CLIP_GAIN_MAPPING, 0.0),
        AutomationTarget::ClipTranspose => (
            "Transposition",
            ParamUnit::Semitones,
            CLIP_TRANSPOSE_MAPPING,
            0.0,
        ),
        AutomationTarget::DeviceEnabled { .. } => {
            return Some(ParamInfo {
                step: None,
                remote: None,
                id: ParamId(0),
                name: "Device On".into(),
                group: None,
                unit: ParamUnit::Toggle,
                min: 0.0,
                max: 1.0,
                default: 1.0,
                scale: ParamScale::Linear,
                labels: Some(vec!["Off".into(), "On".into()]),
                automatable: true,
                hidden: false,
            });
        }
        AutomationTarget::DeviceParam { .. } => return None,
    };
    Some(ParamInfo {
        step: None,
        remote: None,
        id: ParamId(0),
        name: name.into(),
        group: None,
        unit,
        min: m.min,
        max: m.max,
        default,
        scale: m.scale,
        labels: None,
        automatable: true,
        hidden: false,
    })
}

/// Everything besides the document that the compiler needs.
pub struct CompileContext<'a> {
    /// Engine node of a device (`None` = not instantiated: skipped from chains).
    pub nodes: &'a dyn Fn(DeviceId) -> Option<NodeKey>,
    /// Descriptor of a device (param mappings for device automation).
    pub descriptors: &'a dyn Fn(&Device) -> Option<DeviceDescriptor>,
    /// Runtime record-arm state.
    pub armed: &'a dyn Fn(TrackId) -> bool,
    pub version: u64,
}

/// Built-in descriptors only (plugins need the host's instance descriptors).
pub fn builtin_descriptors(device: &Device) -> Option<DeviceDescriptor> {
    match &device.kind {
        DeviceKind::Builtin { device } => Some(ether_devices::descriptor(device.device_type())),
        DeviceKind::Plugin { .. } | DeviceKind::Missing { .. } => None,
    }
}

fn mapping_of(desc: &DeviceDescriptor, param: ParamId) -> Option<ParamMapping> {
    let info = desc.params.iter().find(|p| p.id == param)?;
    Some(ParamMapping {
        min: info.min,
        max: info.max,
        scale: info.scale,
        steps: info
            .labels
            .as_ref()
            .filter(|l| l.len() > 1)
            .map(|l| l.len() as u32),
    })
}

/// Tracks in display order (depth-first), so descs are deterministic.
fn tracks_in_order(p: &Project) -> Vec<&Track> {
    fn walk<'a>(p: &'a Project, t: &'a Track, out: &mut Vec<&'a Track>) {
        out.push(t);
        for c in p.child_tracks(t.id) {
            walk(p, c, out);
        }
    }
    let mut out = Vec::with_capacity(p.tracks.len());
    for t in p.tracks_ordered() {
        walk(p, t, &mut out);
    }
    // Orphans (cannot happen in a validated project) still get compiled.
    if out.len() != p.tracks.len() {
        for t in p.tracks.values() {
            if !out.iter().any(|o| o.id == t.id) {
                out.push(t);
            }
        }
    }
    out
}

/// Resolved output bus of a track.
pub fn resolve_output(p: &Project, t: &Track) -> Option<TrackId> {
    match &t.output {
        TrackOutput::None => None,
        TrackOutput::Track { track } => Some(*track),
        TrackOutput::Default if t.kind == TrackKind::Master => None,
        TrackOutput::Default => t.parent.or_else(|| {
            p.tracks
                .values()
                .find(|m| m.kind == TrackKind::Master)
                .map(|m| m.id)
        }),
    }
}

/// Track whose `TrackDesc` hosts automation of `target`, plus the resolved engine target.
fn resolve_target(
    p: &Project,
    ctx: &CompileContext,
    target: &AutomationTarget,
) -> Option<(TrackId, ResolvedTarget, ParamMapping)> {
    match *target {
        AutomationTarget::TrackVolume { track } => p.tracks.contains_key(&track).then_some((
            track,
            ResolvedTarget::TrackVolume,
            TRACK_VOLUME_MAPPING,
        )),
        AutomationTarget::TrackPan { track } => {
            p.tracks
                .contains_key(&track)
                .then_some((track, ResolvedTarget::TrackPan, PAN_MAPPING))
        }
        AutomationTarget::SendLevel { send } => {
            let s = p.sends.get(&send)?;
            Some((s.from, ResolvedTarget::Send { send }, SEND_LEVEL_MAPPING))
        }
        AutomationTarget::DeviceParam { device, param } => {
            let d = p.devices.get(&device)?;
            let node = (ctx.nodes)(device)?;
            let mapping = mapping_of(&(ctx.descriptors)(d)?, param)?;
            Some((d.track, ResolvedTarget::Node { node, param }, mapping))
        }
        AutomationTarget::DeviceEnabled { device } => {
            let d = p.devices.get(&device)?;
            let node = (ctx.nodes)(device)?;
            Some((
                d.track,
                ResolvedTarget::NodeEnabled { node },
                DEVICE_ENABLED_MAPPING,
            ))
        }
        // Applied by the clip's audio renderer ([`clip_param_envelopes`]), not a track target.
        AutomationTarget::ClipGain | AutomationTarget::ClipTranspose => None,
    }
}

/// The gain and transposition envelopes of an audio clip (enabled lanes with points among
/// `lanes`, the clip's), as its content desc carries them. The transposition's
/// [`RateIntegral`] covers the content the clip can play.
pub fn clip_param_envelopes<'a>(
    lanes: impl IntoIterator<Item = &'a AutomationLane>,
    points: impl Fn(AutomationLaneId) -> Vec<(f64, f64, CurveShape)>,
    clip: &Clip,
    transpose: f32,
    warp: Option<&ether_core::graph::WarpDesc>,
) -> (Option<ClipParamEnvelope>, Option<TransposeEnvelope>) {
    let (mut gain, mut tr) = (None, None);
    for l in lanes {
        if !l.enabled || !l.target.is_clip_property() {
            continue;
        }
        let pts = points(l.id);
        if pts.is_empty() {
            continue;
        }
        match l.target {
            AutomationTarget::ClipGain => {
                gain = Some(ClipParamEnvelope {
                    points: pts,
                    mapping: CLIP_GAIN_MAPPING,
                });
            }
            _ => {
                let env = ClipParamEnvelope {
                    points: pts,
                    mapping: CLIP_TRANSPOSE_MAPPING,
                };
                let offset = clip.offset.0;
                let end = if clip.looping.enabled {
                    clip.looping.end.0.max(offset + clip.length.0)
                } else {
                    offset + clip.length.0
                };
                let lo = offset.min(clip.looping.start.0).min(0.0);
                let rate = RateIntegral::build(warp, transpose, &env, offset.max(0.0), lo, end);
                tr = Some(TransposeEnvelope { env, rate });
            }
        }
    }
    (gain, tr)
}

/// One-pass group-bys of the big tables. The compiler used to scan every
/// note per MIDI clip, every lane per clip, every point per lane, every clip, send and
/// device per track: O(clips × notes). These are built once per compile (O(n log n)) and
/// give each parent its children in the order the scans produced.
pub struct Groups<'a> {
    /// Unmuted notes of each clip, sorted like [`Project::notes_of`] (built only when
    /// [`Self::compiled`] doesn't cover the clips).
    notes: IdMap<ClipId, Vec<&'a Note>>,
    /// Compiled notes (before swing) of MIDI clips, from a [`NoteCache`] or compiled now.
    compiled: IdMap<ClipId, Arc<[NoteDesc]>>,
    /// Envelopes of each clip (all lanes owned by the clip), in id order.
    clip_lanes: IdMap<ClipId, Vec<&'a AutomationLane>>,
    /// Points of each lane, sorted like [`Project::points_of`].
    points: IdMap<AutomationLaneId, Vec<&'a AutomationPoint>>,
    /// Main-lane clips of each track, in id order.
    clips: IdMap<TrackId, Vec<&'a Clip>>,
    /// Sends by source track, in id order.
    sends: IdMap<TrackId, Vec<&'a TrackSend>>,
    /// Track-chain devices of each track, sorted like [`Project::devices_of`].
    devices: IdMap<TrackId, Vec<&'a Device>>,
}

/// A map keyed by entity ids (random 128-bit ULIDs, no attacker-chosen keys): hashed with
/// one multiply per word instead of SipHash, the group-bys hash every note and point of the
/// project on each compile.
type IdMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<IdHasher>>;

#[derive(Default)]
struct IdHasher(u64);

impl std::hash::Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(u64::from(*b));
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u128(&mut self, x: u128) {
        self.write_u64(x as u64);
        self.write_u64((x >> 64) as u64);
    }
    fn write_usize(&mut self, x: usize) {
        self.write_u64(x as u64);
    }
}

/// Group `items` by key, keeping their order within each group. Runs of one key (a clip's
/// notes, created together, are mostly adjacent in id order) cost one map lookup.
fn group<'a, K: std::hash::Hash + Eq + Copy, V: 'a>(
    items: impl Iterator<Item = (K, &'a V)>,
) -> IdMap<K, Vec<&'a V>> {
    let mut m: IdMap<K, Vec<&'a V>> = IdMap::default();
    let mut run: Option<(K, Vec<&'a V>)> = None;
    for (k, v) in items {
        match &mut run {
            Some((rk, vs)) if *rk == k => vs.push(v),
            _ => {
                if let Some((rk, vs)) = run.take() {
                    m.entry(rk).or_default().extend(vs);
                }
                run = Some((k, vec![v]));
            }
        }
    }
    if let Some((rk, vs)) = run {
        m.entry(rk).or_default().extend(vs);
    }
    m
}

fn children<'s, 'a, K: std::hash::Hash + Eq, V>(m: &'s IdMap<K, Vec<&'a V>>, k: &K) -> &'s [&'a V] {
    m.get(k).map_or(&[], Vec::as_slice)
}

/// Sort a clip's notes by (start, pitch, id) like [`Project::notes_of`]: on keys copied out
/// next to each reference (`keyed` is scratch), so comparisons don't chase the pointers into
/// the project's note table. `total_cmp` order of `f64` is the order of these integer keys.
fn sort_notes<'a>(v: &mut [&'a Note], keyed: &mut Vec<(i64, u8, NoteId, &'a Note)>) {
    let total = |x: f64| {
        let b = x.to_bits() as i64;
        b ^ ((((b >> 63) as u64) >> 1) as i64)
    };
    keyed.clear();
    keyed.extend(v.iter().map(|n| (total(n.start.0), n.pitch, n.id, *n)));
    keyed.sort_unstable_by_key(|k| (k.0, k.1, k.2));
    for (slot, k) in v.iter_mut().zip(keyed.iter()) {
        *slot = k.3;
    }
}

/// A clip's compiled notes (before swing), for the next compile.
#[derive(Clone)]
struct CachedNotes {
    /// [`Project::notes_revision`] they were compiled at.
    revision: (u64, u64),
    notes: Arc<[NoteDesc]>,
}

/// Compiled notes of each MIDI clip, kept between compiles of the same document
/// ([`compile_graph_cached`]): an edit recompiles only the clips whose notes changed
/// ([`Project::notes_revision`]), instead of grouping and sorting every note of the project.
#[derive(Clone, Default)]
pub struct NoteCache {
    clips: IdMap<ClipId, CachedNotes>,
}

impl std::fmt::Debug for NoteCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NoteCache({} clips)", self.clips.len())
    }
}

impl NoteCache {
    pub fn clear(&mut self) {
        self.clips.clear();
    }

    /// Clips cached (diagnostics, tests).
    pub fn len(&self) -> usize {
        self.clips.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }
}

fn note_descs(notes: &[&Note]) -> Arc<[NoteDesc]> {
    notes
        .iter()
        .map(|n| NoteDesc {
            start: n.start.0,
            duration: n.duration.0,
            key: n.pitch,
            velocity: n.velocity,
            release_velocity: n.release_velocity,
        })
        .collect()
}

fn group_notes(p: &Project) -> IdMap<ClipId, Vec<&Note>> {
    let mut notes = group(p.notes.values().filter(|n| !n.muted).map(|n| (n.clip, n)));
    let mut keyed = Vec::new();
    for v in notes.values_mut() {
        sort_notes(v, &mut keyed);
    }
    notes
}

/// Compile the MIDI clips' notes through `cache`: unchanged clips are reused, the others
/// compiled (one clip from the index, or every note grouped at once when many changed,
/// e.g. the first compile). `cache` keeps exactly the current clips.
fn cached_notes(p: &Project, cache: &mut NoteCache) -> Option<IdMap<ClipId, Arc<[NoteDesc]>>> {
    let midi: Vec<(ClipId, (u64, u64))> = p
        .clips
        .values()
        .filter(|c| matches!(c.content, ClipContent::Midi))
        .map(|c| p.notes_revision(c.id).map(|r| (c.id, r)))
        .collect::<Option<_>>()?;
    let hit = |id: &ClipId, r: (u64, u64)| {
        cache
            .clips
            .get(id)
            .filter(|c| c.revision == r)
            .map(|c| c.notes.clone())
    };
    let misses = midi.iter().filter(|(id, r)| hit(id, *r).is_none()).count();
    // Many to compile: one pass over the note table beats a lookup per note.
    let all = (misses * 8 > midi.len()).then(|| group_notes(p));
    let mut keyed = Vec::new();
    let mut out = IdMap::default();
    let mut next = NoteCache::default();
    for (id, r) in midi {
        let notes = hit(&id, r).unwrap_or_else(|| match &all {
            Some(g) => note_descs(children(g, &id)),
            None => {
                let mut v: Vec<&Note> = p.notes_of(id).into_iter().filter(|n| !n.muted).collect();
                sort_notes(&mut v, &mut keyed);
                note_descs(&v)
            }
        });
        next.clips.insert(
            id,
            CachedNotes {
                revision: r,
                notes: notes.clone(),
            },
        );
        out.insert(id, notes);
    }
    *cache = next;
    Some(out)
}

impl<'a> Groups<'a> {
    pub fn new(p: &'a Project) -> Self {
        Self::with_cache(p, None)
    }

    /// [`Self::new`], the MIDI clips' notes through `cache` when given.
    pub fn with_cache(p: &'a Project, cache: Option<&mut NoteCache>) -> Self {
        let compiled = cache.and_then(|c| cached_notes(p, c));
        let notes = if compiled.is_some() {
            IdMap::default()
        } else {
            group_notes(p)
        };
        let mut points = group(p.automation_points.values().map(|pt| (pt.lane, pt)));
        for v in points.values_mut() {
            v.sort_unstable_by(|a, b| a.time.0.total_cmp(&b.time.0).then(a.id.cmp(&b.id)));
        }
        let mut devices = group(
            p.devices
                .values()
                .filter(|d| d.pad.is_none() && d.chain.is_none())
                .map(|d| (d.track, d)),
        );
        for v in devices.values_mut() {
            v.sort_unstable_by(|a, b| (&a.order, a.id).cmp(&(&b.order, b.id)));
        }
        Groups {
            notes,
            compiled: compiled.unwrap_or_default(),
            clip_lanes: group(p.automation_lanes.values().filter_map(|l| match l.owner {
                AutomationOwner::Clip { clip } => Some((clip, l)),
                AutomationOwner::Track { .. } => None,
            })),
            points,
            clips: group(
                p.clips
                    .values()
                    .filter(|c| c.lane.is_none())
                    .map(|c| (c.track, c)),
            ),
            sends: group(p.sends.values().map(|s| (s.from, s))),
            devices,
        }
    }

    fn lane_points(&self, lane: AutomationLaneId) -> Vec<(f64, f64, CurveShape)> {
        children(&self.points, &lane)
            .iter()
            .map(|pt| (pt.time.0, pt.value, pt.curve))
            .collect()
    }
}

pub fn clip_desc(
    p: &Project,
    g: &Groups,
    ctx: &CompileContext,
    clip: &Clip,
    start: Beats,
) -> ClipDesc {
    let content = match &clip.content {
        ClipContent::Midi => {
            let mut notes: Vec<NoteDesc> = match g.compiled.get(&clip.id) {
                Some(n) => n.to_vec(),
                None => note_descs(children(&g.notes, &clip.id)).to_vec(),
            };
            crate::groove::swing_notes(&p.settings, start.0, clip.offset.0, &mut notes);
            ClipContentDesc::Midi { notes }
        }
        ClipContent::Audio(a) => {
            let warp = crate::warp::warp_desc(p, clip, a);
            let (gain_envelope, transpose_envelope) = clip_param_envelopes(
                children(&g.clip_lanes, &clip.id).iter().copied(),
                |l| g.lane_points(l),
                clip,
                a.transpose,
                warp.as_ref(),
            );
            ClipContentDesc::Audio {
                media: a.media,
                gain: a.gain.to_linear(),
                transpose: a.transpose,
                fade_in: a.fade_in.0,
                fade_out: a.fade_out.0,
                fade_in_curve: a.fade_in_curve,
                fade_out_curve: a.fade_out_curve,
                reversed: a.reversed,
                warp,
                gain_envelope,
                transpose_envelope,
            }
        }
    };
    let envelopes = children(&g.clip_lanes, &clip.id)
        .iter()
        .filter(|l| l.enabled)
        .filter_map(|l| {
            let (track, resolved, mapping) = resolve_target(p, ctx, &l.target)?;
            let points = g.lane_points(l.id);
            (track == clip.track && !points.is_empty()).then_some(AutomationDesc {
                target: l.target,
                resolved,
                points,
                mapping,
            })
        })
        .collect();
    ClipDesc {
        id: clip.id,
        start: start.0,
        length: clip.length.0,
        offset: clip.offset.0,
        looping: clip
            .looping
            .enabled
            .then_some((clip.looping.start.0, clip.looping.end.0)),
        muted: clip.muted,
        content,
        envelopes,
    }
}

/// Ops that cannot change the compiled graph nor the engine's node set:
/// names and colours, arrangement markers, pinned notes, chat and MIDI mappings, none of
/// which the compiler reads. They don't mark the graph dirty, so renaming a track no longer
/// recompiles a large project. Anything not listed here republishes.
pub fn graph_neutral(op: &Op) -> bool {
    use EntityUpdate as U;
    match op {
        Op::Settings { change } => matches!(
            change,
            SettingsChange::Name(_) | SettingsChange::CountInBars(_)
        ),
        Op::Insert { entity } => graph_neutral_table(entity.key()),
        Op::Remove { key } => graph_neutral_table(*key),
        Op::Update { update } => match update {
            U::Track { change, .. } => {
                matches!(change, TrackChange::Name(_) | TrackChange::Color(_))
            }
            U::Clip { change, .. } => matches!(change, ClipChange::Name(_) | ClipChange::Color(_)),
            U::Device { change, .. } => matches!(change, DeviceChange::Name(_)),
            U::DrumPad { change, .. } => {
                matches!(change, DrumPadChange::Name(_) | DrumPadChange::Color(_))
            }
            U::TakeLane { change, .. } => {
                matches!(change, TakeLaneChange::Name(_) | TakeLaneChange::Color(_))
            }
            U::RackChain { change, .. } => {
                matches!(change, RackChainChange::Name(_) | RackChainChange::Color(_))
            }
            U::Modulator { change, .. } => matches!(change, ModulatorChange::Name(_)),
            other => graph_neutral_table(other.key()),
        },
    }
}

/// Tables the compiler never reads.
fn graph_neutral_table(key: EntityKey) -> bool {
    matches!(
        key,
        EntityKey::Marker(_)
            | EntityKey::PinnedNote(_)
            | EntityKey::ChatMessage(_)
            | EntityKey::MidiMapping(_)
    )
}

/// Compile the document (see the module docs).
pub fn compile_graph_with(p: &Project, ctx: &CompileContext) -> RenderGraphDesc {
    compile(p, ctx, None)
}

/// [`compile_graph_with`] reusing the compiled notes of clips that didn't change since the
/// last compile with `cache` (the same document: [`NoteCache`]). Same graph.
pub fn compile_graph_cached(
    p: &Project,
    ctx: &CompileContext,
    cache: &mut NoteCache,
) -> RenderGraphDesc {
    compile(p, ctx, Some(cache))
}

fn compile(p: &Project, ctx: &CompileContext, cache: Option<&mut NoteCache>) -> RenderGraphDesc {
    // v0.2: frozen devices (a frozen track's chain, or the part up to the
    // freeze point of a partial freeze) compile as if they had no node: they are left out of
    // chains, racks, modulation, automation and taps; a partial freeze's live tail stays.
    let frozen_devices = p.frozen_devices();
    let live_nodes = |d: DeviceId| {
        if frozen_devices.contains(&d) {
            None
        } else {
            (ctx.nodes)(d)
        }
    };
    let live_ctx = CompileContext {
        nodes: &live_nodes,
        descriptors: ctx.descriptors,
        armed: ctx.armed,
        version: ctx.version,
    };
    let ctx = &live_ctx;
    let tempo_map = p.tempo_map();
    let g = Groups::with_cache(p, cache);
    let mut tracks: Vec<TrackDesc> = tracks_in_order(p)
        .into_iter()
        // v0.2: VCA tracks carry no audio (`RenderGraphDesc::vcas`, `crate::groups`).
        .filter(|t| t.kind != TrackKind::Vca)
        .map(|t| {
            let armed = (ctx.armed)(t.id);
            let monitor = match t.monitor {
                MonitorMode::In => true,
                MonitorMode::Off => false,
                MonitorMode::Auto => armed,
            };
            // Main-lane clips only: take-lane clips play through comp regions (v0.2,
            // `crate::comping`).
            let mut clips: Vec<(Beats, &Clip)> = children(&g.clips, &t.id)
                .iter()
                .map(|c| (clip_start(c), *c))
                .collect();
            clips.sort_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.1.id.cmp(&b.1.id)));
            let mut clip_descs: Vec<ClipDesc> = clips
                .into_iter()
                .map(|(s, c)| clip_desc(p, &g, ctx, c, s))
                .collect();
            let comp = crate::comping::comp_clips(p, &g, ctx, t.id);
            if !comp.is_empty() {
                clip_descs.extend(comp);
                clip_descs.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.id.cmp(&b.id)));
            }
            // v0.2: a frozen track plays its render instead of its clips and
            // chain (a partial freeze: then its live tail).
            let frozen = crate::freeze::frozen_desc(p, t);
            if frozen.is_some() {
                clip_descs.clear();
            }
            let whole = frozen.is_some()
                && p.frozen_chain_len(t.id) == Some(children(&g.devices, &t.id).len());
            TrackDesc {
                id: t.id,
                kind: t.kind,
                chain: children(&g.devices, &t.id)
                    .iter()
                    .filter(|_| !whole)
                    .filter_map(|d| {
                        (ctx.nodes)(d.id).map(|node| ChainEntry {
                            node,
                            enabled: d.enabled,
                            sidechain: d.sidechain,
                        })
                    })
                    .collect(),
                output: resolve_output(p, t),
                group: t.parent,
                sends: children(&g.sends, &t.id)
                    .iter()
                    .map(|s| SendDesc {
                        id: s.id,
                        to: s.to,
                        level: s.level.to_linear(),
                        pre_fader: s.pre_fader,
                    })
                    .collect(),
                volume: t.mixer.volume.to_linear(),
                pan: t.mixer.pan.0,
                mute: t.mixer.effective_mute(),
                solo: t.mixer.solo,
                audio_input: match t.input {
                    TrackInput::Audio { first, count } => Some((first, count)),
                    _ => None,
                },
                midi_input: match &t.input {
                    TrackInput::Midi { port, channel } => ether_core::recording::MidiInputFilter {
                        port: port.as_deref().map(ether_core::recording::port_key),
                        channel: *channel,
                    },
                    _ => Default::default(),
                },
                monitor,
                armed,
                clips: clip_descs,
                automation: Vec::new(),
                racks: if whole {
                    Vec::new()
                } else {
                    crate::drum_rack::racks_desc(p, t.id, ctx)
                },
                // --- v0.2 ---
                chain_racks: if whole {
                    Vec::new()
                } else {
                    crate::racks::chain_racks_desc(p, t.id, ctx)
                },
                modulation: if whole {
                    Default::default()
                } else {
                    crate::racks::modulation_desc(p, t.id, ctx)
                },
                input_tap: match t.input {
                    TrackInput::Track { track, tap } => {
                        Some(ether_core::InputTapDesc { track, point: tap })
                    }
                    _ => None,
                },
                vca: t.vca,
                // --- v0.3 ---
                expression: if frozen.is_some() {
                    Default::default()
                } else {
                    crate::expression::track_expression(p, t)
                },
                hw_io: if whole {
                    Vec::new()
                } else {
                    crate::external::hw_io_descs(p, t.id, ctx.nodes)
                },
                // --- v0.4 ---
                sidechain_taps: if whole {
                    Vec::new()
                } else {
                    crate::sidechain::taps_desc(p, t.id, ctx)
                },
                midi_sidechains: if whole {
                    Vec::new()
                } else {
                    crate::midi_sidechain::desc(p, t.id, ctx)
                },
                frozen,
            }
        })
        .collect();

    for lane in p.automation_lanes.values() {
        if !lane.enabled || !matches!(lane.owner, AutomationOwner::Track { .. }) {
            continue;
        }
        let Some((track, resolved, mapping)) = resolve_target(p, ctx, &lane.target) else {
            continue;
        };
        let points = g.lane_points(lane.id);
        if points.is_empty() {
            continue;
        }
        if let Some(td) = tracks.iter_mut().find(|td| td.id == track) {
            td.automation.push(AutomationDesc {
                target: lane.target,
                resolved,
                points,
                mapping,
            });
        }
    }

    // v0.2: soloing a VCA solos its tracks.
    crate::freeze::apply_inert(p, &mut tracks);
    crate::groups::fold_vca_solo(p, &mut tracks);
    RenderGraphDesc {
        version: ctx.version,
        tempo: tempo_map
            .tempo
            .iter()
            .map(|t| TempoPointDesc {
                beat: t.time.0,
                bpm: t.bpm,
                curve: t.curve,
            })
            .collect(),
        signatures: tempo_map
            .signatures
            .iter()
            .map(|s| TimeSignatureDesc {
                beat: s.time.0,
                signature: s.signature,
            })
            .collect(),
        loop_enabled: p.settings.loop_enabled,
        loop_start: p.settings.loop_region.start.0,
        loop_end: p.settings.loop_region.end.0,
        metronome: p.settings.metronome,
        click: crate::tempo::metronome_desc(&p.settings),
        tracks,
        vcas: crate::groups::vca_descs(p, ctx),
    }
}

#[cfg(test)]
mod equivalence_tests;
#[cfg(test)]
mod reference;

#[cfg(test)]
mod group_tests {
    use super::*;

    /// The keyed sort orders notes exactly like the comparator it replaces (`total_cmp` on
    /// the start, then pitch, then id), signed zeros and ties included.
    #[test]
    fn keyed_note_sort_matches_the_comparator() {
        let mut ids = IdGen::new(3);
        let clip: ClipId = ids.next(1);
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let starts = [0.0, -0.0, 0.5, -1.25, 3.0, 1e-300, -1e-300, 7.75];
        let notes: Vec<Note> = (0..2_000)
            .map(|_| Note {
                id: ids.next(1),
                clip,
                pitch: (rnd() % 4) as u8,
                velocity: 0.5,
                release_velocity: 0.5,
                start: Beats(if rnd() % 3 == 0 {
                    starts[(rnd() % 8) as usize]
                } else {
                    (rnd() % 1000) as f64 * 0.125 - 40.0
                }),
                duration: Beats(1.0),
                muted: false,
            })
            .collect();
        let mut expect: Vec<&Note> = notes.iter().collect();
        expect.sort_unstable_by(|a, b| {
            a.start
                .0
                .total_cmp(&b.start.0)
                .then(a.pitch.cmp(&b.pitch))
                .then(a.id.cmp(&b.id))
        });
        let mut got: Vec<&Note> = notes.iter().rev().collect();
        sort_notes(&mut got, &mut Vec::new());
        assert!(got.iter().zip(&expect).all(|(a, b)| a.id == b.id));
    }

    /// Grouping keeps each group's items in input order, adjacent or not.
    #[test]
    fn group_keeps_order_within_runs_and_across_them() {
        let vals: Vec<u32> = (0..12).collect();
        let keys = [1u8, 1, 2, 2, 1, 3, 3, 3, 1, 2, 1, 1];
        let m = group(keys.iter().copied().zip(vals.iter()));
        let get = |k: u8| m[&k].iter().map(|v| **v).collect::<Vec<_>>();
        assert_eq!(get(1), [0, 1, 4, 8, 10, 11]);
        assert_eq!(get(2), [2, 3, 9]);
        assert_eq!(get(3), [5, 6, 7]);
    }
}
