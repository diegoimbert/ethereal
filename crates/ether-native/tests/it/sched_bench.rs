//! Scheduling benchmark on a project shaped like the one in OSS #196 (ignored by default):
//! `cargo test -p ether-native --release --test it sched_bench:: -- --ignored --nocapture`.
//!
//! 44 tracks whose devices burn a fixed CPU time (so only the scheduling differs between
//! runs): eight light drum tracks, one heavy rack track whose compressor is sidechained by
//! the kick, four heavy synths, twenty medium tracks (some sending to two returns), four
//! vocals; groups chained 10 → 8 → 16 → vocals → master, and a heavy serial mastering
//! chain. Prints p50 / p99 / worst `Engine::process` time per 256-frame buffer against the
//! deadline (5.33 ms at 48 kHz) and the buffers over it, for 4, 8 and 16 workers.
//!
//! `ETHER_SCHED_BENCH_BUFFERS` sets the buffers per worker count (default 1500).

use std::time::{Duration, Instant};

use ether_core::graph::{ChainEntry, SendDesc, TrackDesc};
use ether_core::protocol::model::{SendId, TrackId, TrackKind, Ulid};
use ether_core::{
    AudioBuffers, EngineConfig, EngineParts, Node, PrepareConfig, ProcessContext, ProcessStatus,
    RenderGraphDesc, TransportControl, create,
};
use ether_native::rt::enable_flush_denormals;
use ether_native::workers::{PoolOptions, WorkerPool};

const SR: u32 = 48_000;
const BLOCK: usize = 256;

/// Passes audio through after burning `us` microseconds (a heavy plugin's fixed cost).
struct Burn {
    us: u64,
    sidechain: bool,
}

fn burn(us: u64) {
    let until = Duration::from_micros(us);
    let t = Instant::now();
    while t.elapsed() < until {
        std::hint::spin_loop();
    }
}

impl Node for Burn {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        burn(self.us);
        audio.pass_through();
        ProcessStatus::Continue
    }
    fn sidechain_inputs(&self) -> u16 {
        if self.sidechain { 2 } else { 0 }
    }
    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        _: &[&[f32]],
    ) -> ProcessStatus {
        self.process(ctx, audio)
    }
}

fn tid(n: u128) -> TrackId {
    TrackId(Ulid(n))
}

fn track(id: u128, kind: TrackKind, output: Option<u128>) -> TrackDesc {
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id: tid(id),
        kind,
        chain: vec![],
        output: output.map(tid),
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
        clips: vec![],
        automation: vec![],
        racks: Vec::new(),
        expression: Default::default(),
        hw_io: Vec::new(),
        sidechain_taps: Vec::new(),
    }
}

/// The #196 shape (costs in µs; ~15.6 ms of work per buffer, critical path ~4.4 ms:
/// kick → rack → groups 8, 16, vocals → master).
fn project(parts: &mut EngineParts) -> RenderGraphDesc {
    let h = &mut parts.handle;
    let mut chain = |t: &mut TrackDesc, costs: &[u64], sidechain: Option<u128>| {
        for &us in costs {
            let node = h
                .add_node(Box::new(Burn {
                    us,
                    sidechain: false,
                }))
                .unwrap();
            t.chain.push(ChainEntry {
                node,
                enabled: true,
                sidechain: None,
            });
        }
        if let Some(s) = sidechain {
            let node = h
                .add_node(Box::new(Burn {
                    us: 150,
                    sidechain: true,
                }))
                .unwrap();
            t.chain.push(ChainEntry {
                node,
                enabled: true,
                sidechain: Some(tid(s)),
            });
        }
    };
    let (master, vocals, g16, g8, g10, reverb, delay) = (1, 2, 3, 4, 5, 6, 7);
    let mut tracks = Vec::new();
    let mut t = track(master, TrackKind::Master, None);
    chain(&mut t, &[700, 700], None); // "Gullfoss" then "Ozone", serial
    tracks.push(t);
    for (id, out) in [(vocals, master), (g16, vocals), (g8, g16), (g10, g8)] {
        let mut t = track(id, TrackKind::Group, Some(out));
        chain(&mut t, &[150], None);
        tracks.push(t);
    }
    for (id, us) in [(reverb, 400), (delay, 200)] {
        let mut t = track(id, TrackKind::Return, Some(master));
        chain(&mut t, &[us], None);
        tracks.push(t);
    }
    let kick = 10;
    for i in 0..8 {
        let mut t = track(kick + i, TrackKind::Midi, Some(g10));
        chain(&mut t, &[60], None);
        tracks.push(t);
    }
    // The heavy rack, keyed by the kick.
    let mut t = track(20, TrackKind::Midi, Some(g8));
    chain(&mut t, &[600, 600, 600], Some(kick));
    tracks.push(t);
    for (i, us) in [1000, 800, 1200, 700].into_iter().enumerate() {
        let mut t = track(21 + i as u128, TrackKind::Midi, Some(g16));
        chain(&mut t, &[us], None);
        tracks.push(t);
    }
    for i in 0..20u128 {
        let out = [g10, g8, g16][i as usize % 3];
        let mut t = track(30 + i, TrackKind::Midi, Some(out));
        chain(&mut t, &[200], None);
        if i % 4 == 0 {
            t.sends.push(SendDesc {
                id: SendId(Ulid(1000 + i)),
                to: tid(if i % 8 == 0 { reverb } else { delay }),
                level: 0.5,
                pre_fader: false,
            });
        }
        tracks.push(t);
    }
    for i in 0..4 {
        let mut t = track(50 + i, TrackKind::Audio, Some(vocals));
        chain(&mut t, &[250], None);
        t.sends.push(SendDesc {
            id: SendId(Ulid(2000 + i)),
            to: tid(reverb),
            level: 0.3,
            pre_fader: false,
        });
        tracks.push(t);
    }
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

#[test]
#[ignore = "benchmark: run explicitly with --ignored --nocapture"]
fn bench_196_shape() {
    enable_flush_denormals();
    let buffers: usize = std::env::var("ETHER_SCHED_BENCH_BUFFERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    let deadline = Duration::from_micros(BLOCK as u64 * 1_000_000 / SR as u64);
    println!(
        "#196 shape: 44 tracks, {BLOCK}-frame buffers, deadline {:.2} ms, {buffers} buffers",
        deadline.as_secs_f64() * 1e3
    );
    for workers in [4usize, 8, 16] {
        let mut parts = create(EngineConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            max_nodes: 256,
            max_events_per_block: 256,
            ..EngineConfig::default()
        });
        let desc = project(&mut parts);
        parts.handle.publish(desc).unwrap();
        parts.handle.transport(TransportControl::Play).unwrap();
        parts.engine.set_executor(Box::new(WorkerPool::with_options(
            workers,
            PoolOptions {
                period: deadline,
                realtime: false,
                flush_denormals: true,
            },
        )));
        let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
        let mut times = Vec::with_capacity(buffers);
        for b in 0..buffers + 20 {
            let t = Instant::now();
            {
                let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
                parts.engine.process(&[], &mut outs, BLOCK);
            }
            if b >= 20 {
                times.push(t.elapsed()); // after a short warm-up
            }
            parts.gc.collect();
        }
        times.sort();
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        let over = times.iter().filter(|&&t| t > deadline).count();
        println!(
            "workers {workers:>2}: p50 {:.2} ms  p99 {:.2} ms  worst {:.2} ms  over deadline \
             {over}/{buffers}",
            ms(percentile(&times, 0.5)),
            ms(percentile(&times, 0.99)),
            ms(*times.last().unwrap()),
        );
    }
}
