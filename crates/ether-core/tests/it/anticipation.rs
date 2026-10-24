//! Anticipative processing (`EngineConfig::anticipative`, CONTRACTS.md §11.7): tracks not
//! listened to live render one block ahead, the real-time side (master, live tracks and
//! what feeds them) renders the same planned sub-blocks one block later.
//!
//! - With the host's block lengths known (`process_with_hint`), the output is bit-identical
//!   to the real-time engine's on randomized projects (groups, returns, pre/post sends,
//!   sidechains, drum racks, PDC, clips, automation, mute/solo, loops), whatever the worker
//!   count, also when live tracks split the graph and when tracks change sides.
//! - Transport controls take effect one block later; live params one block later on
//!   anticipated tracks only.
//! - Wrong hints (a host block longer or shorter than announced) keep the timeline: same
//!   signal, partitioned differently.

use crate::parallel_graphs;

use assert_no_alloc::assert_no_alloc;
use ether_core::RenderGraphDesc;
use ether_core::parallel::{JobGraph, ParallelExecutor};
use ether_core::protocol::model::Beats;
use ether_core::{EngineConfig, EngineParts, ParamChange, ParamTarget, TransportControl, create};
use parallel_graphs::*;

fn config(anticipative: bool) -> EngineConfig {
    EngineConfig {
        anticipative,
        ..parallel_graphs::config()
    }
}

/// Runs the graph in rank order on the calling thread, but says it has workers (so the
/// engine anticipates) and never allocates.
struct InOrder;

impl ParallelExecutor for InOrder {
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        for i in 0..jobs {
            job(i);
        }
    }
    fn execute_graph(&mut self, graph: &JobGraph, job: &(dyn Fn(usize) + Sync)) {
        for id in graph.order() {
            job(id);
        }
    }
    fn workers(&self) -> usize {
        2
    }
}

enum Ev {
    Publish(RenderGraphDesc),
    Param(ParamChange),
    Transport(TransportControl),
    Anticipate(bool),
}

/// Render project `seed` (`edit` adjusts its desc) block by block: `sizes[k]` frames for
/// block `k`, announcing `hints[k]` as the next block's length (`None`: the real one),
/// sending `events` before their blocks. Returns left ++ right.
fn render(
    seed: u64,
    anticipative: bool,
    workers: usize,
    sizes: &[usize],
    hints: Option<&[usize]>,
    edit: &dyn Fn(&mut RenderGraphDesc),
    events: &dyn Fn(usize, &RenderGraphDesc) -> Vec<Ev>,
) -> Vec<f32> {
    let EngineParts {
        mut engine,
        mut handle,
        mut gc,
    } = create(config(anticipative));
    let mut desc = build(seed, &mut handle, false).desc;
    edit(&mut desc);
    handle.publish(desc.clone()).unwrap();
    let exec: Box<dyn ParallelExecutor> = match workers {
        0 => Box::new(InOrder),
        w => Box::new(Threaded(w)),
    };
    engine.set_executor(exec);
    handle.transport(TransportControl::Play).unwrap();
    let total: usize = sizes.iter().sum();
    let (mut left, mut right) = (Vec::with_capacity(total), Vec::with_capacity(total));
    let (mut l, mut r) = (vec![0.0f32; MAX_BLOCK], vec![0.0f32; MAX_BLOCK]);
    for (k, &n) in sizes.iter().enumerate() {
        for ev in events(k, &desc) {
            match ev {
                Ev::Publish(d) => handle.publish(d).unwrap(),
                Ev::Param(p) => handle.set_param(p).unwrap(),
                Ev::Transport(t) => handle.transport(t).unwrap(),
                Ev::Anticipate(on) => handle.set_anticipative(on),
            }
        }
        let next = match hints {
            Some(h) => h[k],
            None => sizes.get(k + 1).copied().unwrap_or(n),
        };
        {
            let mut outs: [&mut [f32]; 2] = [&mut l[..n], &mut r[..n]];
            engine.process_with_hint(&[], &mut outs, n, next);
        }
        left.extend_from_slice(&l[..n]);
        right.extend_from_slice(&r[..n]);
        gc.collect();
    }
    left.extend_from_slice(&right);
    left
}

