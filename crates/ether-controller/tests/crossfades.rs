//! `crossfades` node: `ClipCommand::SetCrossfade` (explicit centre, length 0 removes it,
//! `curve: None` keeps the curves), one undo step per gesture, and the engine graph.

mod common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::graph::ClipContentDesc;
use ether_core::protocol::clips::ClipCommand;
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
    h.create_project("Crossfades");
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

    fn bounds(&self, id: ClipId) -> (f64, f64, f64) {
        let c = &self.h.project().clips[&id];
        (c.start.0, c.length.0, c.offset.0)
    }

    fn audio(&self, id: ClipId) -> AudioContent {
        match &self.h.project().clips[&id].content {
            ClipContent::Audio(a) => a.clone(),
            ClipContent::Midi => panic!("midi"),
        }
    }

    fn set(&mut self, first: ClipId, second: ClipId, at: f64, length: f64) -> Vec<ServerMessage> {
        self.set_in(first, second, at, length, None, None)
    }

    fn set_in(
        &mut self,
        first: ClipId,
        second: ClipId,
        at: f64,
        length: f64,
        curve: Option<FadeCurve>,
        gesture: Option<GestureId>,
    ) -> Vec<ServerMessage> {
        self.h.send_with(
            Command::Clip(ClipCommand::SetCrossfade {
                first,
                second,
                at: Beats(at),
                length: Beats(length),
                curve,
            }),
            gesture,
        )
    }
}

fn close(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6
}

#[test]
fn set_crossfade_creates_adjusts_and_removes_around_a_fixed_boundary() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    let b = f.clip(3.0, 2.0, 1.0);
    let before = f.h.project().clone();
    ok(&f.set_in(a, b, 3.0, 1.0, Some(FadeCurve::EqualPower), None));
    assert!(close(f.bounds(a), (0.0, 3.5, 0.0)));
    assert!(close(f.bounds(b), (2.5, 2.5, 0.5)));
    assert!(f.audio(a).fade_out.approx_eq(Beats(1.0)));
    assert!(f.audio(b).fade_in.approx_eq(Beats(1.0)));
    assert_eq!(f.audio(b).fade_in_curve, FadeCurve::EqualPower);
    // Longer, then shorter: always centred on 3 (no drift), the curve kept (`None`).
    ok(&f.set(a, b, 3.0, 1.6));
    assert!(close(f.bounds(a), (0.0, 3.8, 0.0)));
    assert!(close(f.bounds(b), (2.2, 2.8, 0.2)));
    ok(&f.set(a, b, 3.0, 0.4));
    assert!(close(f.bounds(a), (0.0, 3.2, 0.0)));
    assert!(close(f.bounds(b), (2.8, 2.2, 0.8)));
    assert_eq!(f.audio(a).fade_out_curve, FadeCurve::EqualPower);
    // Length 0 removes it: the clips meet at the boundary again, fades reset.
    ok(&f.set(a, b, 3.0, 0.0));
    assert!(close(f.bounds(a), (0.0, 3.0, 0.0)));
    assert!(close(f.bounds(b), (3.0, 2.0, 1.0)));
    assert_eq!(f.audio(a).fade_out, Beats(0.0));
    assert_eq!(f.audio(b).fade_in, Beats(0.0));
    let mut expect = before.clone();
    // Only the curves (set explicitly above) differ from the original document.
    for id in [a, b] {
        let ClipContent::Audio(x) = &mut expect.clips.get_mut(&id).unwrap().content else {
            unreachable!()
        };
        let y = f.audio(id);
        x.fade_in_curve = y.fade_in_curve;
        x.fade_out_curve = y.fade_out_curve;
    }
    assert_eq!(f.h.project().clips, expect.clips);
}

