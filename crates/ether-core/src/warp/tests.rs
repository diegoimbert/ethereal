//! Offline render tests of warped playback: stretched duration and pitch, transpose,
//! seeks after locate/loop jumps, stretcher provisioning/GC, and no allocation on the
//! Complex path.

use std::cell::Cell;
use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_protocol::model::{
    BeatRange, Beats, ClipId, MediaId, TrackId, TrackKind, Ulid, WarpMode,
};
use ether_stretch::{RustPreset, RustStretchFactory, SignalsmithFactory, StretcherFactory};

use super::SEEK_FADE_MS;
use crate::graph::{ClipContentDesc, ClipDesc, TrackDesc, WarpDesc};
use crate::{AudioSource, Engine, EngineConfig, EngineParts, RenderGraphDesc, TransportControl};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

const SR: usize = 48_000;
const BLOCK: usize = 512;
/// 120 BPM at 48 kHz.
const SPB: usize = 24_000;

struct Mem {
    l: Vec<f32>,
    r: Vec<f32>,
}

impl AudioSource for Mem {
    fn channels(&self) -> u16 {
        2
    }
    fn frames(&self) -> u64 {
        self.l.len() as u64
    }
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        let src = if channel == 0 { &self.l } else { &self.r };
        for (i, o) in out.iter_mut().enumerate() {
            *o = src.get(start as usize + i).copied().unwrap_or(0.0);
        }
        true
    }
}

/// `seconds` of a sine at `freq` (amplitude 0.5), silent before `onset` seconds.
fn sine(freq: f64, seconds: f64, onset: f64) -> Arc<dyn AudioSource> {
    let n = (seconds * SR as f64) as usize;
    let on = (onset * SR as f64) as usize;
    let l: Vec<f32> = (0..n)
        .map(|i| {
            if i < on {
                0.0
            } else {
                (std::f64::consts::TAU * freq * i as f64 / SR as f64).sin() as f32 * 0.5
            }
        })
        .collect();
    Arc::new(Mem { r: l.clone(), l })
}

