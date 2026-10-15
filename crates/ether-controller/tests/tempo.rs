//! Tempo map editing, metronome settings and the record count-in click
//! (roadmap v2, `tempo-metronome` node).

mod common;

use common::*;
use ether_core::TransportControl;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::tempo::TempoCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::{Command, ErrorCode};

fn tempo(c: TempoCommand) -> Command {
    Command::Tempo(c)
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

fn redo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Redo));
}

fn first_tempo(h: &Harness) -> TempoPoint {
    h.project()
        .tempo_points
        .values()
        .find(|p| p.time.approx_eq(Beats::ZERO))
        .cloned()
        .expect("tempo point at 0")
}

fn first_signature(h: &Harness) -> TimeSignaturePoint {
    h.project()
        .time_signatures
        .values()
        .find(|p| p.time.approx_eq(Beats::ZERO))
        .cloned()
        .expect("time signature at 0")
}

fn sig(numerator: u8, denominator: u8) -> TimeSignature {
    TimeSignature {
        numerator,
        denominator,
    }
}

fn add_point(h: &mut Harness, time: f64, bpm: f64, curve: TempoCurve) -> TempoPointId {
    let id = h.id();
    h.ok(tempo(TempoCommand::AddTempoPoint {
        id,
        time: Beats(time),
        bpm,
        curve,
    }));
    id
}

fn code(h: &mut Harness, c: Command) -> ErrorCode {
    let before = h.project().clone();
    let out = h.send(c);
    assert!(patches(&out).is_empty());
    assert_eq!(h.project(), &before, "a rejected command changes nothing");
    err(&out).code
}

#[test]
fn add_edit_remove_tempo_points_are_one_undo_step_each() {
    let mut h = Harness::with_project();
    let id = add_point(&mut h, 8.0, 90.0, TempoCurve::Step);
    assert_eq!(h.project().tempo_points[&id].bpm, 90.0);
    // Idempotent on an existing id.
    let out = h.send(tempo(TempoCommand::AddTempoPoint {
        id,
        time: Beats(12.0),
        bpm: 60.0,
        curve: TempoCurve::Step,
    }));
    ok(&out);
    assert!(patches(&out).is_empty());
    assert_eq!(h.project().tempo_points[&id].time, Beats(8.0));

    h.ok(tempo(TempoCommand::EditTempoPoint {
        id,
        time: Some(Beats(6.0)),
        bpm: Some(100.0),
        curve: Some(TempoCurve::Linear),
    }));
    let p = &h.project().tempo_points[&id];
    assert_eq!(
        (p.time, p.bpm, p.curve),
        (Beats(6.0), 100.0, TempoCurve::Linear)
    );
    undo(&mut h);
    let p = &h.project().tempo_points[&id];
    assert_eq!(
        (p.time, p.bpm, p.curve),
        (Beats(8.0), 90.0, TempoCurve::Step)
    );
    redo(&mut h);
    assert_eq!(h.project().tempo_points[&id].bpm, 100.0);

    h.ok(tempo(TempoCommand::RemoveTempoPoints { ids: vec![id] }));
    assert!(!h.project().tempo_points.contains_key(&id));
    undo(&mut h);
    assert!(h.project().tempo_points.contains_key(&id));
    undo(&mut h); // the edit
    undo(&mut h); // the add
    assert!(!h.project().tempo_points.contains_key(&id));
    assert_eq!(h.project().tempo_points.len(), 1);
}

#[test]
fn a_drag_gesture_is_one_undo_step() {
    let mut h = Harness::with_project();
    let id = add_point(&mut h, 4.0, 120.0, TempoCurve::Step);
    let g = GestureId(7);
    for bpm in [121.0, 125.0, 130.0] {
        ok(&h.send_with(
            tempo(TempoCommand::EditTempoPoint {
                id,
                time: Some(Beats(bpm / 30.0)),
                bpm: Some(bpm),
                curve: None,
            }),
            Some(g),
        ));
    }
    assert_eq!(h.project().tempo_points[&id].bpm, 130.0);
    undo(&mut h);
    let p = &h.project().tempo_points[&id];
    assert_eq!((p.time, p.bpm), (Beats(4.0), 120.0));
}

