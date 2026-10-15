//! Time-selection edits (v0.2, `time-edits`; CONTRACTS.md §12.3): split across tracks,
//! delete/insert/duplicate time, the time clipboard, global edits, frozen tracks, ids.

mod common;

use common::*;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::markers::MarkerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tempo::TempoCommand;
use ether_core::protocol::time_edit::{TimeEditCommand, TimeSelection};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, ServerMessage};

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn midi_clip(h: &mut Harness, track: TrackId, start: f64, length: f64) -> ClipId {
    let id: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id,
        track,
        start: Beats(start),
        length: Beats(length),
        name: None,
    }));
    let note: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip: id,
        notes: vec![NoteSpec {
            id: note,
            pitch: 60,
            velocity: 0.8,
            start: Beats(0.0),
            duration: Beats(1.0),
        }],
    }));
    id
}

/// A track volume lane with `points` (time, value), linear.
fn volume_lane(h: &mut Harness, track: TrackId, points: &[(f64, f64)]) -> AutomationLaneId {
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Track { track },
        target: AutomationTarget::TrackVolume { track },
    }));
    let points = points
        .iter()
        .map(|&(t, v)| PointSpec {
            id: h.id(),
            time: Beats(t),
            value: v,
            curve: CurveShape::Linear,
        })
        .collect();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane,
        points,
    }));
    lane
}

fn te(h: &mut Harness, c: TimeEditCommand) -> Vec<ServerMessage> {
    h.send(Command::TimeEdit(c))
}

fn te_ok(h: &mut Harness, c: TimeEditCommand) {
    let out = te(h, c);
    ok(&out);
}

fn sel(start: f64, end: f64, tracks: Vec<TrackId>, global: bool) -> TimeSelection {
    TimeSelection {
        start: Beats(start),
        end: Beats(end),
        tracks,
        global,
    }
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

/// Clips of a track as sorted `(start, length, offset)`.
fn spans(h: &Harness, track: TrackId) -> Vec<(f64, f64, f64)> {
    let mut v: Vec<(f64, f64, f64)> = h
        .project()
        .clips
        .values()
        .filter(|c| c.track == track)
        .map(|c| (c.start.0, c.length.0, c.offset.0))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn assert_spans(h: &Harness, track: TrackId, want: &[(f64, f64, f64)]) {
    let got = spans(h, track);
    assert_eq!(got.len(), want.len(), "{got:?} != {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            close(g.0, w.0) && close(g.1, w.1) && close(g.2, w.2),
            "{got:?} != {want:?}"
        );
    }
}

/// Lane value at `t` (right limit), like the engine.
fn value(h: &Harness, lane: AutomationLaneId, t: f64) -> f64 {
    let pts: Vec<(f64, f64, CurveShape)> = h
        .project()
        .points_of(lane)
        .into_iter()
        .map(|p| (p.time.0, p.value, p.curve))
        .collect();
    ether_core::automation::evaluate(&pts, t).unwrap()
}

/// Reopen the project as a copy edited by `f` (for state no command can build yet).
fn reopen_with(h: &mut Harness, f: impl FnOnce(&mut Project)) {
    let mut p = h.project().clone();
    p.id = h.project_id();
    f(&mut p);
    let json = ether_model::file::save(&p, "test").unwrap();
    use ether_controller::store::ProjectStore;
    h.ctl.store.create(p.id).unwrap();
    h.ctl.store.save(p.id, &json).unwrap();
    h.ok(Command::Project(ProjectCommand::Open { id: p.id }));
}

#[test]
fn split_cuts_every_track_at_once_with_derived_ids() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Midi);
    let b = track(&mut h, TrackKind::Midi);
    let ca = midi_clip(&mut h, a, 0.0, 8.0);
    midi_clip(&mut h, b, 2.0, 4.0);
    let seed: ClipId = h.id();
    let out = te(
        &mut h,
        TimeEditCommand::Split {
            tracks: vec![],
            at: Beats(4.0),
            seed,
        },
    );
    ok(&out);
    assert_eq!(patches(&out).len(), 1);
    assert_spans(&h, a, &[(0.0, 4.0, 0.0), (4.0, 4.0, 4.0)]);
    assert_spans(&h, b, &[(2.0, 2.0, 0.0), (4.0, 2.0, 2.0)]);
    // Right parts: derive_id(seed, i) in track order; notes follow.
    let right: ClipId = derive_id(seed, 0);
    assert_eq!(h.project().clips[&right].track, a);
    assert_eq!(h.project().notes_of(right).len(), 1);
    assert_eq!(h.project().clips[&ca].length, Beats(4.0));
    // A replay of the same command is a no-op.
    let out = te(
        &mut h,
        TimeEditCommand::Split {
            tracks: vec![],
            at: Beats(4.0),
            seed,
        },
    );
    ok(&out);
    assert!(patches(&out).is_empty());
    // One undo step.
    undo(&mut h);
    assert_spans(&h, a, &[(0.0, 8.0, 0.0)]);
    assert_spans(&h, b, &[(2.0, 4.0, 0.0)]);
}

