//! Real-time engine benchmark (engine-perf): per-buffer times against the buffer deadline,
//! on synthetic projects whose device costs are known, with the real worker pool.
//! Ignored by default; run in release, alone (`--test-threads 1`):
//!
//! ```text
//! cargo test --release -p ether-native --test it engine_bench:: -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Every device is a [`Burn`] node: a fixed amount of deterministic floating-point work per
//! frame (calibrated once per run to `ns/frame`), so a scenario's total work, its critical
//! path through the track graph and its level-by-level bound are known, and the measured
//! buffer times can be compared with what an ideal scheduler would reach:
//!
//! - `ideal` = max(critical path, total work / threads): no scheduler beats it;
//! - `levels` = Σ over dependency levels of the level's slowest track: what a scheduler
//!   that waits for a whole level before starting the next can reach at best.
//!
//! **Paced** runs (the default) wait for each buffer's deadline before rendering the next,
//! like an audio device, so workers park between buffers and every buffer pays their
//! wake-up. Unpaced runs render back to back (throughput). Every run is checked bit-identical
//! to the sequential engine.
//!
//! Environment:
//! - `ETHER_BENCH_SCENARIOS`: comma list (default: all), see [`SCENARIOS`];
//! - `ETHER_BENCH_WORKERS`: comma list of worker counts (default `0,1,3,7` capped by cores);
//! - `ETHER_BENCH_BLOCK`: buffer frames (default 256); `ETHER_BENCH_SECONDS` (default 4);
//! - `ETHER_BENCH_PACED=0`: back to back; `ETHER_BENCH_RT=1`: real-time priority for the
//!   workers and the render thread (as the host gives its device callback);
//! - `ETHER_BENCH_SCALE`: multiply every device cost (default 1);
//! - `ETHER_BENCH_HOGS`: that many normal-priority threads keep the CPU busy during the runs
//!   (what the UI, plugin editors and other apps do to a real session; default 0), to see
//!   what thread priorities buy;
//! - `ETHER_BENCH_JSON`: append one JSON line per run to this file (compare runs over time).
//!
//! Only public API, so the same file measures the engine before and after a change.

use std::collections::HashMap;
use std::io::Write as _;
use std::time::{Duration, Instant};

use ether_core::graph::{ChainEntry, SendDesc, TrackDesc};
use ether_core::parallel::{Fork, JobGraph, ParallelExecutor, SequentialExecutor};
use ether_core::protocol::model::{RackChainId, SendId, TrackId, TrackKind, Ulid};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::{
    AudioBuffers, EngineConfig, EngineHandle, Node, PrepareConfig, ProcessContext, ProcessStatus,
    RenderGraphDesc, TransportControl, create,
};
use ether_native::rt::enable_flush_denormals;
use ether_native::workers::{PoolOptions, WorkerPool, physical_cores};

const SR: u32 = 48_000;

// ---------------------------------------------------------------------------------------
// The CPU-burning device.

/// A device that costs `iters` dependent multiply-adds per frame (latency-bound, so its
/// time per frame hardly depends on the data), plus a tiny deterministic signal so nothing
/// is ever silent. Sidechained, it mixes a little of its key in (a real dependency).
struct Burn {
    iters: u32,
    state: f32,
    phase: u32,
    sidechain: bool,
}

impl Burn {
    fn new(iters: u32, sidechain: bool) -> Self {
        Self {
            iters,
            state: 0.0,
            phase: 0,
            sidechain,
        }
    }

    #[inline(never)]
    fn burn(&mut self, input: &[f32], left: &mut [f32], key: Option<&[f32]>) {
        let mut s = self.state;
        for (i, (o, &x)) in left.iter_mut().zip(input).enumerate() {
            let mut v = x + ((self.phase % 97) as f32 - 48.0) * 1e-4;
            if let Some(k) = key {
                v += k[i] * 1e-3;
            }
            self.phase = self.phase.wrapping_add(1);
            for _ in 0..self.iters {
                s = s * 0.999_9 + v * 1e-4;
            }
            *o = v + s;
        }
        self.state = s;
    }
}

impl Node for Burn {
    fn prepare(&mut self, _: &PrepareConfig) {}

    fn reset(&mut self) {
        self.state = 0.0;
        self.phase = 0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.process_sidechain(ctx, audio, &[])
    }

    fn sidechain_inputs(&self) -> u16 {
        if self.sidechain { 2 } else { 0 }
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        let n = ctx.frames;
        let (ins, outs) = (audio.inputs, &mut *audio.outputs);
        let key = sidechain.first().map(|k| &k[..n]);
        let [l, r] = outs else {
            return ProcessStatus::Continue;
        };
        self.burn(&ins[0][..n], &mut l[..n], key);
        for (o, (&x, &y)) in r[..n].iter_mut().zip(ins[1][..n].iter().zip(&l[..n])) {
            *o = x * 0.5 + y * 0.5;
        }
        ProcessStatus::Continue
    }
}

/// Nanoseconds per `Burn` iteration and frame on this machine (median of a few runs).
fn calibrate() -> f64 {
    let frames = 256;
    let iters = 2_000;
    let input = vec![0.1f32; frames];
    let mut out = vec![0.0f32; frames];
    let mut b = Burn::new(iters, false);
    let mut runs: Vec<f64> = (0..15)
        .map(|_| {
            let t = Instant::now();
            b.burn(&input, &mut out, None);
            std::hint::black_box(&out);
            t.elapsed().as_secs_f64() * 1e9 / (frames as f64 * iters as f64)
        })
        .collect();
    runs.sort_by(f64::total_cmp);
    runs[runs.len() / 2]
}

// ---------------------------------------------------------------------------------------
// Scenarios: tracks with device costs in ns per frame.

#[derive(Clone, Debug)]
struct SimTrack {
    id: u128,
    kind: TrackKind,
    /// Destination bus (`None` = the hardware: master).
    output: Option<u128>,
    /// Serial device costs (ns/frame), in order.
    chain: Vec<f64>,
    /// An instrument rack before `chain`: its parallel chains' device costs.
    rack: Vec<Vec<f64>>,
    /// The first device of `chain` is sidechained from this track.
    sidechain: Option<u128>,
    /// Post-fader sends to these returns.
    sends: Vec<u128>,
}

