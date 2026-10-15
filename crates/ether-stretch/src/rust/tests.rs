//! [`RustStretcher`] tests: time ratio, pitch under stretch, transpose, seek/reset
//! continuity, no allocation in the RT calls, parity with the C++ Signalsmith (native).

use assert_no_alloc::assert_no_alloc;

use super::*;

#[cfg(debug_assertions)]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

fn sine(freq: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * 0.5)
        .collect()
}

/// Streams `input` (mono) through `st` block by block at `ratio` (input/output) using only
/// the trait; returns the output with the output latency trimmed.
fn stretch(st: &mut dyn Stretcher, input: &[f32], ratio: f64, semitones: f32) -> Vec<f32> {
    st.configure(1, SR, BLOCK);
    st.set_transpose_semitones(semitones);
    let out_total = (input.len() as f64 / ratio).round() as usize;
    let lat = st.output_latency();
    let mut out = Vec::with_capacity(out_total + lat);
    let mut block = vec![0.0f32; BLOCK];
    let zeros = vec![0.0f32; BLOCK * 8];
    let (mut produced, mut consumed) = (0usize, 0usize);
    while produced < out_total + lat {
        let n_out = BLOCK.min(out_total + lat - produced);
        let target_in = ((produced + n_out) as f64 * ratio).round() as usize;
        let n_in = target_in - consumed;
        let src: &[f32] = if consumed + n_in <= input.len() {
            &input[consumed..consumed + n_in]
        } else {
            &zeros[..n_in]
        };
        st.process(&[src], n_in, &mut [&mut block[..n_out]], n_out);
        out.extend_from_slice(&block[..n_out]);
        produced += n_out;
        consumed += n_in;
    }
    out.drain(..lat);
    out.truncate(out_total);
    out
}

