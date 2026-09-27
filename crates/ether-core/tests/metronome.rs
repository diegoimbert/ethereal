//! Metronome click through the real engine (roadmap v2, `tempo-metronome`): sample-exact
//! click positions at constant and changing tempo, in 3/4 and 7/8, across loop wraps, and
//! the recording count-in.

mod common;

use common::*;
use ether_core::graph::MetronomeDesc;
use ether_core::protocol::model::{Beats, MetronomeSound, TempoCurve, TimeSignature};
use ether_core::tempo::{TempoMapRt, TempoPointDesc, TimeSignatureDesc};
use ether_core::{EngineParts, RenderGraphDesc, TransportControl, create};

const VOLUME: f32 = 0.5;

fn tp(beat: f64, bpm: f64, curve: TempoCurve) -> TempoPointDesc {
    TempoPointDesc { beat, bpm, curve }
}

fn ts(beat: f64, numerator: u8, denominator: u8) -> TimeSignatureDesc {
    TimeSignatureDesc {
        beat,
        signature: TimeSignature {
            numerator,
            denominator,
        },
    }
}

fn graph(tempo: Vec<TempoPointDesc>, signatures: Vec<TimeSignatureDesc>) -> RenderGraphDesc {
    RenderGraphDesc {
        version: 1,
        tempo,
        signatures,
        metronome: true,
        click: MetronomeDesc {
            volume: VOLUME,
            accent: true,
            sound: MetronomeSound::Classic,
            count_in_end: None,
        },
        tracks: vec![master()],
        ..Default::default()
    }
}

fn start(d: RenderGraphDesc, from: f64) -> EngineParts {
    let mut p = create(config());
    p.handle.publish(d).unwrap();
    if from != 0.0 {
        p.handle
            .transport(TransportControl::Locate {
                position: Beats(from),
            })
            .unwrap();
    }
    p.handle.transport(TransportControl::Play).unwrap();
    p
}

