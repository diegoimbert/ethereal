//! [`PlusStretcher`] tests: the latency contract (output `o` is input `src(o)`), pitch kept
//! under stretch, transposition, formant preservation, transients, seek, no allocation.

use assert_no_alloc::assert_no_alloc;

use super::*;

// The crate's test allocator (`AllocDisabler`) is declared in `rust/tests.rs`.

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

fn sine(freq: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * 0.5)
        .collect()
}

/// Plays `input` (mono, or stereo pairs) at `ratio` (input/output) as the engine does: a seek
/// with the pre-roll, then blocks fed up to `src(o + out_lat) + in_lat`, `src(o) = o·ratio`.
/// Output sample `o` corresponds to input sample `o·ratio`.
pub(crate) fn drive(
    st: &mut dyn Stretcher,
    input: &[Vec<f32>],
    ratio: f64,
    semitones: f32,
) -> Vec<Vec<f32>> {
    let chans = input.len();
    st.configure(chans, SR, BLOCK);
    st.set_transpose_semitones(semitones);
    let (in_lat, out_lat) = (st.input_latency() as f64, st.output_latency() as f64);
    let len = input[0].len();
    let out_total = (len as f64 / ratio) as usize;
    let at = |i: i64| -> Vec<f32> {
        input
            .iter()
            .map(|c| {
                if i >= 0 && (i as usize) < len {
                    c[i as usize]
                } else {
                    0.0
                }
            })
            .collect()
    };
    let read = |from: i64, to: i64| -> Vec<Vec<f32>> {
        let mut v = vec![Vec::with_capacity((to - from).max(0) as usize); chans];
        for i in from..to {
            for (ch, s) in at(i).into_iter().enumerate() {
                v[ch].push(s);
            }
        }
        v
    };
    let feed = |o: f64| ((o + out_lat) * ratio + in_lat).floor() as i64;
    let mut fed = feed(0.0);
    let pre = read(fed - (SR as i64 / 2), fed);
    let pre_refs: Vec<&[f32]> = pre.iter().map(|c| c.as_slice()).collect();
    st.seek(&pre_refs, ratio);
    let mut out = vec![Vec::with_capacity(out_total); chans];
    let mut bufs = vec![vec![0.0f32; BLOCK]; chans];
    let mut o = 0usize;
    while o < out_total {
        let n = BLOCK.min(out_total - o);
        let target = feed((o + n) as f64);
        let src = read(fed, target);
        let refs: Vec<&[f32]> = src.iter().map(|c| c.as_slice()).collect();
        let mut outs: Vec<&mut [f32]> = bufs.iter_mut().map(|b| &mut b[..n]).collect();
        st.process(&refs, (target - fed) as usize, &mut outs, n);
        for (ch, b) in bufs.iter().enumerate() {
            out[ch].extend_from_slice(&b[..n]);
        }
        fed = target;
        o += n;
    }
    out
}

fn rms(sig: &[f32]) -> f32 {
    (sig.iter().map(|v| v * v).sum::<f32>() / sig.len().max(1) as f32).sqrt()
}

/// Dominant frequency over the steady middle (Goertzel scan, 1 Hz then 0.1 Hz).
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
    let coarse = (40..8000)
        .step_by(5)
        .map(|f| f as f32)
        .max_by(|a, b| power(*a).total_cmp(&power(*b)))
        .unwrap();
    ((coarse - 5.0) as i32 * 10..=(coarse + 5.0) as i32 * 10)
        .map(|f| f as f32 / 10.0)
        .max_by(|a, b| power(*a).total_cmp(&power(*b)))
        .unwrap()
}

fn check(freq: f32, ratio: f64, semitones: f32) {
    let input = sine(freq, 3 * SR as usize);
    let mut st = PlusStretcher::new();
    // A lone sine has no formants to keep.
    st.set_formants(Formants {
        amount: 0.0,
        ..Formants::default()
    });
    let out = drive(&mut st, std::slice::from_ref(&input), ratio, semitones);
    let out = &out[0];
    assert_eq!(out.len(), (input.len() as f64 / ratio) as usize);
    let want = freq * (semitones / 12.0).exp2();
    let got = dominant_freq(out);
    assert!(
        (got - want).abs() < want * 0.004 + 0.5,
        "ratio {ratio} st {semitones}: {got} Hz, want {want}"
    );
    let steady = &out[out.len() / 4..out.len() * 3 / 4];
    let level = rms(steady) / rms(&input);
    assert!(
        (0.8..1.25).contains(&level),
        "ratio {ratio} st {semitones}: level {level}"
    );
}

