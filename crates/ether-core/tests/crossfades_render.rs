//! `crossfades` node: offline renders of two audio clips crossfaded the way the controller
//! lays them out (`ClipCommand::{Crossfade, SetCrossfade}`): the earlier clip extended past
//! the boundary with a fade-out, the later one started before it with a fade-in of the same
//! length. Checks the gain law at the midpoint, continuity (no clicks), constant amplitude on
//! correlated material (linear) and constant power on uncorrelated material (equal power),
//! and that looping, transposed and repitch-warped clips crossfade the same way.

mod common;

use std::sync::Arc;

use common::*;
use ether_core::graph::{ClipContentDesc, ClipDesc, WarpDesc};
use ether_core::protocol::model::{FadeCurve, MediaId, TrackKind, Ulid, WarpMode};
use ether_core::{AudioSource, RenderGraphDesc, TransportControl, create};

/// Samples per beat at 120 BPM / 48 kHz.
const SPB: usize = 24_000;

struct Mono(Vec<f32>);

impl AudioSource for Mono {
    fn channels(&self) -> u16 {
        1
    }
    fn frames(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, _channel: u16, start: u64, out: &mut [f32]) -> bool {
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.0.get(start as usize + i).copied().unwrap_or(0.0);
        }
        true
    }
}

fn media(n: u128) -> MediaId {
    MediaId(Ulid(1000 + n))
}

#[derive(Clone)]
struct Clip {
    media: u128,
    start: f64,
    length: f64,
    offset: f64,
    looping: Option<(f64, f64)>,
    fade_in: f64,
    fade_out: f64,
    curve: FadeCurve,
    transpose: f32,
    warp: Option<WarpDesc>,
}

fn clip(media: u128, start: f64, length: f64, offset: f64) -> Clip {
    Clip {
        media,
        start,
        length,
        offset,
        looping: None,
        fade_in: 0.0,
        fade_out: 0.0,
        curve: FadeCurve::Linear,
        transpose: 0.0,
        warp: None,
    }
}

/// Crossfade `a` into `b` over `len` beats centred on their shared boundary (what the
/// controller does when both sides have material).
fn crossfade(mut a: Clip, mut b: Clip, len: f64, curve: FadeCurve) -> (Clip, Clip) {
    let half = len / 2.0;
    a.length += half;
    b.start -= half;
    b.offset -= half;
    b.length += half;
    a.fade_out = len;
    b.fade_in = len;
    a.curve = curve;
    b.curve = curve;
    (a, b)
}

fn desc(id: u128, c: &Clip) -> ClipDesc {
    ClipDesc {
        id: cid(id),
        start: c.start,
        length: c.length,
        offset: c.offset,
        looping: c.looping,
        muted: false,
        content: ClipContentDesc::Audio {
            media: media(c.media),
            gain: 1.0,
            transpose: c.transpose,
            fade_in: c.fade_in,
            fade_out: c.fade_out,
            fade_in_curve: c.curve,
            fade_out_curve: c.curve,
            reversed: false,
            warp: c.warp.clone(),
            gain_envelope: None,
            transpose_envelope: None,
        },
        envelopes: vec![],
    }
}

/// Render `frames` of the clips (left channel) with the given media.
fn play(sources: &[(u128, Vec<f32>)], clips: &[Clip], frames: usize) -> Vec<f32> {
    let mut p = create(config());
    for (id, s) in sources {
        p.handle
            .add_source(media(*id), Arc::new(Mono(s.clone())))
            .unwrap();
    }
    let mut t = track(tid(2), TrackKind::Audio, Some(tid(1)));
    t.clips = clips
        .iter()
        .enumerate()
        .map(|(i, c)| desc(i as u128 + 1, c))
        .collect();
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master(), t],
            ..Default::default()
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, frames, BLOCK).0
}

