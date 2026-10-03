//! v0.3 (`mpe`): the Poly Synth's response to per-note expression
//! (`EventKind::NoteExpression`): pitch in semitones per voice, pressure (brighter and
//! louder), timbre (cutoff around the neutral 0.5). Numeric "by ear" checks.

use ether_core::protocol::model::NoteExpressionKind;
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};

use super::{PolySynth, poly_synth as p};

const SR: f32 = 48_000.0;

fn synth(params: &[(ether_core::protocol::model::ParamId, f64)]) -> PolySynth {
    let mut s = PolySynth::new();
    s.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: 512,
        max_events_per_block: 256,
    });
    for &(id, v) in params {
        s.set_param(id, v);
    }
    s
}

/// A clean sine voice: open filter, no drive, held sustain.
fn sine() -> PolySynth {
    synth(&[
        (p::OSC1_TYPE, 3.0),
        (p::CUTOFF, 20_000.0),
        (p::RESONANCE, 0.0),
        (p::FILTER_TYPE, 1.0),
        (p::AMP_SUSTAIN, 100.0),
    ])
}

/// A bright saw through a low-pass at 600 Hz (pressure / timbre open it).
fn saw() -> PolySynth {
    synth(&[
        (p::OSC1_TYPE, 0.0),
        (p::CUTOFF, 600.0),
        (p::RESONANCE, 0.0),
        (p::FILTER_TYPE, 1.0),
        (p::AMP_SUSTAIN, 100.0),
    ])
}

/// Render `frames` (mono, left) with `events` at absolute sample times, in `block`s.
fn run(s: &mut PolySynth, frames: usize, block: usize, events: &[(u64, EventKind)]) -> Vec<f32> {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut done = 0;
    while done < frames {
        let n = block.min(frames - done);
        let t0 = done as u64;
        let evs: Vec<ProcessEvent> = events
            .iter()
            .filter(|e| e.0 >= t0 && e.0 < t0 + n as u64)
            .map(|e| ProcessEvent {
                offset: (e.0 - t0) as u32,
                kind: e.1,
            })
            .collect();
        let transport = TransportInfo {
            playing: true,
            sample_time: t0,
            bpm: 120.0,
            beats_per_sample: 2.0 / f64::from(SR),
            position: t0 as f64 * 2.0 / f64::from(SR),
            ..TransportInfo::STOPPED
        };
        let mut outs: [&mut [f32]; 2] = [&mut l[done..done + n], &mut r[done..done + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &evs,
            out_events: &mut out_events,
        };
        let mut buffers = AudioBuffers {
            inputs: &[],
            outputs: &mut outs,
        };
        s.process(&mut ctx, &mut buffers);
        done += n;
    }
    l
}

fn on(key: u8, id: u32) -> EventKind {
    EventKind::NoteOn {
        note_id: id,
        channel: 0,
        key,
        velocity: 1.0,
    }
}

fn expr(id: u32, key: u8, expression: NoteExpressionKind, value: f32) -> EventKind {
    EventKind::NoteExpression {
        note_id: id,
        channel: 0,
        key,
        expression,
        value,
    }
}

/// Frequency from positive-going zero crossings (near-sine signals).
fn frequency(x: &[f32]) -> f32 {
    let c: Vec<f32> = (1..x.len())
        .filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0)
        .map(|i| (i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]))
        .collect();
    assert!(c.len() > 2, "no periodic signal");
    (c.len() - 1) as f32 * SR / (c[c.len() - 1] - c[0])
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Energy of the first difference relative to the signal (a brightness measure).
fn brightness(x: &[f32]) -> f32 {
    let d: f32 = x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    let e: f32 = x.iter().map(|v| v * v).sum();
    (d / e.max(1e-12)).sqrt()
}

