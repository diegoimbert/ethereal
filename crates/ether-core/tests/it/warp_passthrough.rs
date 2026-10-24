//! A warped clip whose mapping plays the source at its own speed, untransposed, is the
//! source itself: Complex (Signalsmith, C++ and Rust port), Complex+ and Repitch render
//! bit for bit what the same clip unwarped renders (no stretcher in the path), with the
//! same gain and fades. Where the mapping leaves and re-enters 1:1 mid-clip the stretcher
//! picks up again without a gap.

use crate::common;

use std::sync::Arc;

use assert_no_alloc::assert_no_alloc;
use common::*;
use ether_core::graph::{ClipContentDesc, ClipDesc, WarpDesc};
use ether_core::protocol::model::{FadeCurve, MediaId, TrackKind, Ulid, WarpMode};
use ether_core::{AudioSource, RenderGraphDesc, TransportControl, create};
use ether_stretch::{PlusStretchFactory, RustPreset, RustStretchFactory, SignalsmithFactory};

/// Samples per beat at 120 BPM / 48 kHz.
const SPB: usize = 24_000;
const MEDIA: MediaId = MediaId(Ulid(42));

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

/// Deterministic noise (every frame different, so any interpolation shows), right = left
/// scaled.
fn noise(n: usize) -> Arc<dyn AudioSource> {
    let mut x: u32 = 0x1234_5678;
    let l: Vec<f32> = (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        })
        .collect();
    let r = l.iter().map(|v| v * 0.75).collect();
    Arc::new(Stereo(l, r))
}

#[derive(Clone, Copy)]
enum Stretch {
    Signalsmith,
    Rust,
}

#[derive(Clone)]
struct Clip {
    start: f64,
    length: f64,
    offset: f64,
    looping: Option<(f64, f64)>,
    fades: (f64, f64),
    reversed: bool,
    gain: f32,
    warp: Option<WarpDesc>,
}

impl Default for Clip {
    fn default() -> Self {
        Clip {
            start: 1.0,
            length: 4.0,
            offset: 0.0,
            looping: None,
            fades: (0.5, 0.75),
            reversed: false,
            gain: 0.8,
            warp: None,
        }
    }
}

fn warp(mode: WarpMode, markers: &[(f64, f64)]) -> Option<WarpDesc> {
    Some(WarpDesc {
        mode,
        markers: markers.to_vec(),
        formants: Default::default(),
    })
}

fn desc(c: &Clip) -> ClipDesc {
    ClipDesc {
        id: cid(1),
        start: c.start,
        length: c.length,
        offset: c.offset,
        looping: c.looping,
        muted: false,
        content: ClipContentDesc::Audio {
            media: MEDIA,
            gain: c.gain,
            transpose: 0.0,
            fade_in: c.fades.0,
            fade_out: c.fades.1,
            fade_in_curve: FadeCurve::EqualPower,
            fade_out_curve: FadeCurve::Linear,
            reversed: c.reversed,
            warp: c.warp.clone(),
            gain_envelope: None,
            transpose_envelope: None,
        },
        envelopes: vec![],
    }
}

/// Render `clip` alone (left and right channels), with both stretchers available.
fn play(src: &Arc<dyn AudioSource>, clip: &Clip, stretch: Stretch, frames: usize) -> Vec<f32> {
    let mut p = create(config());
    match stretch {
        Stretch::Signalsmith => p
            .handle
            .set_stretcher_factory(Arc::new(SignalsmithFactory::default())),
        Stretch::Rust => p.handle.set_stretcher_factory(Arc::new(RustStretchFactory {
            preset: RustPreset::Web,
        })),
    }
    p.handle
        .set_plus_stretcher_factory(Arc::new(PlusStretchFactory));
    p.handle.add_source(MEDIA, src.clone()).unwrap();
    let mut t = track(tid(2), TrackKind::Audio, Some(tid(1)));
    t.clips = vec![desc(clip)];
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master(), t],
            ..Default::default()
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let mut out = Vec::with_capacity(2 * frames);
    let mut bl = vec![0.0; BLOCK];
    let mut br = vec![0.0; BLOCK];
    let mut done = 0;
    while done < frames {
        let n = BLOCK.min(frames - done);
        {
            let mut outs: [&mut [f32]; 2] = [&mut bl[..n], &mut br[..n]];
            assert_no_alloc(|| p.engine.process(&[], &mut outs, n));
        }
        out.extend_from_slice(&bl[..n]);
        out.extend_from_slice(&br[..n]);
        done += n;
    }
    out
}

fn first_diff(a: &[f32], b: &[f32]) -> Option<(usize, f32, f32)> {
    a.iter()
        .zip(b)
        .enumerate()
        .find(|(_, (x, y))| x.to_bits() != y.to_bits())
        .map(|(i, (x, y))| (i, *x, *y))
}