/// Dominant frequency over the steady middle: the strongest of a 1 Hz-step DFT scan
/// (Goertzel) between 50 Hz and 4 kHz.
fn dominant_freq(sig: &[f32]) -> f32 {
    let mid = &sig[sig.len() / 4..sig.len() * 3 / 4];
    let power = |f: f32| {
        let w = std::f32::consts::TAU * f / SR;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &x in mid {
            let s = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        s1 * s1 + s2 * s2 - coeff * s1 * s2
    };
    // Coarse scan, then refine around the best.
    let coarse = (50..4000)
        .step_by(5)
        .map(|f| f as f32)
        .max_by(|a, b| power(*a).total_cmp(&power(*b)))
        .unwrap();
    ((coarse - 5.0) as i32 * 10..=(coarse + 5.0) as i32 * 10)
        .map(|f| f as f32 / 10.0)
        .max_by(|a, b| power(*a).total_cmp(&power(*b)))
        .unwrap()
}

fn rms(sig: &[f32]) -> f32 {
    (sig.iter().map(|v| v * v).sum::<f32>() / sig.len().max(1) as f32).sqrt()
}

fn check(preset: RustPreset, ratio: f64, semitones: f32) {
    let f0 = 440.0;
    let input = sine(f0, SR as usize); // 1 s
    let mut st = RustStretcher::new(preset);
    let out = stretch(&mut st, &input, ratio, semitones);
    assert_eq!(out.len(), (input.len() as f64 / ratio).round() as usize);
    // Energy across the whole stretched duration (not silent or cut short).
    let q = out.len() / 8;
    for seg in out[q..out.len() - q].chunks(q) {
        assert!(rms(seg) > 0.15, "ratio {ratio}: segment rms {}", rms(seg));
    }
    let want = f0 * 2f32.powf(semitones / 12.0);
    let got = dominant_freq(&out);
    assert!(
        (got - want).abs() / want < 0.01,
        "{preset:?} ratio {ratio} st {semitones}: {got} Hz vs {want} Hz"
    );
}

#[test]
fn time_ratio_is_honoured_and_pitch_kept_at_half_speed() {
    check(RustPreset::Web, 0.5, 0.0);
    check(RustPreset::Default, 0.5, 0.0);
}

#[test]
fn pitch_kept_at_double_speed() {
    check(RustPreset::Web, 2.0, 0.0);
    check(RustPreset::Default, 2.0, 0.0);
}

#[test]
fn transpose_shifts_pitch_without_changing_duration() {
    check(RustPreset::Web, 1.0, 7.0);
    check(RustPreset::Web, 1.0, -12.0);
    check(RustPreset::Default, 1.0, 12.0);
}

#[test]
fn transpose_while_stretching() {
    check(RustPreset::Web, 0.75, -7.0);
    check(RustPreset::Web, 1.5, 3.0);
}

#[test]
fn very_slow_stretch_keeps_pitch() {
    // Past 2x, phases are partly randomised.
    check(RustPreset::Web, 0.3, 0.0);
}

/// Largest sample-to-sample step; a 440 Hz sine at 0.5 never steps more than ~0.029.
fn max_step(sig: &[f32]) -> f32 {
    sig.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// Plays `blocks` 128-frame stereo blocks of `input` from `pos` at ratio 1.
fn play(st: &mut dyn Stretcher, input: &[f32], pos: &mut usize, blocks: usize, out: &mut Vec<f32>) {
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    for _ in 0..blocks {
        let src = &input[*pos..*pos + BLOCK];
        st.process(&[src, src], BLOCK, &mut [&mut l, &mut r], BLOCK);
        // Identical channels stay (nearly) identical: phase-locked to the louder one.
        assert!(l.iter().zip(&r).all(|(a, b)| (a - b).abs() < 1e-3));
        out.extend_from_slice(&l);
        *pos += BLOCK;
    }
}

/// Plays 100 blocks, jumps (seek) to another phase of the sine and plays 100 more, then
/// resets + seeks from silence and plays 100 more. Returns the three outputs.
fn jumps(st: &mut dyn Stretcher) -> [Vec<f32>; 3] {
    let input = sine(440.0, 2 * SR as usize);
    st.configure(2, SR, BLOCK);
    let mut outs: [Vec<f32>; 3] = Default::default();
    let mut pos = 0;
    play(st, &input, &mut pos, 100, &mut outs[0]);
    pos = 30_011;
    st.seek(&[&input[..pos], &input[..pos]], 1.0);
    play(st, &input, &mut pos, 100, &mut outs[1]);
    st.reset();
    pos = 7_777;
    st.seek(&[&input[..pos], &input[..pos]], 1.0);
    play(st, &input, &mut pos, 100, &mut outs[2]);
    outs
}

#[test]
fn seek_and_reset_do_not_click() {
    // Natural largest step of the 440 Hz sine at 0.5, with 30% slack for the fade-in.
    let limit = 0.5 * std::f32::consts::TAU * 440.0 / SR * 1.3;
    for preset in [RustPreset::Web, RustPreset::Default] {
        let outs = jumps(&mut RustStretcher::new(preset));
        for (k, out) in outs.iter().enumerate() {
            assert!(
                max_step(out) < limit,
                "{preset:?} part {k}: step {} >= {limit}",
                max_step(out)
            );
            // Primed playback is audible soon after each jump.
            let tail = &out[out.len() - 50 * BLOCK..];
            assert!(rms(tail) > 0.3, "{preset:?} part {k}: rms {}", rms(tail));
        }
    }
    // Signalsmith's `presetCheaper` steps at its first output after a reset (the C++ too):
    // why the web uses `Web`.
    let outs = jumps(&mut RustStretcher::new(RustPreset::Cheaper));
    assert!(max_step(&outs[2]) > limit * 2.0);
    #[cfg(all(feature = "signalsmith", not(target_arch = "wasm32")))]
    {
        use crate::signalsmith::{Preset, SignalsmithStretcher};
        let cpp = jumps(&mut SignalsmithStretcher::new(Preset::Cheaper));
        assert!((max_step(&cpp[2]) - max_step(&outs[2])).abs() < 1e-4);
    }
}

#[test]
fn fresh_voice_seeks_cleanly_anywhere() {
    // The engine's path: a configured stretcher, a seek, then playback (maybe transposed).
    let input = sine(440.0, 2 * SR as usize);
    for (k, pos0) in [1usize, 777, 4_800, 12_345, 30_011].into_iter().enumerate() {
        let semis = [0.0, 3.0, -5.0, 7.0, 0.0][k];
        let f = 440.0 * 2f32.powf(semis / 12.0);
        // Natural largest step of the (transposed) sine, with 40% slack.
        let limit = 0.5 * std::f32::consts::TAU * f / SR * 1.4;
        for preset in [RustPreset::Web, RustPreset::Default] {
            let mut st = RustStretcher::new(preset);
            st.configure(2, SR, BLOCK);
            st.set_transpose_semitones(semis);
            let mut pos = pos0;
            st.seek(&[&input[..pos], &input[..pos]], 1.0);
            let mut out = vec![];
            play(&mut st, &input, &mut pos, 150, &mut out);
            let step = max_step(&out);
            assert!(
                step < limit,
                "{preset:?} pos {pos0} st {semis}: {step} >= {limit}"
            );
            let tail = &out[out.len() - 50 * BLOCK..];
            assert!(rms(tail) > 0.3, "{preset:?} pos {pos0}: rms {}", rms(tail));
        }
    }
}

#[test]
fn seek_primes_output() {
    let input = sine(440.0, SR as usize);
    let first_blocks = |seek: bool| {
        let mut st = RustStretcher::new(RustPreset::Web);
        st.configure(1, SR, BLOCK);
        assert!(st.input_latency() > 0 && st.output_latency() > 0);
        let pos = SR as usize / 2;
        if seek {
            st.seek(&[&input[..pos]], 1.0);
        }
        let mut out = vec![0.0; BLOCK];
        (0..60)
            .map(|b| {
                let src = &input[pos + b * BLOCK..pos + (b + 1) * BLOCK];
                st.process(&[src], BLOCK, &mut [&mut out], BLOCK);
                rms(&out)
            })
            .collect::<Vec<_>>()
    };
    let primed = first_blocks(true);
    let cold = first_blocks(false);
    // Primed: audible after about one interval (33 ms = 13 blocks) of split latency + a
    // short fade-in. Cold: silent for about the output latency (~90 ms).
    assert!(primed[30..].iter().all(|&r| r > 0.3), "primed {primed:?}");
    assert!(cold[..25].iter().all(|&r| r < 0.01), "cold {cold:?}");
}

#[test]
fn latencies_match_signalsmith_presets() {
    let lat = |p| {
        let mut st = RustStretcher::new(p);
        st.configure(2, SR, BLOCK);
        (st.input_latency(), st.output_latency())
    };
    assert_eq!(lat(RustPreset::DefaultUnsplit), (2880, 2880));
    assert_eq!(lat(RustPreset::Default), (2880, 2880 + 1440));
    assert_eq!(lat(RustPreset::Cheaper), (2400, 2400 + 1920));
    assert_eq!(lat(RustPreset::Web), (2400, 2400 + 1600));
}

#[test]
fn rt_calls_do_not_allocate() {
    let input = sine(440.0, 4 * SR as usize);
    let mut st = RustStretchFactory::default().create();
    st.configure(2, SR, BLOCK);
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    assert_no_alloc(|| {
        st.set_transpose_semitones(5.0);
        st.seek(&[&input[..20_000], &input[..20_000]], 1.3);
        let mut pos = 20_000;
        for k in 0..200 {
            // Varying ratios, a mono input into a stereo stretcher, a reset.
            let n_in = [128, 166, 90, 0, 400][k % 5];
            let src = &input[pos..pos + n_in];
            if k % 7 == 0 {
                st.process(&[src], n_in, &mut [&mut l, &mut r], BLOCK);
            } else {
                st.process(&[src, src], n_in, &mut [&mut l, &mut r], BLOCK);
            }
            pos += n_in;
            if k == 100 {
                st.reset();
            }
        }
    });
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

#[test]
fn unconfigured_outputs_silence() {
    let mut st = RustStretchFactory::default().create();
    let mut out = vec![1.0; 16];
    st.process(&[&[0.5; 16]], 16, &mut [&mut out], 16);
    assert!(out.iter().all(|&v| v == 0.0));
    assert_eq!(st.input_latency(), 0);
}

#[test]
fn silence_in_silence_out() {
    let mut st = RustStretcher::default();
    st.configure(2, SR, BLOCK);
    let zeros = vec![0.0; 256];
    let mut l = vec![1.0; BLOCK];
    let mut r = vec![1.0; BLOCK];
    for _ in 0..400 {
        st.process(&[&zeros, &zeros], 200, &mut [&mut l, &mut r], BLOCK);
        assert!(l.iter().chain(&r).all(|v| v.abs() < 1e-6));
    }
}

/// Parity with the C++ Signalsmith (same preset, same input, same calls).
#[cfg(all(feature = "signalsmith", not(target_arch = "wasm32")))]
mod parity {
    use super::*;
    use crate::signalsmith::{Preset, SignalsmithStretcher};

    /// Normalised correlation of two signals.
    fn correlation(a: &[f32], b: &[f32]) -> f64 {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let na: f64 = a.iter().map(|x| *x as f64 * *x as f64).sum();
        let nb: f64 = b.iter().map(|x| *x as f64 * *x as f64).sum();
        dot / (na * nb).sqrt().max(1e-30)
    }

    /// Magnitude spectrum (Hann, 4096) averaged over the signal, in 64 log-spaced bands.
    fn band_spectrum(sig: &[f32]) -> Vec<f64> {
        let n = 4096;
        let mut planner = rustfft::FftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(n);
        let mut acc = vec![0.0f64; n / 2];
        for frame in sig.chunks_exact(n).step_by(1) {
            let mut buf: Vec<_> = frame
                .iter()
                .enumerate()
                .map(|(i, &x)| {
                    let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
                    rustfft::num_complex::Complex64::new(x as f64 * w, 0.0)
                })
                .collect();
            fft.process(&mut buf);
            for (a, c) in acc.iter_mut().zip(&buf) {
                *a += c.norm();
            }
        }
        let mut bands = vec![0.0; 64];
        for (k, a) in acc.iter().enumerate().skip(1) {
            let b = ((k as f64).ln() / ((n / 2) as f64).ln() * 64.0) as usize;
            bands[b.min(63)] += a;
        }
        bands.iter().map(|b| (b + 1e-9).log10() * 20.0).collect()
    }

    /// A chord with vibrato plus decaying clicks (tonal + transient content).
    fn music(frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                let t = i as f32 / SR;
                let vib = 1.0 + 0.01 * (t * 5.0 * std::f32::consts::TAU).sin();
                let tone: f32 = [220.0, 277.18, 329.63, 880.0]
                    .iter()
                    .map(|f| (std::f32::consts::TAU * f * vib * t).sin())
                    .sum::<f32>()
                    * 0.12;
                let beat = (t * 4.0).fract();
                let click =
                    (-beat * 60.0).exp() * ((i * 2_654_435_761) % 1000) as f32 / 1000.0 * 0.3;
                tone + click
            })
            .collect()
    }

    fn compare(ratio: f64, semitones: f32) {
        let input = music(2 * SR as usize);
        let rust = stretch(
            &mut RustStretcher::new(RustPreset::Default),
            &input,
            ratio,
            semitones,
        );
        let cpp = stretch(
            &mut SignalsmithStretcher::new(Preset::Default),
            &input,
            ratio,
            semitones,
        );
        assert_eq!(rust.len(), cpp.len());
        let corr = correlation(&rust, &cpp);
        let (sr, sc) = (band_spectrum(&rust), band_spectrum(&cpp));
        let max_db = sr
            .iter()
            .zip(&sc)
            .skip(8) // below ~30 Hz: nothing there
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        println!("ratio {ratio} st {semitones}: corr {corr:.5}, max band diff {max_db:.2} dB");
        assert!(
            max_db < 0.1,
            "ratio {ratio} st {semitones}: band diff {max_db} dB"
        );
        assert!(corr > 0.999, "ratio {ratio} st {semitones}: corr {corr}");
    }

    #[test]
    fn matches_cpp_signalsmith() {
        compare(1.0, 0.0);
        compare(0.8, 0.0);
        compare(1.25, 0.0);
        compare(1.0, 5.0);
        compare(0.9, -3.0);
    }
}

/// Per-block cost of one stereo voice (128 frames at 48 kHz, ratio 1.1, transposed).
/// Ignored; run in release with `--ignored --nocapture`.
#[test]
#[ignore = "perf harness"]
fn block_costs() {
    let input = sine(440.0, 8 * SR as usize);
    let run = |name: &str, st: &mut dyn Stretcher| {
        st.configure(2, SR, BLOCK);
        st.set_transpose_semitones(2.0);
        let mut l = vec![0.0; BLOCK];
        let mut r = vec![0.0; BLOCK];
        let n_in = BLOCK * 11 / 10;
        let mut times = vec![];
        let mut pos = 0;
        while pos + n_in <= input.len() {
            let src = &input[pos..pos + n_in];
            let t = std::time::Instant::now();
            st.process(&[src, src], n_in, &mut [&mut l, &mut r], BLOCK);
            times.push(t.elapsed().as_nanos() as f64);
            pos += n_in;
        }
        times.sort_by(f64::total_cmp);
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let p99 = times[times.len() * 99 / 100];
        let max = times[times.len() - 1];
        println!(
            "{name}: mean {mean:.0} ns, p99 {p99:.0} ns, max {max:.0} ns per 128-frame stereo block"
        );
    };
    run("rust Web", &mut RustStretcher::new(RustPreset::Web));
    run("rust Cheaper", &mut RustStretcher::new(RustPreset::Cheaper));
    run("rust Default", &mut RustStretcher::new(RustPreset::Default));
    run(
        "rust DefaultUnsplit",
        &mut RustStretcher::new(RustPreset::DefaultUnsplit),
    );
    #[cfg(all(feature = "signalsmith", not(target_arch = "wasm32")))]
    {
        use crate::signalsmith::{Preset, SignalsmithStretcher};
        run(
            "c++ Cheaper",
            &mut SignalsmithStretcher::new(Preset::Cheaper),
        );
        run(
            "c++ Default",
            &mut SignalsmithStretcher::new(Preset::Default),
        );
    }
}