#[test]
fn split_only_on_listed_tracks_and_groups_bring_children() {
    let mut h = Harness::with_project();
    let group = track(&mut h, TrackKind::Group);
    let inner = track(&mut h, TrackKind::Midi);
    h.ok(Command::Track(TrackCommand::Move {
        id: inner,
        parent: Some(group),
        before: None,
    }));
    let other = track(&mut h, TrackKind::Midi);
    midi_clip(&mut h, inner, 0.0, 8.0);
    midi_clip(&mut h, other, 0.0, 8.0);
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::Split {
            tracks: vec![group],
            at: Beats(2.0),
            seed,
        },
    );
    assert_eq!(spans(&h, inner).len(), 2);
    assert_eq!(spans(&h, other).len(), 1);
}

#[test]
fn delete_time_removes_and_shifts_left() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let keep = track(&mut h, TrackKind::Midi);
    midi_clip(&mut h, t, 0.0, 6.0);
    midi_clip(&mut h, t, 8.0, 4.0);
    midi_clip(&mut h, keep, 8.0, 4.0);
    let lane = volume_lane(&mut h, t, &[(0.0, 0.0), (12.0, 1.0)]);
    let marker: MarkerId = h.id();
    h.ok(Command::Marker(MarkerCommand::Add {
        id: marker,
        position: Beats(10.0),
        name: None,
        color: None,
    }));
    let seed: ClipId = h.id();
    let out = te(
        &mut h,
        TimeEditCommand::DeleteTime {
            selection: sel(4.0, 9.0, vec![t], false),
            seed,
        },
    );
    ok(&out);
    assert_eq!(patches(&out).len(), 1);
    assert_spans(&h, t, &[(0.0, 4.0, 0.0), (4.0, 3.0, 1.0)]);
    assert_spans(&h, keep, &[(8.0, 4.0, 0.0)]);
    // Automation: unchanged before the cut, the old curve from 9 on continues at 4.
    assert!(close(value(&h, lane, 2.0), 2.0 / 12.0));
    assert!(close(value(&h, lane, 4.0), 9.0 / 12.0));
    assert!(close(value(&h, lane, 5.5), 10.5 / 12.0));
    assert!(close(value(&h, lane, 7.0), 1.0));
    // Not global: markers stay.
    assert_eq!(h.project().markers[&marker].position, Beats(10.0));
    undo(&mut h);
    assert_spans(&h, t, &[(0.0, 6.0, 0.0), (8.0, 4.0, 0.0)]);
    assert!(close(value(&h, lane, 6.0), 0.5));
}

