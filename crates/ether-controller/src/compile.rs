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
//! - Audio clips reference their media by id even if it is not loaded yet (silence until
//!   the source arrives).

use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, ParamMapping, ResolvedTarget,
    SendDesc, TrackDesc,
};
use ether_core::protocol::devices::{DeviceDescriptor, ParamInfo, ParamScale, ParamUnit};
use ether_core::protocol::model::*;
use ether_core::tempo::{TempoPointDesc, TimeSignatureDesc};
use ether_core::{NodeKey, RenderGraphDesc};

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
        AutomationTarget::DeviceParam { .. } => return None,
    };
    Some(ParamInfo {
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
        DeviceKind::Plugin { .. } => None,
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
pub(crate) fn resolve_output(p: &Project, t: &Track) -> Option<TrackId> {
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
    }
}

fn lane_points(p: &Project, lane: AutomationLaneId) -> Vec<(f64, f64, CurveShape)> {
    p.points_of(lane)
        .into_iter()
        .map(|pt| (pt.time.0, pt.value, pt.curve))
        .collect()
}

fn clip_desc(p: &Project, ctx: &CompileContext, clip: &Clip, start: Beats) -> ClipDesc {
    let content = match &clip.content {
        ClipContent::Midi => {
            let mut notes: Vec<NoteDesc> = p
                .notes_of(clip.id)
                .into_iter()
                .filter(|n| !n.muted)
                .map(|n| NoteDesc {
                    start: n.start.0,
                    duration: n.duration.0,
                    key: n.pitch,
                    velocity: n.velocity,
                    release_velocity: n.release_velocity,
                })
                .collect();
            crate::groove::swing_notes(&p.settings, clip.offset.0, &mut notes);
            ClipContentDesc::Midi { notes }
        }
        ClipContent::Audio(a) => ClipContentDesc::Audio {
            media: a.media,
            gain: a.gain.to_linear(),
            transpose: a.transpose,
            fade_in: a.fade_in.0,
            fade_out: a.fade_out.0,
            fade_in_curve: a.fade_in_curve,
            fade_out_curve: a.fade_out_curve,
            reversed: a.reversed,
            warp: crate::warp::warp_desc(p, clip, a),
        },
    };
    let envelopes = p
        .automation_lanes
        .values()
        .filter(|l| {
            l.enabled && matches!(l.owner, AutomationOwner::Clip { clip: c } if c == clip.id)
        })
        .filter_map(|l| {
            let (track, resolved, mapping) = resolve_target(p, ctx, &l.target)?;
            let points = lane_points(p, l.id);
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

/// Compile the document (see the module docs).
pub fn compile_graph_with(p: &Project, ctx: &CompileContext) -> RenderGraphDesc {
    let tempo_map = p.tempo_map();
    let mut tracks: Vec<TrackDesc> = tracks_in_order(p)
        .into_iter()
        .map(|t| {
            let armed = (ctx.armed)(t.id);
            let monitor = match t.monitor {
                MonitorMode::In => true,
                MonitorMode::Off => false,
                MonitorMode::Auto => armed,
            };
            let mut clips: Vec<(Beats, &Clip)> = p
                .clips
                .values()
                .filter(|c| c.track == t.id)
                .map(|c| (clip_start(c), c))
                .collect();
            clips.sort_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.1.id.cmp(&b.1.id)));
            TrackDesc {
                id: t.id,
                kind: t.kind,
                chain: p
                    .devices_of(t.id)
                    .into_iter()
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
                sends: p
                    .sends
                    .values()
                    .filter(|s| s.from == t.id)
                    .map(|s| SendDesc {
                        id: s.id,
                        to: s.to,
                        level: s.level.to_linear(),
                        pre_fader: s.pre_fader,
                    })
                    .collect(),
                volume: t.mixer.volume.to_linear(),
                pan: t.mixer.pan.0,
                mute: t.mixer.mute,
                solo: t.mixer.solo,
                audio_input: match t.input {
                    TrackInput::Audio { first, count } => Some((first, count)),
                    _ => None,
                },
                monitor,
                armed,
                clips: clips
                    .into_iter()
                    .map(|(s, c)| clip_desc(p, ctx, c, s))
                    .collect(),
                automation: Vec::new(),
                racks: crate::drum_rack::racks_desc(p, t.id, ctx),
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
        let points = lane_points(p, lane.id);
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
    }
}
