//! base-144: transposing an audio clip that plays at a transposed rate (unwarped or
//! Repitch) resizes it on the arrangement, trimmed at the next clip; warped (Complex) clips
//! keep their length. One undo step.

use crate::common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::warp::WarpCommand;
use ether_core::protocol::*;

const SR: u32 = 44_100;
/// 4 s = 8 beats at 120 BPM.
const FRAMES: usize = 176_400;

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
    h.create_project("Transpose");
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

    fn get(&self, id: ClipId) -> Clip {
        self.h.project().clips[&id].clone()
    }

    fn audio(&self, id: ClipId) -> AudioContent {
        match self.get(id).content {
            ClipContent::Audio(a) => a,
            ClipContent::Midi => panic!("midi"),
        }
    }

    fn len(&self, id: ClipId) -> f64 {
        self.get(id).length.0
    }

    fn transpose(&mut self, id: ClipId, semitones: f32) {
        self.h
            .ok(Command::Clip(ClipCommand::SetTranspose { id, semitones }));
    }

    fn undo(&mut self) {
        self.h.ok(Command::Edit(EditCommand::Undo));
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn transpose_up_shortens_down_lengthens_and_undoes_in_one_step() {
    let mut f = fx();
    let c = f.clip(0.0, 4.0, 1.0);
    let before = f.h.project().clone();
    f.transpose(c, 12.0);
    assert!(near(f.len(c), 2.0), "{}", f.len(c));
    assert_eq!(f.get(c).offset, Beats(1.0));
    assert_eq!(f.audio(c).transpose, 12.0);
    f.undo();
    assert_eq!(f.h.project(), &before);

    f.transpose(c, -12.0);
    assert!(near(f.len(c), 8.0), "{}", f.len(c));
    // Back to 0: the original length.
    f.transpose(c, 0.0);
    assert!(near(f.len(c), 4.0), "{}", f.len(c));
    // A fifth: length / 2^(7/12).
    f.transpose(c, 7.0);
    assert!(near(f.len(c), 4.0 / (7.0f64 / 12.0).exp2()));
}

#[test]
fn lengthening_stops_at_the_next_clip() {
    let mut f = fx();
    let a = f.clip(0.0, 2.0, 0.0);
    let b = f.clip(3.0, 2.0, 0.0);
    // A clip after B doesn't matter.
    let c = f.clip(8.0, 1.0, 0.0);
    let before = f.h.project().clone();
    f.transpose(a, -12.0);
    // Would be 4 beats: trimmed at B's start.
    assert_eq!(f.get(a).start.0 + f.len(a), 3.0);
    assert!(near(f.get(b).start.0, 3.0) && near(f.len(b), 2.0));
    assert!(near(f.len(c), 1.0));
    f.undo();
    assert_eq!(f.h.project(), &before);

    // Transposing up afterwards halves the (trimmed) clip.
    f.transpose(a, -12.0);
    f.transpose(a, 0.0);
    assert!(near(f.len(a), 1.5), "{}", f.len(a));

    // Shorter than the gap: no trim.
    f.transpose(a, -5.0);
    assert!(near(f.len(a), 1.5 * (5.0f64 / 12.0).exp2()));
}

#[test]
fn a_clip_already_overlapping_the_next_one_does_not_grow_or_shrink_into_it() {
    let mut f = fx();
    let a = f.clip(0.0, 3.0, 0.0);
    let b = f.clip(3.0, 2.0, 1.0);
    f.h.ok(Command::Clip(ClipCommand::Crossfade {
        first: a,
        second: b,
        length: Beats(1.0),
        curve: FadeCurve::EqualPower,
    }));
    let len = f.len(a);
    assert!(f.get(a).start.0 + len > f.get(b).start.0);
    f.transpose(a, -12.0);
    assert!(near(f.len(a), len));
}

#[test]
fn looped_clip_scales_its_loop_region_and_length() {
    let mut f = fx();
    let c = f.clip(0.0, 6.0, 0.0);
    f.h.ok(Command::Clip(ClipCommand::SetLoop {
        id: c,
        looping: ClipLoop {
            enabled: true,
            start: Beats(0.0),
            end: Beats(2.0),
        },
    }));
    f.transpose(c, 12.0);
    let cl = f.get(c);
    assert!(near(cl.looping.start.0, 0.0) && near(cl.looping.end.0, 1.0));
    // Same number of repetitions (3).
    assert!(near(cl.length.0, 3.0));
    assert!(cl.looping.enabled);
    // An offset into the loop scales around the offset (the repitch pivot).
    let d = f.clip(20.0, 4.0, 1.0);
    f.h.ok(Command::Clip(ClipCommand::SetLoop {
        id: d,
        looping: ClipLoop {
            enabled: true,
            start: Beats(0.0),
            end: Beats(3.0),
        },
    }));
    f.transpose(d, -12.0);
    let cl = f.get(d);
    assert_eq!(cl.offset, Beats(1.0));
    // Content 0 sits 1 beat before the pivot → 2 beats before it at half speed: clamped
    // to 0 (loop bounds stay >= 0); the end 3 → 1 + 2·2 = 5.
    assert!(near(cl.looping.start.0, 0.0) && near(cl.looping.end.0, 5.0));
    assert!(near(cl.length.0, 8.0));
}

#[test]
fn fades_are_clamped_to_the_new_length() {
    let mut f = fx();
    let c = f.clip(0.0, 4.0, 0.0);
    f.h.ok(Command::Clip(ClipCommand::SetFades {
        id: c,
        fade_in: Beats(3.0),
        fade_out: Beats(1.0),
    }));
    let before = f.h.project().clone();
    f.transpose(c, 12.0);
    let a = f.audio(c);
    assert!(near(f.len(c), 2.0));
    assert!(near(a.fade_in.0, 2.0) && near(a.fade_out.0, 1.0));
    f.undo();
    assert_eq!(f.h.project(), &before);
}

#[test]
fn warped_complex_clip_keeps_its_length_repitch_warp_follows() {
    let mut f = fx();
    let c = f.clip(0.0, 4.0, 0.0);
    f.h.ok(Command::Warp(WarpCommand::SetWarp {
        clip: c,
        warp: WarpSettings {
            enabled: true,
            mode: WarpMode::Complex,
            source_bpm: Some(120.0),
        },
    }));
    f.transpose(c, -12.0);
    assert!(near(f.len(c), 4.0));
    assert_eq!(f.audio(c).transpose, -12.0);

    // Repitch warp mode changes the speed: the clip follows.
    let d = f.clip(10.0, 4.0, 0.0);
    f.h.ok(Command::Warp(WarpCommand::SetWarp {
        clip: d,
        warp: WarpSettings {
            enabled: true,
            mode: WarpMode::Repitch,
            source_bpm: Some(120.0),
        },
    }));
    f.transpose(d, 12.0);
    assert!(near(f.len(d), 2.0), "{}", f.len(d));
}

#[test]
fn leading_gap_keeps_its_length() {
    let mut f = fx();
    let c = f.clip(4.0, 4.0, 0.0);
    // A leading gap of 2 beats (negative offset; rejected on builds without it).
    let out = f.h.send(Command::Clip(ClipCommand::SetBounds {
        id: c,
        start: Beats(2.0),
        length: Beats(6.0),
        offset: Beats(-2.0),
    }));
    if !matches!(
        out.last(),
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Ok { .. },
            ..
        }))
    ) {
        return;
    }
    f.transpose(c, 12.0);
    // 2 beats of gap + 4 beats of audio at double speed.
    assert!(near(f.len(c), 4.0), "{}", f.len(c));
}