impl SimTrack {
    fn new(id: u128, kind: TrackKind, output: Option<u128>, chain: &[f64]) -> Self {
        Self {
            id,
            kind,
            output,
            chain: chain.to_vec(),
            rack: vec![],
            sidechain: None,
            sends: vec![],
        }
    }

    fn cost(&self) -> f64 {
        self.chain.iter().sum::<f64>() + self.rack.iter().flatten().sum::<f64>()
    }

    /// Tracks this one reads (they must finish first).
    fn sources<'a>(&'a self, all: &'a [SimTrack]) -> impl Iterator<Item = u128> + 'a {
        all.iter()
            .filter(move |t| t.output == Some(self.id) || t.sends.contains(&self.id))
            .map(|t| t.id)
            .chain(self.sidechain)
    }
}

const MASTER: u128 = 1;

/// The owner-shaped project: drums (a light kick keying the heaviest track's compressor), two
/// instrument groups, vocals, two returns, a mix bus of groups (nested) and a heavy serial
/// master (two mastering plugins). `sidechain = false` drops the one sidechain edge.
fn owner(sidechain: bool) -> Vec<SimTrack> {
    use TrackKind::*;
    let mut t = vec![SimTrack::new(MASTER, Master, None, &[1500.0, 2500.0])];
    let (mixbus, drums, keys, pads, vox) = (2, 3, 4, 5, 6);
    let (reverb, delay) = (7, 8);
    t.push(SimTrack::new(mixbus, Group, Some(MASTER), &[150.0, 150.0]));
    t.push(SimTrack::new(drums, Group, Some(mixbus), &[100.0, 100.0]));
    t.push(SimTrack::new(keys, Group, Some(mixbus), &[200.0]));
    t.push(SimTrack::new(pads, Group, Some(mixbus), &[200.0]));
    t.push(SimTrack::new(vox, Group, Some(mixbus), &[150.0, 150.0]));
    t.push(SimTrack::new(reverb, Return, Some(MASTER), &[1000.0]));
    t.push(SimTrack::new(delay, Return, Some(MASTER), &[300.0]));
    let kick = 100;
    t.push(SimTrack::new(kick, Midi, Some(drums), &[20.0]));
    for i in 0..7 {
        t.push(SimTrack::new(101 + i, Midi, Some(drums), &[60.0, 60.0]));
    }
    for i in 0..16 {
        let mut s = SimTrack::new(
            200 + i,
            Midi,
            Some(if i < 8 { keys } else { pads }),
            &[600.0, 150.0, 150.0, 150.0],
        );
        s.sends = vec![if i % 2 == 0 { reverb } else { delay }];
        t.push(s);
    }
    // The heavy rack: three parallel chains of synth + two effects, then a compressor
    // keyed by the kick.
    let mut heavy = SimTrack::new(300, Midi, Some(mixbus), &[100.0, 200.0]);
    heavy.rack = vec![vec![800.0, 400.0, 400.0]; 3];
    heavy.sidechain = sidechain.then_some(kick);
    heavy.sends = vec![reverb];
    t.push(heavy);
    for i in 0..8 {
        let mut s = SimTrack::new(400 + i, Audio, Some(vox), &[200.0; 4]);
        s.sends = vec![reverb];
        t.push(s);
    }
    t
}

/// `n` independent tracks of `cost` ns/frame each into a light master.
fn flat(n: u128, cost: f64) -> Vec<SimTrack> {
    let mut t = vec![SimTrack::new(MASTER, TrackKind::Master, None, &[50.0])];
    for i in 0..n {
        t.push(SimTrack::new(
            100 + i,
            TrackKind::Midi,
            Some(MASTER),
            &[cost / 2.0, cost / 2.0],
        ));
    }
    t
}

/// `n` independent tracks of `devices` devices of `cost` ns/frame each into a bare master
/// (per-track and per-device engine overhead).
fn chains(n: u128, devices: usize, cost: f64) -> Vec<SimTrack> {
    let mut t = vec![SimTrack::new(MASTER, TrackKind::Master, None, &[])];
    for i in 0..n {
        t.push(SimTrack::new(
            100 + i,
            TrackKind::Midi,
            Some(MASTER),
            &vec![cost; devices],
        ));
    }
    t
}

/// `depth` nested groups, each with `width` light tracks: a long serial chain of buses.
fn deep(depth: u128, width: u128) -> Vec<SimTrack> {
    let mut t = vec![SimTrack::new(MASTER, TrackKind::Master, None, &[100.0])];
    let mut parent = MASTER;
    for d in 0..depth {
        let g = 10 + d;
        t.push(SimTrack::new(g, TrackKind::Group, Some(parent), &[100.0]));
        for w in 0..width {
            t.push(SimTrack::new(
                1000 + d * 100 + w,
                TrackKind::Midi,
                Some(g),
                &[200.0],
            ));
        }
        parent = g;
    }
    t
}

/// Name, description, tracks.
type Scenario = (&'static str, &'static str, fn() -> Vec<SimTrack>);

const SCENARIOS: &[Scenario] = &[
    (
        "owner",
        "owner-shaped: 41 tracks, a heavy rack keyed by the kick, nested groups, heavy master",
        || owner(true),
    ),
    ("owner-nosc", "the same without the sidechain", || {
        owner(false)
    }),
    ("flat64", "64 independent tracks of 400 ns/frame", || {
        flat(64, 400.0)
    }),
    (
        "tiny64",
        "64 independent tracks of 4 ns/frame (scheduling overhead)",
        || flat(64, 4.0),
    ),
    (
        "rack4",
        "one track: an instrument rack of 4 chains of 1600 ns/frame (rack chain parallelism)",
        || {
            let mut t = vec![SimTrack::new(MASTER, TrackKind::Master, None, &[50.0])];
            let mut s = SimTrack::new(100, TrackKind::Midi, Some(MASTER), &[50.0]);
            s.rack = vec![vec![800.0, 400.0, 400.0]; 4];
            t.push(s);
            t
        },
    ),
    (
        "empty64",
        "64 tracks without devices (per-track overhead)",
        || chains(64, 0, 0.0),
    ),
    (
        "chain8x64",
        "64 tracks of 8 near-free devices (per-device overhead)",
        || chains(64, 8, 0.0),
    ),
    (
        "deep8",
        "8 nested groups of 4 tracks (serial buses)",
        || deep(8, 4),
    ),
];

