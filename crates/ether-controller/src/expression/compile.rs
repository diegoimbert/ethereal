//! Compile hook: a track's expression → `TrackDesc::expression` (`ether_core::expression`).

use std::collections::BTreeMap;

use ether_core::expression::{
    ClipExpressionDesc, CurveDesc, ExpressionLaneDesc, NoteExpressionDesc, TrackExpressionDesc,
};
use ether_core::graph::NoteDesc;
use ether_core::protocol::model::*;

/// The expression of `track`'s main-lane MIDI clips (the clips `compile.rs` plays by id;
/// comp pieces of take lanes play without expression until flattened), plus its MPE
/// settings. Empty curves are skipped.
pub(crate) fn track_expression(project: &Project, track: &Track) -> TrackExpressionDesc {
    let mut desc = TrackExpressionDesc {
        clips: Vec::new(),
        mpe: if track.kind == TrackKind::Midi {
            track.mpe
        } else {
            None
        },
    };
    if track.kind != TrackKind::Midi
        || (project.expression_lanes.is_empty() && project.note_expressions.is_empty())
    {
        return desc;
    }
    let is_ours = |clip: ClipId| {
        project.clips.get(&clip).is_some_and(|c| {
            c.track == track.id && c.lane.is_none() && matches!(c.content, ClipContent::Midi)
        })
    };
    let mut clips: BTreeMap<ClipId, ClipExpressionDesc> = BTreeMap::new();
    let entry = |clips: &mut BTreeMap<ClipId, ClipExpressionDesc>, clip: ClipId| {
        clips.entry(clip).or_insert_with(|| ClipExpressionDesc {
            clip,
            lanes: Vec::new(),
            notes: Vec::new(),
        });
    };
    for l in project.expression_lanes.values() {
        if l.points.is_empty() || !is_ours(l.clip) {
            continue;
        }
        entry(&mut clips, l.clip);
        clips.get_mut(&l.clip).unwrap().lanes.push(ExpressionLaneDesc {
            kind: l.kind,
            points: curve(&l.points),
        });
    }
    // Note expressions, grouped by clip; indices resolved once per clip.
    let mut by_clip: BTreeMap<ClipId, Vec<&NoteExpression>> = BTreeMap::new();
    for e in project.note_expressions.values() {
        if e.points.is_empty() {
            continue;
        }
        let Some(n) = project.notes.get(&e.note) else {
            continue;
        };
        if n.muted || !is_ours(n.clip) {
            continue;
        }
        by_clip.entry(n.clip).or_default().push(e);
    }
    for (clip, exprs) in by_clip {
        let order = compiled_note_order(project, clip);
        let index: BTreeMap<NoteId, u32> = order
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i as u32))
            .collect();
        entry(&mut clips, clip);
        let cx = clips.get_mut(&clip).unwrap();
        for e in exprs {
            if let Some(&note) = index.get(&e.note) {
                cx.notes.push(NoteExpressionDesc {
                    note,
                    kind: e.kind,
                    points: curve(&e.points),
                });
            }
        }
    }
    desc.clips = clips
        .into_values()
        .map(|mut c| {
            c.lanes.sort_by_key(|l| l.kind);
            c.notes.sort_by_key(|n| (n.note, n.kind));
            c
        })
        .filter(|c| !c.lanes.is_empty() || !c.notes.is_empty())
        .collect();
    desc
}

fn curve(points: &[ExpressionPoint]) -> CurveDesc {
    points
        .iter()
        .map(|p| (p.time.0, p.value, p.curve))
        .collect()
}

/// The ids of `clip`'s notes in the order of its compiled `ClipContentDesc::Midi { notes }`
/// (`compile::clip_desc`: unmuted notes sorted by start, then swing applied and re-sorted
/// stably). The swing pass is run on the same descs so the order matches exactly.
pub(crate) fn compiled_note_order(project: &Project, clip: ClipId) -> Vec<NoteId> {
    let Some(c) = project.clips.get(&clip) else {
        return Vec::new();
    };
    let notes: Vec<&Note> = project
        .notes_of(clip)
        .into_iter()
        .filter(|n| !n.muted)
        .collect();
    // `velocity` carries the original index through the (stable) swing sort.
    let mut descs: Vec<NoteDesc> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| NoteDesc {
            start: n.start.0,
            duration: n.duration.0,
            key: n.pitch,
            velocity: i as f32,
            release_velocity: 0.0,
        })
        .collect();
    crate::groove::swing_notes(&project.settings, c.offset.0, &mut descs);
    descs.iter().map(|d| notes[d.velocity as usize].id).collect()
}

/// Value of a model curve at `t` (`ether_core::expression::evaluate`) and the shape to
/// continue with from there (`Curve` segments continue `Linear`: a partial tension
/// segment has no exact `CurveShape`). `None` without points.
pub(crate) fn evaluate_points(points: &[ExpressionPoint], t: f64) -> Option<(f32, CurveShape)> {
    let first = points.first()?;
    let i = points.partition_point(|p| p.time.0 <= t);
    if i == 0 {
        return Some((first.value, first.curve));
    }
    if i >= points.len() {
        return Some((points[points.len() - 1].value, CurveShape::Step));
    }
    let seg = curve(&points[i - 1..=i]);
    let v = ether_core::expression::evaluate(&seg, t)?;
    let shape = match points[i - 1].curve {
        CurveShape::Curve { .. } => CurveShape::Linear,
        s => s,
    };
    Some((v, shape))
}
