//! Offline render tests of clip fades (fade law, curves, crossfades) and reverse playback
//! (unwarped, looped, Repitch-warped and Complex-warped with a stretcher), plus no
//! allocation on those render paths.

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::fades::fade_gain;
use ether_core::graph::{ClipContentDesc, ClipDesc, WarpDesc};
use ether_core::protocol::model::{FadeCurve, MediaId, TrackKind, Ulid, WarpMode};
use ether_core::{AudioSource, EngineParts, RenderGraphDesc, TransportControl, create};
use ether_stretch::SignalsmithFactory;

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

/// Samples per beat at 120 BPM / 48 kHz.
const SPB: usize = 24_000;
const MEDIA: MediaId = MediaId(Ulid(42));

/// Stereo in-memory source.
struct Stereo(Vec<f32>, Vec<f32>);

impl AudioSource for Stereo {
    fn channels(&self) -> u16 {
        2
    }
    fn frames(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        let src = if channel == 0 { &self.0 } else { &self.1 };
        for (i, o) in out.iter_mut().enumerate() {
            *o = src.get(start as usize + i).copied().unwrap_or(0.0);
        }
        true
    }
}

/// A ramp with a distinct value per frame (left) and its negation (right).
fn ramp(n: usize) -> Vec<f32> {
    (0..n).map(|i| (i as f32 + 1.0) / n as f32).collect()
}

fn source(l: Vec<f32>) -> Arc<dyn AudioSource> {
    let r = l.iter().map(|v| -v).collect();
    Arc::new(Stereo(l, r))
}

fn reversed(v: &[f32]) -> Vec<f32> {
    v.iter().rev().copied().collect()
}

struct Clip {
    start: f64,
    length: f64,
    offset: f64,
    looping: Option<(f64, f64)>,
    fade_in: (f64, FadeCurve),
    fade_out: (f64, FadeCurve),
    reversed: bool,
    transpose: f32,
    warp: Option<WarpDesc>,
    gain: f32,
}

impl Default for Clip {
    fn default() -> Self {
        Clip {
            start: 1.0,
            length: 2.0,
            offset: 0.0,
            looping: None,
            fade_in: (0.0, FadeCurve::Linear),
            fade_out: (0.0, FadeCurve::Linear),
            reversed: false,
            transpose: 0.0,
            warp: None,
            gain: 1.0,
        }
    }
}

fn clip_desc(id: u128, c: &Clip) -> ClipDesc {
    ClipDesc {
        id: cid(id),
        start: c.start,
        length: c.length,
        offset: c.offset,
        looping: c.looping,
        muted: false,
        content: ClipContentDesc::Audio {
            media: MEDIA,
            gain: c.gain,
            transpose: c.transpose,
            fade_in: c.fade_in.0,
            fade_out: c.fade_out.0,
            fade_in_curve: c.fade_in.1,
            fade_out_curve: c.fade_out.1,
            reversed: c.reversed,
            warp: c.warp.clone(),
        },
        envelopes: vec![],
    }
}

fn engine(src: Arc<dyn AudioSource>, clips: &[Clip], stretch: bool) -> EngineParts {
    let mut p = create(config());
    if stretch {
        p.handle
            .set_stretcher_factory(Arc::new(SignalsmithFactory::default()));
    }
    p.handle.add_source(MEDIA, src).unwrap();
    let mut t = track(tid(2), TrackKind::Audio, Some(tid(1)));
    t.clips = clips
        .iter()
        .enumerate()
        .map(|(i, c)| clip_desc(i as u128 + 1, c))
        .collect();
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master(), t],
            ..Default::default()
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    p
}

fn play(src: Arc<dyn AudioSource>, clips: &[Clip], stretch: bool, frames: usize) -> Vec<f32> {
    let mut p = engine(src, clips, stretch);
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    let mut bl = vec![0.0; BLOCK];
    let mut br = vec![0.0; BLOCK];
    let mut done = 0;
    while done < frames {
        let n = BLOCK.min(frames - done);
        {
            let mut outs: [&mut [f32]; 2] = [&mut bl[..n], &mut br[..n]];
            assert_no_alloc(|| p.engine.process(&[], &mut outs, n));
        }
        l[done..done + n].copy_from_slice(&bl[..n]);
        r[done..done + n].copy_from_slice(&br[..n]);
        done += n;
    }
    // Stereo sources are ±: the right channel mirrors the left.
    for i in 0..frames {
        assert!((l[i] + r[i]).abs() < 1e-5, "channel mismatch at {i}");
    }
    l
}