/// Samples where a click starts (silence → sound), with the click's peak level.
fn clicks(buf: &[f32]) -> Vec<(usize, f32)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        if buf[i] != 0.0 {
            let end = (i + 200).min(buf.len());
            let peak = buf[i..end].iter().fold(0.0f32, |m, s| m.max(s.abs()));
            v.push((i, peak));
            // Skip to the end of this click.
            while i < buf.len() && buf[i] != 0.0 {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    v
}

fn positions(buf: &[f32]) -> Vec<usize> {
    clicks(buf).into_iter().map(|c| c.0).collect()
}

/// Expected start sample of the click on `beat` when playback starts at `from`: the
/// first sample at or after the beat on the engine's timeline. The engine ends a sub-block
/// on the first sample at or after each tempo/signature boundary and restarts the timeline
/// there, so each boundary lands on a whole sample (as the audio does); in between, time
/// follows the tempo map exactly.
fn expected(map: &TempoMapRt, from: f64, beat: f64) -> usize {
    let sr = f64::from(SR);
    let sec = |b: f64| map.beats_to_seconds(b);
    let mut start = from;
    let mut sample = 0usize;
    while let Some(nb) = map.next_boundary(start) {
        if nb > beat + 1e-9 {
            break;
        }
        sample += ((sec(nb) - sec(start)) * sr - 1e-7).ceil().max(1.0) as usize;
        start = nb;
    }
    sample + ((sec(beat) - sec(start)) * sr - 1e-4).ceil().max(0.0) as usize
}

fn is_accent(peak: f32) -> bool {
    (peak - VOLUME).abs() < 0.02
}

#[test]
fn constant_tempo_4_4_is_sample_exact_with_accents() {
    let d = graph(vec![tp(0.0, 120.0, TempoCurve::Step)], vec![ts(0.0, 4, 4)]);
    let mut p = start(d, 0.0);
    let (l, r) = render(&mut p.engine, 48_000 * 5, 500);
    assert_eq!(l, r);
    let c = clicks(&l);
    let at: Vec<usize> = c.iter().map(|c| c.0).collect();
    assert_eq!(at, (0..10).map(|b| b * 24_000).collect::<Vec<_>>());
    let accents: Vec<bool> = c.iter().map(|c| is_accent(c.1)).collect();
    assert_eq!(
        accents,
        vec![
            true, false, false, false, true, false, false, false, true, false
        ]
    );
    // Normal clicks are quieter.
    assert!(c[1].1 < VOLUME * 0.7 && c[1].1 > VOLUME * 0.5);
}

#[test]
fn off_by_default_and_accent_setting() {
    let mut d = graph(vec![tp(0.0, 120.0, TempoCurve::Step)], vec![]);
    d.metronome = false;
    let mut p = start(d, 0.0);
    let (l, _) = render(&mut p.engine, 60_000, 512);
    assert!(l.iter().all(|s| *s == 0.0));

    let mut d = graph(vec![tp(0.0, 120.0, TempoCurve::Step)], vec![]);
    d.click.accent = false;
    d.click.sound = MetronomeSound::Beep;
    let mut p = start(d, 0.0);
    let (l, _) = render(&mut p.engine, 100_000, 512);
    let c = clicks(&l);
    assert_eq!(c.len(), 5);
    assert!(c.iter().all(|c| !is_accent(c.1)));
}

#[test]
fn three_four_and_seven_eight() {
    // 3/4 at 100 BPM, then 7/8 from beat 6 (bar 3): eighth-note clicks.
    let tempo = vec![tp(0.0, 100.0, TempoCurve::Step)];
    let sigs = vec![ts(0.0, 3, 4), ts(6.0, 7, 8)];
    let map = TempoMapRt::compile(&tempo, &sigs);
    let mut p = start(graph(tempo, sigs), 0.0);
    let frames = expected(&map, 0.0, 13.5) + 10;
    let (l, _) = render(&mut p.engine, frames, 441);
    let c = clicks(&l);
    let mut beats: Vec<f64> = (0..6).map(f64::from).collect();
    beats.extend((0..16).map(|i| 6.0 + f64::from(i) * 0.5));
    let want: Vec<usize> = beats.iter().map(|b| expected(&map, 0.0, *b)).collect();
    assert_eq!(c.iter().map(|c| c.0).collect::<Vec<_>>(), want);
    let accents: Vec<usize> = c
        .iter()
        .enumerate()
        .filter(|(_, c)| is_accent(c.1))
        .map(|(i, _)| i)
        .collect();
    // Beats 0 and 3 (3/4 bars), then every 7 eighths from beat 6.
    assert_eq!(accents, vec![0, 3, 6, 13, 20]);
}

#[test]
fn changing_tempo_steps_and_ramps() {
    let tempo = vec![
        tp(0.0, 120.0, TempoCurve::Step),
        tp(3.0, 90.0, TempoCurve::Linear),
        tp(7.0, 150.0, TempoCurve::Step),
        tp(10.5, 60.0, TempoCurve::Step),
    ];
    let map = TempoMapRt::compile(&tempo, &[]);
    for block in [512, 97, 1] {
        let mut p = start(graph(tempo.clone(), vec![]), 0.0);
        let frames = expected(&map, 0.0, 13.0) + 10;
        let (l, _) = render(&mut p.engine, frames, block);
        let want: Vec<usize> = (0..=13)
            .map(|b| expected(&map, 0.0, f64::from(b)))
            .collect();
        assert_eq!(positions(&l), want, "block {block}");
    }
}

#[test]
fn loop_wraps_click_on_the_loop_start() {
    // 2.5-beat loop at 110 BPM: 65454.5 samples, so each pass lasts 65455 samples (the
    // engine ends a pass on the first sample at or after the loop end).
    let mut d = graph(vec![tp(0.0, 110.0, TempoCurve::Step)], vec![]);
    d.loop_enabled = true;
    d.loop_start = 0.0;
    d.loop_end = 2.5;
    let map = TempoMapRt::compile(&d.tempo, &[]);
    let pass = (map.beats_to_seconds(2.5) * f64::from(SR) - 1e-7).ceil() as usize;
    assert_eq!(pass, 65_455);
    let mut p = start(d, 0.0);
    let (l, _) = render(&mut p.engine, pass * 3 + 1000, 512);
    let mut want = Vec::new();
    for i in 0..4 {
        for b in [0.0, 1.0, 2.0] {
            let s = i * pass + expected(&map, 0.0, b);
            if s < pass * 3 + 1000 {
                want.push(s);
            }
        }
    }
    assert_eq!(positions(&l), want);
}

#[test]
fn count_in_clicks_with_the_metronome_off() {
    // Record at beat 8 with a one-bar count-in: pre-roll from beat 4.
    let mut d = graph(vec![tp(0.0, 120.0, TempoCurve::Step)], vec![]);
    d.metronome = false;
    d.click.count_in_end = Some(8.0);
    // A loop that wraps back before the record start must not count in again.
    d.loop_enabled = true;
    d.loop_start = 6.0;
    d.loop_end = 10.0;
    let mut p = create(config());
    p.handle.publish(d.clone()).unwrap();
    p.handle
        .transport(TransportControl::SetRecording { enabled: true })
        .unwrap();
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(4.0),
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 24_000 * 12, 512);
    assert_eq!(positions(&l), vec![0, 24_000, 48_000, 72_000]);
    assert!(is_accent(clicks(&l)[0].1));

    // Not recording: no count-in click.
    let mut p = start(d, 4.0);
    let (l, _) = render(&mut p.engine, 24_000 * 5, 512);
    assert!(positions(&l).is_empty());
}