/// Every 1:1 mapping, played in every mode with every stretcher, renders exactly the
/// unwarped clip.
#[test]
fn unit_rate_warp_is_bit_exact_with_the_unwarped_clip() {
    let src = noise(12 * SPB);
    let frames = 7 * SPB;
    // (warp markers, the unwarped clip's offset that plays the same source).
    let cases: [(&[(f64, f64)], Clip); 5] = [
        // 2 beats = 1 s at 120 BPM.
        (&[(0.0, 0.0), (2.0, 1.0)], Clip::default()),
        // Several markers, all 1:1, source starting 0.25 s in.
        (
            &[(0.0, 0.25), (1.0, 0.75), (3.0, 1.75), (8.0, 4.25)],
            Clip {
                offset: 0.5,
                ..Default::default()
            },
        ),
        // Clip offset into the content.
        (
            &[(0.0, 0.0), (4.0, 2.0)],
            Clip {
                offset: 1.25,
                ..Default::default()
            },
        ),
        // Looped content.
        (
            &[(0.0, 0.0), (4.0, 2.0)],
            Clip {
                looping: Some((0.5, 2.0)),
                fades: (0.0, 0.0),
                ..Default::default()
            },
        ),
        // Reversed media.
        (
            &[(0.0, 0.0), (4.0, 2.0)],
            Clip {
                reversed: true,
                ..Default::default()
            },
        ),
    ];
    for (markers, plain) in cases {
        let want = play(&src, &plain, Stretch::Signalsmith, frames);
        assert!(want.iter().any(|v| v.abs() > 0.1), "the clip sounds");
        for (mode, stretch) in [
            (WarpMode::Repitch, Stretch::Signalsmith),
            (WarpMode::Complex, Stretch::Signalsmith),
            (WarpMode::Complex, Stretch::Rust),
            (WarpMode::ComplexPlus, Stretch::Signalsmith),
        ] {
            let warped = Clip {
                // The markers carry the source position: no content offset beyond the
                // plain clip's own.
                offset: plain.offset - unwarped_shift(markers),
                warp: warp(mode, markers),
                ..plain.clone()
            };
            let got = play(&src, &warped, stretch, frames);
            if let Some((i, a, b)) = first_diff(&got, &want) {
                panic!(
                    "{mode:?} markers {markers:?}: sample {i} (block-major) {a} != unwarped {b}"
                );
            }
        }
    }
}

/// Content beats the plain clip's offset is ahead of the warped one's: the first marker's
/// source position, in beats at 120 BPM.
fn unwarped_shift(markers: &[(f64, f64)]) -> f64 {
    let (b, s) = markers[0];
    s * 2.0 - b
}

/// The mapping goes 1:1 in the middle of the clip only: there the clip is the unwarped
/// source exactly (once the crossfade from the stretcher is over), and where the mapping
/// stretches again the stretcher comes back in. Both changes of path crossfade: no click
/// (sample-to-sample step of a sine never above its own) and no gap.
#[test]
fn stretcher_hands_over_to_the_straight_read_and_back_without_a_click() {
    let n = 12 * SPB;
    let sine: Vec<f32> = (0..n)
        .map(|i| (std::f64::consts::TAU * 440.0 * i as f64 / 48_000.0).sin() as f32 * 0.5)
        .collect();
    // Steepest step of the source itself.
    let src_step = sine
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0f32, f32::max);
    let src: Arc<dyn AudioSource> = Arc::new(Stereo(sine.clone(), sine));
    // Content 0..2 at 1.5x, 2..4 at 1x, 4..6 at 0.75x.
    let markers = [(0.0, 0.0), (2.0, 1.5), (4.0, 2.5), (6.0, 3.25)];
    let frames = 8 * SPB;
    let left = |v: &[f32], from: usize, to: usize| -> Vec<f32> {
        (from..to)
            .map(|i| v[(i / BLOCK) * 2 * BLOCK + i % BLOCK])
            .collect()
    };
    for (mode, stretch) in [
        (WarpMode::Complex, Stretch::Signalsmith),
        (WarpMode::Complex, Stretch::Rust),
        (WarpMode::ComplexPlus, Stretch::Signalsmith),
    ] {
        let clip = Clip {
            start: 1.0,
            length: 6.0,
            fades: (0.0, 0.0),
            gain: 1.0,
            warp: warp(mode, &markers),
            ..Default::default()
        };
        let got = play(&src, &clip, stretch, frames);
        // Unwarped: content c plays source c / 2 + 0.5 s on 2..4, i.e. offset 1 beat.
        let plain = Clip {
            offset: 1.0,
            warp: None,
            ..clip.clone()
        };
        let want = play(&src, &plain, stretch, frames);
        // Whole blocks inside timeline beats 3..5 (content 2..4), past the 5 ms crossfade,
        // play straight.
        let a = (3 * SPB).div_ceil(BLOCK) * BLOCK + 240;
        let b = (5 * SPB) / BLOCK * BLOCK - BLOCK;
        let (g, w) = (left(&got, a, b), left(&want, a, b));
        if let Some((i, x, y)) = first_diff(&g, &w) {
            panic!("{mode:?}: sample {} {x} != unwarped {y}", a + i);
        }
        // From past the clip's start (Signalsmith's seek fade-in) to before its end
        // (declick), the stretched and straight parts join without a click or a gap.
        let body = left(&got, SPB + 48 * 60, 7 * SPB - 100);
        let step = body
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0f32, f32::max);
        assert!(
            step < 1.6 * src_step,
            "{mode:?}: step {step} (source {src_step})"
        );
        let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
        for seg in body.chunks(480) {
            assert!(rms(seg) > 0.2, "{mode:?}: 10 ms rms {}", rms(seg));
        }
    }
}