fn dc(n: usize) -> Arc<dyn AudioSource> {
    source(vec![0.5; n])
}

#[test]
fn fade_in_and_out_follow_the_fade_law() {
    for curve in [
        FadeCurve::Linear,
        FadeCurve::EqualPower,
        FadeCurve::Curve { tension: 0.6 },
        FadeCurve::Curve { tension: -0.6 },
    ] {
        let clip = Clip {
            fade_in: (0.5, curve),
            fade_out: (1.0, curve),
            ..Default::default()
        };
        let out = play(dc(4 * SPB), &[clip], false, 4 * SPB);
        let start = SPB;
        let fin = SPB / 2;
        let end = 3 * SPB;
        for k in [0usize, 1200, 3000, 6000, 9000, 11_999] {
            let want = 0.5 * fade_gain(curve, k as f32 / fin as f32);
            let got = out[start + k];
            assert!((got - want).abs() < 1e-4, "{curve:?} in {k}: {got} {want}");
        }
        assert!((out[start + fin + 10] - 0.5).abs() < 1e-5);
        for k in [1usize, 2400, 12_000, 20_000, SPB] {
            let want = 0.5 * fade_gain(curve, k as f32 / SPB as f32);
            let got = out[end - k];
            assert!((got - want).abs() < 1e-4, "{curve:?} out {k}: {got} {want}");
        }
        assert_eq!(out[end + 5], 0.0);
    }
}

#[test]
fn short_fades_keep_the_linear_declick() {
    // A 0-length fade still ramps over the 64-sample anti-click, whatever the curve.
    let clip = Clip {
        fade_in: (0.0, FadeCurve::Curve { tension: 1.0 }),
        ..Default::default()
    };
    let out = play(dc(4 * SPB), &[clip], false, 2 * SPB);
    assert!((out[SPB + 32] - 0.25).abs() < 1e-3, "{}", out[SPB + 32]);
    assert!((out[SPB + 64] - 0.5).abs() < 1e-5);
}

