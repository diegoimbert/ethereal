//! Clip editing (roadmap v2): fade curves, reverse, crossfades (and overlap trimming that
//! keeps them), markers CRUD; each command is one undo step.

mod common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::graph::ClipContentDesc;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::markers::MarkerCommand;
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

const SR: u32 = 44_100;
/// 2.5 s = 5 beats at 120 BPM.
const FRAMES: usize = 110_250;

struct Fx {
    h: Harness,
    track: TrackId,
    media: MediaId,
}

fn fx() -> Fx {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    lib.add_file("lib", "loop.wav", wav(SR, &[sine(SR, 440.0, FRAMES, 0.5)]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Clips");
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "loop.wav".into(),
        },
    }));
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    Fx { h, track, media }
}

impl Fx {
    /// An audio clip at `start` playing content `[offset, offset + length)`.
    fn clip(&mut self, start: f64, length: f64, offset: f64) -> ClipId {
        let id: ClipId = self.h.id();
        self.h.ok(Command::Clip(ClipCommand::CreateAudio {
            id,
            track: self.track,
            start: Beats(start),
            media: self.media,
        }));
        self.h.ok(Command::Clip(ClipCommand::SetBounds {
            id,
            start: Beats(start),
            length: Beats(length),
            offset: Beats(offset),
        }));
        id
    }

    fn get(&self, id: ClipId) -> Clip {
        self.h.project().clips[&id].clone()
    }

    fn audio(&self, id: ClipId) -> AudioContent {
        match self.get(id).content {
            ClipContent::Audio(a) => a,
            ClipContent::Midi => panic!("midi"),
        }
    }

    fn undo(&mut self) {
        self.h.ok(Command::Edit(EditCommand::Undo));
    }

    fn bounds(&self, id: ClipId) -> (f64, f64, f64) {
        let c = self.get(id);
        (c.start.0, c.length.0, c.offset.0)
    }
}

fn close(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6
}

#[test]
fn fade_curves_and_reverse_reach_the_engine_and_undo_in_one_step() {
    let mut f = fx();
    let c = f.clip(0.0, 4.0, 0.0);
    let before = f.h.project().clone();
    f.h.ok(Command::Clip(ClipCommand::SetFadeCurves {
        id: c,
        fade_in: Some(FadeCurve::EqualPower),
        fade_out: Some(FadeCurve::Curve { tension: -0.5 }),
    }));
    // `None` leaves a curve unchanged.
    f.h.ok(Command::Clip(ClipCommand::SetFadeCurves {
        id: c,
        fade_in: None,
        fade_out: Some(FadeCurve::Curve { tension: 0.25 }),
    }));
    f.h.ok(Command::Clip(ClipCommand::SetReversed {
        id: c,
        reversed: true,
    }));
    let a = f.audio(c);
    assert_eq!(a.fade_in_curve, FadeCurve::EqualPower);
    assert_eq!(a.fade_out_curve, FadeCurve::Curve { tension: 0.25 });
    assert!(a.reversed);
    // Reverse doesn't move the clip's bounds (they are on the reversed timeline).
    assert!(close(f.bounds(c), (0.0, 4.0, 0.0)));

    f.h.advance(100);
    f.h.tick();
    let g = f.h.ctl.bridge.last_graph();
    let cd = &g.tracks.iter().find(|t| t.id == f.track).unwrap().clips[0];
    let ClipContentDesc::Audio {
        fade_in_curve,
        fade_out_curve,
        reversed,
        ..
    } = &cd.content
    else {
        panic!()
    };
    assert_eq!(*fade_in_curve, FadeCurve::EqualPower);
    assert_eq!(*fade_out_curve, FadeCurve::Curve { tension: 0.25 });
    assert!(*reversed);

    f.undo();
    assert!(!f.audio(c).reversed);
    f.undo();
    assert_eq!(
        f.audio(c).fade_out_curve,
        FadeCurve::Curve { tension: -0.5 }
    );
    f.undo();
    assert_eq!(f.h.project(), &before);
}

