//! `ClipCommand::Stretch` (ctrl+shift edge drag): new bounds, content time-scaled to fill
//! them. MIDI: notes, expression lanes, per-note expressions, loop, offset. Audio: warp
//! markers (an unwarped clip gets two pinning the audio it played), fades, envelopes.
//! One undo step; overlaps resolve like a resize.

use crate::common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::expression::ExpressionCommand;
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::warp::WarpCommand;
use ether_core::protocol::*;

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

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
    id
}

fn stretch(h: &mut Harness, id: ClipId, start: f64, length: f64) {
    h.ok(Command::Clip(ClipCommand::Stretch {
        id,
        start: Beats(start),
        length: Beats(length),
    }));
}

fn pt(time: f64, value: f32) -> ExpressionPoint {
    ExpressionPoint {
        time: Beats(time),
        value,
        curve: CurveShape::Linear,
    }
}

#[test]
fn midi_content_scales_with_the_clip_and_undoes_in_one_step() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let c = midi_clip(&mut h, t, 4.0, 4.0);
    let n: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip: c,
        notes: vec![NoteSpec {
            id: n,
            pitch: 60,
            velocity: 0.8,
            start: Beats(1.0),
            duration: Beats(0.5),
        }],
    }));
    let lane: ExpressionLaneId = h.id();
    h.ok(Command::Expression(ExpressionCommand::CreateLane {
        id: lane,
        clip: c,
        kind: ExpressionKind::Cc { controller: 1 },
    }));
    h.ok(Command::Expression(ExpressionCommand::SetPoints {
        lane,
        points: vec![pt(0.0, 0.1), pt(3.0, 0.9)],
    }));
    let e: NoteExpressionId = h.id();
    h.ok(Command::Expression(ExpressionCommand::SetNoteExpression {
        id: e,
        note: n,
        kind: NoteExpressionKind::Pressure,
        points: vec![pt(0.0, 0.2), pt(0.5, 0.8)],
    }));
    h.ok(Command::Clip(ClipCommand::SetLoop {
        id: c,
        looping: ClipLoop {
            enabled: true,
            start: Beats(0.0),
            end: Beats(2.0),
        },
    }));
    let before = h.project().clone();

    // The end edge dragged from 8 to 12: twice as long, twice as slow.
    stretch(&mut h, c, 4.0, 8.0);
    let p = h.project();
    let cl = &p.clips[&c];
    near(cl.start.0, 4.0);
    near(cl.length.0, 8.0);
    near(cl.looping.end.0, 4.0);
    near(p.notes[&n].start.0, 2.0);
    near(p.notes[&n].duration.0, 1.0);
    let times = |pts: &[ExpressionPoint]| pts.iter().map(|p| p.time.0).collect::<Vec<_>>();
    assert_eq!(times(&p.expression_lanes[&lane].points), vec![0.0, 6.0]);
    assert_eq!(times(&p.note_expressions[&e].points), vec![0.0, 1.0]);

    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &before);

    // The start edge dragged from 4 to 6 (end stays at 8): twice as fast.
    stretch(&mut h, c, 6.0, 2.0);
    let p = h.project();
    near(p.clips[&c].start.0, 6.0);
    near(p.notes[&n].start.0, 0.5);
    near(p.notes[&n].duration.0, 0.25);
}

#[test]
fn a_longer_clip_trims_the_next_one_and_bad_lengths_are_refused() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let a = midi_clip(&mut h, t, 0.0, 4.0);
    let b = midi_clip(&mut h, t, 4.0, 4.0);
    stretch(&mut h, a, 0.0, 6.0);
    let p = h.project();
    near(p.clips[&b].start.0, 6.0);
    near(p.clips[&b].length.0, 2.0);
    for length in [0.0, -1.0, f64::NAN] {
        let out = h.send(Command::Clip(ClipCommand::Stretch {
            id: a,
            start: Beats(0.0),
            length: Beats(length),
        }));
        assert_eq!(
            err(&out).code,
            ErrorCode::InvalidArgument,
            "length {length}"
        );
    }
}