#[test]
fn count_in_before_beat_zero() {
    // Record at 0 with two bars of 3/4 count-in: the pre-roll starts at beat -6.
    let mut d = graph(vec![tp(0.0, 120.0, TempoCurve::Step)], vec![ts(0.0, 3, 4)]);
    d.metronome = false;
    d.click.count_in_end = Some(0.0);
    let mut p = create(config());
    p.handle.publish(d).unwrap();
    p.handle
        .transport(TransportControl::SetRecording { enabled: true })
        .unwrap();
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(-6.0),
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 24_000 * 9, 512);
    let c = clicks(&l);
    assert_eq!(
        c.iter().map(|c| c.0).collect::<Vec<_>>(),
        (0..6).map(|b| b * 24_000).collect::<Vec<_>>()
    );
    let accents: Vec<bool> = c.iter().map(|c| is_accent(c.1)).collect();
    assert_eq!(accents, vec![true, false, false, true, false, false]);
}

#[test]
fn locate_silences_the_click() {
    let d = graph(vec![tp(0.0, 120.0, TempoCurve::Step)], vec![]);
    let mut p = start(d, 0.0);
    let (l, _) = render(&mut p.engine, 100, 100);
    assert!(l[99] != 0.0);
    // Jump just after beat 1: the running click is cut, the next one is on beat 2.
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(1.25),
        })
        .unwrap();
    let (l, _) = render(&mut p.engine, 24_000, 512);
    assert_eq!(positions(&l), vec![18_000]);
    assert_eq!(l[0], 0.0);
}

/// Outputs a one-sample impulse on every beat (as the metronome places clicks), or
/// silence when `on` is false.
struct BeatImpulse {
    on: bool,
}

impl ether_core::Node for BeatImpulse {
    fn prepare(&mut self, _: &ether_core::PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
    ) -> ether_core::ProcessStatus {
        for ch in 0..2 {
            audio.outputs[ch].fill(0.0);
        }
        let t = ctx.transport;
        if !self.on || !t.playing || t.beats_per_sample <= 0.0 {
            return ether_core::ProcessStatus::Continue;
        }
        let end = t.position + t.beats_per_sample * ctx.frames as f64;
        let mut k = (t.position - 1e-7).ceil();
        while k < end - 1e-7 {
            let i = ((k - t.position) / t.beats_per_sample - 1e-4)
                .ceil()
                .max(0.0) as usize;
            if i < ctx.frames {
                audio.outputs[0][i] = 4.0;
                audio.outputs[1][i] = 4.0;
            }
            k += 1.0;
        }
        ether_core::ProcessStatus::Continue
    }
}

