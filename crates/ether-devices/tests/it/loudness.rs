//! `Loudness` (fx-analysis): the EBU R 128 compliance cases of EBU Tech 3341 (momentary,
//! short-term, gated integrated loudness, true peak) and Tech 3342 (loudness range), the
//! analysis frame layout and history, pass-through, reset on play, allocation-free processing.

use assert_no_alloc::assert_no_alloc;
use ether_core::analysis::{ANALYSIS_FRAMES_PER_PASS, AnalysisFrame, AnalysisKind, AnalysisSink};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::fx_analysis::loudness as p;
use ether_devices::fx_analysis::loudness_device::Frame;

const BLOCK: usize = 1024;

struct Meter {
    d: Box<dyn Device>,
    sr: f32,
    phase: f64,
    playing: bool,
    frames: Box<[AnalysisFrame; ANALYSIS_FRAMES_PER_PASS]>,
    out_events: EventBuffer,
}

impl Meter {
    fn new(sr: f32) -> Self {
        let mut d =
            ether_devices::create(&BuiltinDevice::new(BuiltinDeviceType::Loudness), &NoSamples);
        d.prepare(&PrepareConfig {
            sample_rate: sr,
            max_block_size: BLOCK,
            max_events_per_block: 64,
        });
        Self {
            d,
            sr,
            phase: 0.0,
            playing: true,
            frames: Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]),
            out_events: EventBuffer::with_capacity(64),
        }
    }

    /// One block of `left`/`right` (≤ BLOCK); returns the output.
    fn block(&mut self, left: &[f32], right: &[f32], events: &[ProcessEvent]) -> [Vec<f32>; 2] {
        let n = left.len();
        let mut out = [vec![0.0f32; n], vec![0.0f32; n]];
        let transport = TransportInfo {
            playing: self.playing,
            ..TransportInfo::STOPPED
        };
        let ins: [&[f32]; 2] = [left, right];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        let mut ctx = ProcessContext {
            sample_rate: self.sr,
            frames: n,
            transport: &transport,
            events,
            out_events: &mut self.out_events,
        };
        let mut buffers = AudioBuffers {
            inputs: &ins,
            outputs: &mut outs,
        };
        self.d.process(&mut ctx, &mut buffers);
        out
    }

    /// `seconds` of a stereo sine at `dbfs` peak (both channels), phase-continuous.
    fn tone(&mut self, hz: f64, dbfs: f64, seconds: f64) {
        let amp = 10f64.powf(dbfs / 20.0);
        let total = (seconds * f64::from(self.sr)).round() as usize;
        let inc = std::f64::consts::TAU * hz / f64::from(self.sr);
        let mut buf = vec![0.0f32; BLOCK];
        let mut done = 0;
        while done < total {
            let n = BLOCK.min(total - done);
            for x in &mut buf[..n] {
                *x = (amp * self.phase.sin()) as f32;
                self.phase = (self.phase + inc) % std::f64::consts::TAU;
            }
            let b = buf[..n].to_vec();
            self.block(&b, &b, &[]);
            done += n;
        }
    }

    fn frame(&mut self) -> Vec<f32> {
        let mut sink = AnalysisSink::new(&mut self.frames[..]);
        self.d.analysis(&mut sink);
        assert_eq!(sink.len(), 1);
        let f = &self.frames[0];
        assert_eq!(f.kind, AnalysisKind::Levels);
        f.values().to_vec()
    }
}

fn near(what: &str, got: f32, want: f32, tol: f32) {
    eprintln!("{what}: {got:.3} (expected {want} ± {tol})");
    assert!(
        (got - want).abs() <= tol,
        "{what}: {got:.3}, expected {want} ± {tol}"
    );
}

#[test]
fn tech_3341_cases_1_and_2_steady_tones() {
    for (db, sr) in [
        (-23.0, 48_000.0),
        (-33.0, 48_000.0),
        (-23.0, 44_100.0),
        (-23.0, 96_000.0),
    ] {
        let mut m = Meter::new(sr);
        m.tone(1000.0, db, 20.0);
        let f = m.frame();
        let want = db as f32;
        near(&format!("{sr} momentary"), f[Frame::MOMENTARY], want, 0.1);
        near(&format!("{sr} short-term"), f[Frame::SHORT_TERM], want, 0.1);
        near(&format!("{sr} integrated"), f[Frame::INTEGRATED], want, 0.1);
        // A steady tone has no loudness range.
        near(&format!("{sr} range"), f[Frame::RANGE], 0.0, 0.1);
    }
}

#[test]
fn tech_3341_cases_3_to_5_gating() {
    let cases: [&[(f64, f64)]; 3] = [
        &[(-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0)],
        &[
            (-72.0, 10.0),
            (-36.0, 10.0),
            (-23.0, 60.0),
            (-36.0, 10.0),
            (-72.0, 10.0),
        ],
        &[(-26.0, 20.0), (-20.0, 20.1), (-26.0, 20.0)],
    ];
    for (k, segments) in cases.iter().enumerate() {
        let mut m = Meter::new(48_000.0);
        for &(db, s) in *segments {
            m.tone(1000.0, db, s);
        }
        near(
            &format!("case {}", k + 3),
            m.frame()[Frame::INTEGRATED],
            -23.0,
            0.1,
        );
    }
}

