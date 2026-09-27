//! Shared by the `multicore*` tests: a project of real built-in devices (the benchmark
//! shape: N MIDI tracks with a synth and effects, groups, two returns, sidechained
//! compressors, a drum rack of samplers) and a render loop that can use the worker pool.
#![allow(dead_code)]

#[path = "../../ether-core/tests/parallel_graphs.rs"]
pub mod graphs;

use std::sync::Arc;
use std::time::{Duration, Instant};

use ether_core::graph::{
    ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, PadDesc, RackDesc, SendDesc, TrackDesc,
};
use ether_core::parallel::ParallelExecutor;
use ether_core::protocol::model::{
    ClipId, DrumPadId, SendId, TrackId, TrackKind, Ulid,
};
use ether_core::{
    Device, Engine, EngineConfig, EngineHandle, EngineParts, RenderGraphDesc,
    TransportControl, create,
};
use ether_native::workers::{PoolOptions, WorkerPool};

pub const SR: u32 = 48_000;
pub const BLOCK: usize = 256;

pub fn config() -> EngineConfig {
    EngineConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_nodes: 2048,
        max_events_per_block: 512,
        ..EngineConfig::default()
    }
}

/// A pool for tests: no RT priority (shared CI/dev machines), flush-to-zero like the
/// audio thread.
pub fn pool(workers: usize) -> WorkerPool {
    WorkerPool::with_options(
        workers,
        PoolOptions {
            period: Duration::from_micros(BLOCK as u64 * 1_000_000 / SR as u64),
            realtime: false,
            flush_denormals: true,
        },
    )
}

fn tid(n: u128) -> TrackId {
    TrackId(Ulid(n))
}

fn track(id: TrackId, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
    TrackDesc {
        id,
        kind,
        chain: vec![],
        output,
        group: None,
        sends: vec![],
        volume: 0.5,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        monitor: false,
        armed: false,
        clips: vec![],
        automation: vec![],
        racks: Vec::new(),
    }
}

fn add(h: &mut EngineHandle, node: Box<dyn Device>) -> ChainEntry {
    ChainEntry {
        node: h.add_node(node).expect("node table"),
        enabled: true,
        sidechain: None,
    }
}

fn notes(seed: usize, keys: &[u8]) -> ClipDesc {
    let notes = (0..16)
        .map(|k| NoteDesc {
            start: k as f64 * 0.5,
            duration: 0.4 + (seed % 3) as f64 * 0.3,
            key: keys[(k + seed) % keys.len()],
            velocity: 0.8,
            release_velocity: 0.0,
        })
        .collect();
    ClipDesc {
        id: ClipId(Ulid(seed as u128 + 1)),
        start: 0.0,
        length: 8.0,
        offset: 0.0,
        looping: Some((0.0, 8.0)),
        muted: false,
        content: ClipContentDesc::Midi { notes },
        envelopes: vec![],
    }
}

/// `tracks` MIDI tracks (synth → EQ → compressor → reverb/delay), 4 groups, 2 returns,
/// every 8th track's compressor sidechained from track 0, every 16th track a drum rack of
/// samplers.
pub fn build(h: &mut EngineHandle, tracks: usize) -> RenderGraphDesc {
    use ether_devices::{Compressor, Delay, Sampler, Synth, drum_rack, eq, limiter, reverb};
    let master_id = tid(1);
    let mut master = track(master_id, TrackKind::Master, None);
    master.chain.push(add(h, limiter::create()));
    let mut all = vec![];
    let groups: Vec<TrackId> = (0..4).map(|g| tid(100 + g)).collect();
    for &g in &groups {
        let mut t = track(g, TrackKind::Group, Some(master_id));
        t.chain.push(add(h, eq::create()));
        all.push(t);
    }
    let returns = [tid(200), tid(201)];
    for (k, &r) in returns.iter().enumerate() {
        let mut t = track(r, TrackKind::Return, Some(master_id));
        t.chain.push(add(
            h,
            if k == 0 {
                reverb::create()
            } else {
                Box::new(Delay::new())
            },
        ));
        all.push(t);
    }
    let sample = graphs::source();
    for i in 0..tracks {
        let id = tid(1000 + i as u128);
        let g = groups[i % groups.len()];
        let mut t = track(id, TrackKind::Midi, Some(g));
        t.group = Some(g);
        if i % 16 == 15 {
            let rack = add(h, drum_rack::create());
            let pads = (0..4)
                .map(|p| PadDesc {
                    pad: DrumPadId(Ulid(50_000 + i as u128 * 8 + p as u128)),
                    note: 36 + p as u8,
                    choke_group: (p >= 2).then_some(1),
                    chain: vec![add(h, Box::new(Sampler::new(Some(sample.clone()))))],
                    volume: 0.8,
                    pan: p as f32 * 0.3 - 0.45,
                    mute: false,
                })
                .collect();
            t.racks.push(RackDesc {
                rack: rack.node,
                pads,
            });
            t.chain.push(rack);
            t.clips.push(notes(i, &[36, 37, 38, 39]));
        } else {
            t.chain.push(add(h, Box::new(Synth::new())));
            t.clips.push(notes(i, &[48, 52, 55, 60, 64, 67]));
        }
        t.chain.push(add(h, eq::create()));
        let mut comp = add(h, Box::new(Compressor::new()));
        if i % 8 == 7 {
            comp.sidechain = Some(tid(1000));
        }
        t.chain.push(comp);
        if i % 2 == 0 {
            t.chain.push(add(h, reverb::create()));
        } else {
            t.chain.push(add(h, Box::new(Delay::new())));
        }
        t.sends.push(SendDesc {
            id: SendId(Ulid(i as u128 + 1)),
            to: returns[i % 2],
            level: 0.3,
            pre_fader: i % 3 == 0,
        });
        t.pan = (i % 7) as f32 / 3.0 - 1.0;
        all.push(t);
    }
    all.push(master);
    RenderGraphDesc {
        version: 1,
        tracks: all,
        ..Default::default()
    }
}

/// An engine with `build(tracks)` published and playing, with `executor`.
pub fn engine(tracks: usize, executor: Option<Box<dyn ParallelExecutor>>) -> EngineParts {
    let mut parts = create(config());
    let desc = build(&mut parts.handle, tracks);
    parts.handle.publish(desc).unwrap();
    parts.handle.transport(TransportControl::Play).unwrap();
    if let Some(e) = executor {
        parts.engine.set_executor(e);
    }
    parts
}

/// Render `blocks` blocks; returns (left ++ right, time spent in `process`).
pub fn run(engine: &mut Engine, blocks: usize) -> (Vec<f32>, Duration) {
    let mut out = vec![0.0f32; blocks * BLOCK * 2];
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut spent = Duration::ZERO;
    for b in 0..blocks {
        let t = Instant::now();
        {
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            engine.process(&[], &mut outs, BLOCK);
        }
        spent += t.elapsed();
        out[b * BLOCK..(b + 1) * BLOCK].copy_from_slice(&l);
        let o = blocks * BLOCK;
        out[o + b * BLOCK..o + (b + 1) * BLOCK].copy_from_slice(&r);
    }
    (out, spent)
}

pub fn first_diff(a: &[f32], b: &[f32]) -> Option<usize> {
    graphs::first_diff(a, b)
}

pub fn stretch() -> Option<Arc<dyn ether_stretch::StretcherFactory>> {
    Some(Arc::new(ether_stretch::SignalsmithFactory::default()))
}