// ---------------------------------------------------------------------------------------
// Cost model.

struct Model {
    /// Σ device costs, µs per buffer.
    total: f64,
    /// Longest dependency path, µs per buffer.
    critical: f64,
    /// Σ over levels of the slowest track of the level, µs per buffer.
    levels: f64,
    n_levels: usize,
    /// Per track (in `tracks` order): dependency level, cost (µs), sources (indices).
    level: Vec<usize>,
    cost: Vec<f64>,
    sources: Vec<Vec<usize>>,
}

impl Model {
    fn scaled(mut self, k: f64) -> Self {
        self.total *= k;
        self.critical *= k;
        self.levels *= k;
        self.cost.iter_mut().for_each(|c| *c *= k);
        self
    }

    /// Simulated buffer time (µs) with `threads` threads (audio thread + workers) and no
    /// overhead: greedy list scheduling, either level by level with a barrier between levels
    /// (single-track levels on the audio thread), as the engine dispatches today, or
    /// dependency-driven (a track starts when its sources finished; ready tracks with the
    /// longest remaining path first).
    fn simulate(&self, threads: usize, barrier: bool) -> f64 {
        let n = self.cost.len();
        let threads = threads.max(1);
        if barrier {
            let mut t = 0.0f64;
            for l in 0..self.n_levels {
                let mut free = vec![t; threads];
                let mut end = t;
                for i in (0..n).filter(|&i| self.level[i] == l) {
                    let k = (0..threads)
                        .min_by(|&a, &b| free[a].total_cmp(&free[b]))
                        .unwrap();
                    free[k] += self.cost[i];
                    end = end.max(free[k]);
                }
                t = end;
            }
            return t;
        }
        // Remaining path from each track to the end (its cost + its slowest consumer's).
        let mut consumers = vec![vec![]; n];
        for (i, s) in self.sources.iter().enumerate() {
            for &j in s {
                consumers[j].push(i);
            }
        }
        let mut rank = vec![0.0f64; n];
        let mut by_level: Vec<usize> = (0..n).collect();
        by_level.sort_by_key(|&i| std::cmp::Reverse(self.level[i]));
        for &i in &by_level {
            let tail = consumers[i].iter().map(|&c| rank[c]).fold(0.0, f64::max);
            rank[i] = self.cost[i] + tail;
        }
        let mut finish = vec![f64::NAN; n];
        let mut free = vec![0.0f64; threads];
        let mut done = 0;
        while done < n {
            // The thread that frees first takes the ready track with the longest path; if
            // none is ready yet, it idles until the next one becomes ready.
            let k = (0..threads)
                .min_by(|&a, &b| free[a].total_cmp(&free[b]))
                .unwrap();
            let ready_at = |i: usize| {
                self.sources[i]
                    .iter()
                    .map(|&j| finish[j])
                    .fold(0.0, f64::max)
            };
            let unscheduled = (0..n).filter(|&i| finish[i].is_nan());
            let schedulable: Vec<usize> = unscheduled
                .filter(|&i| self.sources[i].iter().all(|&j| !finish[j].is_nan()))
                .collect();
            let now = free[k];
            let pick = schedulable
                .iter()
                .copied()
                .filter(|&i| ready_at(i) <= now)
                .max_by(|&a, &b| rank[a].total_cmp(&rank[b]))
                .unwrap_or_else(|| {
                    *schedulable
                        .iter()
                        .min_by(|&&a, &&b| ready_at(a).total_cmp(&ready_at(b)))
                        .expect("a DAG always has a schedulable track")
                });
            let start = now.max(ready_at(pick));
            finish[pick] = start + self.cost[pick];
            free[k] = finish[pick];
            done += 1;
        }
        finish.iter().copied().fold(0.0, f64::max)
    }
}

fn model(tracks: &[SimTrack], frames: usize) -> Model {
    let us = |ns: f64| (ns * frames as f64 / 1e3).max(0.0);
    let by_id: HashMap<u128, &SimTrack> = tracks.iter().map(|t| (t.id, t)).collect();
    let mut finish: HashMap<u128, f64> = HashMap::new();
    let mut level: HashMap<u128, usize> = HashMap::new();
    fn visit(
        id: u128,
        all: &[SimTrack],
        by_id: &HashMap<u128, &SimTrack>,
        finish: &mut HashMap<u128, f64>,
        level: &mut HashMap<u128, usize>,
    ) -> (f64, usize) {
        if let (Some(&f), Some(&l)) = (finish.get(&id), level.get(&id)) {
            return (f, l);
        }
        let t = by_id[&id];
        let (mut start, mut lv) = (0.0f64, 0usize);
        let mut any = false;
        for s in t.sources(all).collect::<Vec<_>>() {
            let (f, l) = visit(s, all, by_id, finish, level);
            start = start.max(f);
            lv = lv.max(l);
            any = true;
        }
        let l = if any { lv + 1 } else { 0 };
        let f = start + t.cost();
        finish.insert(id, f);
        level.insert(id, l);
        (f, l)
    }
    for t in tracks {
        visit(t.id, tracks, &by_id, &mut finish, &mut level);
    }
    let n_levels = level.values().max().map_or(0, |l| l + 1);
    let mut slowest = vec![0.0f64; n_levels];
    for t in tracks {
        let l = level[&t.id];
        slowest[l] = slowest[l].max(t.cost());
    }
    let index: HashMap<u128, usize> = tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
    Model {
        total: us(tracks.iter().map(SimTrack::cost).sum()),
        critical: us(finish.values().copied().fold(0.0, f64::max)),
        levels: us(slowest.iter().sum()),
        n_levels,
        level: tracks.iter().map(|t| level[&t.id]).collect(),
        cost: tracks.iter().map(|t| us(t.cost())).collect(),
        sources: tracks
            .iter()
            .map(|t| t.sources(tracks).map(|s| index[&s]).collect())
            .collect(),
    }
}