fn steady(block: usize, frames: usize) -> Vec<usize> {
    vec![block; frames.div_ceil(block)]
}

fn assert_same(a: &[f32], b: &[f32], what: &str) {
    if let Some(i) = first_diff(a, b) {
        panic!(
            "{what}: first difference at sample {i}: {} vs {}",
            a[i], b[i]
        );
    }
}

fn no_events(_: usize, _: &RenderGraphDesc) -> Vec<Ev> {
    Vec::new()
}

/// Mark track `k` (in desc order, skipping master) monitored: it, what feeds it and what
/// it feeds stay real-time.
fn monitor(k: usize) -> impl Fn(&mut RenderGraphDesc) {
    move |d: &mut RenderGraphDesc| {
        let regular: Vec<usize> = (0..d.tracks.len())
            .filter(|&i| d.tracks[i].kind != ether_core::protocol::model::TrackKind::Master)
            .collect();
        let i = regular[k % regular.len()];
        d.tracks[i].monitor = true;
    }
}

#[test]
fn bit_identical_to_real_time_on_random_projects() {
    let mut audible = 0;
    for seed in 0..8u64 {
        for block in [64, 333, MAX_BLOCK] {
            let sizes = steady(block, SR as usize / 2);
            let reference = render(seed, false, 0, &sizes, None, &|_| {}, &no_events);
            audible += reference.iter().any(|s| *s != 0.0) as usize;
            for workers in [0usize, 3] {
                let out = render(seed, true, workers, &sizes, None, &|_| {}, &no_events);
                assert_same(
                    &reference,
                    &out,
                    &format!("seed {seed} block {block} w{workers}"),
                );
            }
        }
    }
    assert!(audible > 6, "the projects make sound: {audible}");
}

#[test]
fn live_tracks_split_the_graph_bit_identical() {
    for seed in 0..8u64 {
        let sizes = steady(256, SR as usize / 2);
        let edit = monitor(seed as usize * 3);
        let reference = render(seed, false, 0, &sizes, None, &edit, &no_events);
        for workers in [0usize, 4] {
            let out = render(seed, true, workers, &sizes, None, &edit, &no_events);
            assert_same(&reference, &out, &format!("seed {seed} w{workers}"));
        }
    }
}

#[test]
fn varying_blocks_with_hints_bit_identical() {
    let mut rng = Rng::new(99);
    for seed in 0..6u64 {
        let mut sizes = Vec::new();
        let mut total = 0;
        while total < SR as usize / 2 {
            let n = 1 + rng.below(MAX_BLOCK);
            sizes.push(n);
            total += n;
        }
        let reference = render(seed, false, 0, &sizes, None, &|_| {}, &no_events);
        let out = render(seed, true, 2, &sizes, None, &|_| {}, &no_events);
        assert_same(&reference, &out, &format!("seed {seed}"));
    }
}

#[test]
fn tracks_changing_sides_bit_identical() {
    // Monitoring a track (no hardware input, so it changes no signal) moves it, its
    // sources and everything downstream to the real-time side, and back. A snapshot swap
    // reaches anticipated tracks one block later, so it is only compared where the swaps
    // change nothing in the real-time engine either (state a swap restarts, like sidechain
    // PDC delay lines and re-sent automation, makes it audible).
    let mut compared = 0;
    for seed in 0..16u64 {
        let sizes = steady(128, SR as usize / 2);
        let events = |k: usize, d: &RenderGraphDesc| -> Vec<Ev> {
            let mut d = d.clone();
            match k {
                40 | 41 | 90 => {
                    monitor(seed as usize + k)(&mut d);
                    d.version = 10 + k as u64;
                    vec![Ev::Publish(d)]
                }
                60 | 120 => {
                    d.version = 10 + k as u64;
                    vec![Ev::Publish(d)]
                }
                _ => Vec::new(),
            }
        };
        let plain = render(seed, false, 0, &sizes, None, &|_| {}, &no_events);
        let reference = render(seed, false, 0, &sizes, None, &|_| {}, &events);
        if first_diff(&plain, &reference).is_some() || plain.iter().all(|s| *s == 0.0) {
            continue;
        }
        compared += 1;
        for workers in [0usize, 3] {
            let out = render(seed, true, workers, &sizes, None, &|_| {}, &events);
            assert_same(&reference, &out, &format!("seed {seed} w{workers}"));
        }
    }
    assert!(compared >= 4, "only {compared} projects compared");
}