#[test]
fn a_drag_gesture_is_one_undo_step() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    let b = f.clip(3.0, 2.0, 1.0);
    let before = f.h.project().clone();
    let g = GestureId(41);
    for len in [0.2, 0.6, 1.2, 0.8] {
        ok(&f.set_in(a, b, 3.0, len, Some(FadeCurve::EqualPower), Some(g)));
    }
    f.h.ok(Command::Edit(EditCommand::EndGesture { gesture: g }));
    assert!(close(f.bounds(a), (0.0, 3.4, 0.0)));
    f.h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(f.h.project(), &before);
    // A drag back to zero within one gesture leaves the document as it was.
    let g = GestureId(42);
    ok(&f.set_in(a, b, 3.0, 1.0, Some(FadeCurve::EqualPower), Some(g)));
    ok(&f.set_in(a, b, 3.0, 0.0, None, Some(g)));
    f.h.ok(Command::Edit(EditCommand::EndGesture { gesture: g }));
    assert!(close(f.bounds(a), (0.0, 3.0, 0.0)));
    assert!(close(f.bounds(b), (3.0, 2.0, 1.0)));
}

#[test]
fn source_limits_shift_the_overlap_but_keep_the_requested_length() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    // B plays from its content start: no material before it, A covers the whole overlap.
    let b = f.clip(3.0, 2.0, 0.0);
    ok(&f.set(a, b, 3.0, 1.0));
    assert!(close(f.bounds(a), (0.0, 4.0, 0.0)));
    assert!(close(f.bounds(b), (3.0, 2.0, 0.0)));
    assert!(f.audio(a).fade_out.approx_eq(Beats(1.0)));
    // Removing at the original boundary restores the clips.
    ok(&f.set(a, b, 3.0, 0.0));
    assert!(close(f.bounds(a), (0.0, 3.0, 0.0)));
    // Neither side has material left: InvalidState, nothing changes.
    let mut g = fx();
    let a = g.clip(0.0, 5.0, 0.0);
    let b = g.clip(5.0, 2.0, 0.0);
    let out = g.set(a, b, 5.0, 1.0);
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    assert!(patches(&out).is_empty());
}

#[test]
fn set_crossfade_validates_its_arguments() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    let b = f.clip(3.0, 2.0, 1.0);
    let far = f.clip(8.0, 1.0, 1.0);
    for (first, second, at, len) in [
        (a, b, 3.0, -1.0),
        (a, b, f64::NAN, 1.0),
        (a, b, 0.0, 1.0),   // centre at A's start
        (a, b, 5.0, 1.0),   // centre at B's end
        (b, a, 3.0, 1.0),   // wrong order
        (a, a, 1.0, 1.0),   // same clip
        (b, far, 6.0, 1.0), // a gap between them
    ] {
        let out = f.set(first, second, at, len);
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument, "{at} {len}");
    }
}

#[test]
fn crossfaded_clips_reach_the_engine_overlapping_with_matching_fades() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    let b = f.clip(3.0, 2.0, 1.0);
    ok(&f.set_in(a, b, 3.0, 1.0, Some(FadeCurve::EqualPower), None));
    f.h.advance(100);
    f.h.tick();
    let g = f.h.ctl.bridge.last_graph();
    let clips: Vec<_> = g.tracks.iter().flat_map(|t| t.clips.iter()).collect();
    assert_eq!(clips.len(), 2);
    let (ca, cb) = if clips[0].start < clips[1].start {
        (clips[0], clips[1])
    } else {
        (clips[1], clips[0])
    };
    assert!((ca.start + ca.length - 3.5).abs() < 1e-9);
    assert!((cb.start - 2.5).abs() < 1e-9);
    let (
        ClipContentDesc::Audio {
            fade_out,
            fade_out_curve,
            ..
        },
        ClipContentDesc::Audio {
            fade_in,
            fade_in_curve,
            ..
        },
    ) = (&ca.content, &cb.content)
    else {
        panic!("audio clips")
    };
    assert!((fade_out - 1.0).abs() < 1e-9 && (fade_in - 1.0).abs() < 1e-9);
    assert_eq!(*fade_out_curve, FadeCurve::EqualPower);
    assert_eq!(*fade_in_curve, FadeCurve::EqualPower);
}