#[test]
fn clip_editing_rejects_bad_input() {
    let mut f = fx();
    let c = f.clip(0.0, 4.0, 0.0);
    let bad = f.h.send(Command::Clip(ClipCommand::SetFadeCurves {
        id: c,
        fade_in: Some(FadeCurve::Curve { tension: 1.5 }),
        fade_out: None,
    }));
    assert_eq!(err(&bad).code, ErrorCode::InvalidArgument);
    let midi_track: TrackId = f.h.id();
    f.h.ok(Command::Track(TrackCommand::Create {
        id: midi_track,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let m: ClipId = f.h.id();
    f.h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: m,
        track: midi_track,
        start: Beats(0.0),
        length: Beats(4.0),
        name: None,
    }));
    let out = f.h.send(Command::Clip(ClipCommand::SetReversed {
        id: m,
        reversed: true,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let missing: ClipId = f.h.id();
    let out = f.h.send(Command::Clip(ClipCommand::SetReversed {
        id: missing,
        reversed: true,
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    assert!(patches(&out).is_empty());
}

#[test]
fn crossfade_extends_both_clips_around_the_boundary() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    let b = f.clip(3.0, 2.0, 1.0);
    let before = f.h.project().clone();
    f.h.ok(Command::Clip(ClipCommand::Crossfade {
        first: a,
        second: b,
        length: Beats(1.0),
        curve: FadeCurve::EqualPower,
    }));
    assert!(close(f.bounds(a), (0.0, 3.5, 0.0)));
    assert!(close(f.bounds(b), (2.5, 2.5, 0.5)));
    let (fa, fb) = (f.audio(a), f.audio(b));
    assert!(fa.fade_out.approx_eq(Beats(1.0)));
    assert!(fb.fade_in.approx_eq(Beats(1.0)));
    assert_eq!(fa.fade_out_curve, FadeCurve::EqualPower);
    assert_eq!(fb.fade_in_curve, FadeCurve::EqualPower);
    // One undo step.
    f.undo();
    assert_eq!(f.h.project(), &before);
}

#[test]
fn crossfade_uses_the_other_side_when_one_runs_out_of_source() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    // B starts at its content start: it can't extend earlier, so A covers the whole overlap.
    let b = f.clip(3.0, 2.0, 0.0);
    f.h.ok(Command::Clip(ClipCommand::Crossfade {
        first: a,
        second: b,
        length: Beats(1.0),
        curve: FadeCurve::Linear,
    }));
    assert!(close(f.bounds(a), (0.0, 4.0, 0.0)));
    assert!(close(f.bounds(b), (3.0, 2.0, 0.0)));
    // A plays the whole file (5 beats) and B its start: A can't extend at all, B can't
    // either: nothing to crossfade.
    let mut g = fx();
    let a = g.clip(0.0, 5.0, 0.0);
    let b = g.clip(5.0, 2.0, 0.0);
    let out = g.h.send(Command::Clip(ClipCommand::Crossfade {
        first: a,
        second: b,
        length: Beats(1.0),
        curve: FadeCurve::Linear,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    assert!(patches(&out).is_empty());
}

#[test]
fn crossfade_of_an_existing_overlap_recentres_it() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    let b = f.clip(3.0, 2.0, 2.0);
    // Crossfade 2 beats: A → 4, B → 2 (offset 1).
    f.h.ok(Command::Clip(ClipCommand::Crossfade {
        first: a,
        second: b,
        length: Beats(2.0),
        curve: FadeCurve::Linear,
    }));
    assert!(close(f.bounds(a), (0.0, 4.0, 0.0)));
    assert!(close(f.bounds(b), (2.0, 3.0, 1.0)));
    // Shorten to 1 beat around the same centre (3).
    f.h.ok(Command::Clip(ClipCommand::Crossfade {
        first: a,
        second: b,
        length: Beats(1.0),
        curve: FadeCurve::Linear,
    }));
    assert!(close(f.bounds(a), (0.0, 3.5, 0.0)));
    assert!(close(f.bounds(b), (2.5, 2.5, 1.5)));
    assert!(f.audio(a).fade_out.approx_eq(Beats(1.0)));
}

#[test]
fn crossfade_rejects_invalid_pairs() {
    let mut f = fx();
    let a = f.clip(0.0, 2.0, 0.0);
    let b = f.clip(3.0, 2.0, 1.0);
    for (first, second, len) in [(a, b, 1.0), (b, a, 1.0), (a, a, 1.0), (a, b, 0.0)] {
        let out = f.h.send(Command::Clip(ClipCommand::Crossfade {
            first,
            second,
            length: Beats(len),
            curve: FadeCurve::Linear,
        }));
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument, "{len}");
    }
}

#[test]
fn overlap_trimming_keeps_crossfades() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    let b = f.clip(3.0, 2.0, 1.0);
    f.h.ok(Command::Clip(ClipCommand::Crossfade {
        first: a,
        second: b,
        length: Beats(1.0),
        curve: FadeCurve::EqualPower,
    }));
    // Re-applying A's bounds (e.g. a no-op drag) keeps B intact.
    f.h.ok(Command::Clip(ClipCommand::SetBounds {
        id: a,
        start: Beats(0.0),
        length: Beats(3.5),
        offset: Beats(0.0),
    }));
    assert!(close(f.bounds(b), (2.5, 2.5, 0.5)));
    // Shrinking the crossfade overlap is still a crossfade.
    f.h.ok(Command::Clip(ClipCommand::SetBounds {
        id: a,
        start: Beats(0.0),
        length: Beats(3.2),
        offset: Beats(0.0),
    }));
    assert!(close(f.bounds(b), (2.5, 2.5, 0.5)));
    // Moving B so the overlap exceeds the fades trims A as before.
    f.h.ok(Command::Clip(ClipCommand::Move {
        moves: vec![ether_core::protocol::clips::ClipMove {
            id: b,
            track: f.track,
            start: Beats(1.5),
        }],
    }));
    assert!(close(f.bounds(a), (0.0, 1.5, 0.0)));
}

#[test]
fn markers_crud_and_undo() {
    let mut h = Harness::with_project();
    let empty = h.project().clone();
    let m: MarkerId = h.id();
    let add = Command::Marker(MarkerCommand::Add {
        id: m,
        position: Beats(8.0),
        name: None,
        color: None,
    });
    h.ok(add.clone());
    let one = h.project().clone();
    // Idempotent.
    let out = h.send(add);
    ok(&out);
    assert!(patches(&out).is_empty());
    assert_eq!(h.project().markers[&m].name, "Marker 1");
    let m2: MarkerId = h.id();
    h.ok(Command::Marker(MarkerCommand::Add {
        id: m2,
        position: Beats(16.0),
        name: Some("Chorus".into()),
        color: Some(Color(0x3366ff)),
    }));
    h.ok(Command::Marker(MarkerCommand::Move {
        id: m,
        position: Beats(4.0),
    }));
    h.ok(Command::Marker(MarkerCommand::Rename {
        id: m,
        name: "Intro".into(),
    }));
    h.ok(Command::Marker(MarkerCommand::SetColor {
        id: m,
        color: Some(Color(0xff0000)),
    }));
    let mk = &h.project().markers[&m];
    assert_eq!(
        (mk.position, mk.name.as_str(), mk.color),
        (Beats(4.0), "Intro", Some(Color(0xff0000)))
    );
    let sorted: Vec<&str> = h
        .project()
        .markers_sorted()
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(sorted, ["Intro", "Chorus"]);
    // Errors.
    for c in [
        MarkerCommand::Move {
            id: m,
            position: Beats(-1.0),
        },
        MarkerCommand::Add {
            id: h.id(),
            position: Beats(f64::NAN),
            name: None,
            color: None,
        },
    ] {
        assert_eq!(
            err(&h.send(Command::Marker(c))).code,
            ErrorCode::InvalidArgument
        );
    }
    let ghost: MarkerId = h.id();
    assert_eq!(
        err(&h.send(Command::Marker(MarkerCommand::Remove { ids: vec![ghost] }))).code,
        ErrorCode::NotFound
    );
    h.ok(Command::Marker(MarkerCommand::Remove { ids: vec![m, m2] }));
    assert!(h.project().markers.is_empty());
    // Every command was one undo step.
    for _ in 0..5 {
        h.ok(Command::Edit(EditCommand::Undo));
    }
    assert_eq!(h.project().markers, one.markers);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &empty);
}