#[test]
fn transpose_rejects_midi_and_non_finite() {
    let mut f = fx();
    let c = f.clip(0.0, 4.0, 0.0);
    let out = f.h.send(Command::Clip(ClipCommand::SetTranspose {
        id: c,
        semitones: f32::NAN,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert!(near(f.len(c), 4.0));
}

/// base-150: transposing several selected clips is one `Edit::Batch`; each clip still
/// resizes (and is trimmed at its next clip) as a lone `SetTranspose` would, and the drag
/// of a multi-clip field (one batch per step, one gesture) is one undo step.
#[test]
fn a_batch_transposes_and_resizes_each_clip_in_one_undo_step() {
    let mut f = fx();
    let a = f.clip(0.0, 2.0, 0.0);
    let b = f.clip(3.0, 2.0, 0.0);
    let before = f.h.project().clone();
    let batch = |semis: [f32; 2]| {
        Command::Edit(EditCommand::Batch {
            key: None,
            label: "Transpose".into(),
            commands: vec![
                Command::Clip(ClipCommand::SetTranspose {
                    id: a,
                    semitones: semis[0],
                }),
                Command::Clip(ClipCommand::SetTranspose {
                    id: b,
                    semitones: semis[1],
                }),
            ],
        })
    };
    f.h.ok(batch([-12.0, -12.0]));
    assert_eq!(f.audio(a).transpose, -12.0);
    assert_eq!(f.audio(b).transpose, -12.0);
    // A would be 4 beats: trimmed at B's start. B has nothing after it: 4 beats.
    assert!(near(f.len(a), 3.0), "{}", f.len(a));
    assert!(near(f.len(b), 4.0), "{}", f.len(b));
    f.undo();
    assert_eq!(f.h.project(), &before);

    // A drag: several batches in one gesture, one undo step.
    let gesture = GestureId(150);
    for st in [1.0, 2.0, 12.0] {
        ok(&f.h.send_with(batch([st, st + 2.0]), Some(gesture)));
    }
    f.h.ok(Command::Edit(EditCommand::EndGesture { gesture }));
    assert!(near(f.len(a), 1.0), "{}", f.len(a));
    assert!(
        near(f.len(b), 2.0 / (14.0f64 / 12.0).exp2()),
        "{}",
        f.len(b)
    );
    f.undo();
    assert_eq!(f.h.project(), &before);
}
