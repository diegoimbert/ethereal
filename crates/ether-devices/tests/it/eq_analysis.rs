//! The EQ's pre/post spectrum analysis (CONTRACTS.md §12.15): frames are
//! published only while watched, both stages in the same pass, and neither `process` nor
//! `analysis` allocates (`assert_no_alloc`).

use assert_no_alloc::assert_no_alloc;
use ether_core::analysis::{
    ANALYSIS_FRAMES_PER_PASS, ANALYSIS_MAX_VALUES, AnalysisFrame, AnalysisKind, AnalysisSink,
};
use ether_core::{
    AudioBuffers, Device, EventBuffer, Node, PrepareConfig, ProcessContext, TransportInfo,
};
use ether_devices::eq::{self, spectrum};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn prepared() -> eq::Eq {
    let mut d = eq::Eq::new();
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

/// Process `blocks` blocks of a stereo sine (`freq` Hz, `amp`) under `assert_no_alloc`.
fn run(d: &mut eq::Eq, freq: f32, amp: f32, blocks: usize, phase: &mut usize) {
    let mut out_events = EventBuffer::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut l = [0.0f32; BLOCK];
    let mut ol = [0.0f32; BLOCK];
    let mut or = [0.0f32; BLOCK];
    for _ in 0..blocks {
        for (i, v) in l.iter_mut().enumerate() {
            *v = amp * (std::f32::consts::TAU * freq * (*phase + i) as f32 / SR).sin();
        }
        *phase += BLOCK;
        let inputs: [&[f32]; 2] = [&l, &l];
        let mut outputs: [&mut [f32]; 2] = [&mut ol, &mut or];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: BLOCK,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        assert_no_alloc(|| {
            d.process(&mut ctx, &mut audio);
        });
    }
}

/// One analysis pass under `assert_no_alloc`: the frames written.
fn pass(d: &mut eq::Eq, frames: &mut [AnalysisFrame]) -> usize {
    let mut n = 0;
    assert_no_alloc(|| {
        let mut sink = AnalysisSink::new(frames);
        d.analysis(&mut sink);
        n = sink.len();
    });
    n
}

/// dB of the bin whose centre is nearest `hz` in a spectrum frame.
fn bin_db(f: &AnalysisFrame, hz: f32) -> f32 {
    let v = f.values();
    let (lo, hi, bins) = (v[0], v[1], &v[2..]);
    let t = (hz / lo).ln() / (hi / lo).ln() * bins.len() as f32 - 0.5;
    bins[t.round().clamp(0.0, bins.len() as f32 - 1.0) as usize]
}

#[test]
fn opts_in_and_is_silent_until_watched() {
    let mut d = prepared();
    assert!(d.has_analysis());
    let mut phase = 0;
    run(&mut d, 1000.0, 0.5, 200, &mut phase);
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    // Never asked before: nothing was computed.
    assert_eq!(pass(&mut d, &mut frames[..]), 0);
}

#[test]
fn publishes_pre_and_post_spectra_in_one_pass() {
    let mut d = prepared();
    // Band 4 (a 1 kHz bell) +12 dB.
    d.set_param(eq::params::band(3, eq::params::GAIN), 12.0);
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    let mut phase = 0;
    assert_eq!(pass(&mut d, &mut frames[..]), 0);
    // Half a second of 1 kHz at -6 dBFS, watched every ~33 ms.
    for _ in 0..15 {
        run(&mut d, 1000.0, 0.5, 7, &mut phase);
        pass(&mut d, &mut frames[..]);
    }
    let n = pass(&mut d, &mut frames[..]);
    assert_eq!(n, 2);
    assert_eq!(frames[0].kind, AnalysisKind::SpectrumPre);
    assert_eq!(frames[1].kind, AnalysisKind::Spectrum);
    for f in &frames[..2] {
        let v = f.values();
        assert_eq!(v.len(), 2 + spectrum::BINS);
        assert!(v.len() <= ANALYSIS_MAX_VALUES && spectrum::BINS <= 256);
        assert_eq!(v[0], spectrum::MIN_HZ);
        assert_eq!(v[1], spectrum::MAX_HZ);
        assert!(
            v[2..]
                .iter()
                .all(|db| db.is_finite() && *db >= spectrum::FLOOR_DB)
        );
    }
    let pre = bin_db(&frames[0], 1000.0);
    let post = bin_db(&frames[1], 1000.0);
    assert!((pre - -6.02).abs() < 1.5, "pre peak {pre} dB");
    assert!((post - pre - 12.0).abs() < 1.0, "post {post} vs pre {pre}");
    // Far from the tone the spectrum is well below the peak.
    assert!(bin_db(&frames[0], 100.0) < pre - 40.0);
    assert!(bin_db(&frames[0], 10_000.0) < pre - 40.0);
}

#[test]
fn stops_computing_when_no_longer_watched() {
    let mut d = prepared();
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    let mut phase = 0;
    pass(&mut d, &mut frames[..]);
    run(&mut d, 440.0, 0.5, 20, &mut phase);
    assert_eq!(pass(&mut d, &mut frames[..]), 2);
    assert!(bin_db(&frames[1], 440.0) > -12.0);
    // Unwatched for 1.5 s (the first second still refreshes): the first pass after that
    // publishes nothing stale, the next block refreshes from the audio that just played.
    run(&mut d, 5000.0, 0.5, 280, &mut phase);
    assert_eq!(pass(&mut d, &mut frames[..]), 0);
    run(&mut d, 5000.0, 0.5, 1, &mut phase);
    assert_eq!(pass(&mut d, &mut frames[..]), 2);
    assert!(bin_db(&frames[1], 440.0) < -60.0);
    assert!(bin_db(&frames[1], 5000.0) > -12.0);
}

/// Analyzer ballistics: a tone shows within two frames (≈ 67 ms) to 1 dB; once it stops the
/// curve falls at [`spectrum::FALL_DB_PER_S`] instead of dropping to the floor in a few
/// frames (the old symmetric average fell 3 dB per frame, ~90 dB/s, and flickered).
#[test]
fn analyzer_rises_fast_and_falls_smoothly() {
    let sr = 48_000.0f32;
    let mut a = spectrum::Analyzer::new();
    let bin = |a: &spectrum::Analyzer| {
        let b = a.bins();
        let (lo, hi) = (spectrum::MIN_HZ, spectrum::max_hz(sr));
        let t = (1000f32 / lo).ln() / (hi / lo).ln() * b.len() as f32 - 0.5;
        b[t.round() as usize]
    };
    let mut phase = 0usize;
    let mut feed = |a: &mut spectrum::Analyzer, amp: f32| {
        for _ in 0..spectrum::FFT_SIZE {
            a.push(amp * (std::f32::consts::TAU * 1000.0 * phase as f32 / sr).sin());
            phase += 1;
        }
    };
    // Settle on silence, then a tone at -6 dBFS.
    feed(&mut a, 0.0);
    a.compute(sr);
    feed(&mut a, 0.5);
    a.compute(sr);
    let first = bin(&a);
    a.compute(sr);
    let second = bin(&a);
    for _ in 0..10 {
        a.compute(sr);
    }
    let level = bin(&a);
    assert!((level + 6.02).abs() < 1.5, "tone at {level} dB");
    assert!(level - second < 1.0, "two frames: {second} vs {level}");
    assert!(level - first < 6.0, "one frame: {first}");
    // The tone stops: half a second later (15 frames) the curve is about 12 dB down.
    feed(&mut a, 0.0);
    let frames = ether_core::analysis::ANALYSIS_HZ as usize / 2;
    for _ in 0..frames {
        a.compute(sr);
    }
    let fallen = level - bin(&a);
    let want = spectrum::FALL_DB_PER_S * frames as f32 / ether_core::analysis::ANALYSIS_HZ as f32;
    assert!(
        (fallen - want).abs() < 1.0,
        "fell {fallen} dB in 0.5 s (want {want})"
    );
}
