//! Engine block-time harness for perf audit #2, #5 and #8 with real devices and the real
//! worker pool. Ignored by default; run in release:
//!
//! ```text
//! cargo test --release -p ether-native --test perf_audio -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Only public API, so the same file measures the code before and after.

mod multicore_real;

use std::sync::Arc;
use std::time::Instant;

use ether_core::graph::{ChainEntry, ClipContentDesc, ClipDesc, TrackDesc, WarpDesc};
use ether_core::protocol::model::{ClipId, MediaId, TrackId, TrackKind, Ulid, WarpMode};
use ether_core::{AudioSource, Engine, RenderGraphDesc, TransportControl, create};
use multicore_real::*;

const WORKERS: usize = 4;

/// Per-block times (µs) of `blocks` blocks.
fn times(engine: &mut Engine, blocks: usize) -> Vec<f64> {
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut t = Vec::with_capacity(blocks);
    for _ in 0..blocks {
        let start = Instant::now();
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        engine.process(&[], &mut outs, BLOCK);
        t.push(start.elapsed().as_secs_f64() * 1e6);
    }
    t
}

fn stats(mut t: Vec<f64>) -> String {
    t.sort_by(f64::total_cmp);
    let mean = t.iter().sum::<f64>() / t.len() as f64;
    let p50 = t[t.len() / 2];
    let p99 = t[t.len() * 99 / 100];
    format!(
        "mean {mean:7.1}  p50 {p50:7.1}  p99 {p99:7.1}  max {:7.1} µs",
        t[t.len() - 1]
    )
}

fn blocks(seconds: f64) -> usize {
    (seconds * SR as f64 / BLOCK as f64) as usize
}

/// #2 / #5: 32 tracks of synth → EQ → compressor → reverb/delay (+ groups, returns, a
/// drum rack, sidechains), 4 workers: playing, right after Stop (tails ringing), and
/// stopped once everything is quiet.
#[test]
#[ignore = "perf harness: run in release with --ignored --nocapture"]
fn stopped_and_idle_block_times() {
    let budget = BLOCK as f64 * 1e6 / SR as f64;
    println!("32 tracks, {BLOCK}-frame blocks ({budget:.0} µs budget), {WORKERS} workers");
    let mut p = engine(32, Some(Box::new(pool(WORKERS))));
    let _ = times(&mut p.engine, blocks(1.0));
    println!(
        "playing          {}",
        stats(times(&mut p.engine, blocks(4.0)))
    );
    p.handle.transport(TransportControl::Stop).unwrap();
    println!(
        "stopped, tails   {}",
        stats(times(&mut p.engine, blocks(1.0)))
    );
    let _ = times(&mut p.engine, blocks(12.0));
    println!(
        "stopped, quiet   {}",
        stats(times(&mut p.engine, blocks(4.0)))
    );
}

/// Stereo source at 48 kHz.
struct Tone(Vec<f32>);

impl AudioSource for Tone {
    fn channels(&self) -> u16 {
        2
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

/// #8: 8 audio tracks, each 24 back-to-back Complex-warped clips of 2 beats (played 1.25x
/// faster than recorded) and an EQ, 4 workers.
#[test]
#[ignore = "perf harness: run in release with --ignored --nocapture"]
fn complex_warp_block_times() {
    let mut p = create(config());
    p.handle.set_stretcher_factory(stretch().unwrap());
    let media = MediaId(Ulid(77));
    let src: Vec<f32> = (0..SR as usize * 30)
        .map(|i| {
            let t = i as f32 / SR as f32;
            0.3 * (t * 220.0 * std::f32::consts::TAU).sin() + 0.1 * (t * 1234.5).sin()
        })
        .collect();
    p.handle
        .add_source(media, Arc::new(Tone(src)) as Arc<dyn AudioSource>)
        .unwrap();
    let mut tracks = vec![TrackDesc {
        id: TrackId(Ulid(1)),
        kind: TrackKind::Master,
        output: None,
        ..audio_track(0, vec![], vec![])
    }];
    for t in 0..8u128 {
        let clips = (0..24u128)
            .map(|k| ClipDesc {
                id: ClipId(Ulid(10_000 + t * 100 + k)),
                start: k as f64 * 2.0,
                length: 2.0,
                offset: 0.0,
                looping: None,
                muted: false,
                content: ClipContentDesc::Audio {
                    media,
                    gain: 0.5,
                    transpose: 0.0,
                    fade_in: 0.0,
                    fade_out: 0.0,
                    fade_in_curve: Default::default(),
                    fade_out_curve: Default::default(),
                    reversed: false,
                    // 2 beats (1 s at 120 BPM) of timeline play 1.25 s of source.
                    warp: Some(WarpDesc {
                        mode: WarpMode::Complex,
                        markers: vec![(0.0, 0.0), (2.0, 1.25)],
                    }),
                    gain_envelope: None,
                    transpose_envelope: None,
                },
                envelopes: vec![],
            })
            .collect();
        let eq = ChainEntry {
            node: p.handle.add_node(ether_devices::eq::create()).unwrap(),
            enabled: true,
            sidechain: None,
        };
        tracks.push(audio_track(100 + t, clips, vec![eq]));
    }
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks,
            ..Default::default()
        })
        .unwrap();
    println!(
        "8 tracks x 24 Complex clips, {WORKERS} workers: {} stretchers",
        p.handle.stretcher_count()
    );
    p.engine.set_executor(Box::new(pool(WORKERS)));
    p.handle.transport(TransportControl::Play).unwrap();
    let _ = times(&mut p.engine, blocks(0.5));
    println!(
        "playing          {}",
        stats(times(&mut p.engine, blocks(10.0)))
    );
}

fn audio_track(id: u128, clips: Vec<ClipDesc>, chain: Vec<ChainEntry>) -> TrackDesc {
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id: TrackId(Ulid(id)),
        kind: TrackKind::Audio,
        chain,
        output: Some(TrackId(Ulid(1))),
        group: None,
        sends: vec![],
        volume: 0.5,
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