// ---------------------------------------------------------------------------------------
// Building the engine.

fn tid(n: u128) -> TrackId {
    TrackId(Ulid(n))
}

fn track_desc(id: TrackId, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id,
        kind,
        chain: vec![],
        output,
        group: None,
        sends: vec![],
        volume: 0.7,
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

fn build(
    h: &mut EngineHandle,
    tracks: &[SimTrack],
    ns_per_iter: f64,
    scale: f64,
) -> RenderGraphDesc {
    let iters = |ns: f64| ((ns * scale / ns_per_iter).round() as u32).max(1);
    let entry = |h: &mut EngineHandle, ns: f64, sidechain: Option<u128>| ChainEntry {
        node: h
            .add_node(Box::new(Burn::new(iters(ns), sidechain.is_some())))
            .expect("node table"),
        enabled: true,
        sidechain: sidechain.map(tid),
    };
    let mut send_id = 1u128;
    let descs = tracks
        .iter()
        .map(|s| {
            let output = s.output.map(tid);
            let mut t = track_desc(tid(s.id), s.kind, output);
            // Groups nest: a track's group is its destination bus when that is a group.
            t.group = output.filter(|o| {
                tracks
                    .iter()
                    .any(|g| tid(g.id) == *o && g.kind == TrackKind::Group)
            });
            if !s.rack.is_empty() {
                // The rack node itself is a pass-through-sized device (1 iteration).
                let rack = entry(h, 0.0, None);
                let chains = s
                    .rack
                    .iter()
                    .enumerate()
                    .map(|(k, c)| RackChainDesc {
                        id: RackChainId(Ulid(s.id * 100 + k as u128)),
                        chain: c.iter().map(|&ns| entry(h, ns, None)).collect(),
                        volume: 0.5,
                        pan: 0.0,
                        mute: false,
                        keys: (0, 127),
                        velocities: (0, 127),
                        select: (0, 127),
                    })
                    .collect();
                t.chain_racks.push(ChainRackDesc {
                    rack: rack.node,
                    kind: ChainRackKind::Instrument,
                    chains,
                    selector: 0,
                });
                t.chain.push(rack);
            }
            for (k, &ns) in s.chain.iter().enumerate() {
                let sc = if k == 0 { s.sidechain } else { None };
                t.chain.push(entry(h, ns, sc));
            }
            t.sends = s
                .sends
                .iter()
                .map(|&to| {
                    send_id += 1;
                    SendDesc {
                        id: SendId(Ulid(send_id)),
                        to: tid(to),
                        level: 0.3,
                        pre_fader: false,
                    }
                })
                .collect();
            t
        })
        .collect();
    RenderGraphDesc {
        version: 1,
        tracks: descs,
        ..Default::default()
    }
}

fn config(block: usize) -> EngineConfig {
    EngineConfig {
        sample_rate: SR,
        max_block_size: block,
        max_nodes: 1024,
        max_events_per_block: 256,
        ..EngineConfig::default()
    }
}

// ---------------------------------------------------------------------------------------
// Running.

struct Run {
    /// Per-buffer `Engine::process` time, µs.
    times: Vec<f64>,
    /// Buffers that started after their deadline (paced: the previous one overran).
    late_starts: usize,
    /// Process CPU time / wall time (cores busy, spinning included), when known.
    cores_busy: Option<f64>,
    out: Vec<f32>,
    /// When the measured buffers started (after the warm-up).
    measured_from: Instant,
}

/// Process CPU time (user + system), from `/proc/self/stat` (Linux only).
fn cpu_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // Fields 14 and 15 of stat(5) (utime, stime), here 0-based from field 3.
    let ticks: u64 = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
    Some(Duration::from_secs_f64(ticks as f64 / 100.0))
}

fn render(
    tracks: &[SimTrack],
    executor: Box<dyn ParallelExecutor>,
    opts: &Opts,
    ns_per_iter: f64,
) -> Run {
    let mut parts = create(config(opts.block));
    let desc = build(&mut parts.handle, tracks, ns_per_iter, opts.scale);
    parts.handle.publish(desc).expect("graph compiles");
    parts.handle.transport(TransportControl::Play).unwrap();
    parts.engine.set_executor(executor);
    let engine = &mut parts.engine;
    let (handle, gc) = (&mut parts.handle, &mut parts.gc);
    let mut polled = ether_core::EngineOutputs::default();
    let block = opts.block;
    let period = Duration::from_secs_f64(block as f64 / SR as f64);
    let blocks = ((opts.seconds * SR as f64) as usize / block).max(1);
    let warmup = (0.25 * SR as f64) as usize / block;
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut out = Vec::with_capacity(blocks * block * 2);
    let mut times = Vec::with_capacity(blocks);
    let mut late_starts = 0;
    let mut next = Instant::now();
    let mut cpu0 = None;
    let mut wall0 = Instant::now();
    for b in 0..warmup + blocks {
        if b == warmup {
            cpu0 = cpu_time();
            wall0 = Instant::now();
            next = wall0;
        }
        if opts.paced && b >= warmup {
            let now = Instant::now();
            if now > next + period / 4 {
                late_starts += 1;
                next = now;
            }
            // Sleep most of the wait, spin the rest (a device callback starts on time).
            while Instant::now() + Duration::from_micros(300) < next {
                std::thread::sleep(Duration::from_micros(200));
            }
            while Instant::now() < next {
                std::hint::spin_loop();
            }
        }
        let t = Instant::now();
        {
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            engine.process(&[], &mut outs, block);
        }
        let dt = t.elapsed();
        if b >= warmup {
            times.push(dt.as_secs_f64() * 1e6);
            out.extend_from_slice(&l);
            out.extend_from_slice(&r);
            next += period;
        }
        // The controller's side, at UI rate like a host (meters, measured job times and
        // their re-ranking, garbage): outside the timed render.
        if b % 8 == 0 {
            handle.poll(&mut polled);
            gc.collect();
        }
    }
    let wall = wall0.elapsed();
    let cores_busy = match (cpu0, cpu_time()) {
        (Some(a), Some(b)) => Some((b - a).as_secs_f64() / wall.as_secs_f64()),
        _ => None,
    };
    Run {
        times,
        late_starts,
        cores_busy,
        out,
        measured_from: wall0,
    }
}