const SR: u32 = 44_100;
/// 4 s = 8 beats at 120 BPM.
const FRAMES: usize = 176_400;

fn audio_fx() -> (Harness, TrackId, MediaId) {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    lib.add_file("lib", "loop.wav", wav(SR, &[sine(SR, 440.0, FRAMES, 0.5)]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Stretch");
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "loop.wav".into(),
        },
    }));
    let t = track(&mut h, TrackKind::Audio);
    (h, t, media)
}

fn audio_clip(h: &mut Harness, track: TrackId, media: MediaId, start: f64) -> ClipId {
    let id: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateAudio {
        id,
        track,
        start: Beats(start),
        media,
    }));
    id
}

/// Source seconds at content beat `c` through the clip's (two or more) markers.
fn source_at(h: &Harness, clip: ClipId, c: f64) -> f64 {
    let m = h.project().warp_markers_of(clip);
    assert!(m.len() >= 2, "markers: {m:?}");
    let (a, b) = (m[0], m[m.len() - 1]);
    a.source.0 + (c - a.beat.0) * (b.source.0 - a.source.0) / (b.beat.0 - a.beat.0)
}

#[test]
fn an_unwarped_audio_clip_gets_warped_and_keeps_its_audio() {
    let (mut h, t, media) = audio_fx();
    let c = audio_clip(&mut h, t, media, 0.0);
    h.ok(Command::Clip(ClipCommand::SetBounds {
        id: c,
        start: Beats(0.0),
        length: Beats(4.0),
        offset: Beats(2.0),
    }));
    h.ok(Command::Clip(ClipCommand::SetFades {
        id: c,
        fade_in: Beats(1.0),
        fade_out: Beats(0.5),
    }));
    let env: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: env,
        owner: AutomationOwner::Clip { clip: c },
        target: AutomationTarget::ClipGain,
    }));
    let ep = h.id();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane: env,
        points: vec![PointSpec {
            id: ep,
            time: Beats(2.0),
            value: 0.5,
            curve: CurveShape::Linear,
        }],
    }));
    let mode = match &h.project().clips[&c].content {
        ClipContent::Audio(a) => a.warp.mode,
        ClipContent::Midi => unreachable!(),
    };
    // Unwarped at 120 BPM: content 2..6 plays source 1 s..3 s.
    stretch(&mut h, c, 0.0, 8.0);
    let cl = h.project().clips[&c].clone();
    let ClipContent::Audio(a) = &cl.content else {
        unreachable!()
    };
    assert!(a.warp.enabled);
    assert_eq!(a.warp.mode, mode);
    near(cl.offset.0, 4.0);
    near(source_at(&h, c, cl.offset.0), 1.0);
    near(source_at(&h, c, cl.offset.0 + cl.length.0), 3.0);
    near(a.fade_in.0, 2.0);
    near(a.fade_out.0, 1.0);
    near(h.project().automation_points[&ep].time.0, 4.0);
}

#[test]
fn a_warped_clip_scales_its_markers() {
    let (mut h, t, media) = audio_fx();
    let c = audio_clip(&mut h, t, media, 0.0);
    h.ok(Command::Warp(WarpCommand::SetWarp {
        clip: c,
        warp: WarpSettings {
            enabled: true,
            mode: WarpMode::Complex,
            source_bpm: Some(120.0),
            formants: Default::default(),
        },
    }));
    let before: Vec<(f64, f64)> = h
        .project()
        .warp_markers_of(c)
        .iter()
        .map(|m| (m.beat.0, m.source.0))
        .collect();
    let len = h.project().clips[&c].length.0;
    stretch(&mut h, c, 0.0, len / 2.0);
    let after: Vec<(f64, f64)> = h
        .project()
        .warp_markers_of(c)
        .iter()
        .map(|m| (m.beat.0, m.source.0))
        .collect();
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(&after) {
        near(a.0, b.0 / 2.0);
        near(a.1, b.1);
    }
}
