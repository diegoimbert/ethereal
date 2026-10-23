//! The compiler as it was before the one-pass group-bys, kept verbatim
//! as the reference of the equivalence test: every table is scanned per clip / track /
//! lane. Only the comp clips are inlined (they call the new `clip_desc`).

use ether_core::RenderGraphDesc;
use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, SendDesc, TrackDesc,
};
use ether_core::protocol::model::*;
use ether_core::tempo::{TempoPointDesc, TimeSignatureDesc};

use super::{CompileContext, resolve_output, resolve_target, tracks_in_order};
use crate::doc::clip_start;

fn comp_clips(p: &Project, ctx: &CompileContext, track: TrackId) -> Vec<ClipDesc> {
    crate::comping::comp_pieces(p, track)
        .iter()
        .filter_map(|piece| {
            let clip = p.clips.get(&piece.clip)?;
            let shaped = crate::comping::piece_clip(clip, piece);
            let mut desc = clip_desc(p, ctx, &shaped, shaped.start);
            desc.id = crate::comping::piece_id(piece);
            Some(desc)
        })
        .collect()
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
            crate::groove::swing_notes(&p.settings, start.0, clip.offset.0, &mut notes);
            ClipContentDesc::Midi { notes }
        }
        ClipContent::Audio(a) => {
            let warp = crate::warp::warp_desc(p, clip, a);
            let mut lanes: Vec<&AutomationLane> = p
                .automation_lanes
                .values()
                .filter(|l| matches!(l.owner, AutomationOwner::Clip { clip: c } if c == clip.id))
                .collect();
            lanes.sort_by_key(|l| l.id);
            let (gain_envelope, transpose_envelope) = super::clip_param_envelopes(
                lanes,
                |l| {
                    p.points_of(l)
                        .into_iter()
                        .map(|pt| (pt.time.0, pt.value, pt.curve))
                        .collect()
                },
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
pub(super) fn compile_graph_with(p: &Project, ctx: &CompileContext) -> RenderGraphDesc {
    let tempo_map = p.tempo_map();
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
            let mut clips: Vec<(Beats, &Clip)> = p
                .clips
                .values()
                .filter(|c| c.track == t.id && c.lane.is_none())
                .map(|c| (clip_start(c), c))
                .collect();
            clips.sort_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.1.id.cmp(&b.1.id)));
            let mut clip_descs: Vec<ClipDesc> = clips
                .into_iter()
                .map(|(s, c)| clip_desc(p, ctx, c, s))
                .collect();
            let comp = comp_clips(p, ctx, t.id);
            if !comp.is_empty() {
                clip_descs.extend(comp);
                clip_descs.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.id.cmp(&b.id)));
            }
            // v0.2: a frozen track plays its render instead of its clips and
            // chain.
            let frozen = crate::freeze::frozen_desc(p, t);
            if frozen.is_some() {
                clip_descs.clear();
            }
            TrackDesc {
                id: t.id,
                kind: t.kind,
                chain: p
                    .devices_of(t.id)
                    .into_iter()
                    .filter(|_| frozen.is_none())
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
                racks: if frozen.is_some() {
                    Vec::new()
                } else {
                    crate::drum_rack::racks_desc(p, t.id, ctx)
                },
                // --- v0.2 ---
                chain_racks: if frozen.is_some() {
                    Vec::new()
                } else {
                    crate::racks::chain_racks_desc(p, t.id, ctx)
                },
                modulation: if frozen.is_some() {
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
                hw_io: if frozen.is_some() {
                    Vec::new()
                } else {
                    crate::external::hw_io_descs(p, t.id, ctx.nodes)
                },
                sidechain_taps: if frozen.is_some() {
                    Vec::new()
                } else {
                    crate::sidechain::taps_desc(p, t.id, ctx)
                },
                midi_sidechains: if frozen.is_some() {
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