#[test]
fn switching_off_and_on_is_seamless() {
    for seed in 0..6u64 {
        let sizes = steady(200, SR as usize / 2);
        let events = |k: usize, _: &RenderGraphDesc| match k {
            30 => vec![Ev::Anticipate(false)],
            31 => vec![Ev::Anticipate(true)],
            50 => vec![Ev::Anticipate(false)],
            80 => vec![Ev::Anticipate(true)],
            _ => Vec::new(),
        };
        let reference = render(seed, false, 0, &sizes, None, &|_| {}, &no_events);
        let out = render(seed, true, 2, &sizes, None, &|_| {}, &events);
        assert_same(&reference, &out, &format!("seed {seed}"));
    }
}

#[test]
fn transport_takes_effect_one_block_later() {
    for seed in 0..6u64 {
        let sizes = steady(256, SR as usize / 2);
        let at = |k: usize| match k {
            30 => vec![Ev::Transport(TransportControl::Locate {
                position: Beats(1.5),
            })],
            50 => vec![Ev::Transport(TransportControl::Stop)],
            60 => vec![Ev::Transport(TransportControl::Play)],
            _ => Vec::new(),
        };
        let reference = render(seed, false, 0, &sizes, None, &|_| {}, &|k, _| {
            at(k.wrapping_sub(1))
        });
        let out = render(seed, true, 2, &sizes, None, &|_| {}, &|k, _| at(k));
        assert_same(&reference, &out, &format!("seed {seed}"));
    }
}

#[test]
fn live_params_reach_anticipated_tracks_one_block_later() {
    // Project 0's regular tracks are anticipated; master is not. A volume change on a
    // regular track sent at block k sounds like the real-time engine's at k + 1; on
    // master, at k.
    for seed in 0..4u64 {
        let sizes = steady(256, SR as usize / 2);
        let regular = |d: &RenderGraphDesc| {
            d.tracks
                .iter()
                .find(|t| t.kind == ether_core::protocol::model::TrackKind::Midi)
                .map(|t| t.id)
        };
        let master = tid(1);
        let change = |track, value| {
            Ev::Param(ParamChange {
                target: ParamTarget::TrackVolume { track },
                value,
            })
        };
        let reference = render(seed, false, 0, &sizes, None, &|_| {}, &|k, d| {
            let mut v = Vec::new();
            if k == 31
                && let Some(t) = regular(d)
            {
                v.push(change(t, 0.3));
            }
            if k == 40 {
                v.push(change(master, 0.5));
            }
            v
        });
        let out = render(seed, true, 2, &sizes, None, &|_| {}, &|k, d| {
            let mut v = Vec::new();
            if k == 30
                && let Some(t) = regular(d)
            {
                v.push(change(t, 0.3));
            }
            if k == 40 {
                v.push(change(master, 0.5));
            }
            v
        });
        assert_same(&reference, &out, &format!("seed {seed}"));
    }
}

#[test]
fn irregular_blocks_hinted_by_the_longest_keep_the_timeline() {
    // Device buffers that alternate around a size (shared-mode WASAPI, resampling hosts),
    // hinted with the longest recent one as the native host does: the queue holds more
    // than a short buffer, the rest stays queued. Sub-blocks split where the real-time
    // engine doesn't: rounding-level differences only.
    for seed in 0..6u64 {
        let mut sizes = Vec::new();
        let mut total = 0;
        let mut k = 0;
        while total < SR as usize / 2 {
            let n = if k % 3 == 2 { 112 } else { 128 };
            sizes.push(n);
            total += n;
            k += 1;
        }
        let hints = vec![128; sizes.len()];
        let reference = render(seed, false, 0, &sizes, None, &|_| {}, &no_events);
        let out = render(seed, true, 2, &sizes, Some(&hints), &|_| {}, &no_events);
        let worst = reference
            .iter()
            .zip(&out)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "seed {seed}: worst difference {worst}");
    }
}