struct Opts {
    block: usize,
    seconds: f64,
    paced: bool,
    realtime: bool,
    scale: f64,
}

/// Busy threads at normal priority until dropped (`ETHER_BENCH_HOGS`).
struct Hogs {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Hogs {
    fn start(n: usize) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let threads = (0..n)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    // Integer and cache work, a little sleep now and then like a UI thread.
                    let mut v = vec![1u64; 1 << 16];
                    let mut k = 0usize;
                    while !stop.load(Ordering::Relaxed) {
                        for x in v.iter_mut() {
                            *x = x.wrapping_mul(6364136223846793005).wrapping_add(k as u64);
                        }
                        k += 1;
                        if k.is_multiple_of(64) {
                            std::thread::sleep(Duration::from_micros(100));
                        }
                    }
                    std::hint::black_box(&v);
                })
            })
            .collect();
        Self { stop, threads }
    }
}

impl Drop for Hogs {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn env_list<T: std::str::FromStr>(name: &str) -> Option<Vec<T>> {
    let v = std::env::var(name).ok()?;
    Some(v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
}

fn env_num(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture --test-threads 1"]
fn realtime_buffers() {
    enable_flush_denormals();
    let opts = Opts {
        block: env_num("ETHER_BENCH_BLOCK", 256.0) as usize,
        seconds: env_num("ETHER_BENCH_SECONDS", 4.0),
        paced: env_num("ETHER_BENCH_PACED", 1.0) != 0.0,
        realtime: env_num("ETHER_BENCH_RT", 0.0) != 0.0,
        scale: env_num("ETHER_BENCH_SCALE", 1.0),
    };
    let cores = physical_cores();
    let workers: Vec<usize> = env_list("ETHER_BENCH_WORKERS").unwrap_or_else(|| {
        let mut w: Vec<usize> = [0, 1, 3, 7].into_iter().filter(|&w| w < cores).collect();
        w.dedup();
        w
    });
    let wanted: Option<Vec<String>> = env_list("ETHER_BENCH_SCENARIOS");
    let json = std::env::var("ETHER_BENCH_JSON").ok();
    if opts.realtime {
        // Like the host's device callback (`audio.rs`): the render thread gets the
        // workers' priority too, or real-time workers preempt it.
        ether_native::workers::promote_current_thread(Duration::from_secs_f64(
            opts.block as f64 / SR as f64,
        ));
    }
    let ns_per_iter = calibrate();
    let hogs = env_num("ETHER_BENCH_HOGS", 0.0) as usize;
    let _hogs = Hogs::start(hogs);
    let budget = opts.block as f64 * 1e6 / SR as f64;
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    println!(
        "engine_bench: {} frames ({budget:.0} µs budget), {} s per run, {}, {} physical cores, \
         burn {ns_per_iter:.3} ns/iter, scale {}, {} worker priority, {hogs} hog threads, loadavg {}",
        opts.block,
        opts.seconds,
        if opts.paced { "paced" } else { "back to back" },
        cores,
        opts.scale,
        if opts.realtime { "real-time" } else { "normal" },
        load.split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" "),
    );
    for &(name, about, make) in SCENARIOS {
        if wanted
            .as_ref()
            .is_some_and(|w| !w.iter().any(|x| x == name))
        {
            continue;
        }
        let tracks = make();
        let m = model(&tracks, opts.block).scaled(opts.scale);
        println!(
            "\n{name}: {about}\n  {} tracks, {} levels; model: total {:.0} µs, critical path {:.0} µs, \
             level bound {:.0} µs (ideal: max(critical, total/threads); simlev/simdag: simulated \
             level-barrier / dependency-driven schedules, no overhead)",
            tracks.len(),
            m.n_levels,
            m.total,
            m.critical,
            m.levels
        );
        println!(
            "  {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>6} {:>7} {:>7} {:>7} {:>6}",
            "workers",
            "mean",
            "p50",
            "p99",
            "p99.9",
            "max",
            "%budg",
            "over",
            "late",
            "ideal",
            "simlev",
            "simdag",
            "cores"
        );
        let mut reference: Option<Vec<f32>> = None;
        for &w in &workers {
            let executor: Box<dyn ParallelExecutor> = if w == 0 {
                Box::new(SequentialExecutor)
            } else {
                Box::new(WorkerPool::with_options(
                    w,
                    PoolOptions {
                        period: Duration::from_secs_f64(opts.block as f64 / SR as f64),
                        realtime: opts.realtime,
                        flush_denormals: true,
                    },
                ))
            };
            let run = render(&tracks, executor, &opts, ns_per_iter);
            match &reference {
                None => reference = Some(run.out.clone()),
                Some(r) => assert!(
                    r == &run.out,
                    "{name}: {w} workers differ from the sequential engine"
                ),
            }
            let mut t = run.times.clone();
            t.sort_by(f64::total_cmp);
            let mean = t.iter().sum::<f64>() / t.len() as f64;
            let (p50, p99, p999, max) = (
                percentile(&t, 0.5),
                percentile(&t, 0.99),
                percentile(&t, 0.999),
                t[t.len() - 1],
            );
            let over = t.iter().filter(|&&x| x > budget).count();
            let ideal = m.critical.max(m.total / (w + 1) as f64);
            let (sim_levels, sim_dag) = (m.simulate(w + 1, true), m.simulate(w + 1, false));
            println!(
                "  {w:>7} {mean:>7.0} {p50:>7.0} {p99:>7.0} {p999:>7.0} {max:>7.0} {:>5.0}% {over:>6} {:>6} {ideal:>7.0} {sim_levels:>7.0} {sim_dag:>7.0} {:>6}",
                p99 / budget * 100.0,
                run.late_starts,
                run.cores_busy
                    .map_or("-".to_string(), |c| format!("{c:.1}")),
            );
            if let Some(path) = &json {
                let line = format!(
                    "{{\"scenario\":\"{name}\",\"workers\":{w},\"block\":{},\"paced\":{},\"scale\":{},\"rt\":{},\"hogs\":{hogs},\
                     \"mean_us\":{mean:.1},\"p50_us\":{p50:.1},\"p99_us\":{p99:.1},\"p999_us\":{p999:.1},\
                     \"max_us\":{max:.1},\"budget_us\":{budget:.1},\"over\":{over},\"late\":{},\
                     \"ideal_us\":{ideal:.1},\"sim_levels_us\":{sim_levels:.1},\"sim_dag_us\":{sim_dag:.1},\"critical_us\":{:.1},\"levels_us\":{:.1},\"total_us\":{:.1},\
                     \"cores_busy\":{},\"buffers\":{}}}",
                    opts.block,
                    opts.paced,
                    opts.scale,
                    opts.realtime,
                    run.late_starts,
                    m.critical,
                    m.levels,
                    m.total,
                    run.cores_busy.map_or("null".into(), |c| format!("{c:.2}")),
                    t.len(),
                );
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .expect("ETHER_BENCH_JSON");
                writeln!(f, "{line}").unwrap();
            }
        }
    }
}

/// The cost model: the owner graph's sidechain moves the heavy rack a level down (its
/// level bound grows, its critical path barely), and the bounds are ordered.
#[test]
fn model_bounds() {
    for &(name, _, make) in SCENARIOS {
        let m = model(&make(), 256);
        assert!(m.critical <= m.levels + 1e-6, "{name}");
        assert!(m.levels <= m.total + 1e-6, "{name}");
    }
    let with = model(&owner(true), 256);
    let without = model(&owner(false), 256);
    assert!(with.levels > without.levels);
    assert_eq!(model(&deep(8, 4), 256).n_levels, 10);
    for &(name, _, make) in SCENARIOS {
        let m = model(&make(), 256);
        for threads in [1, 2, 4, 8] {
            let (lv, dag) = (m.simulate(threads, true), m.simulate(threads, false));
            let ideal = m.critical.max(m.total / threads as f64);
            assert!(
                dag >= ideal - 1e-6 && lv >= ideal - 1e-6,
                "{name} {threads}"
            );
            if threads == 1 {
                assert!(
                    (lv - m.total).abs() < 1e-6 && (dag - m.total).abs() < 1e-6,
                    "{name}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Timeline: where a buffer's time goes.

/// Per-thread index (0 = the first thread that ran a job here: the audio thread).
fn thread_index() -> u32 {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    thread_local!(static INDEX: Cell<u32> = const { Cell::new(u32::MAX) });
    INDEX.with(|i| {
        if i.get() == u32::MAX {
            i.set(NEXT.fetch_add(1, Ordering::Relaxed));
        }
        i.get()
    })
}

/// One `execute` call (a dependency level with more than one track) or one job of it.
#[derive(Clone, Copy, Default)]
struct Span {
    /// `u32::MAX` = the dispatch itself.
    thread: u32,
    start_ns: u64,
    end_ns: u64,
}

/// Records spans into a preallocated table (no allocation while rendering).
struct Recorder {
    epoch: Instant,
    spans: std::sync::Arc<Vec<[std::sync::atomic::AtomicU64; 3]>>,
    len: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Recorder {
    fn record(&self, thread: u32, start: Instant, end: Instant) {
        use std::sync::atomic::Ordering;
        let i = self.len.fetch_add(1, Ordering::Relaxed);
        if let Some(s) = self.spans.get(i) {
            s[0].store(thread as u64, Ordering::Relaxed);
            s[1].store((start - self.epoch).as_nanos() as u64, Ordering::Relaxed);
            s[2].store((end - self.epoch).as_nanos() as u64, Ordering::Relaxed);
        }
    }
}

/// Wraps an executor and records every dispatch and job.
struct Timeline {
    inner: Box<dyn ParallelExecutor>,
    rec: Recorder,
}

impl ParallelExecutor for Timeline {
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        self.execute_pinned(jobs, 0, job);
    }

    fn execute_pinned(&mut self, jobs: usize, pinned: usize, job: &(dyn Fn(usize) + Sync)) {
        let t0 = Instant::now();
        let rec = &self.rec;
        let wrapped = |i: usize| {
            let s = Instant::now();
            job(i);
            rec.record(thread_index(), s, Instant::now());
        };
        self.inner.execute_pinned(jobs, pinned, &wrapped);
        self.rec.record(u32::MAX, t0, Instant::now());
    }

    fn execute_graph(&mut self, graph: &JobGraph, job: &(dyn Fn(usize) + Sync)) {
        self.execute_graph_forking(graph, &|id, _| job(id));
    }

    fn execute_graph_forking(&mut self, graph: &JobGraph, job: &(dyn Fn(usize, &dyn Fork) + Sync)) {
        let t0 = Instant::now();
        let rec = &self.rec;
        let wrapped = |id: usize, fork: &dyn Fork| {
            let s = Instant::now();
            job(id, fork);
            rec.record(thread_index(), s, Instant::now());
        };
        self.inner.execute_graph_forking(graph, &wrapped);
        self.rec.record(u32::MAX, t0, Instant::now());
    }

    fn workers(&self) -> usize {
        self.inner.workers()
    }
}

/// Where the time of a paced buffer goes, for one scenario and worker count
/// (`ETHER_BENCH_SCENARIOS`'s first entry, default `owner`; `ETHER_BENCH_WORKERS`'s first,
/// default 7): per buffer, the time in dispatched levels vs. outside them (single-track levels
/// and the audio thread's own work), the workers' wake-up delay (dispatch → their first job),
/// and the waits at the end of each level.
#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture --test-threads 1"]
fn timeline() {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    enable_flush_denormals();
    let name = env_list::<String>("ETHER_BENCH_SCENARIOS")
        .and_then(|v| v.into_iter().next())
        .unwrap_or_else(|| "owner".into());
    let workers = env_list::<usize>("ETHER_BENCH_WORKERS")
        .and_then(|v| v.into_iter().next())
        .unwrap_or(7);
    let opts = Opts {
        block: env_num("ETHER_BENCH_BLOCK", 256.0) as usize,
        seconds: env_num("ETHER_BENCH_SECONDS", 2.0),
        paced: env_num("ETHER_BENCH_PACED", 1.0) != 0.0,
        realtime: env_num("ETHER_BENCH_RT", 0.0) != 0.0,
        scale: env_num("ETHER_BENCH_SCALE", 1.0),
    };
    let (_, about, make) = SCENARIOS
        .iter()
        .find(|s| s.0 == name)
        .expect("ETHER_BENCH_SCENARIOS: unknown scenario");
    let tracks = make();
    let ns_per_iter = calibrate();
    let _hogs = Hogs::start(env_num("ETHER_BENCH_HOGS", 0.0) as usize);
    let capacity = 4_000_000;
    let spans: std::sync::Arc<Vec<[AtomicU64; 3]>> = std::sync::Arc::new(
        (0..capacity)
            .map(|_| [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)])
            .collect(),
    );
    let len = std::sync::Arc::new(AtomicUsize::new(0));
    let epoch = Instant::now();
    let pool: Box<dyn ParallelExecutor> = if workers == 0 {
        Box::new(SequentialExecutor)
    } else {
        Box::new(WorkerPool::with_options(
            workers,
            PoolOptions {
                period: Duration::from_secs_f64(opts.block as f64 / SR as f64),
                realtime: opts.realtime,
                flush_denormals: true,
            },
        ))
    };
    let tl = Timeline {
        inner: pool,
        rec: Recorder {
            epoch,
            spans: spans.clone(),
            len: len.clone(),
        },
    };
    // The audio (test) thread is thread 0.
    assert_eq!(thread_index(), 0);
    let run = render(&tracks, Box::new(tl), &opts, ns_per_iter);
    let n = len.load(Ordering::Relaxed).min(capacity);
    let from = (run.measured_from - epoch).as_nanos() as u64;
    let all: Vec<Span> = spans[..n]
        .iter()
        .map(|s| Span {
            thread: s[0].load(Ordering::Relaxed) as u32,
            start_ns: s[1].load(Ordering::Relaxed),
            end_ns: s[2].load(Ordering::Relaxed),
        })
        .filter(|s| s.start_ns >= from)
        .collect();
    // Group: a dispatch span is recorded after its jobs.
    let mut wake = vec![];
    let mut first_start = vec![];
    let mut tail_wait = vec![];
    let mut level_span = vec![];
    let mut level_longest = vec![];
    let mut job_us = vec![];
    let mut pending: Vec<Span> = vec![];
    for s in all {
        if s.thread != u32::MAX {
            pending.push(s);
            continue;
        }
        let jobs = std::mem::take(&mut pending);
        if jobs.is_empty() {
            continue;
        }
        let span = (s.end_ns - s.start_ns) as f64 / 1e3;
        level_span.push(span);
        let longest = jobs
            .iter()
            .map(|j| (j.end_ns - j.start_ns) as f64 / 1e3)
            .fold(0.0, f64::max);
        level_longest.push(longest);
        let last_end = jobs.iter().map(|j| j.end_ns).max().unwrap();
        tail_wait.push((s.end_ns.saturating_sub(last_end)) as f64 / 1e3);
        first_start
            .push((jobs.iter().map(|j| j.start_ns).min().unwrap() - s.start_ns) as f64 / 1e3);
        let mut threads: Vec<u32> = jobs.iter().map(|j| j.thread).collect();
        threads.sort_unstable();
        threads.dedup();
        for t in threads {
            let first = jobs
                .iter()
                .filter(|j| j.thread == t)
                .map(|j| j.start_ns)
                .min()
                .unwrap();
            if t != 0 {
                wake.push((first - s.start_ns) as f64 / 1e3);
            }
        }
        for j in &jobs {
            job_us.push((j.end_ns - j.start_ns) as f64 / 1e3);
        }
    }
    let buffers = run.times.len() as f64;
    let sum = |v: &[f64]| v.iter().sum::<f64>();
    let pct = |v: &[f64], p: f64| {
        let mut v = v.to_vec();
        v.sort_by(f64::total_cmp);
        if v.is_empty() { 0.0 } else { percentile(&v, p) }
    };
    let mean_buffer = sum(&run.times) / buffers;
    println!(
        "timeline: {name} ({about}), {workers} workers, {} frames, {} buffers, {}",
        opts.block,
        run.times.len(),
        if opts.paced { "paced" } else { "back to back" }
    );
    println!(
        "  buffer: mean {mean_buffer:.0} µs, p99 {:.0} µs",
        pct(&run.times, 0.99)
    );
    println!(
        "  dispatched graphs per buffer: {:.1}; in them {:.0} µs/buffer (longest job {:.0}, \
         end wait {:.1}); outside {:.0} µs/buffer",
        level_span.len() as f64 / buffers,
        sum(&level_span) / buffers,
        sum(&level_longest) / buffers,
        sum(&tail_wait) / buffers,
        mean_buffer - sum(&level_span) / buffers,
    );
    println!(
        "  first job after dispatch: p50 {:.1} p99 {:.1} µs; worker wake-up (dispatch → its \
         first job): p50 {:.1} p90 {:.1} p99 {:.1} max {:.1} µs ({} samples)",
        pct(&first_start, 0.5),
        pct(&first_start, 0.99),
        pct(&wake, 0.5),
        pct(&wake, 0.9),
        pct(&wake, 0.99),
        pct(&wake, 1.0),
        wake.len()
    );
    println!(
        "  jobs: {:.1}/buffer, p50 {:.1} µs, p99 {:.1} µs, total {:.0} µs/buffer (model total {:.0})",
        job_us.len() as f64 / buffers,
        pct(&job_us, 0.5),
        pct(&job_us, 0.99),
        sum(&job_us) / buffers,
        model(&tracks, opts.block).total * opts.scale,
    );
}

// ---------------------------------------------------------------------------------------
// Idle plugins (sleep on silence).

/// Many idle VST3 effects on silent tracks while playing: the fixture's tail effect
/// (`ether_vst3::testing::TAIL_ID`, an echo whose `Cost` param makes it an expensive
/// plugin), every instance always processing, then sleeping on silence (CONTRACTS.md
/// §13.19, the default). One render thread, back to back: the time is the CPU spent.
///
/// ```text
/// cargo test --release -p ether-native --features plugins-vst3 --test it \
///   engine_bench::idle_vst3_plugins -- --ignored --nocapture --test-threads 1
/// ```
///
/// `ETHER_BENCH_IDLE_TRACKS` (default 32), `ETHER_BENCH_IDLE_PLUGINS` per track (4),
/// `ETHER_BENCH_IDLE_COST` (the `Cost` param, default 0.01 = 10 multiply-adds per frame),
/// `ETHER_BENCH_BLOCK`, `ETHER_BENCH_SECONDS`.
#[cfg(feature = "plugins-vst3")]
#[test]
#[ignore = "benchmark: run in release with --features plugins-vst3 --ignored --nocapture --test-threads 1"]
fn idle_vst3_plugins() {
    use ether_core::plugin::PluginController;
    use ether_core::protocol::model::ParamId;
    use std::sync::atomic::Ordering;

    enable_flush_denormals();
    let n_tracks = env_num("ETHER_BENCH_IDLE_TRACKS", 32.0) as u128;
    let per_track = env_num("ETHER_BENCH_IDLE_PLUGINS", 4.0) as usize;
    let cost = env_num("ETHER_BENCH_IDLE_COST", 0.01);
    let block = env_num("ETHER_BENCH_BLOCK", 256.0) as usize;
    let seconds = env_num("ETHER_BENCH_SECONDS", 4.0);
    let dir = ether_vst3::testing::temp_dir("engine-bench-idle");
    let bundle = ether_vst3::testing::make_bundle(&dir, "EtherBenchIdle");
    let budget = block as f64 * 1e6 / SR as f64;
    println!(
        "idle_vst3_plugins: {n_tracks} silent tracks x {per_track} VST3 effects (cost {cost}), \
         {block} frames ({budget:.0} µs budget), playing, 1 thread, back to back"
    );
    println!(
        "  {:>16} {:>8} {:>8} {:>8} {:>7}",
        "", "mean", "p50", "p99", "%budg"
    );
    let mut means = Vec::new();
    let mut reference: Option<Vec<f32>> = None;
    for always in [true, false] {
        let mut parts = create(config(block));
        let prepare = PrepareConfig {
            sample_rate: SR as f32,
            max_block_size: block,
            max_events_per_block: 256,
        };
        let mut plugins = Vec::new();
        let mut tracks = vec![track_desc(tid(MASTER), TrackKind::Master, None)];
        for i in 0..n_tracks {
            let mut t = track_desc(tid(100 + i), TrackKind::Audio, Some(tid(MASTER)));
            for _ in 0..per_track {
                let mut plugin =
                    ether_vst3::Vst3Plugin::load(&bundle, ether_vst3::testing::TAIL_ID)
                        .expect("load the tail effect");
                plugin.set_param_value(ParamId(23), cost).expect("cost");
                let node = plugin.activate(&prepare).expect("activate");
                node.always_process()
                    .expect("switch")
                    .store(always, Ordering::Relaxed);
                t.chain.push(ChainEntry {
                    node: parts.handle.add_node(node).expect("node table"),
                    enabled: true,
                    sidechain: None,
                });
                plugins.push(plugin);
            }
            tracks.push(t);
        }
        parts
            .handle
            .publish(RenderGraphDesc {
                version: 1,
                tracks,
                ..Default::default()
            })
            .expect("graph compiles");
        parts.handle.transport(TransportControl::Play).unwrap();
        let blocks = ((seconds * SR as f64) as usize / block).max(1);
        let warmup = (0.25 * SR as f64) as usize / block;
        let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
        let mut out = Vec::with_capacity(blocks * block * 2);
        let mut times = Vec::with_capacity(blocks);
        let mut polled = ether_core::EngineOutputs::default();
        for b in 0..warmup + blocks {
            let t = Instant::now();
            {
                let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
                parts.engine.process(&[], &mut outs, block);
            }
            if b >= warmup {
                times.push(t.elapsed().as_secs_f64() * 1e6);
                out.extend_from_slice(&l);
                out.extend_from_slice(&r);
            }
            if b % 8 == 0 {
                parts.handle.poll(&mut polled);
                parts.gc.collect();
            }
        }
        match &reference {
            None => reference = Some(out),
            Some(r) => assert!(r == &out, "sleeping changed the output"),
        }
        times.sort_by(f64::total_cmp);
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let (p50, p99) = (percentile(&times, 0.5), percentile(&times, 0.99));
        println!(
            "  {:>16} {mean:>8.1} {p50:>8.1} {p99:>8.1} {:>6.1}%",
            if always {
                "always process"
            } else {
                "sleep on silence"
            },
            mean / budget * 100.0
        );
        means.push(mean);
        // The nodes go back (and are dropped) with the engine, before their controllers.
        drop(parts);
        drop(plugins);
    }
    println!(
        "  saved: {:.1} µs per buffer ({:.0}%)",
        means[0] - means[1],
        (1.0 - means[1] / means[0]) * 100.0
    );
}
