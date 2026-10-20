//! Reversing an audio clip (#442): a clip playing media frames `a ..= b` plays `b ..= a`
//! once reversed (the offset, loop region, warp markers and clip envelopes are mirrored),
//! fades stay at the clip edges, reversing twice gives the clip back, and a split of a
//! reversed clip keeps both halves on their own material. Rendered offline through the
//! real engine and compared with the source.

use crate::common;

use std::sync::Arc;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::offline::OfflineRenderer;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::warp::WarpCommand;
use ether_core::protocol::*;
use ether_core::{AudioSource, EngineConfig, RenderGraphDesc};
use ether_media::{DecodedAudio, InMemorySource};

const SR: u32 = 44_100;
/// 2.5 s = 5 beats at 120 BPM (22 050 frames a beat).
const FRAMES: usize = 110_250;
const BEAT: usize = 22_050;
/// The engine's anti-click ramp at clip edges (`ether_core::sched::DECLICK_SAMPLES`).
const EDGE: usize = 64;

/// Deterministic noise: no two windows look alike, so a wrong offset can't pass.
fn noise() -> Vec<f32> {
    let mut x: u32 = 0x1234_5678;
    (0..FRAMES)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((x >> 16) as i16) as f32 / 65_536.0
        })
        .collect()
}

struct Fx {
    h: Harness,
    track: TrackId,
    media: MediaId,
    src: Vec<f32>,
}

/// A slow chirp (5 → 45 Hz): smooth, so a sub-sample shift (a warped mirror over a slope
/// other than the native rate) moves it by little, while a wrong offset moves it a lot.
fn chirp() -> Vec<f32> {
    (0..FRAMES)
        .map(|i| {
            let t = i as f64 / SR as f64;
            (0.5 * (2.0 * std::f64::consts::PI * (5.0 * t + 8.0 * t * t)).sin()) as f32
        })
        .collect()
}

fn fx() -> Fx {
    fx_with(noise())
}