#[test]
fn clicks_line_up_with_the_audio_under_pdc() {
    const LATENCY: usize = 700;
    let run = |impulses: bool, metronome: bool| {
        let mut p = create(config());
        let src = p
            .handle
            .add_node(Box::new(BeatImpulse { on: impulses }))
            .unwrap();
        let delay = p.handle.add_node(Box::new(Delay::new(LATENCY))).unwrap();
        let mut d = graph(vec![tp(0.0, 120.0, TempoCurve::Step)], vec![]);
        d.metronome = metronome;
        d.tracks.push(with_chain(
            track(
                tid(2),
                ether_core::protocol::model::TrackKind::Audio,
                Some(tid(1)),
            ),
            &[src, delay],
        ));
        p.handle.publish(d).unwrap();
        p.handle.transport(TransportControl::Play).unwrap();
        // Let the graph (and its latency) land before counting.
        render(&mut p.engine, 1, 1);
        assert_eq!(p.handle.latency() as usize, LATENCY);
        let (l, _) = render(&mut p.engine, 24_000 * 4, 256);
        l
    };
    let audio = run(true, false);
    let clicks = run(false, true);
    let hits: Vec<usize> = audio
        .iter()
        .enumerate()
        .filter(|(_, s)| **s != 0.0)
        .map(|(i, _)| i)
        .collect();
    let want: Vec<usize> = (0..4).map(|b| b * 24_000 + LATENCY - 1).collect();
    assert_eq!(hits, want, "the latent track's beats");
    assert_eq!(
        positions(&clicks),
        want,
        "clicks are delayed like the audio"
    );
}

/// Click onsets when every click is a normal one: a click starts at its full level
/// (`VOLUME * 0.6`, the buffer's first sample) and only decays after, so a restart shows
/// as the exact level even when fast clicks cut each other.
fn onsets(buf: &[f32]) -> Vec<usize> {
    let level = VOLUME * 0.6;
    buf.iter()
        .enumerate()
        .filter(|(_, s)| (**s - level).abs() < 1e-6)
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn steep_ramps_are_sample_exact_at_large_blocks() {
    // Ramps are linear in beats (the shared tempo model), so positions grow exponentially
    // in time: 20 → 999 BPM over one beat, 999 → 20 over four, 32nd-note clicks.
    let tempo = vec![
        tp(0.0, 120.0, TempoCurve::Step),
        tp(1.0, 20.0, TempoCurve::Linear),
        tp(2.0, 999.0, TempoCurve::Step),
        tp(3.0, 999.0, TempoCurve::Linear),
        tp(7.0, 20.0, TempoCurve::Linear),
        tp(7.5, 300.0, TempoCurve::Step),
    ];
    let sigs = vec![ts(0.0, 4, 32)];
    let map = TempoMapRt::compile(&tempo, &sigs);
    let want: Vec<usize> = (0..=64)
        .map(|i| expected(&map, 0.0, f64::from(i) * 0.125))
        .collect();
    for block in [1024, 2048, 333] {
        let mut cfg = config();
        cfg.max_block_size = block;
        let mut p = create(cfg);
        let mut d = graph(tempo.clone(), sigs.clone());
        d.click.accent = false;
        p.handle.publish(d).unwrap();
        p.handle.transport(TransportControl::Play).unwrap();
        let (l, _) = render(&mut p.engine, want.last().unwrap() + 100, block);
        assert_eq!(onsets(&l), want, "block {block}");
    }
}

#[test]
fn a_latency_decrease_keeps_clicks_in_order() {
    let mut p = create(config());
    let delay = p.handle.add_node(Box::new(Delay::new(20_000))).unwrap();
    let mut d = graph(vec![tp(0.0, 120.0, TempoCurve::Step)], vec![]);
    d.click.accent = false;
    let mut latent = d.clone();
    latent.tracks.push(with_chain(
        track(
            tid(2),
            ether_core::protocol::model::TrackKind::Audio,
            Some(tid(1)),
        ),
        &[delay],
    ));
    p.handle.publish(latent).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    // Beat 0 is queued for sample 20000; then the latency drops to 0 before beat 1.
    let (a, _) = render(&mut p.engine, 12_000, 500);
    d.version = 2;
    p.handle.publish(d).unwrap();
    let (b, _) = render(&mut p.engine, 36_000, 500);
    let mut all = a;
    all.extend(b);
    // Beat 0 (queued with the old latency) and beat 1 (no latency) both sound, in order.
    assert_eq!(onsets(&all), vec![20_000, 24_000]);
}