#[test]
fn global_delete_and_insert_move_markers_tempo_and_loop() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    midi_clip(&mut h, t, 8.0, 4.0);
    let (m1, m2): (MarkerId, MarkerId) = (h.id(), h.id());
    for (id, at) in [(m1, 6.0), (m2, 16.0)] {
        h.ok(Command::Marker(MarkerCommand::Add {
            id,
            position: Beats(at),
            name: None,
            color: None,
        }));
    }
    let tp: TempoPointId = h.id();
    h.ok(Command::Tempo(TempoCommand::AddTempoPoint {
        id: tp,
        time: Beats(12.0),
        bpm: 90.0,
        curve: TempoCurve::Step,
    }));
    // Not the whole song: refused.
    let seed: ClipId = h.id();
    let other = track(&mut h, TrackKind::Audio);
    let out = te(
        &mut h,
        TimeEditCommand::DeleteTime {
            selection: sel(4.0, 8.0, vec![t], true),
            seed,
        },
    );
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::DeleteTime {
            selection: sel(4.0, 8.0, vec![t, other], true),
            seed,
        },
    );
    let p = h.project();
    assert!(!p.markers.contains_key(&m1), "inside the range: removed");
    assert_eq!(p.markers[&m2].position, Beats(12.0));
    assert_eq!(p.tempo_points[&tp].time, Beats(8.0));
    assert_spans(&h, t, &[(4.0, 4.0, 0.0)]);
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::InsertSilence {
            tracks: vec![],
            at: Beats(2.0),
            length: Beats(2.0),
            global: true,
            seed,
        },
    );
    let p = h.project();
    assert_eq!(p.markers[&m2].position, Beats(14.0));
    assert_eq!(p.tempo_points[&tp].time, Beats(10.0));
    assert_spans(&h, t, &[(6.0, 4.0, 0.0)]);
    // The tempo point at beat 0 never moves.
    assert!(p.tempo_points.values().any(|p| p.time == Beats(0.0)));
}

#[test]
fn global_delete_keeps_the_tempo_after_the_range() {
    let mut h = Harness::with_project();
    track(&mut h, TrackKind::Midi);
    let tp: TempoPointId = h.id();
    h.ok(Command::Tempo(TempoCommand::AddTempoPoint {
        id: tp,
        time: Beats(6.0),
        bpm: 90.0,
        curve: TempoCurve::Step,
    }));
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::DeleteTime {
            selection: sel(4.0, 8.0, vec![], true),
            seed,
        },
    );
    let map = h.project().tempo_map();
    assert!(close(map.bpm_at(Beats(5.0)), 90.0));
    assert!(close(map.bpm_at(Beats(3.0)), 120.0));
    assert!(!h.project().tempo_points.contains_key(&tp));
}

#[test]
fn insert_silence_splits_and_shifts_right() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    midi_clip(&mut h, t, 0.0, 8.0);
    midi_clip(&mut h, t, 8.0, 2.0);
    let lane = volume_lane(&mut h, t, &[(0.0, 0.0), (8.0, 1.0)]);
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::InsertSilence {
            tracks: vec![t],
            at: Beats(4.0),
            length: Beats(4.0),
            global: false,
            seed,
        },
    );
    assert_spans(&h, t, &[(0.0, 4.0, 0.0), (8.0, 4.0, 4.0), (12.0, 2.0, 0.0)]);
    // The gap holds the value at the insertion point.
    assert!(close(value(&h, lane, 2.0), 0.25));
    assert!(close(value(&h, lane, 6.0), 0.5));
    assert!(close(value(&h, lane, 10.0), 0.75));
    assert!(close(value(&h, lane, 12.0), 1.0));
}