#[test]
fn crossfade_midpoint_sums_to_unity() {
    // A = [1, 3) fading out over its last beat, B = [2, 4) fading in over its first.
    for (curve, power) in [(FadeCurve::Linear, false), (FadeCurve::EqualPower, true)] {
        let a = Clip {
            start: 1.0,
            length: 2.0,
            fade_out: (1.0, curve),
            ..Default::default()
        };
        let b = Clip {
            start: 2.0,
            length: 2.0,
            fade_in: (1.0, curve),
            ..Default::default()
        };
        let n = 5 * SPB;
        let only_a = play(dc(8 * SPB), &[a], false, n);
        let only_b = play(dc(8 * SPB), &[b], false, n);
        let both = play(
            dc(8 * SPB),
            &[
                Clip {
                    fade_out: (1.0, curve),
                    ..Default::default()
                },
                Clip {
                    start: 2.0,
                    fade_in: (1.0, curve),
                    ..Default::default()
                },
            ],
            false,
            n,
        );
        let mid = 2 * SPB + SPB / 2;
        let (ga, gb) = (only_a[mid] / 0.5, only_b[mid] / 0.5);
        assert!((ga - gb).abs() < 1e-4, "{curve:?}: symmetric");
        if power {
            assert!(
                (ga * ga + gb * gb - 1.0).abs() < 1e-3,
                "{curve:?}: {ga} {gb}"
            );
            assert!((ga - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        } else {
            assert!((ga + gb - 1.0).abs() < 1e-4, "{curve:?}: {ga} {gb}");
        }
        // The engine sums the two clips.
        for i in [2 * SPB + 100, mid, 3 * SPB - 100] {
            assert!((both[i] - only_a[i] - only_b[i]).abs() < 1e-5);
        }
        if !power {
            // Linear crossfade of identical material: constant level across the overlap.
            for i in (2 * SPB + 100..3 * SPB - 100).step_by(997) {
                assert!((both[i] - 0.5).abs() < 1e-4, "{i}: {}", both[i]);
            }
        }
    }
}

#[test]
fn reverse_is_sample_exact() {
    let n = 3 * SPB;
    let m = ramp(n);
    // Offset 0.25 beats on the reversed timeline: the clip starts at reversed frame 6000,
    // i.e. media frame n - 1 - 6000, and walks backwards one frame per sample.
    let clip = Clip {
        offset: 0.25,
        reversed: true,
        ..Default::default()
    };
    let out = play(source(m.clone()), &[clip], false, 4 * SPB);
    assert_eq!(out[SPB - 1], 0.0);
    for k in [64usize, 65, 100, 1000, 12_345, 2 * SPB - 65] {
        let want = m[n - 1 - (6000 + k)];
        assert!(
            (out[SPB + k] - want).abs() < 1e-6,
            "{k}: {} != {want}",
            out[SPB + k]
        );
    }
    assert_eq!(out[3 * SPB + 1], 0.0);
}

#[test]
fn reversed_clip_equals_forward_clip_of_reversed_media() {
    let n = 3 * SPB + 777;
    let m: Vec<f32> = (0..n)
        .map(|i| ((i as f32 * 0.013).sin() * 0.4) + (i % 97) as f32 / 400.0)
        .collect();
    let warp = WarpDesc {
        mode: WarpMode::Repitch,
        markers: vec![(0.0, 0.0), (1.0, 0.6), (4.0, 1.4)],
    };
    let variants = [
        Clip::default(),
        Clip {
            offset: 0.3,
            looping: Some((0.2, 0.9)),
            length: 2.5,
            ..Default::default()
        },
        Clip {
            transpose: 5.0,
            fade_in: (0.4, FadeCurve::EqualPower),
            fade_out: (0.7, FadeCurve::Curve { tension: 0.3 }),
            ..Default::default()
        },
        Clip {
            warp: Some(warp),
            offset: 0.5,
            ..Default::default()
        },
    ];
    for (i, v) in variants.into_iter().enumerate() {
        let rev = play(
            source(m.clone()),
            &[Clip {
                reversed: true,
                warp: v.warp.clone(),
                ..v
            }],
            false,
            4 * SPB,
        );
        let fwd = play(
            source(reversed(&m)),
            &[Clip {
                reversed: false,
                warp: v.warp.clone(),
                ..v
            }],
            false,
            4 * SPB,
        );
        let diff = rev
            .iter()
            .zip(&fwd)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(diff < 1e-4, "variant {i}: max diff {diff}");
        assert!(rev.iter().any(|v| v.abs() > 0.1), "variant {i} is silent");
    }
}

fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
}

/// Complex warp with a stretcher: reverse feeds the stretcher the reversed media (same
/// meaning as for Repitch: offset, loop and warp markers are on the reversed timeline), and
/// fades use the fade law.
#[test]
fn complex_warp_reverse_and_fades() {
    // 1 s of silence then 2 s of a 440 Hz sine.
    let n = 3 * 48_000;
    let m: Vec<f32> = (0..n)
        .map(|i| {
            if i < 48_000 {
                0.0
            } else {
                (std::f64::consts::TAU * 440.0 * i as f64 / 48_000.0).sin() as f32 * 0.5
            }
        })
        .collect();
    // Content beat b plays source second b / 2 (unity speed at 120 BPM).
    let warp = WarpDesc {
        mode: WarpMode::Complex,
        markers: vec![(0.0, 0.0), (6.0, 3.0)],
    };
    let clip = |reversed: bool| Clip {
        start: 1.0,
        length: 6.0,
        reversed,
        warp: Some(warp.clone()),
        fade_in: (1.0, FadeCurve::EqualPower),
        ..Default::default()
    };
    let frames = 8 * SPB;
    let fwd = play(source(m.clone()), &[clip(false)], true, frames);
    let rev = play(source(m.clone()), &[clip(true)], true, frames);
    let seg = |v: &[f32], beat: f64| {
        let i = (beat * SPB as f64) as usize;
        rms(&v[i..i + 4800])
    };
    // Forward: silence for the first 2 beats of the clip (beats 1..3), then the sine.
    assert!(seg(&fwd, 1.5) < 0.01);
    assert!(seg(&fwd, 4.5) > 0.3);
    // Reversed: the sine first (fading in), silence in the last 2 beats (beats 5..7).
    assert!(seg(&rev, 3.5) > 0.3, "{}", seg(&rev, 3.5));
    assert!(seg(&rev, 5.8) < 0.01, "{}", seg(&rev, 5.8));
    // The equal-power fade-in shapes the level of the reversed clip's first beat.
    let early = seg(&rev, 1.1);
    let late = seg(&rev, 1.7);
    assert!(
        early < late && late < seg(&rev, 3.5) + 0.02,
        "{early} {late}"
    );
    // Identical to the forward Complex rendering of a reversed file.
    let fwd_of_rev = play(source(reversed(&m)), &[clip(false)], true, frames);
    let diff = rev
        .iter()
        .zip(&fwd_of_rev)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(diff < 1e-4, "max diff {diff}");
}