fn sine(n: usize, hz: f64, amp: f32) -> Vec<f32> {
    (0..n)
        .map(|i| (std::f64::consts::TAU * hz * i as f64 / SR as f64).sin() as f32 * amp)
        .collect()
}

/// Deterministic white noise in -amp..amp.
fn noise(n: usize, seed: u64, amp: f32) -> Vec<f32> {
    let mut s = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as f32 / (1u64 << 31) as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
}

/// Largest sample-to-sample step in `v[range]`.
fn max_step(v: &[f32], range: std::ops::Range<usize>) -> f32 {
    v[range.start.max(1)..range.end]
        .iter()
        .zip(&v[range.start.max(1) - 1..range.end - 1])
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn linear_crossfade_of_a_split_clip_is_seamless() {
    // One 220 Hz sine split at beat 3 and crossfaded over a beat: the output is the original
    // sine, sample for sample (linear gains sum to 1 on identical material).
    let src = sine(8 * SPB, 220.0, 0.5);
    let (a, b) = crossfade(
        clip(1, 0.0, 3.0, 0.0),
        clip(1, 3.0, 3.0, 3.0),
        1.0,
        FadeCurve::Linear,
    );
    let out = play(&[(1, src.clone())], &[a, b], 7 * SPB);
    // After the 64-sample anti-click at the clip start, until the anti-click at the end.
    let err = (100..6 * SPB - 100)
        .map(|i| (out[i] - src[i]).abs())
        .fold(0.0, f32::max);
    assert!(err < 1e-4, "max error {err}");
    // No discontinuity: steps stay within the sine's own slope (2π·220/48k·0.5 ≈ 0.0144).
    let step = max_step(&out, 2 * SPB..4 * SPB);
    assert!(step < 0.016, "step {step}");
}

#[test]
fn gain_law_at_the_midpoint() {
    let dc = vec![0.5; 8 * SPB];
    for (curve, want) in [
        (FadeCurve::Linear, 0.5f32),
        (FadeCurve::EqualPower, std::f32::consts::FRAC_1_SQRT_2),
        (
            FadeCurve::Curve { tension: 0.5 },
            ether_core::fades::fade_gain(FadeCurve::Curve { tension: 0.5 }, 0.5),
        ),
    ] {
        let (a, b) = crossfade(clip(1, 0.0, 3.0, 0.0), clip(2, 3.0, 3.0, 3.0), 2.0, curve);
        let mid = 3 * SPB;
        let only_a = play(
            &[(1, dc.clone()), (2, dc.clone())],
            std::slice::from_ref(&a),
            5 * SPB,
        );
        let only_b = play(
            &[(1, dc.clone()), (2, dc.clone())],
            std::slice::from_ref(&b),
            5 * SPB,
        );
        let both = play(&[(1, dc.clone()), (2, dc.clone())], &[a, b], 5 * SPB);
        assert!(
            (only_a[mid] / 0.5 - want).abs() < 1e-3,
            "{curve:?} a {}",
            only_a[mid]
        );
        assert!(
            (only_b[mid] / 0.5 - want).abs() < 1e-3,
            "{curve:?} b {}",
            only_b[mid]
        );
        // The crossfade region is the sum of the two clips, from its start to its end.
        for i in (2 * SPB..4 * SPB).step_by(1009) {
            assert!(
                (both[i] - only_a[i] - only_b[i]).abs() < 1e-5,
                "{curve:?} {i}"
            );
        }
        // Continuous across the whole region (DC: steps only from the fade shapes).
        let step = max_step(&both, 2 * SPB - 10..4 * SPB + 10);
        assert!(step < 1e-3, "{curve:?} step {step}");
    }
}

#[test]
fn equal_power_keeps_power_constant_on_uncorrelated_material() {
    // A plays noise, B independent noise of the same level: with equal-power fades the sum's
    // power stays at the level outside the crossfade (within the estimate's noise); with
    // linear fades it dips by ~3 dB at the midpoint.
    let na = noise(10 * SPB, 1, 0.5);
    let nb = noise(10 * SPB, 2, 0.5);
    let sources = [(1, na), (2, nb)];
    let window = 2400;
    let level = |v: &[f32], beat: f64| {
        let i = (beat * SPB as f64) as usize - window / 2;
        rms(&v[i..i + window])
    };
    for (curve, dip) in [(FadeCurve::EqualPower, false), (FadeCurve::Linear, true)] {
        let (a, b) = crossfade(clip(1, 0.0, 4.0, 0.0), clip(2, 4.0, 4.0, 4.0), 2.0, curve);
        let out = play(&sources, &[a, b], 9 * SPB);
        let reference = level(&out, 1.5);
        let mid = level(&out, 4.0);
        if dip {
            let db = 20.0 * (mid / reference).log10();
            assert!((db + 3.0).abs() < 0.8, "linear dip {db} dB");
        } else {
            for beat in [3.2, 3.6, 4.0, 4.4, 4.8] {
                let db = 20.0 * (level(&out, beat) / reference).log10();
                assert!(db.abs() < 0.8, "equal power at {beat}: {db} dB");
            }
        }
    }
}

#[test]
fn equal_power_on_correlated_material_peaks_at_plus_3_db() {
    // The documented trade-off: identical material crossfaded with equal power sums to √2 at
    // the midpoint (choose Linear for a split of the same take).
    let src = vec![0.5; 8 * SPB];
    let (a, b) = crossfade(
        clip(1, 0.0, 3.0, 0.0),
        clip(1, 3.0, 3.0, 3.0),
        1.0,
        FadeCurve::EqualPower,
    );
    let out = play(&[(1, src)], &[a, b], 5 * SPB);
    assert!(
        (out[3 * SPB] - 0.5 * std::f32::consts::SQRT_2).abs() < 1e-3,
        "{}",
        out[3 * SPB]
    );
}

#[test]
fn looping_transposed_and_repitched_clips_crossfade_without_clicks() {
    let src = sine(8 * SPB, 110.0, 0.5);
    let sources = [(1, src)];
    let cases: Vec<(&str, Clip, Clip)> = vec![
        (
            // A loops one beat; the crossfade extends it past its end (one more repeat).
            "looping",
            Clip {
                looping: Some((0.0, 1.0)),
                ..clip(1, 0.0, 3.0, 0.0)
            },
            clip(1, 3.0, 3.0, 3.0),
        ),
        (
            "transposed",
            Clip {
                transpose: 7.0,
                ..clip(1, 0.0, 3.0, 0.0)
            },
            Clip {
                transpose: -5.0,
                ..clip(1, 3.0, 3.0, 3.0)
            },
        ),
        (
            "repitch warp",
            Clip {
                warp: Some(WarpDesc {
                    mode: WarpMode::Repitch,
                    markers: vec![(0.0, 0.0), (8.0, 3.0)],
                }),
                ..clip(1, 0.0, 3.0, 0.0)
            },
            clip(1, 3.0, 3.0, 3.0),
        ),
    ];
    for (name, a, b) in cases {
        for curve in [FadeCurve::Linear, FadeCurve::EqualPower] {
            let (a, b) = crossfade(a.clone(), b.clone(), 1.0, curve);
            let out = play(&sources, &[a, b], 7 * SPB);
            // Steps bounded by twice the fastest sine's slope (7 semitones up, ×1.5) plus
            // the fade slopes: a click would be a step of the order of the amplitude.
            let step = max_step(&out, SPB..5 * SPB);
            assert!(step < 0.05, "{name} {curve:?}: step {step}");
            // Never silent inside the crossfade.
            for beat in [2.6, 3.0, 3.4] {
                let i = (beat * SPB as f64) as usize;
                assert!(
                    rms(&out[i - 1200..i + 1200]) > 0.1,
                    "{name} {curve:?} {beat}"
                );
            }
        }
    }
}