#[test]
fn copy_paste_overwrites_and_inserts() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Midi);
    let b = track(&mut h, TrackKind::Midi);
    let audio = track(&mut h, TrackKind::Audio);
    midi_clip(&mut h, a, 0.0, 4.0);
    midi_clip(&mut h, b, 0.0, 16.0);
    let lane = volume_lane(&mut h, a, &[(0.0, 0.0), (4.0, 1.0)]);
    // Paste before any copy: nothing to paste.
    let seed: ClipId = h.id();
    let out = te(
        &mut h,
        TimeEditCommand::Paste {
            at: Beats(0.0),
            tracks: vec![],
            insert: false,
            seed,
        },
    );
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    // Copy is not an edit.
    let out = te(
        &mut h,
        TimeEditCommand::Copy {
            selection: sel(1.0, 3.0, vec![a], false),
        },
    );
    ok(&out);
    assert!(patches(&out).is_empty());
    // Overwrite onto b at 8: b's clip is cut around the pasted piece.
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::Paste {
            at: Beats(8.0),
            tracks: vec![b],
            insert: false,
            seed,
        },
    );
    assert_spans(
        &h,
        b,
        &[(0.0, 8.0, 0.0), (8.0, 2.0, 1.0), (10.0, 6.0, 10.0)],
    );
    let pasted = h
        .project()
        .clips
        .values()
        .find(|c| c.track == b && c.start == Beats(8.0))
        .unwrap()
        .clone();
    // The piece 1..3 doesn't hold the source note (at 0): only the notes playing in it
    // are copied (section-edit).
    assert_eq!(h.project().notes_of(pasted.id).len(), 0);
    // The volume envelope went to b's volume (a new lane) with the copied ramp.
    let b_lane = h
        .project()
        .automation_lanes
        .values()
        .find(|l| l.target == AutomationTarget::TrackVolume { track: b })
        .unwrap()
        .id;
    assert!(close(value(&h, b_lane, 8.0), 0.25));
    assert!(close(value(&h, b_lane, 9.0), 0.5));
    // Kind mismatch: skipped, so nothing to paste.
    let seed: ClipId = h.id();
    let out = te(
        &mut h,
        TimeEditCommand::Paste {
            at: Beats(0.0),
            tracks: vec![audio],
            insert: false,
            seed,
        },
    );
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // Insert onto the original track at 0: a's material moves right by 2.
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::Paste {
            at: Beats(0.0),
            tracks: vec![],
            insert: true,
            seed,
        },
    );
    assert_spans(&h, a, &[(0.0, 2.0, 1.0), (2.0, 4.0, 0.0)]);
    assert!(close(value(&h, lane, 0.0), 0.25));
    assert!(close(value(&h, lane, 4.0), 0.5));
    // One undo step per paste.
    undo(&mut h);
    assert_spans(&h, a, &[(0.0, 4.0, 0.0)]);
    assert!(close(value(&h, lane, 2.0), 0.5));
}

#[test]
fn cut_is_copy_then_delete_time_and_duplicate_repeats() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    midi_clip(&mut h, t, 0.0, 4.0);
    midi_clip(&mut h, t, 4.0, 4.0);
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::Cut {
            selection: sel(0.0, 4.0, vec![t], false),
            seed,
        },
    );
    assert_spans(&h, t, &[(0.0, 4.0, 0.0)]);
    // The clipboard survives undo (runtime state).
    undo(&mut h);
    assert_eq!(spans(&h, t).len(), 2);
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::Paste {
            at: Beats(8.0),
            tracks: vec![],
            insert: false,
            seed,
        },
    );
    assert_spans(&h, t, &[(0.0, 4.0, 0.0), (4.0, 4.0, 0.0), (8.0, 4.0, 0.0)]);
    undo(&mut h);
    // Duplicate 2..6: the copy lands at 6 (the clip crossing 6 is split), the rest moves
    // right by 4.
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::DuplicateTime {
            selection: sel(2.0, 6.0, vec![t], false),
            seed,
        },
    );
    assert_spans(
        &h,
        t,
        &[
            (0.0, 4.0, 0.0),
            (4.0, 2.0, 0.0),
            (6.0, 2.0, 2.0),
            (8.0, 2.0, 0.0),
            (10.0, 2.0, 2.0),
        ],
    );
    undo(&mut h);
    assert_spans(&h, t, &[(0.0, 4.0, 0.0), (4.0, 4.0, 0.0)]);
}