#[test]
fn tech_3342_loudness_range() {
    for ((a, b), want) in [
        ((-20.0, -30.0), 10.0),
        ((-20.0, -15.0), 5.0),
        ((-40.0, -20.0), 20.0),
    ] {
        let mut m = Meter::new(48_000.0);
        m.tone(1000.0, a, 20.0);
        m.tone(1000.0, b, 20.0);
        near(&format!("{a} then {b}"), m.frame()[Frame::RANGE], want, 1.0);
    }
}

#[test]
fn true_peak_sees_between_the_samples() {
    // A quarter-rate sine at 45°: every sample at -9 dBFS, the waveform at -6.
    let mut m = Meter::new(48_000.0);
    let amp = 0.5f32;
    let x: Vec<f32> = (0..BLOCK)
        .map(|i| amp * (std::f32::consts::FRAC_PI_2 * i as f32 + std::f32::consts::FRAC_PI_4).sin())
        .collect();
    for _ in 0..20 {
        m.block(&x, &x, &[]);
    }
    let f = m.frame();
    near("true peak max", f[Frame::TRUE_PEAK_MAX], -6.02, 0.3);
    near("true peak now", f[Frame::TRUE_PEAK], -6.02, 0.3);
    // A low tone's true peak is its sample peak.
    let mut m = Meter::new(48_000.0);
    m.tone(997.0, -1.0, 1.0);
    near("1 kHz", m.frame()[Frame::TRUE_PEAK_MAX], -1.0, 0.05);
}

#[test]
fn frame_carries_the_history() {
    let mut m = Meter::new(48_000.0);
    m.tone(1000.0, -30.0, 5.0);
    m.tone(1000.0, -20.0, 5.0);
    let f = m.frame();
    // Default window 30 s: 300 points of 100 ms, 100 of them filled.
    near("window", f[Frame::WINDOW], 30.0, 1e-4);
    near("step", f[Frame::STEP], 0.1, 1e-6);
    let n = f[Frame::POINTS] as usize;
    assert_eq!(n, 100);
    assert_eq!(f.len(), Frame::HISTORY + 2 * n);
    let momentary = &f[Frame::HISTORY..Frame::HISTORY + n];
    let short = &f[Frame::HISTORY + n..];
    near("momentary, 2.5 s in", momentary[25], -30.0, 0.1);
    near("momentary, now", momentary[n - 1], -20.0, 0.1);
    near("short-term, now", short[n - 1], -20.0, 0.1);
    // 15-minute window: one point per 3 s bucket (max of the bucket).
    let mut m2 = Meter::new(48_000.0);
    m2.d.set_param(p::HISTORY, 4.0);
    m2.tone(1000.0, -20.0, 7.0);
    let f = m2.frame();
    near("step", f[Frame::STEP], 3.0, 1e-4);
    assert_eq!(f[Frame::POINTS] as usize, 3);
}

#[test]
fn passes_audio_through_and_resets_on_play() {
    let mut m = Meter::new(48_000.0);
    let x: Vec<f32> = (0..BLOCK)
        .map(|i| ((i as f32) * 0.01).sin() * 0.3)
        .collect();
    let y: Vec<f32> = x.iter().map(|v| -v).collect();
    let out = m.block(&x, &y, &[]);
    assert_eq!(out[0], x);
    assert_eq!(out[1], y);
    m.tone(1000.0, -18.0, 3.0);
    assert!(m.frame()[Frame::INTEGRATED] > -19.0);
    // Stop, then play again: the measurement starts over.
    m.playing = false;
    m.tone(1000.0, -18.0, 0.1);
    m.playing = true;
    m.tone(1000.0, -40.0, 1.0);
    near("after the reset", m.frame()[Frame::INTEGRATED], -40.0, 0.2);
    // With `Reset On Play` off, it keeps going.
    m.d.set_param(p::RESET_ON_PLAY, 0.0);
    m.playing = false;
    m.tone(1000.0, -40.0, 0.1);
    m.playing = true;
    // The -40 blocks still count (within the -10 LU gate of the -34 ones).
    m.tone(1000.0, -34.0, 1.0);
    let i = m.frame()[Frame::INTEGRATED];
    assert!(i < -35.0 && i > -39.0, "{i}");
}

#[test]
fn processes_and_reports_without_allocating() {
    let mut m = Meter::new(48_000.0);
    m.tone(1000.0, -20.0, 1.0);
    let x = vec![0.25f32; BLOCK];
    let events = [ProcessEvent {
        offset: 3,
        kind: EventKind::Param {
            param: ParamId(1),
            value: 4.0,
            step: false,
        },
    }];
    let mut out = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    assert_no_alloc(|| {
        let ins: [&[f32]; 2] = [&x, &x];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: BLOCK,
            transport: &transport,
            events: &events,
            out_events: &mut m.out_events,
        };
        let mut buffers = AudioBuffers {
            inputs: &ins,
            outputs: &mut outs,
        };
        m.d.process(&mut ctx, &mut buffers);
        let mut sink = AnalysisSink::new(&mut frames[..]);
        m.d.analysis(&mut sink);
    });
    assert_eq!(m.d.param(ParamId(1)), Some(4.0));
}