#[test]
fn keeps_pitch_across_ratios() {
    for ratio in [0.25, 0.5, 0.8, 1.0, 1.5, 2.0, 4.0] {
        for f in [110.0, 440.0, 2500.0, 7000.0] {
            check(f, ratio, 0.0);
        }
    }
}

#[test]
fn transposes() {
    for st in [-24.0, -7.0, -1.0, 3.0, 12.0, 24.0] {
        check(220.0, 1.0, st);
        check(330.0, 0.7, st);
    }
}

/// At ratio 1 without transposition the output reproduces the input exactly (aligned:
/// output `o` = input `o`).
#[test]
fn unity_is_transparent_and_aligned() {
    let mut seed = 7u32;
    let noise: Vec<f32> = (0..SR as usize * 2)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.5
        })
        .collect();
    let mut st = PlusStretcher::new();
    let out = drive(&mut st, &[noise.clone(), noise.clone()], 1.0, 0.0);
    let (a, b) = (&out[0][4800..90_000], &noise[4800..90_000]);
    let err: f32 = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>();
    let sig: f32 = b.iter().map(|y| y * y).sum();
    let snr = 10.0 * (sig / err.max(1e-20)).log10();
    assert!(snr > 60.0, "unity SNR {snr} dB");
}

/// At other ratios the output stays aligned with the time map: a click at input `t` comes
/// out at `t / ratio`.
#[test]
fn clicks_land_on_the_time_map() {
    for ratio in [0.5, 1.0, 1.7] {
        let mut x = vec![0.0f32; SR as usize * 2];
        let hits = [24_000usize, 52_000, 80_000];
        for &h in &hits {
            for i in 0..200 {
                x[h + i] = 0.8 * (-(i as f32) / 40.0).exp() * if i % 2 == 0 { 1.0 } else { -1.0 };
            }
        }
        let mut st = PlusStretcher::new();
        let out = drive(&mut st, &[x], ratio, 0.0);
        for &h in &hits {
            let want = (h as f64 / ratio) as i64;
            let w = &out[0][(want - 2400) as usize..(want + 2400) as usize];
            let peak = w
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .unwrap()
                .0 as i64
                + want
                - 2400;
            assert!(
                (peak - want).abs() < 200,
                "ratio {ratio}: click at {peak}, want {want}"
            );
        }
    }
}

#[test]
fn seek_primes_output() {
    let input = sine(440.0, SR as usize);
    let mut st = PlusStretcher::new();
    st.configure(1, SR, BLOCK);
    st.seek(&[&input[..20_000]], 1.0);
    let mut out = vec![0.0; BLOCK];
    let n_in = BLOCK;
    st.process(
        &[&input[20_000..20_000 + n_in]],
        n_in,
        &mut [&mut out],
        BLOCK,
    );
    assert!(rms(&out) > 0.25, "primed output rms {}", rms(&out));
}

#[test]
fn rt_calls_do_not_allocate() {
    let input = sine(440.0, 4 * SR as usize);
    let mut st = PlusStretchFactory.create();
    st.configure(2, SR, BLOCK);
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    assert_no_alloc(|| {
        st.set_transpose_semitones(5.0);
        st.set_formants(Formants {
            amount: 1.0,
            shift_semitones: 2.0,
            envelope: 64.0,
        });
        st.seek(&[&input[..20_000], &input[..20_000]], 1.3);
        let mut pos = 20_000;
        for k in 0..300 {
            let n_in = [128, 166, 90, 0, 400, 1100][k % 6];
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
    let mut st = PlusStretcher::new();
    let mut out = vec![1.0; 64];
    st.process(&[&[0.5; 64]], 64, &mut [&mut out], 64);
    assert!(out.iter().all(|v| *v == 0.0));
}

#[test]
fn silence_in_silence_out() {
    let mut st = PlusStretcher::new();
    let out = drive(&mut st, &[vec![0.0; 48_000]], 0.6, 7.0);
    assert!(out[0].iter().all(|v| *v == 0.0));
}

#[test]
fn latencies() {
    let mut st = PlusStretcher::new();
    st.configure(2, SR, BLOCK);
    assert_eq!(
        (st.input_latency(), st.output_latency()),
        (2048 + 1536, 2048)
    );
    st.configure(2, 44_100.0, BLOCK);
    assert_eq!(
        (st.input_latency(), st.output_latency()),
        (2048 + 1536, 2048)
    );
    st.configure(2, 96_000.0, BLOCK);
    assert_eq!(
        (st.input_latency(), st.output_latency()),
        (4096 + 3072, 4096)
    );
}