fn fx_with(src: Vec<f32>) -> Fx {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    lib.add_file("lib", "noise.wav", wav(SR, std::slice::from_ref(&src)));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Reverse");
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "noise.wav".into(),
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
    Fx {
        h,
        track,
        media,
        src,
    }
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

    fn reverse(&mut self, id: ClipId, reversed: bool) {
        self.h
            .ok(Command::Clip(ClipCommand::SetReversed { id, reversed }));
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

    fn markers(&self, id: ClipId) -> Vec<(f64, f64)> {
        self.h
            .project()
            .warp_markers_of(id)
            .into_iter()
            .map(|m| (m.beat.0, m.source.0))
            .collect()
    }

    /// The left channel of `frames` frames from the start, through the real engine.
    fn render(&mut self, frames: usize) -> Vec<f32> {
        self.h.advance(100);
        self.h.tick();
        let mut g: RenderGraphDesc = self.h.ctl.bridge.last_graph().clone();
        for t in &mut g.tracks {
            t.chain.clear();
        }
        let mut r = OfflineRenderer::new(EngineConfig {
            sample_rate: SR,
            max_block_size: 256,
            ..EngineConfig::default()
        });
        let audio = Arc::new(DecodedAudio {
            sample_rate: SR,
            channels: vec![self.src.clone()],
        });
        let src: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio));
        r.handle().add_source(self.media, src).unwrap();
        r.publish(g).unwrap();
        r.start(Beats(0.0)).unwrap();
        let mut l = vec![0.0f32; frames];
        let mut rr = vec![0.0f32; frames];
        r.render(frames, &mut [&mut l, &mut rr]);
        l
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// `out[i] == g * want[i]` away from the clip edges' anti-click ramps, with `g` the track's
/// fixed output gain (read off the first compared sample's neighbourhood).
fn assert_plays(out: &[f32], want: &[f32], what: &str) {
    assert_eq!(out.len(), want.len());
    let n = out.len();
    let num: f64 = (EDGE..n - EDGE)
        .map(|i| out[i] as f64 * want[i] as f64)
        .sum();
    let den: f64 = (EDGE..n - EDGE).map(|i| (want[i] as f64).powi(2)).sum();
    let g = num / den;
    assert!(
        g > 0.1,
        "{what}: no correlation with the expected material (g = {g})"
    );
    for i in EDGE..n - EDGE {
        let e = (out[i] as f64 - g * want[i] as f64).abs();
        assert!(
            e < 1e-5,
            "{what}: sample {i}: {} vs {}",
            out[i],
            g * want[i] as f64
        );
    }
}

#[test]
fn a_trimmed_clip_reversed_plays_its_own_material_backwards() {
    let mut f = fx();
    // Content beats 1 .. 3: media frames 22 050 .. 66 150.
    let c = f.clip(0.0, 2.0, 1.0);
    let n = 2 * BEAT;
    let fwd = f.render(n);
    let slice = f.src[BEAT..3 * BEAT].to_vec();
    assert_plays(&fwd, &slice, "forward");

    let before = f.h.project().clone();
    f.reverse(c, true);
    // On the reversed file the same frames are 44 100 .. 88 200: content 2 .. 4.
    assert!(near(f.get(c).offset.0, 2.0), "{}", f.get(c).offset.0);
    assert!(near(f.get(c).length.0, 2.0));
    let rev = f.render(n);
    let backwards: Vec<f32> = slice.iter().rev().copied().collect();
    assert_plays(&rev, &backwards, "reversed");

    // Setting the current value again changes nothing (no second mirror).
    f.reverse(c, true);
    assert!(near(f.get(c).offset.0, 2.0));

    // One undo step; reversing back gives the clip back.
    f.h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(f.h.project(), &before);
    f.h.ok(Command::Edit(EditCommand::Redo));
    f.reverse(c, false);
    assert!(near(f.get(c).offset.0, 1.0));
    assert!(!f.audio(c).reversed);
    assert_plays(&f.render(n), &slice, "reversed twice");
}

#[test]
fn fades_stay_at_the_clip_edges() {
    let mut f = fx();
    let c = f.clip(0.0, 2.0, 1.0);
    f.h.ok(Command::Clip(ClipCommand::SetFades {
        id: c,
        fade_in: Beats(0.5),
        fade_out: Beats(0.25),
    }));
    f.reverse(c, true);
    let a = f.audio(c);
    assert_eq!((a.fade_in.0, a.fade_out.0), (0.5, 0.25));
    // The fade-in still shapes the clip start (silent at 0), the middle plays at full level.
    let out = f.render(2 * BEAT);
    let backwards: Vec<f32> = f.src[BEAT..3 * BEAT].iter().rev().copied().collect();
    assert!(out[EDGE].abs() < 0.01);
    assert_plays(
        &out[BEAT / 2..BEAT * 3 / 2],
        &backwards[BEAT / 2..BEAT * 3 / 2],
        "between the fades",
    );
}

#[test]
fn a_looped_clip_mirrors_its_loop_region() {
    let mut f = fx();
    // Loop content 1 .. 2 played twice from the loop start.
    let c = f.clip(0.0, 2.0, 1.0);
    f.h.ok(Command::Clip(ClipCommand::SetLoop {
        id: c,
        looping: ClipLoop {
            enabled: true,
            start: Beats(1.0),
            end: Beats(2.0),
        },
    }));
    f.reverse(c, true);
    let cl = f.get(c);
    assert!(near(cl.looping.start.0, 3.0) && near(cl.looping.end.0, 4.0));
    assert!(near(cl.offset.0, 3.0));
    let out = f.render(2 * BEAT);
    let once: Vec<f32> = f.src[BEAT..2 * BEAT].iter().rev().copied().collect();
    let twice: Vec<f32> = once.iter().chain(once.iter()).copied().collect();
    // The loop seam is inside the clip: no anti-click ramp there.
    assert_plays(&out, &twice, "reversed loop");
    f.reverse(c, false);
    let cl = f.get(c);
    assert!(near(cl.looping.start.0, 1.0) && near(cl.looping.end.0, 2.0));
    assert!(near(cl.offset.0, 1.0));
}

#[test]
fn a_warped_clip_mirrors_its_markers() {
    let mut f = fx_with(chirp());
    let c = f.clip(0.0, 3.0, 1.0);
    f.h.ok(Command::Warp(WarpCommand::SetWarp {
        clip: c,
        warp: WarpSettings {
            enabled: true,
            mode: WarpMode::Repitch,
            source_bpm: Some(120.0),
        },
    }));
    // Replace the pinned markers by a non-linear map: 0 → 0 s, 2 → 0.75 s, 6 → 2.5 s.
    for m in
        f.h.project()
            .warp_markers_of(c)
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>()
    {
        f.h.ok(Command::Warp(WarpCommand::RemoveMarker { id: m }));
    }
    for (b, s) in [(0.0, 0.0), (2.0, 0.75), (6.0, 2.5)] {
        let id: WarpMarkerId = f.h.id();
        f.h.ok(Command::Warp(WarpCommand::AddMarker {
            id,
            clip: c,
            beat: Beats(b),
            source: Seconds(s),
        }));
    }
    let markers = f.markers(c);
    // (Re-timing the markers moves the offset so the clip keeps its audio.)
    let offset = f.get(c).offset.0;
    let n = 3 * BEAT;
    let fwd = f.render(n);
    f.reverse(c, true);
    // The map mirrors over the file: K = inv(0) + inv(2.5 s) = 6.
    let got = f.markers(c);
    let want = [(0.0, 0.0), (4.0, 1.75), (6.0, 2.5)];
    assert_eq!(got.len(), 3);
    for (g, w) in got.iter().zip(want) {
        assert!(near(g.0, w.0) && near(g.1, w.1), "{got:?}");
    }
    assert!(near(f.get(c).offset.0, 6.0 - offset - 3.0));
    let rev = f.render(n);
    // The same material backwards, up to a sub-sample shift.
    for i in EDGE..n - EDGE {
        assert!(
            (rev[i] - fwd[n - 1 - i]).abs() < 1e-2,
            "sample {i}: {} vs {}",
            rev[i],
            fwd[n - 1 - i]
        );
    }
    f.reverse(c, false);
    for (g, w) in f.markers(c).iter().zip(&markers) {
        assert!(near(g.0, w.0) && near(g.1, w.1));
    }
    assert!(near(f.get(c).offset.0, offset));
}

#[test]
fn split_halves_of_a_reversed_clip_keep_their_material() {
    let mut f = fx();
    let c = f.clip(0.0, 2.0, 1.0);
    f.reverse(c, true);
    let right: ClipId = f.h.id();
    f.h.ok(Command::Clip(ClipCommand::Split {
        id: c,
        at: Beats(1.0),
        new_id: right,
    }));
    // Played backwards: the left half is media 2 .. 3 beats, the right half 1 .. 2.
    let out = f.render(2 * BEAT);
    let backwards: Vec<f32> = f.src[BEAT..3 * BEAT].iter().rev().copied().collect();
    assert_plays(&out[..BEAT], &backwards[..BEAT], "left half");
    assert_plays(&out[BEAT..], &backwards[BEAT..], "right half");
    // Unreversed, each half plays its own material forwards.
    f.reverse(c, false);
    f.reverse(right, false);
    assert!(near(f.get(c).offset.0, 2.0) && near(f.get(right).offset.0, 1.0));
    let out = f.render(2 * BEAT);
    assert_plays(
        &out[..BEAT],
        &f.src[2 * BEAT..3 * BEAT],
        "left half forwards",
    );
    assert_plays(&out[BEAT..], &f.src[BEAT..2 * BEAT], "right half forwards");
}

#[test]
fn transposed_clip_reversed_plays_the_same_samples_backwards() {
    let mut f = fx();
    let c = f.clip(0.0, 2.0, 0.5);
    f.h.ok(Command::Clip(ClipCommand::SetTranspose {
        id: c,
        semitones: 5.0,
    }));
    // A whole number of samples, so the played frames reverse onto themselves.
    f.h.ok(Command::Clip(ClipCommand::SetBounds {
        id: c,
        start: Beats(0.0),
        length: Beats(1.0),
        offset: Beats(0.5),
    }));
    let n = BEAT;
    let fwd = f.render(n);
    f.reverse(c, true);
    let rev = f.render(n);
    let backwards: Vec<f32> = fwd.iter().rev().copied().collect();
    for i in EDGE..n - EDGE {
        assert!((rev[i] - backwards[i]).abs() < 1e-4, "sample {i}");
    }
    f.reverse(c, false);
    assert!(near(f.get(c).offset.0, 0.5));
}

#[test]
fn the_gain_envelope_follows_the_material() {
    let mut f = fx();
    let c = f.clip(0.0, 2.0, 1.0);
    let lane: AutomationLaneId = f.h.id();
    f.h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Clip { clip: c },
        target: AutomationTarget::ClipGain,
    }));
    let points = [(1.0, 0.2), (3.0, 0.9)]
        .iter()
        .map(|&(t, v)| PointSpec {
            id: f.h.id(),
            time: Beats(t),
            value: v,
            curve: CurveShape::Linear,
        })
        .collect();
    f.h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane,
        points,
    }));
    let n = 2 * BEAT;
    let fwd = f.render(n);
    f.reverse(c, true);
    let mut pts: Vec<(f64, f64)> =
        f.h.project()
            .points_of(lane)
            .into_iter()
            .map(|p| (p.time.0, p.value))
            .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(pts.len(), 2);
    assert!(near(pts[0].0, 2.0) && near(pts[0].1, 0.9), "{pts:?}");
    assert!(near(pts[1].0, 4.0) && near(pts[1].1, 0.2), "{pts:?}");
    let rev = f.render(n);
    for i in EDGE..n - EDGE {
        assert!((rev[i] - fwd[n - 1 - i]).abs() < 1e-3, "sample {i}");
    }
    f.reverse(c, false);
    let back: Vec<f64> =
        f.h.project()
            .points_of(lane)
            .into_iter()
            .map(|p| p.time.0)
            .collect();
    assert!(back.iter().any(|t| near(*t, 1.0)) && back.iter().any(|t| near(*t, 3.0)));
}