#[test]
fn take_clips_and_comp_regions_move_with_time() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let other = track(&mut h, TrackKind::Midi);
    let take = midi_clip(&mut h, t, 0.0, 8.0);
    let (lane, r1, r2): (TakeLaneId, CompRegionId, CompRegionId) = (h.id(), h.id(), h.id());
    reopen_with(&mut h, |p| {
        p.take_lanes.insert(
            lane,
            TakeLane {
                id: lane,
                track: t,
                order: OrderKey::between(None, None),
                name: "Take 1".into(),
                color: None,
            },
        );
        p.clips.get_mut(&take).unwrap().lane = Some(lane);
        for (id, s, e) in [(r1, 0.0, 3.0), (r2, 3.0, 8.0)] {
            p.comp_regions.insert(
                id,
                CompRegion {
                    id,
                    track: t,
                    lane,
                    start: Beats(s),
                    end: Beats(e),
                    crossfade: DEFAULT_COMP_CROSSFADE,
                },
            );
        }
    });
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::InsertSilence {
            tracks: vec![t],
            at: Beats(4.0),
            length: Beats(2.0),
            global: false,
            seed,
        },
    );
    let p = h.project();
    assert!(
        p.clips
            .values()
            .filter(|c| c.track == t)
            .all(|c| c.lane == Some(lane))
    );
    assert_spans(&h, t, &[(0.0, 4.0, 0.0), (6.0, 4.0, 4.0)]);
    let mut regions: Vec<(f64, f64)> = p
        .comp_regions
        .values()
        .map(|r| (r.start.0, r.end.0))
        .collect();
    regions.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(regions, vec![(0.0, 3.0), (3.0, 4.0), (6.0, 10.0)]);
    // Delete 2..7: regions cut and shifted, never overlapping on the way.
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::DeleteTime {
            selection: sel(2.0, 7.0, vec![t], false),
            seed,
        },
    );
    let mut regions: Vec<(f64, f64)> = h
        .project()
        .comp_regions
        .values()
        .map(|r| (r.start.0, r.end.0))
        .collect();
    regions.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(regions, vec![(0.0, 2.0), (2.0, 5.0)]);
    // Copy/paste onto another track: take clips and regions don't go (no such lane there).
    te_ok(
        &mut h,
        TimeEditCommand::Copy {
            selection: sel(0.0, 4.0, vec![t], false),
        },
    );
    let seed: ClipId = h.id();
    te_ok(
        &mut h,
        TimeEditCommand::Paste {
            at: Beats(0.0),
            tracks: vec![other],
            insert: false,
            seed,
        },
    );
    assert!(spans(&h, other).is_empty());
}

#[test]
fn frozen_tracks_reject_time_edits() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    midi_clip(&mut h, t, 0.0, 8.0);
    let media: MediaId = h.id();
    reopen_with(&mut h, |p| {
        p.media.insert(
            media,
            MediaRef {
                id: media,
                name: "freeze.wav".into(),
                file: "media/freeze.wav".into(),
                sample_rate: 48_000,
                channels: 2,
                frames: 48_000,
                hash: None,
                location: MediaLocation::default(),
            },
        );
        p.tracks.get_mut(&t).unwrap().freeze = Some(TrackFreeze {
            media,
            start: Seconds(0.0),
        });
    });
    let before = h.project().clone();
    let seed: ClipId = h.id();
    for c in [
        TimeEditCommand::Split {
            tracks: vec![],
            at: Beats(4.0),
            seed,
        },
        TimeEditCommand::DeleteTime {
            selection: sel(0.0, 2.0, vec![t], false),
            seed,
        },
        TimeEditCommand::InsertSilence {
            tracks: vec![t],
            at: Beats(0.0),
            length: Beats(1.0),
            global: false,
            seed,
        },
    ] {
        let out = te(&mut h, c);
        let e = err(&out);
        assert_eq!(e.code, ErrorCode::InvalidState);
        assert!(e.message.contains("frozen"), "{}", e.message);
    }
    assert_eq!(h.project(), &before);
}

#[test]
fn invalid_ranges_are_rejected() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let seed: ClipId = h.id();
    for c in [
        TimeEditCommand::DeleteTime {
            selection: sel(4.0, 4.0, vec![t], false),
            seed,
        },
        TimeEditCommand::InsertSilence {
            tracks: vec![t],
            at: Beats(-1.0),
            length: Beats(1.0),
            global: false,
            seed,
        },
        TimeEditCommand::InsertSilence {
            tracks: vec![t],
            at: Beats(0.0),
            length: Beats(0.0),
            global: false,
            seed,
        },
    ] {
        let out = te(&mut h, c);
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    }
    let missing: TrackId = h.id();
    let out = te(
        &mut h,
        TimeEditCommand::Split {
            tracks: vec![missing],
            at: Beats(1.0),
            seed,
        },
    );
    assert_eq!(err(&out).code, ErrorCode::NotFound);
}
