//! The Compressor's `Detector` param (`ableton-more`): Peak (default, unchanged), RMS (a
//! 10 ms mean-square window) and Expand (downward expander). Every `process` call runs under
//! `assert_no_alloc` (debug builds abort on allocation on the audio path).

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::BuiltinDeviceType;
use ether_core::{
    AudioBuffers, Device, EventBuffer, Node, PrepareConfig, ProcessContext, TransportInfo,
};
use ether_devices::{Compressor, compressor};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn comp(detector: Option<f64>, threshold: f64, ratio: f64) -> Compressor {
    let mut c = Compressor::new();
    c.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    if let Some(d) = detector {
        c.set_param(compressor::params::DETECTOR, d);
    }
    c.set_param(compressor::params::THRESHOLD, threshold);
    c.set_param(compressor::params::RATIO, ratio);
    c.set_param(compressor::params::ATTACK, 0.1);
    c.set_param(compressor::params::RELEASE, 50.0);
    c
}

/// Renders `main` (both channels) and returns the left output and the gain reduction (dB)
/// after every block.
fn render(c: &mut Compressor, main: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let frames = main.len();
    let mut out_l = vec![0.0f32; frames];
    let mut out_r = vec![0.0f32; frames];
    let mut grs = Vec::new();
    let mut out_events = EventBuffer::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut pos = 0;
    while pos < frames {
        let n = BLOCK.min(frames - pos);
        let inputs: [&[f32]; 2] = [&main[pos..pos + n], &main[pos..pos + n]];
        let mut outputs: [&mut [f32]; 2] = [&mut out_l[pos..pos + n], &mut out_r[pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        assert_no_alloc(|| {
            c.process(&mut ctx, &mut audio);
        });
        grs.push(c.gain_reduction());
        pos += n;
    }
    (out_l, grs)
}

fn sine(amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * 1000.0 * i as f32 / SR).sin())
        .collect()
}

#[test]
fn detector_param_defaults_to_peak() {
    let d = ether_devices::descriptor(BuiltinDeviceType::Compressor);
    let p = d
        .params
        .iter()
        .find(|p| p.id == compressor::params::DETECTOR)
        .expect("Detector param");
    assert_eq!(p.default, 0.0);
    assert_eq!(
        p.labels.as_deref(),
        Some(&["Peak".to_owned(), "RMS".to_owned(), "Expand".to_owned()][..])
    );
    assert_eq!(
        Compressor::new().param(compressor::params::DETECTOR),
        Some(0.0)
    );
}

#[test]
fn peak_is_bit_identical_to_the_default() {
    let x = sine(0.8, 9_600);
    let (a, _) = render(&mut comp(None, -20.0, 4.0), &x);
    let (b, _) = render(&mut comp(Some(0.0), -20.0, 4.0), &x);
    assert_eq!(a, b);
    // Switching away and back restores the peak path too.
    let mut c = comp(Some(1.0), -20.0, 4.0);
    c.set_param(compressor::params::DETECTOR, 0.0);
    c.reset();
    let (c_out, _) = render(&mut c, &x);
    assert_eq!(a, c_out);
}

#[test]
fn rms_reads_a_sine_3_db_below_its_peak() {
    // A 0.5 sine: peak -6 dBFS, RMS -9 dBFS. Threshold -12 dB, 4:1 (hard region: > 3 dB
    // over the 6 dB knee centre).
    let x = sine(0.5, 48_000);
    let (_, peak) = render(&mut comp(Some(0.0), -12.0, 4.0), &x);
    let (_, rms) = render(&mut comp(Some(1.0), -12.0, 4.0), &x);
    let peak = *peak.last().unwrap();
    let rms = *rms.last().unwrap();
    assert!((peak - 4.5).abs() < 0.3, "peak GR {peak}");
    assert!((rms - 2.25).abs() < 0.3, "rms GR {rms}");
}

#[test]
fn rms_window_ignores_short_transients() {
    // A 0.5 ms full-scale click, then silence: the peak detector clamps hard, the 10 ms RMS
    // window barely moves.
    let mut x = vec![0.0f32; 4_800];
    for (i, v) in x.iter_mut().enumerate().take(24) {
        *v = if i % 2 == 0 { 1.0 } else { -1.0 };
    }
    let (_, peak) = render(&mut comp(Some(0.0), -30.0, 10.0), &x);
    let (_, rms) = render(&mut comp(Some(1.0), -30.0, 10.0), &x);
    let peak_max = peak.iter().copied().fold(0.0, f32::max);
    let rms_max = rms.iter().copied().fold(0.0, f32::max);
    assert!(peak_max > 20.0, "peak {peak_max}");
    assert!(rms_max < peak_max - 8.0, "rms {rms_max} vs peak {peak_max}");
    // A sustained signal at the same level is read fully once the window filled.
    let (_, rms) = render(&mut comp(Some(1.0), -30.0, 10.0), &vec![1.0; 9_600]);
    assert!((rms.last().unwrap() - 27.0).abs() < 0.5, "{:?}", rms.last());
}

#[test]
fn rms_releases_on_silence() {
    let mut x = sine(1.0, 9_600);
    x.extend(vec![0.0; 48_000]);
    let (out, grs) = render(&mut comp(Some(1.0), -20.0, 4.0), &x);
    assert!(grs[30] > 5.0);
    assert_eq!(*grs.last().unwrap(), 0.0);
    assert!(out[9_600..].iter().all(|v| *v == 0.0));
}

#[test]
fn expand_attenuates_below_the_threshold_only() {
    // -26 dBFS steady signal under a -20 dB threshold at 1:3 -> 12 dB down.
    let quiet = vec![0.05f32; 48_000];
    let (out, _) = render(&mut comp(Some(2.0), -20.0, 3.0), &quiet);
    let db = 20.0 * (out.last().unwrap() / 0.05).log10();
    assert!((db + 12.0).abs() < 0.2, "{db}");
    // Above the threshold: unity.
    let loud = vec![0.5f32; 48_000];
    let (out, grs) = render(&mut comp(Some(2.0), -20.0, 3.0), &loud);
    assert_eq!(*grs.last().unwrap(), 0.0);
    assert!((out.last().unwrap() - 0.5).abs() < 1e-6);
    // Far below: capped.
    let (_, grs) = render(&mut comp(Some(2.0), -20.0, 20.0), &vec![0.001f32; 48_000]);
    assert!((grs.last().unwrap() - compressor::MAX_EXPAND_DB).abs() < 0.01);
}