thread_local! {
    /// Stretcher of the tests on this thread: the C++ Signalsmith (native default) or the
    /// pure-Rust port with the web preset ([`with_rust_stretcher`]).
    static RUST: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` with engines using the pure-Rust stretcher (web preset).
fn with_rust_stretcher(f: impl FnOnce()) {
    RUST.set(true);
    f();
    RUST.set(false);
}

fn factory() -> Arc<dyn StretcherFactory> {
    if RUST.get() {
        Arc::new(RustStretchFactory {
            preset: RustPreset::Web,
        })
    } else {
        Arc::new(SignalsmithFactory::default())
    }
}

fn engine(stretch: bool) -> EngineParts {
    let mut p = crate::create(EngineConfig {
        sample_rate: SR as u32,
        max_block_size: BLOCK,
        ..Default::default()
    });
    if stretch {
        p.handle.set_stretcher_factory(factory());
    }
    p
}

fn tid(n: u128) -> TrackId {
    TrackId(Ulid(n))
}

const MEDIA: MediaId = MediaId(Ulid(77));

fn track(id: TrackId, kind: TrackKind, output: Option<TrackId>, clips: Vec<ClipDesc>) -> TrackDesc {
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id,
        kind,
        chain: vec![],
        output,
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        midi_input: Default::default(),
        monitor: false,
        armed: false,
        clips,
        automation: vec![],
        racks: Vec::new(),
        expression: Default::default(),
        hw_io: Vec::new(),
        sidechain_taps: Vec::new(),
    }
}

fn audio_clip(
    id: u128,
    start: f64,
    length: f64,
    transpose: f32,
    warp: Option<WarpDesc>,
) -> ClipDesc {
    ClipDesc {
        id: ClipId(Ulid(id)),
        start,
        length,
        offset: 0.0,
        looping: None,
        muted: false,
        content: ClipContentDesc::Audio {
            media: MEDIA,
            gain: 1.0,
            transpose,
            fade_in: 0.0,
            fade_out: 0.0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            warp,
            gain_envelope: None,
            transpose_envelope: None,
        },
        envelopes: vec![],
    }
}

fn warp(mode: WarpMode, markers: &[(f64, f64)]) -> Option<WarpDesc> {
    Some(WarpDesc {
        mode,
        markers: markers.to_vec(),
    })
}

fn publish(p: &mut EngineParts, clips: Vec<ClipDesc>) {
    let desc = RenderGraphDesc {
        version: 1,
        tracks: vec![
            track(tid(1), TrackKind::Master, None, vec![]),
            track(tid(2), TrackKind::Audio, Some(tid(1)), clips),
        ],
        ..Default::default()
    };
    p.handle.publish(desc).unwrap();
}

fn render(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut left = Vec::with_capacity(frames);
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    while left.len() < frames {
        let n = BLOCK.min(frames - left.len());
        {
            let mut outs: [&mut [f32]; 2] = [&mut l[..n], &mut r[..n]];
            assert_no_alloc(|| engine.process(&[], &mut outs, n));
        }
        left.extend_from_slice(&l[..n]);
    }
    left
}

fn rms(sig: &[f32]) -> f32 {
    (sig.iter().map(|v| v * v).sum::<f32>() / sig.len().max(1) as f32).sqrt()
}

/// Frequency from rising zero crossings.
fn freq(sig: &[f32]) -> f64 {
    let c = sig.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    c as f64 * SR as f64 / sig.len() as f64
}

fn ms(ms: f64) -> usize {
    (ms * SR as f64 / 1000.0) as usize
}

/// First sample at which the 5 ms rms exceeds `level`.
fn onset(sig: &[f32], level: f32) -> Option<usize> {
    let w = ms(5.0);
    (0..sig.len().saturating_sub(w)).find(|&i| rms(&sig[i..i + w]) > level)
}

fn play(p: &mut EngineParts, frames: usize) -> Vec<f32> {
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, frames)
}

fn assert_close(got: f64, want: f64, tol: f64, what: &str) {
    assert!(
        (got - want).abs() / want < tol,
        "{what}: got {got}, want {want}"
    );
}

/// 1 s of 440 Hz source pinned to 4 beats (240 BPM material) at 120 BPM: 2 s on the
/// timeline. Complex keeps the pitch, Repitch halves it; both last 2 s.
#[test]
fn stretched_clip_has_the_timeline_duration_and_keeps_pitch() {
    for (mode, want_hz) in [(WarpMode::Complex, 440.0), (WarpMode::Repitch, 220.0)] {
        let mut p = engine(true);
        p.handle.add_source(MEDIA, sine(440.0, 1.0, 0.0)).unwrap();
        let markers = [(0.0, 0.0), (4.0, 1.0)];
        publish(
            &mut p,
            vec![audio_clip(1, 0.0, 4.0, 0.0, warp(mode, &markers))],
        );
        let out = play(&mut p, 3 * SR);
        // Sound across the whole 2 s (4 beats) ...
        for seg in out[ms(60.0)..ms(1950.0)].chunks(ms(100.0)) {
            assert!(rms(seg) > 0.2, "{mode:?}: segment rms {}", rms(seg));
        }
        // ... and nothing after the clip end.
        assert!(rms(&out[4 * SPB + ms(2.0)..]) < 1e-6, "{mode:?}: tail");
        assert_close(
            freq(&out[ms(200.0)..ms(1800.0)]),
            want_hz,
            0.03,
            &format!("{mode:?} pitch"),
        );
    }
}

/// A clip at 1:1 speed plays each source position at its timeline time: the onset at 0.5 s
/// source comes out at 0.5 s, i.e. the stretcher latency is compensated.
#[test]
fn complex_output_is_latency_compensated() {
    let mut p = engine(true);
    p.handle.add_source(MEDIA, sine(440.0, 2.0, 0.5)).unwrap();
    let markers = [(0.0, 0.0), (2.0, 1.0)];
    publish(
        &mut p,
        vec![audio_clip(
            1,
            0.0,
            4.0,
            0.0,
            warp(WarpMode::Complex, &markers),
        )],
    );
    let out = play(&mut p, 2 * SR);
    let at = onset(&out, 0.1).expect("onset");
    let want = SR / 2;
    assert!(
        at.abs_diff(want) < ms(15.0),
        "onset at {at} ({:.1} ms off)",
        (at as f64 - want as f64) * 1000.0 / SR as f64
    );
}

#[test]
fn transpose_shifts_pitch_in_both_modes() {
    // +12 st: Complex keeps the duration and doubles the pitch; Repitch doubles the playback
    // rate (so the 1:1 clip reads the source twice as fast: 880 Hz).
    for mode in [WarpMode::Complex, WarpMode::Repitch] {
        let mut p = engine(true);
        p.handle.add_source(MEDIA, sine(440.0, 8.0, 0.0)).unwrap();
        let markers = [(0.0, 0.0), (2.0, 1.0)];
        publish(
            &mut p,
            vec![audio_clip(1, 0.0, 4.0, 12.0, warp(mode, &markers))],
        );
        let out = play(&mut p, 2 * SR + ms(100.0));
        for seg in out[ms(60.0)..ms(1950.0)].chunks(ms(100.0)) {
            assert!(rms(seg) > 0.2, "{mode:?}: segment rms {}", rms(seg));
        }
        assert!(rms(&out[2 * SR + ms(2.0)..]) < 1e-6);
        assert_close(
            freq(&out[ms(200.0)..ms(1800.0)]),
            880.0,
            0.03,
            &format!("{mode:?}"),
        );
    }
    // Unwarped clips repitch too.
    let mut p = engine(true);
    p.handle.add_source(MEDIA, sine(440.0, 8.0, 0.0)).unwrap();
    publish(&mut p, vec![audio_clip(1, 0.0, 4.0, -12.0, None)]);
    let out = play(&mut p, SR);
    assert_close(freq(&out[ms(100.0)..ms(900.0)]), 220.0, 0.03, "unwarped");
}

/// Without a stretcher factory (web) Complex plays like Repitch.
#[test]
fn complex_falls_back_to_repitch_without_a_factory() {
    let mut p = engine(false);
    assert!(!p.handle.stretching_supported());
    p.handle.add_source(MEDIA, sine(440.0, 1.0, 0.0)).unwrap();
    let markers = [(0.0, 0.0), (4.0, 1.0)];
    publish(
        &mut p,
        vec![audio_clip(
            1,
            0.0,
            4.0,
            0.0,
            warp(WarpMode::Complex, &markers),
        )],
    );
    assert_eq!(p.handle.stretcher_count(), 0);
    let out = play(&mut p, 2 * SR);
    assert_close(
        freq(&out[ms(200.0)..ms(1800.0)]),
        220.0,
        0.03,
        "fallback pitch",
    );
}

/// After a locate (and a transport loop jump) into the middle of a Complex clip, sound is
/// back within `SEEK_FADE_MS`: no gap of the ~stretcher latency.
#[test]
fn locate_and_loop_jumps_resume_immediately() {
    let mut p = engine(true);
    p.handle.add_source(MEDIA, sine(440.0, 8.0, 0.0)).unwrap();
    let markers = [(0.0, 0.0), (4.0, 1.5)]; // slightly sped up
    publish(
        &mut p,
        vec![audio_clip(
            1,
            0.0,
            16.0,
            0.0,
            warp(WarpMode::Complex, &markers),
        )],
    );
    play(&mut p, SR / 2);
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(6.0),
        })
        .unwrap();
    let out = render(&mut p.engine, SR / 2);
    let at = onset(&out, 0.2).expect("sound after locate");
    assert!(at < ms(SEEK_FADE_MS), "gap after locate: {at} samples");
    assert!(rms(&out[ms(SEEK_FADE_MS)..]) > 0.3);

    // Loop [8, 9): every wrap seeks back.
    p.handle
        .transport(TransportControl::SetLoop {
            enabled: true,
            region: BeatRange {
                start: Beats(8.0),
                end: Beats(9.0),
            },
        })
        .unwrap();
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(8.0),
        })
        .unwrap();
    let out = render(&mut p.engine, 4 * SPB);
    for wrap in 1..4 {
        let seg = &out[wrap * SPB..wrap * SPB + ms(100.0)];
        let at = onset(seg, 0.2).expect("sound after loop wrap");
        assert!(at < ms(SEEK_FADE_MS), "gap after wrap {wrap}: {at} samples");
    }
}

/// Stretchers are provisioned per track by how many Complex clips can play at once (not
/// one per clip), and given back to the handle (dropped there, never on the audio thread)
/// when no longer needed.
#[test]
fn stretchers_are_pooled_per_track_and_collected_off_the_audio_thread() {
    let mut p = engine(true);
    assert!(p.handle.stretching_supported());
    p.handle.add_source(MEDIA, sine(440.0, 1.0, 0.0)).unwrap();
    let m = [(0.0, 0.0), (4.0, 1.0)];
    // Back to back (the end of one and the start of the next can share a sub-block): 2.
    publish(
        &mut p,
        vec![
            audio_clip(1, 0.0, 4.0, 0.0, warp(WarpMode::Complex, &m)),
            audio_clip(2, 4.0, 4.0, 0.0, warp(WarpMode::Complex, &m)),
            audio_clip(3, 8.0, 4.0, 0.0, warp(WarpMode::Repitch, &m)),
            audio_clip(4, 12.0, 4.0, 0.0, None),
        ],
    );
    assert_eq!(p.handle.stretcher_count(), 2);
    render(&mut p.engine, BLOCK);
    // Far apart: one voice serves both; one stretcher comes back.
    publish(
        &mut p,
        vec![
            audio_clip(1, 0.0, 4.0, 0.0, warp(WarpMode::Complex, &m)),
            audio_clip(2, 4.0, 4.0, 0.0, warp(WarpMode::Repitch, &m)),
            audio_clip(3, 8.0, 4.0, 0.0, warp(WarpMode::Complex, &m)),
        ],
    );
    assert_eq!(p.handle.stretcher_count(), 1);
    render(&mut p.engine, BLOCK);
    assert_eq!(p.handle.collect_stretchers(), 1);
    // Many clips, at most two at once: two voices.
    let many: Vec<_> = (0..40)
        .map(|k| {
            audio_clip(
                10 + k,
                k as f64 * 6.0,
                4.0,
                0.0,
                warp(WarpMode::Complex, &m),
            )
        })
        .chain([audio_clip(99, 1.0, 2.0, 0.0, warp(WarpMode::Complex, &m))])
        .collect();
    publish(&mut p, many);
    assert_eq!(p.handle.stretcher_count(), 2);
    publish(&mut p, vec![]);
    render(&mut p.engine, BLOCK);
    assert_eq!(p.handle.stretcher_count(), 0);
    assert_eq!(p.handle.collect_stretchers(), 2);
    assert_eq!(p.engine.warp.leaked, 0);
}

/// One voice plays clip after clip (each takes it over and seeks), and a clip played at its
/// source's own speed without transpose needs no stretching: exactly the source.
#[test]
fn a_voice_moves_from_clip_to_clip_and_unit_rate_plays_straight() {
    let mut p = engine(true);
    p.handle.add_source(MEDIA, sine(440.0, 8.0, 0.0)).unwrap();
    // 2 beats = 1 s of source at 120 BPM: 1:1.
    let unit = [(0.0, 0.0), (2.0, 1.0)];
    let fast = [(0.0, 0.0), (2.0, 1.5)];
    publish(
        &mut p,
        vec![
            audio_clip(1, 0.0, 2.0, 0.0, warp(WarpMode::Complex, &fast)),
            audio_clip(2, 4.0, 2.0, 0.0, warp(WarpMode::Complex, &fast)),
            audio_clip(3, 8.0, 2.0, 0.0, warp(WarpMode::Complex, &unit)),
        ],
    );
    assert_eq!(p.handle.stretcher_count(), 1);
    let out = play(&mut p, 5 * SR + SR / 2);
    // Both stretched clips sound (pitch kept), the second right from its start.
    for (from, to) in [
        (ms(60.0), ms(950.0)),
        (2 * SR + ms(60.0), 2 * SR + ms(950.0)),
    ] {
        for seg in out[from..to].chunks(ms(100.0)) {
            assert!(rms(seg) > 0.2, "segment rms {}", rms(seg));
        }
        assert_close(freq(&out[from..to]), 440.0, 0.03, "stretched pitch");
    }
    // The 1:1 clip is the source itself (sample for sample, up to its fades).
    let src = sine(440.0, 8.0, 0.0);
    let mut want = vec![0.0f32; SR / 2];
    src.read(0, ms(100.0) as u64, &mut want);
    let got = &out[4 * SR + ms(100.0)..4 * SR + ms(100.0) + SR / 2];
    let diff = got
        .iter()
        .zip(&want)
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(
        diff < 1e-6,
        "unit-rate clip differs from its source by {diff}"
    );
}

/// The Complex path allocates nothing on the audio thread: first play (seek), steady
/// state, stretch ratio changes across markers, transpose, clip-loop wraps, locate and
/// transport loop jumps, snapshot swaps and stretcher add/remove (`render` wraps every
/// `process` in `assert_no_alloc`).
#[test]
fn complex_path_never_allocates() {
    let mut p = engine(true);
    p.handle.add_source(MEDIA, sine(330.0, 4.0, 0.0)).unwrap();
    let markers = [(0.0, 0.0), (2.0, 0.5), (4.0, 2.0), (8.0, 2.5)];
    let mut looped = audio_clip(2, 8.0, 8.0, 5.0, warp(WarpMode::Complex, &markers));
    looped.looping = Some((1.0, 3.0));
    let clips = vec![
        audio_clip(1, 0.0, 8.0, -3.0, warp(WarpMode::Complex, &markers)),
        looped,
    ];
    publish(&mut p, clips.clone());
    play(&mut p, 2 * SR);
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(3.0),
        })
        .unwrap();
    render(&mut p.engine, SR);
    publish(&mut p, clips[..1].to_vec());
    p.handle
        .transport(TransportControl::SetLoop {
            enabled: true,
            region: BeatRange {
                start: Beats(1.0),
                end: Beats(2.5),
            },
        })
        .unwrap();
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(1.0),
        })
        .unwrap();
    render(&mut p.engine, 2 * SR);
    publish(&mut p, clips);
    render(&mut p.engine, SR);
    p.handle.transport(TransportControl::Stop).unwrap();
    render(&mut p.engine, BLOCK * 4);
}

/// The web's stretcher (pure-Rust Signalsmith port, `RustPreset::Web`) passes the same
/// playback tests as the C++: duration and pitch under stretch, latency compensation,
/// transpose, jumps resuming within `SEEK_FADE_MS`, voice hand-over and no allocation.
#[test]
fn rust_stretcher_passes_the_warp_tests() {
    with_rust_stretcher(|| {
        stretched_clip_has_the_timeline_duration_and_keeps_pitch();
        complex_output_is_latency_compensated();
        transpose_shifts_pitch_in_both_modes();
        locate_and_loop_jumps_resume_immediately();
        stretchers_are_pooled_per_track_and_collected_off_the_audio_thread();
        a_voice_moves_from_clip_to_clip_and_unit_rate_plays_straight();
        complex_path_never_allocates();
    });
}

/// Past the host's voice limit, Complex clips get no stretcher and play Repitch.
#[test]
fn voice_limit_caps_stretchers() {
    with_rust_stretcher(|| {
        let mut p = engine(true);
        p.handle.set_stretch_voice_limit(1);
        p.handle.add_source(MEDIA, sine(440.0, 1.0, 0.0)).unwrap();
        let m = [(0.0, 0.0), (4.0, 1.0)];
        let desc = RenderGraphDesc {
            version: 1,
            tracks: vec![
                track(tid(1), TrackKind::Master, None, vec![]),
                track(
                    tid(2),
                    TrackKind::Audio,
                    Some(tid(1)),
                    vec![audio_clip(1, 0.0, 4.0, 0.0, warp(WarpMode::Complex, &m))],
                ),
                track(
                    tid(3),
                    TrackKind::Audio,
                    Some(tid(1)),
                    vec![audio_clip(2, 0.0, 4.0, 0.0, warp(WarpMode::Complex, &m))],
                ),
            ],
            ..Default::default()
        };
        p.handle.publish(desc.clone()).unwrap();
        assert_eq!(p.handle.stretcher_count(), 1);
        // Both clips sound: the first keeps its pitch (440 Hz), the second repitches
        // (220 Hz), with about the same amplitude.
        let out = play(&mut p, 2 * SR);
        let seg = &out[ms(200.0)..ms(1800.0)];
        let amp = |f: f64| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, &v) in seg.iter().enumerate() {
                let ph = std::f64::consts::TAU * f * i as f64 / SR as f64;
                re += v as f64 * ph.cos();
                im += v as f64 * ph.sin();
            }
            2.0 * (re * re + im * im).sqrt() / seg.len() as f64
        };
        let (a440, a220) = (amp(440.0), amp(220.0));
        assert!(a440 > 0.35 && a220 > 0.35, "440 Hz {a440}, 220 Hz {a220}");
        p.handle.set_stretch_voice_limit(8);
        p.handle.publish(desc).unwrap();
        assert_eq!(p.handle.stretcher_count(), 2);
    });
}

/// Shared vectors (`vectors.json`, also run by ether-controller and the UI): the engine's
/// content beat → source seconds mapping, per mode, transpose and stretcher availability.
#[test]
fn shared_mapping_vectors() {
    use super::repitch_source_seconds;
    use crate::sched::source_seconds;
    let doc: serde_json::Value = serde_json::from_str(include_str!("vectors.json")).unwrap();
    let f = |v: &serde_json::Value| v.as_f64().unwrap();
    for case in doc["vectors"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mode = if case["warp"]["mode"] == "Complex" {
            WarpMode::Complex
        } else {
            WarpMode::Repitch
        };
        let desc = case["compiled"].as_array().map(|pins| WarpDesc {
            mode,
            markers: pins.iter().map(|p| (f(&p[0]), f(&p[1]))).collect(),
        });
        let stretch = case["stretch"].as_bool().unwrap();
        let (ref_bpm, anchor) = (f(&case["ref_bpm"]), f(&case["anchor"]));
        let transpose = f(&case["transpose"]) as f32;
        let complex = matches!(&desc, Some(d) if d.mode == WarpMode::Complex) && stretch;
        for p in case["points"].as_array().unwrap() {
            let (c, want) = (f(&p[0]), f(&p[1]));
            let got = if complex {
                source_seconds(desc.as_ref(), ref_bpm, c)
            } else {
                repitch_source_seconds(desc.as_ref(), transpose, ref_bpm, anchor, c)
            };
            assert!((got - want).abs() < 1e-9, "{name}: c={c}: {got} vs {want}");
        }
    }
}

// --- Clip gain / transposition envelopes ------------------------------------------------------

use crate::graph::{ClipParamEnvelope, ParamMapping, TransposeEnvelope};
use ether_protocol::devices::ParamScale;
use ether_protocol::model::CurveShape;

/// ± 48 semitones, linear (the controller's `CLIP_TRANSPOSE_MAPPING`).
const TRANSPOSE_MAP: ParamMapping = ParamMapping {
    min: -48.0,
    max: 48.0,
    scale: ParamScale::Linear,
    steps: None,
};

/// Normalized value of `st` semitones on [`TRANSPOSE_MAP`].
fn st(st: f64) -> f64 {
    (st + 48.0) / 96.0
}

/// A step envelope: `values` (plain, via `norm`) held from each beat.
fn steps(at: &[(f64, f64)], mapping: ParamMapping) -> ClipParamEnvelope {
    ClipParamEnvelope {
        points: at.iter().map(|&(t, v)| (t, v, CurveShape::Step)).collect(),
        mapping,
    }
}

fn with_transpose_env(mut clip: ClipDesc, env: ClipParamEnvelope) -> ClipDesc {
    let (offset, end) = (clip.offset, clip.offset + clip.length);
    if let ClipContentDesc::Audio {
        transpose,
        warp,
        transpose_envelope,
        ..
    } = &mut clip.content
    {
        let rate = super::RateIntegral::build(warp.as_ref(), *transpose, &env, offset, 0.0, end);
        *transpose_envelope = Some(TransposeEnvelope { env, rate });
    }
    clip
}

#[test]
fn transpose_envelope_shifts_pitch_where_it_changes() {
    // 0 st for the first 2 beats (1 s), then +12 st: 440 Hz then 880 Hz, in all three
    // playback paths. Complex keeps the timing; Repitch reads faster after the change.
    let markers = [(0.0, 0.0), (2.0, 1.0)];
    for (what, w) in [
        ("complex", warp(WarpMode::Complex, &markers)),
        ("repitch", warp(WarpMode::Repitch, &markers)),
        ("unwarped", None),
    ] {
        let mut p = engine(true);
        p.handle.add_source(MEDIA, sine(440.0, 8.0, 0.0)).unwrap();
        let env = steps(&[(0.0, st(0.0)), (2.0, st(12.0))], TRANSPOSE_MAP);
        publish(
            &mut p,
            vec![with_transpose_env(audio_clip(1, 0.0, 4.0, 0.0, w), env)],
        );
        let out = play(&mut p, 2 * SR);
        assert_close(freq(&out[ms(150.0)..ms(900.0)]), 440.0, 0.03, what);
        assert_close(freq(&out[ms(1150.0)..ms(1900.0)]), 880.0, 0.03, what);
    }
}

#[test]
fn transpose_envelope_adds_to_the_clip_transpose() {
    // Clip at -12 st, envelope +12 st: back to the source pitch.
    let mut p = engine(true);
    p.handle.add_source(MEDIA, sine(440.0, 8.0, 0.0)).unwrap();
    let env = steps(&[(0.0, st(12.0))], TRANSPOSE_MAP);
    publish(
        &mut p,
        vec![with_transpose_env(
            audio_clip(1, 0.0, 4.0, -12.0, None),
            env,
        )],
    );
    let out = play(&mut p, SR);
    assert_close(freq(&out[ms(100.0)..ms(900.0)]), 440.0, 0.03, "sum");
}

#[test]
fn rate_integral_matches_constant_transpose() {
    // A constant envelope is a constant transpose: the same mapping as
    // `repitch_source_seconds`, around the anchor, inside and outside the table.
    let w = WarpDesc {
        mode: WarpMode::Repitch,
        markers: vec![(0.0, 0.0), (2.0, 1.0), (6.0, 1.5)],
    };
    let env = steps(&[(0.0, st(5.0))], TRANSPOSE_MAP);
    for warp in [Some(&w), None] {
        let rate = super::RateIntegral::build(warp, 2.0, &env, 1.0, 0.0, 8.0);
        for c in [-1.0, 0.0, 0.5, 1.0, 2.5, 5.0, 7.9, 12.0] {
            let want = super::repitch_source_seconds(warp, 7.0, 100.0, 1.0, c);
            let got = rate.seconds(warp, 2.0, &env, 100.0, 1.0, c);
            assert!((got - want).abs() < 1e-9, "{c}: {got} vs {want}");
        }
    }
}

#[test]
fn rate_integral_is_clamped_to_the_transpose_range() {
    // +40 st clip + 40 st envelope: clamped to +48.
    let env = steps(&[(0.0, st(40.0))], TRANSPOSE_MAP);
    let rate = super::RateIntegral::build(None, 40.0, &env, 0.0, 0.0, 4.0);
    let got = rate.seconds(None, 40.0, &env, 60.0, 0.0, 1.0);
    assert!((got - 16.0).abs() < 1e-9, "{got}");
}

#[test]
fn gain_envelope_scales_the_clip() {
    // Silence (the fader law's bottom) for 2 beats, then -6 dB: silent, then half level.
    let mut p = engine(true);
    p.handle.add_source(MEDIA, sine(440.0, 8.0, 0.0)).unwrap();
    let fader = ParamMapping {
        min: -144.0,
        max: 6.0,
        scale: ParamScale::Fader,
        steps: None,
    };
    let minus6 = crate::automation::to_plain(&fader, 0.0);
    assert_eq!(minus6, -144.0);
    // Normalized of -6 dB on the fader law: search it (the law isn't inverted in core).
    let n6 = (0..=10_000)
        .map(|i| i as f64 / 10_000.0)
        .min_by(|a, b| {
            let da = (crate::automation::to_plain(&fader, *a) + 6.0).abs();
            let db = (crate::automation::to_plain(&fader, *b) + 6.0).abs();
            da.total_cmp(&db)
        })
        .unwrap();
    let mut clip = audio_clip(1, 0.0, 4.0, 0.0, None);
    if let ClipContentDesc::Audio { gain_envelope, .. } = &mut clip.content {
        *gain_envelope = Some(steps(&[(0.0, 0.0), (2.0, n6)], fader));
    }
    publish(&mut p, vec![clip]);
    let out = play(&mut p, 2 * SR);
    assert!(rms(&out[ms(50.0)..ms(950.0)]) < 1e-6);
    // A 0.5 sine has an rms of 0.354; -6 dB halves it.
    assert_close(
        rms(&out[ms(1100.0)..ms(1900.0)]) as f64,
        0.354 / 2.0,
        0.03,
        "-6 dB",
    );
}