#[test]
fn wrong_hints_keep_the_timeline() {
    // The host announces lengths it doesn't keep: more or less than queued. Sub-blocks are
    // split where the real-time engine doesn't split them, so allow rounding-level
    // differences, but the timeline (and so the signal) must line up.
    let mut rng = Rng::new(5);
    for seed in 0..6u64 {
        let mut sizes = Vec::new();
        let mut total = 0;
        while total < SR as usize / 2 {
            let n = 16 + rng.below(MAX_BLOCK - 16);
            sizes.push(n);
            total += n;
        }
        let hints: Vec<usize> = (0..sizes.len()).map(|_| 1 + rng.below(MAX_BLOCK)).collect();
        let reference = render(seed, false, 0, &sizes, None, &|_| {}, &no_events);
        let out = render(seed, true, 2, &sizes, Some(&hints), &|_| {}, &no_events);
        let worst = reference
            .iter()
            .zip(&out)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "seed {seed}: worst difference {worst}");
    }
}

#[test]
fn a_forced_change_with_more_queued_than_the_block_recovers() {
    // The host announced a long block and delivers short ones while a new graph makes an
    // ahead group read a live track: the engine drops what it rendered ahead (a skip of up
    // to one block on those tracks) and carries on.
    let sizes = [vec![256; 20], vec![64; 10], vec![256; 20]].concat();
    let hints: Vec<usize> = sizes.iter().map(|_| 512).collect();
    let events = |k: usize, d: &RenderGraphDesc| {
        if k == 22 {
            let mut d = d.clone();
            monitor(0)(&mut d);
            d.version = 77;
            vec![Ev::Publish(d)]
        } else {
            Vec::new()
        }
    };
    let mut audible = 0;
    for seed in 0..4u64 {
        let reference = render(seed, false, 0, &sizes, None, &|_| {}, &events);
        let out = render(seed, true, 2, &sizes, Some(&hints), &|_| {}, &events);
        assert!(out.iter().all(|s| s.is_finite()));
        if reference.iter().any(|s| *s != 0.0) {
            audible += 1;
            assert!(out.iter().any(|s| *s != 0.0), "seed {seed} went silent");
        }
    }
    assert!(audible > 0);
}

#[test]
fn anticipation_never_allocates() {
    let EngineParts {
        mut engine,
        mut handle,
        mut gc,
    } = create(config(true));
    let project = build(2, &mut handle, false);
    handle.publish(project.desc.clone()).unwrap();
    engine.set_executor(Box::new(InOrder));
    handle.transport(TransportControl::Play).unwrap();
    let (mut l, mut r) = (vec![0.0f32; MAX_BLOCK], vec![0.0f32; MAX_BLOCK]);
    let mut ahead_seen = false;
    let mut monitored = project.desc.clone();
    monitor(1)(&mut monitored);
    monitored.version = 9;
    for k in 0..200usize {
        match k {
            20 => handle.publish(project.swapped.clone()).unwrap(),
            40 => handle.publish(monitored.clone()).unwrap(),
            60 => handle.publish(project.desc.clone()).unwrap(),
            80 => {
                for c in &project.live {
                    handle.set_param(*c).unwrap();
                }
                handle
                    .transport(TransportControl::Locate {
                        position: Beats(1.0),
                    })
                    .unwrap();
            }
            100 => handle.set_anticipative(false),
            110 => handle.set_anticipative(true),
            _ => {}
        }
        let n = [256, 256, 100, 512][k % 4];
        let next = [256, 300, 512, 64][k % 4];
        let mut outs: [&mut [f32]; 2] = [&mut l[..n], &mut r[..n]];
        assert_no_alloc(|| engine.process_with_hint(&[], &mut outs, n, next));
        gc.collect();
        ahead_seen |= engine.frames_ahead() > 0;
        if k == 108 {
            assert_eq!(engine.frames_ahead(), 0, "switched off: nothing ahead");
        }
    }
    assert!(ahead_seen, "the engine anticipated");
    assert!(engine.frames_ahead() > 0);
}