#[test]
fn per_note_pitch_moves_only_its_voice() {
    // An octave up on the first sample (comes with the note-on: no glide from 0).
    let mut s = sine();
    let l = run(
        &mut s,
        24_000,
        256,
        &[
            (0, on(57, 1)),
            (0, expr(1, 57, NoteExpressionKind::Pitch, 12.0)),
        ],
    );
    let f = frequency(&l[480..]);
    assert!((f / 440.0 - 1.0).abs() < 0.003, "{f}");

    // Two voices: a pitch expression on one leaves the other alone.
    let mut a = sine();
    let both = run(
        &mut a,
        24_000,
        256,
        &[
            (0, on(57, 1)),
            (0, on(69, 2)),
            (0, expr(1, 57, NoteExpressionKind::Pitch, 7.0)),
        ],
    );
    let mut b = sine();
    let reference = run(&mut b, 24_000, 256, &[(0, on(64, 1)), (0, on(69, 2))]);
    // 57 + 7 = 64: the same two pitches as playing 64 and 69 (phases differ: compare RMS).
    assert!((rms(&both[4_800..]) - rms(&reference[4_800..])).abs() < 0.05 * rms(&reference));

    // A later change glides (3 ms one-pole) and lands exactly.
    let mut s = sine();
    let l = run(
        &mut s,
        48_000,
        256,
        &[
            (0, on(69, 1)),
            (12_000, expr(1, 69, NoteExpressionKind::Pitch, -12.0)),
        ],
    );
    assert!((frequency(&l[2_000..11_000]) / 440.0 - 1.0).abs() < 0.003);
    assert!((frequency(&l[16_000..]) / 220.0 - 1.0).abs() < 0.003);
    // Fractional semitones (1/128 steps are what the engine sends).
    let mut s = sine();
    let l = run(
        &mut s,
        24_000,
        256,
        &[
            (0, on(69, 1)),
            (0, expr(1, 69, NoteExpressionKind::Pitch, 0.5)),
        ],
    );
    let want = 440.0 * 2f32.powf(0.5 / 12.0);
    assert!((frequency(&l[480..]) / want - 1.0).abs() < 0.002);
}

#[test]
fn pressure_brightens_and_lifts_the_level() {
    let render = |pressure: Option<f32>| {
        let mut s = saw();
        let mut ev = vec![(0, on(45, 1))];
        if let Some(v) = pressure {
            ev.push((0, expr(1, 45, NoteExpressionKind::Pressure, v)));
        }
        run(&mut s, 24_000, 256, &ev)
    };
    let none = render(None);
    let zero = render(Some(0.0));
    let full = render(Some(1.0));
    // Pressure 0 is the neutral (same as no expression).
    assert_eq!(none, zero);
    let (b0, b1) = (brightness(&none[4_800..]), brightness(&full[4_800..]));
    assert!(b1 > b0 * 1.5, "brightness {b0} -> {b1}");
    assert!(rms(&full[4_800..]) > rms(&none[4_800..]) * 1.2);
}

#[test]
fn timbre_moves_the_cutoff_around_neutral() {
    let render = |timbre: Option<f32>| {
        let mut s = saw();
        let mut ev = vec![(0, on(45, 1))];
        if let Some(v) = timbre {
            ev.push((0, expr(1, 45, NoteExpressionKind::Timbre, v)));
        }
        run(&mut s, 24_000, 256, &ev)
    };
    let none = render(None);
    assert_eq!(none, render(Some(0.5)), "0.5 is neutral");
    let dark = brightness(&render(Some(0.0))[4_800..]);
    let mid = brightness(&none[4_800..]);
    let bright = brightness(&render(Some(1.0))[4_800..]);
    assert!(dark < mid && mid < bright, "{dark} {mid} {bright}");
}

#[test]
fn expression_for_another_note_or_after_the_voice_is_ignored() {
    let mut a = sine();
    let x = run(
        &mut a,
        12_000,
        256,
        &[
            (0, on(69, 1)),
            (100, expr(7, 69, NoteExpressionKind::Pitch, 12.0)),
            (100, expr(1, 69, NoteExpressionKind::Pitch, f32::NAN)),
        ],
    );
    let mut b = sine();
    let y = run(&mut b, 12_000, 256, &[(0, on(69, 1))]);
    assert_eq!(x, y);
}

#[test]
fn expression_renders_are_independent_of_the_block_size() {
    let events = [
        (10, on(60, 1)),
        (10, expr(1, 60, NoteExpressionKind::Pitch, 2.0)),
        (333, on(64, 2)),
        (1_000, expr(2, 64, NoteExpressionKind::Pressure, 0.7)),
        (2_345, expr(1, 60, NoteExpressionKind::Pitch, -3.5)),
        (3_000, expr(2, 64, NoteExpressionKind::Timbre, 0.9)),
        (6_000, expr(1, 60, NoteExpressionKind::Pitch, 0.0)),
    ];
    let render = |block: usize| {
        let mut s = saw();
        run(&mut s, 9_600, block, &events)
    };
    let a = render(512);
    for block in [16, 64, 128] {
        assert_eq!(render(block), a, "block {block}");
    }
}