#[test]
fn tempo_points_validate_and_clamp() {
    let mut h = Harness::with_project();
    let zero = first_tempo(&h);
    // The point at 0 can be edited but not moved or removed.
    h.ok(tempo(TempoCommand::EditTempoPoint {
        id: zero.id,
        time: None,
        bpm: Some(5000.0),
        curve: Some(TempoCurve::Linear),
    }));
    assert_eq!(first_tempo(&h).bpm, 999.0);
    assert_eq!(first_tempo(&h).curve, TempoCurve::Linear);
    let e = code(
        &mut h,
        tempo(TempoCommand::EditTempoPoint {
            id: zero.id,
            time: Some(Beats(2.0)),
            bpm: None,
            curve: None,
        }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument);
    let e = code(
        &mut h,
        tempo(TempoCommand::RemoveTempoPoints { ids: vec![zero.id] }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument);

    // Bad times and BPMs.
    for (time, bpm) in [(-1.0, 120.0), (f64::NAN, 120.0), (4.0, f64::INFINITY)] {
        let id = h.id();
        let e = code(
            &mut h,
            tempo(TempoCommand::AddTempoPoint {
                id,
                time: Beats(time),
                bpm,
                curve: TempoCurve::Step,
            }),
        );
        assert_eq!(e, ErrorCode::InvalidArgument, "{time} {bpm}");
    }
    // BPM clamps low.
    let slow = add_point(&mut h, 4.0, 1.0, TempoCurve::Step);
    assert_eq!(h.project().tempo_points[&slow].bpm, 20.0);
    // Two points never share a position (add or move).
    let id = h.id();
    let e = code(
        &mut h,
        tempo(TempoCommand::AddTempoPoint {
            id,
            time: Beats(4.0),
            bpm: 90.0,
            curve: TempoCurve::Step,
        }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument);
    let other = add_point(&mut h, 8.0, 90.0, TempoCurve::Step);
    let e = code(
        &mut h,
        tempo(TempoCommand::EditTempoPoint {
            id: other,
            time: Some(Beats(4.0)),
            bpm: None,
            curve: None,
        }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument);
    let e = code(
        &mut h,
        tempo(TempoCommand::EditTempoPoint {
            id: other,
            time: Some(Beats(0.0)),
            bpm: None,
            curve: None,
        }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument);
    // Unknown ids; a failing id in a remove list removes nothing.
    let missing: TempoPointId = h.id();
    let e = code(
        &mut h,
        tempo(TempoCommand::RemoveTempoPoints {
            ids: vec![other, missing],
        }),
    );
    assert_eq!(e, ErrorCode::NotFound);
    assert!(h.project().tempo_points.contains_key(&other));
}

fn add_sig(h: &mut Harness, time: f64, s: TimeSignature) -> TimeSignatureId {
    let id = h.id();
    h.ok(tempo(TempoCommand::AddTimeSignature {
        id,
        time: Beats(time),
        signature: s,
    }));
    id
}

/// `(bar, beat)` of a position (1-based), from the project's tempo map.
fn bb(h: &Harness, beats: f64) -> (i32, u32) {
    let r = h.project().tempo_map().bar_beat(Beats(beats));
    (r.bar, r.beat)
}

#[test]
fn time_signatures_anywhere_on_the_grid() {
    let mut h = Harness::with_project();
    let zero = first_signature(&h);
    // 4/4 from 0, then 3/4 at beat 2.5 (mid bar 1): bar 1 is a partial bar of 2.5 beats
    // and bar 2 (3/4) starts at the change.
    let three = add_sig(&mut h, 2.5, sig(3, 4));
    assert_eq!(bb(&h, 0.0), (1, 1));
    assert_eq!(bb(&h, 2.0), (1, 3));
    assert_eq!(bb(&h, 2.5), (2, 1));
    assert_eq!(bb(&h, 4.5), (2, 3));
    assert_eq!(bb(&h, 5.5), (3, 1));
    let r = h.project().tempo_map().bar_beat(Beats(3.0));
    assert_eq!((r.bar, r.beat, r.fraction), (2, 1, 0.5));
    // A 7/8 change mid-bar in 3/4 (beat 7 = 1.5 beats into bar 3).
    let seven = add_sig(&mut h, 7.0, sig(7, 8));
    assert_eq!(bb(&h, 6.5), (3, 2));
    assert_eq!(bb(&h, 7.0), (4, 1));
    assert_eq!(bb(&h, 7.5), (4, 2));
    assert_eq!(bb(&h, 10.5), (5, 1)); // 7/8 bar = 3.5 beats
    assert_eq!(h.project().tempo_map().signature_at(Beats(8.0)), sig(7, 8));
    // Still rejected: an occupied position, negative or non-finite times, bad signatures.
    for t in [2.5, -1.0, f64::NAN] {
        let id: TimeSignatureId = h.id();
        let e = code(
            &mut h,
            tempo(TempoCommand::AddTimeSignature {
                id,
                time: Beats(t),
                signature: sig(5, 4),
            }),
        );
        assert_eq!(e, ErrorCode::InvalidArgument, "time {t}");
    }
    for s in [sig(0, 4), sig(4, 3), sig(100, 4)] {
        let e = code(
            &mut h,
            tempo(TempoCommand::EditTimeSignature {
                id: three,
                time: None,
                signature: Some(s),
            }),
        );
        assert_eq!(e, ErrorCode::InvalidArgument);
    }
    let e = code(
        &mut h,
        tempo(TempoCommand::EditTimeSignature {
            id: three,
            time: Some(Beats(7.0)),
            signature: None,
        }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument, "onto another change");
    // Changing an earlier signature no longer requires later changes to stay on its bar
    // lines (it used to be rejected); numbering follows.
    h.ok(tempo(TempoCommand::EditTimeSignature {
        id: zero.id,
        time: None,
        signature: Some(sig(6, 8)),
    }));
    assert_eq!(bb(&h, 2.5), (2, 1));
    undo(&mut h);
    // Move the 3/4 to beat 1.75; later numbering follows.
    h.ok(tempo(TempoCommand::EditTimeSignature {
        id: three,
        time: Some(Beats(1.75)),
        signature: None,
    }));
    assert_eq!(bb(&h, 1.75), (2, 1));
    assert_eq!(bb(&h, 4.75), (3, 1));
    assert_eq!(bb(&h, 7.0), (4, 1)); // 3/4 bar 3 is cut short at 7 by the 7/8
    undo(&mut h);
    assert_eq!(h.project().time_signatures[&three].time, Beats(2.5));
    redo(&mut h);
    assert_eq!(h.project().time_signatures[&three].time, Beats(1.75));
    undo(&mut h);
    // Delete the mid-bar 3/4: the 7/8 at 7 now cuts 4/4 bar 2 short (beat 3 of bar 2).
    h.ok(tempo(TempoCommand::RemoveTimeSignatures {
        ids: vec![three],
    }));
    assert_eq!(bb(&h, 6.5), (2, 3));
    assert_eq!(bb(&h, 7.0), (3, 1));
    undo(&mut h);
    assert_eq!(bb(&h, 7.0), (4, 1));
    // The one at 0: not movable or removable.
    let e = code(
        &mut h,
        tempo(TempoCommand::EditTimeSignature {
            id: zero.id,
            time: Some(Beats(4.0)),
            signature: None,
        }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument);
    let e = code(
        &mut h,
        tempo(TempoCommand::RemoveTimeSignatures { ids: vec![zero.id] }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument);
    h.ok(tempo(TempoCommand::RemoveTimeSignatures {
        ids: vec![three, seven],
    }));
    assert_eq!(h.project().time_signatures.len(), 1);
    undo(&mut h);
    assert_eq!(h.project().time_signatures.len(), 3);
}

#[test]
fn mid_bar_changes_reach_the_engine_bar_grid() {
    let mut h = Harness::with_project();
    add_sig(&mut h, 2.5, sig(7, 8));
    h.tick();
    let g = h.ctl.bridge.last_graph();
    let rt = ether_core::tempo::TempoMapRt::compile(&g.tempo, &g.signatures);
    // The engine's bar grid (metronome downbeats, plugin bar start) restarts at 2.5.
    assert_eq!(rt.signature_at(2.0), (sig(4, 4), 0.0));
    assert_eq!(rt.signature_at(3.0), (sig(7, 8), 2.5));
    assert_eq!(rt.signature_at(6.25).1, 6.0);
    assert_eq!(rt.next_boundary(0.0), Some(2.5));
}

#[test]
fn tempo_map_reaches_the_engine() {
    let mut h = Harness::with_project();
    add_point(&mut h, 4.0, 60.0, TempoCurve::Linear);
    add_point(&mut h, 8.0, 90.0, TempoCurve::Step);
    let id: TimeSignatureId = h.id();
    h.ok(tempo(TempoCommand::AddTimeSignature {
        id,
        time: Beats(8.0),
        signature: sig(7, 8),
    }));
    h.tick();
    let g = h.ctl.bridge.last_graph();
    let pts: Vec<(f64, f64, TempoCurve)> =
        g.tempo.iter().map(|t| (t.beat, t.bpm, t.curve)).collect();
    assert_eq!(
        pts,
        vec![
            (0.0, 120.0, TempoCurve::Step),
            (4.0, 60.0, TempoCurve::Linear),
            (8.0, 90.0, TempoCurve::Step)
        ]
    );
    assert_eq!(g.signatures.len(), 2);
    assert_eq!(g.signatures[1].signature, sig(7, 8));
}

#[test]
fn metronome_settings_are_undoable_and_compiled() {
    let mut h = Harness::with_project();
    let s = h.project().settings.clone();
    assert_eq!(s.metronome_volume, Decibels(-6.0));
    h.ok(tempo(TempoCommand::SetMetronomeSettings {
        volume: Some(Decibels(-12.0)),
        accent: Some(false),
        sound: Some(MetronomeSound::Wood),
    }));
    let s = &h.project().settings;
    assert_eq!(
        (s.metronome_volume, s.metronome_accent, s.metronome_sound),
        (Decibels(-12.0), false, MetronomeSound::Wood)
    );
    h.tick();
    let click = h.ctl.bridge.last_graph().click;
    assert!((click.volume - Decibels(-12.0).to_linear()).abs() < 1e-6);
    assert!(!click.accent);
    assert_eq!(click.sound, MetronomeSound::Wood);
    assert_eq!(click.count_in_end, None);
    // Partial edit + clamp.
    h.ok(tempo(TempoCommand::SetMetronomeSettings {
        volume: Some(Decibels(40.0)),
        accent: None,
        sound: None,
    }));
    assert_eq!(h.project().settings.metronome_volume, Decibels(6.0));
    assert_eq!(h.project().settings.metronome_sound, MetronomeSound::Wood);
    let e = code(
        &mut h,
        tempo(TempoCommand::SetMetronomeSettings {
            volume: Some(Decibels(f32::NAN)),
            accent: None,
            sound: None,
        }),
    );
    assert_eq!(e, ErrorCode::InvalidArgument);
    undo(&mut h);
    assert_eq!(h.project().settings.metronome_volume, Decibels(-12.0));
    undo(&mut h);
    let s = &h.project().settings;
    assert_eq!(
        (s.metronome_volume, s.metronome_accent, s.metronome_sound),
        (Decibels(-6.0), true, MetronomeSound::Classic)
    );
    // The on/off switch stays `Transport::SetMetronome`.
    h.ok(Command::Transport(TransportCommand::SetMetronome {
        enabled: true,
    }));
    h.tick();
    assert!(h.ctl.bridge.last_graph().metronome);
}

#[test]
fn count_in_sets_the_click_end_on_the_published_graph() {
    let mut h = Harness::with_project();
    h.ok(Command::Recording(RecordingCommand::SetCountIn { bars: 1 }));
    h.ok(Command::Transport(TransportCommand::Locate {
        position: Beats(8.0),
    }));
    h.tick();
    assert!(!h.ctl.bridge.last_graph().metronome);
    let publishes = h.ctl.bridge.publishes();
    h.ok(Command::Recording(RecordingCommand::SetRecording {
        enabled: true,
    }));
    // Published before the pre-roll starts, with the record start as the count-in end.
    assert!(h.ctl.bridge.publishes() > publishes);
    assert_eq!(h.ctl.bridge.last_graph().click.count_in_end, Some(8.0));
    let calls = &h.ctl.bridge.calls;
    let publish_at = calls
        .iter()
        .rposition(|c| matches!(c, Call::Publish(_)))
        .unwrap();
    let play_at = calls
        .iter()
        .rposition(|c| matches!(c, Call::Transport(TransportControl::Play)))
        .unwrap();
    assert!(
        publish_at < play_at,
        "the click end is live before playback"
    );
    assert!(calls.contains(&Call::Transport(TransportControl::Locate {
        position: Beats(4.0)
    })));
    // Stopping clears it.
    h.ok(Command::Transport(TransportCommand::Stop));
    h.tick();
    assert_eq!(h.ctl.bridge.last_graph().click.count_in_end, None);
}

#[test]
fn no_count_in_without_pre_roll() {
    let mut h = Harness::with_project();
    h.ok(Command::Recording(RecordingCommand::SetCountIn { bars: 0 }));
    h.ok(Command::Recording(RecordingCommand::SetRecording {
        enabled: true,
    }));
    h.tick();
    assert_eq!(h.ctl.bridge.last_graph().click.count_in_end, None);
    h.ok(Command::Recording(RecordingCommand::SetRecording {
        enabled: false,
    }));
}
