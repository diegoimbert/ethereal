//! Graph snapshot cost measurements, shared by the native test
//! (`tests/web_perf.rs`) and the wasm export [`crate::web::bench_graph_snapshot`] (run
//! in V8 by `apps/web/src/engine/graphSnapshot.bench.test.ts`), so the numbers come from
//! the real `Publish` path: `EngineMsg::encode` (Worker) → ring → `EngineHost` (Worklet:
//! frame decode, key remap, compile, one render quantum).

use std::collections::BTreeMap;

use ether_core::codec::{BinaryCodec, GraphCodec};
use ether_core::protocol::model::BuiltinDevice;
use ether_core::{NodeKey, RenderGraphDesc};
use serde::Serialize;

use crate::bridge::{self, Shared, WebBridge};
use crate::proto::EngineMsg;
use crate::ring::HeapMemory;
use crate::worklet::{EngineHost, RENDER_QUANTUM, web_engine_config};

/// Test fixtures (the large project: 64 tracks, 500 clips, automation). Shared with the
/// codec tests in `ether-core`.
#[path = "../../ether-core/tests/it/codec_fixture.rs"]
pub mod fixture;

/// Fastest-of-N timings in milliseconds.
#[derive(Clone, Debug, Serialize)]
pub struct SnapshotCosts {
    pub tracks: usize,
    pub clips: usize,
    pub json_bytes: usize,
    pub binary_bytes: usize,
    pub json_encode_ms: f64,
    pub json_decode_ms: f64,
    pub binary_encode_ms: f64,
    pub binary_decode_ms: f64,
    /// `EngineHandle::publish` alone (compile + swap send), no codec.
    pub compile_ms: f64,
    /// Slowest render quantum while a binary `Publish` is applied by the Worklet host
    /// (ring read, decode, remap, compile, render one block, GC).
    pub publish_quantum_ms: f64,
    /// One render quantum's real-time budget at 48 kHz.
    pub quantum_budget_ms: f64,
}

/// Every node key the desc references (chains and pad chains).
pub fn node_keys(d: &RenderGraphDesc) -> Vec<NodeKey> {
    let mut keys: Vec<NodeKey> = d
        .tracks
        .iter()
        .flat_map(|t| {
            t.chain.iter().map(|e| e.node).chain(
                t.racks
                    .iter()
                    .flat_map(|r| r.pads.iter().flat_map(|p| p.chain.iter().map(|e| e.node))),
            )
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// A Worklet host (heap rings) with a `Utility` node created for every key `d` references,
/// under the desc's (virtual) keys.
pub struct Pipeline {
    pub bridge: WebBridge<HeapMemory>,
    pub engine: EngineHost<HeapMemory>,
    pub shared: Shared<HeapMemory>,
}

impl Pipeline {
    pub fn new(d: &RenderGraphDesc) -> Self {
        let control = HeapMemory::new(1 << 20);
        let reports = HeapMemory::new(1 << 14);
        let shared = bridge::shared(control.clone(), reports.clone());
        let mut p = Self {
            bridge: WebBridge::new(shared.clone()),
            engine: EngineHost::new(48_000, control, reports),
            shared,
        };
        for key in node_keys(d) {
            p.send(&EngineMsg::CreateBuiltin {
                key,
                device: BuiltinDevice::Utility,
                params: vec![],
            });
        }
        p.drain(|| 0.0);
        p
    }

    pub fn send(&mut self, msg: &EngineMsg) {
        let mut shared = self.shared.borrow_mut();
        for frame in msg.encode() {
            shared.control.send(&frame);
        }
    }

    /// Render until the control ring is empty, plus [`Self::SETTLE_QUANTA`] (so deferred
    /// work, e.g. dropping the retired snapshot, is included); returns the slowest quantum
    /// (per `now`).
    pub fn drain(&mut self, now: impl Fn() -> f64) -> f64 {
        let mut worst: f64 = 0.0;
        let mut settle = Self::SETTLE_QUANTA;
        for _ in 0..100_000 {
            self.shared.borrow_mut().control.flush();
            if self.engine.control_backlog() == 0
                && self.shared.borrow().control.pending_bytes() == 0
            {
                if settle == 0 {
                    return worst;
                }
                settle -= 1;
            }
            let t = now();
            self.engine.render(RENDER_QUANTUM);
            worst = worst.max(now() - t);
        }
        panic!("control ring never drained");
    }

    pub const SETTLE_QUANTA: usize = 2;
}

fn best<T>(runs: usize, now: &impl Fn() -> f64, mut f: impl FnMut() -> T) -> f64 {
    (0..runs.max(1))
        .map(|_| {
            let t = now();
            let out = f();
            let e = now() - t;
            drop(std::hint::black_box(out));
            e
        })
        .fold(f64::INFINITY, f64::min)
}

/// Measure the large fixture's snapshot costs; `now` returns milliseconds.
pub fn measure(runs: usize, now: impl Fn() -> f64) -> SnapshotCosts {
    let desc = fixture::large_project();
    let json = serde_json::to_vec(&desc).expect("desc serializes");
    let mut bin = Vec::new();
    BinaryCodec.encode(&desc, &mut bin);

    let json_encode_ms = best(runs, &now, || serde_json::to_vec(&desc).unwrap());
    let json_decode_ms = best(runs, &now, || {
        serde_json::from_slice::<RenderGraphDesc>(&json).unwrap()
    });
    let binary_encode_ms = best(runs, &now, || {
        let mut out = Vec::new();
        BinaryCodec.encode(&desc, &mut out);
        out
    });
    let binary_decode_ms = best(runs, &now, || BinaryCodec.decode(&bin).unwrap());

    let mut p = Pipeline::new(&desc);
    let msg = EngineMsg::Publish {
        graph: Box::new(desc.clone()),
    };
    let mut publish_quantum_ms = f64::INFINITY;
    for _ in 0..runs.max(1) {
        p.send(&msg);
        publish_quantum_ms = publish_quantum_ms.min(p.drain(&now));
    }

    // Compile alone: publish straight into a core engine, keys mapped like the Worklet does.
    let parts = ether_core::create(web_engine_config(48_000));
    let (mut handle, mut engine, mut gc) = (parts.handle, parts.engine, parts.gc);
    let mut map = BTreeMap::new();
    for key in node_keys(&desc) {
        let node = ether_devices::create(&BuiltinDevice::Utility, &NoSamples);
        map.insert(key, handle.add_node(node).expect("node slot"));
    }
    let mut remapped = desc.clone();
    for t in &mut remapped.tracks {
        let pads = t.racks.iter_mut().flat_map(|r| r.pads.iter_mut());
        for e in t
            .chain
            .iter_mut()
            .chain(pads.flat_map(|p| p.chain.iter_mut()))
        {
            e.node = map[&e.node];
        }
        for r in &mut t.racks {
            r.rack = map[&r.rack];
        }
        let lanes = t
            .automation
            .iter_mut()
            .chain(t.clips.iter_mut().flat_map(|c| c.envelopes.iter_mut()));
        for a in lanes {
            if let Some(node) = a.resolved.node_mut() {
                *node = map[node];
            }
        }
    }
    let mut compile_ms = f64::INFINITY;
    let mut l = [0.0; RENDER_QUANTUM];
    let mut r = [0.0; RENDER_QUANTUM];
    for _ in 0..runs.max(1) {
        let d = remapped.clone();
        let t = now();
        handle.publish(d).expect("large fixture compiles");
        compile_ms = compile_ms.min(now() - t);
        // Swap and retire, so the control queue never fills.
        engine.process(&[], &mut [&mut l, &mut r], RENDER_QUANTUM);
        gc.collect();
    }

    SnapshotCosts {
        tracks: desc.tracks.len(),
        clips: desc.tracks.iter().map(|t| t.clips.len()).sum(),
        json_bytes: json.len(),
        binary_bytes: bin.len(),
        json_encode_ms,
        json_decode_ms,
        binary_encode_ms,
        binary_decode_ms,
        compile_ms,
        publish_quantum_ms,
        quantum_budget_ms: RENDER_QUANTUM as f64 / 48.0,
    }
}

struct NoSamples;

impl ether_devices::SampleResolver for NoSamples {
    fn resolve(
        &self,
        _media: ether_core::protocol::model::MediaId,
    ) -> Option<std::sync::Arc<dyn ether_core::AudioSource>> {
        None
    }
}

/// IR swap cost on the Worklet: a convolution reverb behind a playing synth is
/// switched to the longest factory IR through the real path (`WebBridge::update_builtin` →
/// ring → `EngineHost`), and every render quantum until the swap has settled is timed.
#[derive(Clone, Debug, Serialize)]
pub struct IrSwapCosts {
    /// The factory IR switched to (the longest) and its length.
    pub ir: String,
    pub ir_seconds: f64,
    /// Slowest quantum while the reverb plays that IR, nothing in flight (fastest of `runs`).
    pub steady_quantum_ms: f64,
    /// Slowest quantum from the switch until the swap's crossfade is over (fastest of
    /// `runs` switches).
    pub swap_quantum_ms: f64,
    /// One render quantum's real-time budget at 48 kHz.
    pub quantum_budget_ms: f64,
}

/// Quanta timed per switch (1.6 s): the prepared IR's frames, its assembly and the 200 ms
/// crossfade (75 quanta at 48 kHz) all fit, with room to spare.
const IR_SWAP_QUANTA: usize = 600;

/// A master track and a MIDI track playing notes through `synth` → `reverb`.
fn reverb_graph(synth: NodeKey, reverb: NodeKey) -> RenderGraphDesc {
    use ether_core::graph::{ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, TrackDesc};
    use ether_core::protocol::model::{ClipId, TrackId, TrackKind, Ulid};
    let master = TrackId(Ulid(1));
    let entry = |node| ChainEntry {
        node,
        enabled: true,
        sidechain: None,
    };
    let track = |id, kind, chain, output, clips| TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id,
        kind,
        chain,
        output,
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
        clips,
        automation: vec![],
        racks: Vec::new(),
        expression: Default::default(),
        hw_io: Vec::new(),
        sidechain_taps: Vec::new(),
        midi_sidechains: Vec::new(),
    };
    let notes = (0..256)
        .map(|i| NoteDesc {
            start: i as f64,
            duration: 0.5,
            key: 48 + (i % 12) as u8,
            velocity: 0.8,
            release_velocity: 0.5,
        })
        .collect();
    let clip = ClipDesc {
        id: ClipId(Ulid(3)),
        start: 0.0,
        length: 256.0,
        offset: 0.0,
        looping: None,
        muted: false,
        content: ClipContentDesc::Midi { notes },
        envelopes: vec![],
    };
    RenderGraphDesc {
        version: 1,
        tracks: vec![
            track(master, TrackKind::Master, vec![], None, vec![]),
            track(
                TrackId(Ulid(2)),
                TrackKind::Midi,
                vec![entry(synth), entry(reverb)],
                Some(master),
                vec![clip],
            ),
        ],
        ..Default::default()
    }
}

/// Measure switching a playing reverb from the shortest to the longest factory IR, `runs`
/// times; `now` returns milliseconds.
pub fn measure_ir_swap(runs: usize, now: impl Fn() -> f64) -> IrSwapCosts {
    use ether_controller::EngineBridge;
    use ether_core::EngineOutputs;
    use ether_core::protocol::model::{DeviceId, IrSource, Ulid};
    use ether_devices::fx_space::FACTORY_IRS;

    let mut irs = FACTORY_IRS.to_vec();
    irs.sort_by(|a, b| a.length_seconds.total_cmp(&b.length_seconds));
    let (short, long) = (irs[0], irs[irs.len() - 1]);
    let reverb = |id: &str| BuiltinDevice::ConvolutionReverb {
        ir: Some(IrSource::Factory { id: id.into() }),
    };
    let control = crate::ring::HeapMemory::new(1 << 20);
    let reports = crate::ring::HeapMemory::new(1 << 16);
    let shared = bridge::shared(control.clone(), reports.clone());
    let mut p = Pipeline {
        bridge: WebBridge::new(shared.clone()),
        engine: EngineHost::new(48_000, control, reports),
        shared,
    };
    let synth = p
        .bridge
        .create_builtin(DeviceId(Ulid(10)), &BuiltinDevice::Synth, &[])
        .expect("synth");
    let dev = DeviceId(Ulid(11));
    let rev = p
        .bridge
        .create_builtin(dev, &reverb(short.id), &[])
        .expect("reverb");
    p.bridge.publish(reverb_graph(synth, rev)).expect("publish");
    p.bridge
        .transport(ether_core::TransportControl::Play)
        .expect("play");
    let mut out = EngineOutputs::default();
    let mut timed = |p: &mut Pipeline, quanta: usize| {
        let mut worst: f64 = 0.0;
        for _ in 0..quanta {
            p.shared.borrow_mut().control.flush();
            let t = now();
            p.engine.render(RENDER_QUANTUM);
            worst = worst.max(now() - t);
        }
        p.bridge.poll(&mut out);
        worst
    };
    timed(&mut p, IR_SWAP_QUANTA);
    let mut swap_quantum_ms = f64::INFINITY;
    let mut steady_quantum_ms = f64::INFINITY;
    for _ in 0..runs.max(1) {
        p.bridge
            .update_builtin(dev, &reverb(short.id))
            .expect("short");
        timed(&mut p, IR_SWAP_QUANTA);
        p.bridge
            .update_builtin(dev, &reverb(long.id))
            .expect("long");
        swap_quantum_ms = swap_quantum_ms.min(timed(&mut p, IR_SWAP_QUANTA));
        steady_quantum_ms = steady_quantum_ms.min(timed(&mut p, IR_SWAP_QUANTA));
    }
    let errors = std::mem::take(&mut p.shared.borrow_mut().errors);
    assert!(errors.is_empty(), "{errors:?}");
    IrSwapCosts {
        ir: long.id.to_owned(),
        ir_seconds: long.length_seconds,
        steady_quantum_ms,
        swap_quantum_ms,
        quantum_budget_ms: RENDER_QUANTUM as f64 / 48.0,
    }
}

/// Fastest-of-N worst quanta (ms) of switching a playing reverb to a user's media IR.
#[derive(Clone, Debug, Serialize)]
pub struct MediaIrSwapCosts {
    /// The media IR's length (stereo, 48 kHz).
    pub ir_seconds: f64,
    /// Built by the Worklet in one go (before `UpdateBuiltin` with the media IR).
    pub before_quantum_ms: f64,
    /// Prepared on the Worker from its audio, assembled by the Worklet.
    pub swap_quantum_ms: f64,
    pub quantum_budget_ms: f64,
}

/// Measure switching a playing reverb from a short factory IR to `seconds` of stereo
/// media IR, the old way and the prepared way, `runs` times each; `now` returns ms.
pub fn measure_media_ir_swap(seconds: f64, runs: usize, now: impl Fn() -> f64) -> MediaIrSwapCosts {
    use std::sync::Arc;

    use ether_controller::EngineBridge;
    use ether_core::EngineOutputs;
    use ether_core::protocol::model::{DeviceId, IrSource, MediaId, MediaRef, Ulid};
    use ether_media::DecodedAudio;

    let media = MediaId(Ulid(20));
    let frames = (seconds * 48_000.0) as usize;
    let mut seed = 0x2545_f491_u32;
    let mut channel = || {
        (0..frames)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32 * 2.0 - 1.0)
                    * (-3.0 * i as f32 / frames as f32).exp()
            })
            .collect::<Vec<f32>>()
    };
    let audio = Arc::new(DecodedAudio {
        sample_rate: 48_000,
        channels: vec![channel(), channel()],
    });
    let media_ref = MediaRef {
        location: Default::default(),
        id: media,
        name: "ir".into(),
        file: "media/ir.wav".into(),
        sample_rate: 48_000,
        channels: 2,
        frames: frames as u64,
        hash: None,
    };
    let factory = BuiltinDevice::ConvolutionReverb {
        ir: Some(IrSource::Factory { id: "room".into() }),
    };
    let user = BuiltinDevice::ConvolutionReverb {
        ir: Some(IrSource::Media { media }),
    };
    let control = crate::ring::HeapMemory::new(1 << 20);
    let reports = crate::ring::HeapMemory::new(1 << 16);
    let shared = bridge::shared(control.clone(), reports.clone());
    let mut p = Pipeline {
        bridge: WebBridge::new(shared.clone()),
        engine: EngineHost::new(48_000, control, reports),
        shared,
    };
    let synth = p
        .bridge
        .create_builtin(DeviceId(Ulid(10)), &BuiltinDevice::Synth, &[])
        .expect("synth");
    let dev = DeviceId(Ulid(11));
    let rev = p.bridge.create_builtin(dev, &factory, &[]).expect("reverb");
    p.bridge.publish(reverb_graph(synth, rev)).expect("publish");
    p.bridge
        .load_media(&media_ref, audio.clone())
        .expect("media");
    p.bridge
        .transport(ether_core::TransportControl::Play)
        .expect("play");
    let mut out = EngineOutputs::default();
    let mut timed = |p: &mut Pipeline, quanta: usize| {
        let mut worst: f64 = 0.0;
        for _ in 0..quanta {
            p.shared.borrow_mut().control.flush();
            let t = now();
            p.engine.render(RENDER_QUANTUM);
            worst = worst.max(now() - t);
        }
        p.bridge.poll(&mut out);
        worst
    };
    timed(&mut p, IR_SWAP_QUANTA);
    let mut before_quantum_ms = f64::INFINITY;
    let mut swap_quantum_ms = f64::INFINITY;
    for _ in 0..runs.max(1) {
        p.bridge.update_builtin(dev, &factory).expect("factory");
        timed(&mut p, IR_SWAP_QUANTA);
        let frames = EngineMsg::UpdateBuiltin {
            key: rev,
            device: user.clone(),
        }
        .encode();
        for f in frames {
            p.shared.borrow_mut().control.send(&f);
        }
        before_quantum_ms = before_quantum_ms.min(timed(&mut p, IR_SWAP_QUANTA));

        p.bridge.update_builtin(dev, &factory).expect("factory");
        timed(&mut p, IR_SWAP_QUANTA);
        p.bridge.update_builtin(dev, &user).expect("media");
        for m in p.bridge.take_ir_media_requests() {
            p.bridge.ir_media(m, audio.clone()).expect("ir media");
        }
        swap_quantum_ms = swap_quantum_ms.min(timed(&mut p, IR_SWAP_QUANTA));
    }
    let errors = std::mem::take(&mut p.shared.borrow_mut().errors);
    assert!(errors.is_empty(), "{errors:?}");
    MediaIrSwapCosts {
        ir_seconds: seconds,
        before_quantum_ms,
        swap_quantum_ms,
        quantum_budget_ms: RENDER_QUANTUM as f64 / 48.0,
    }
}

/// One stretcher voice per web warp mode on 128-frame quanta (the Worklet's), stereo,
/// ×1.25 (and +3 st with formants kept, Complex+): mean time per quantum and the worst one,
/// fastest of `runs`. Feature `warp-complex` (the web's stretchers).
#[derive(Clone, Debug, Serialize)]
pub struct StretchCosts {
    /// Complex (Signalsmith Stretch's Rust port, web preset).
    pub complex_quantum_ms: f64,
    pub complex_worst_ms: f64,
    /// Complex+.
    pub plus_quantum_ms: f64,
    pub plus_worst_ms: f64,
    /// Complex+ transposing (+3 st) with formants kept.
    pub plus_transposed_quantum_ms: f64,
    pub plus_transposed_worst_ms: f64,
    pub quantum_budget_ms: f64,
}

#[cfg(feature = "warp-complex")]
pub fn measure_stretch(seconds: f64, runs: usize, now: impl Fn() -> f64) -> StretchCosts {
    use ether_stretch::{
        PlusStretchFactory, RustPreset, RustStretchFactory, Stretcher, StretcherFactory,
    };
    const Q: usize = RENDER_QUANTUM;
    let frames = (seconds * 48_000.0) as usize;
    // A chord over a pulse of noise bursts (deterministic).
    let mut seed = 1u32;
    let x: Vec<f32> = (0..frames)
        .map(|i| {
            let t = i as f32 / 48_000.0;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            let burst = (-(t * 4.0).fract() * 30.0).exp();
            [220.0f32, 277.2, 329.6, 440.0]
                .iter()
                .map(|f| 0.1 * (std::f32::consts::TAU * f * t).sin())
                .sum::<f32>()
                + 0.3 * noise * burst
        })
        .collect();
    let run = |f: &dyn StretcherFactory, semitones: f32| -> (f64, f64) {
        let mut st: Box<dyn Stretcher> = f.create();
        st.configure(2, 48_000.0, Q);
        st.set_transpose_semitones(semitones);
        let (mut l, mut r) = (vec![0.0f32; Q], vec![0.0f32; Q]);
        let (mut pos, mut o, mut worst, mut total, mut n) = (0usize, 0usize, 0.0f64, 0.0, 0);
        while pos + 2 * Q < frames {
            let n_in = ((o + Q) as f64 * 0.8) as usize - (o as f64 * 0.8) as usize;
            let src = &x[pos..pos + n_in];
            let t0 = now();
            st.process(&[src, src], n_in, &mut [&mut l, &mut r], Q);
            let dt = now() - t0;
            worst = f64::max(worst, dt);
            total += dt;
            n += 1;
            pos += n_in;
            o += Q;
        }
        std::hint::black_box((&l, &r));
        (total / n.max(1) as f64, worst)
    };
    let best = |f: &dyn StretcherFactory, semitones: f32| {
        (0..runs.max(1))
            .map(|_| run(f, semitones))
            .fold((f64::MAX, f64::MAX), |a, b| (a.0.min(b.0), a.1.min(b.1)))
    };
    let complex = best(
        &RustStretchFactory {
            preset: RustPreset::Web,
        },
        0.0,
    );
    let plus = best(&PlusStretchFactory, 0.0);
    let plus_t = best(&PlusStretchFactory, 3.0);
    StretchCosts {
        complex_quantum_ms: complex.0,
        complex_worst_ms: complex.1,
        plus_quantum_ms: plus.0,
        plus_worst_ms: plus.1,
        plus_transposed_quantum_ms: plus_t.0,
        plus_transposed_worst_ms: plus_t.1,
        quantum_budget_ms: Q as f64 / 48.0,
    }
}

/// Mean and worst Worklet quantum (ms) of a playing project, per scenario
/// ([`measure_render`]).
#[derive(Clone, Debug, Serialize)]
pub struct RenderCost {
    pub scenario: String,
    pub tracks: usize,
    pub devices: usize,
    pub mean_ms: f64,
    pub worst_ms: f64,
}

/// Per-quantum render costs of the Worklet engine, fastest of `runs`.
#[derive(Clone, Debug, Serialize)]
pub struct RenderCosts {
    pub scenarios: Vec<RenderCost>,
    pub quantum_budget_ms: f64,
}

/// Render scenarios: name, tracks, each track's chain after its Synth (None = no
/// devices at all: the engine's per-track cost), the master's chain.
type RenderScenario = (
    &'static str,
    usize,
    Option<&'static [BuiltinDevice]>,
    &'static [BuiltinDevice],
);

const RENDER_SCENARIOS: &[RenderScenario] = &[
    // Tracks with a clip and no devices: the engine alone.
    ("empty32", 32, None, &[]),
    // A synth per track (the base of the two next).
    ("synth16", 16, Some(&[]), &[]),
    // A synth and four near-free devices per track: per-device overhead.
    (
        "utility16x4",
        16,
        Some(&[
            BuiltinDevice::Utility,
            BuiltinDevice::Utility,
            BuiltinDevice::Utility,
            BuiltinDevice::Utility,
        ]),
        &[],
    ),
    // A small mix: synth, EQ, compressor, utility per track; limiter on master.
    (
        "mix8",
        8,
        Some(&[
            BuiltinDevice::Eq,
            BuiltinDevice::Compressor,
            BuiltinDevice::Utility,
        ]),
        &[BuiltinDevice::Limiter],
    ),
];

/// Play each scenario for `seconds` through the Worklet host, `runs` times; `now`
/// returns milliseconds.
pub fn measure_render(seconds: f64, runs: usize, now: impl Fn() -> f64) -> RenderCosts {
    use ether_controller::EngineBridge;
    use ether_core::EngineOutputs;
    use ether_core::protocol::model::{DeviceId, Ulid};

    let quanta = ((seconds * 48_000.0) as usize / RENDER_QUANTUM).max(1);
    let mut scenarios = Vec::new();
    for &(name, n_tracks, chain, master_chain) in RENDER_SCENARIOS {
        let (mut mean_ms, mut worst_ms, mut devices) = (f64::INFINITY, f64::INFINITY, 0);
        for _ in 0..runs.max(1) {
            let control = HeapMemory::new(1 << 20);
            let reports = HeapMemory::new(1 << 16);
            let shared = bridge::shared(control.clone(), reports.clone());
            let mut p = Pipeline {
                bridge: WebBridge::new(shared.clone()),
                engine: EngineHost::new(48_000, control, reports),
                shared,
            };
            let mut next_id = 100u128;
            let mut create = |p: &mut Pipeline, d: &BuiltinDevice| {
                next_id += 1;
                p.bridge
                    .create_builtin(DeviceId(Ulid(next_id)), d, &[])
                    .expect("built-in device")
            };
            let chains: Vec<Vec<NodeKey>> = (0..n_tracks)
                .map(|_| match chain {
                    None => Vec::new(),
                    Some(c) => std::iter::once(create(&mut p, &BuiltinDevice::Synth))
                        .chain(c.iter().map(|d| create(&mut p, d)))
                        .collect(),
                })
                .collect();
            let master: Vec<NodeKey> = master_chain.iter().map(|d| create(&mut p, d)).collect();
            devices = chains.iter().map(Vec::len).sum::<usize>() + master.len();
            p.bridge
                .publish(render_graph(&chains, &master))
                .expect("publish");
            p.bridge
                .transport(ether_core::TransportControl::Play)
                .expect("play");
            p.drain(&now);
            let mut out = EngineOutputs::default();
            let (mut total, mut worst) = (0.0, 0.0f64);
            for q in 0..quanta {
                let t = now();
                p.engine.render(RENDER_QUANTUM);
                let dt = now() - t;
                total += dt;
                worst = worst.max(dt);
                if q % 64 == 0 {
                    p.bridge.poll(&mut out);
                }
            }
            let errors = std::mem::take(&mut p.shared.borrow_mut().errors);
            assert!(errors.is_empty(), "{name}: {errors:?}");
            mean_ms = mean_ms.min(total / quanta as f64);
            worst_ms = worst_ms.min(worst);
        }
        scenarios.push(RenderCost {
            scenario: name.into(),
            tracks: n_tracks,
            devices,
            mean_ms,
            worst_ms,
        });
    }
    RenderCosts {
        scenarios,
        quantum_budget_ms: RENDER_QUANTUM as f64 / 48.0,
    }
}

/// MIDI tracks with `chains` (each with a clip of notes) into a master with `master`.
fn render_graph(chains: &[Vec<NodeKey>], master: &[NodeKey]) -> RenderGraphDesc {
    use ether_core::graph::ChainEntry;
    use ether_core::protocol::model::{ClipId, TrackId, TrackKind, Ulid};
    // The reverb graph's master and synth track, with other chains.
    let none = NodeKey {
        index: 0,
        generation: 0,
    };
    let template = reverb_graph(none, none);
    let entry = |node: &NodeKey| ChainEntry {
        node: *node,
        enabled: true,
        sidechain: None,
    };
    let mut master_track = template.tracks[0].clone();
    master_track.chain = master.iter().map(entry).collect();
    let mut tracks = vec![master_track];
    for (i, c) in chains.iter().enumerate() {
        let mut t = template.tracks[1].clone();
        t.id = TrackId(Ulid(1000 + i as u128));
        t.chain = c.iter().map(entry).collect();
        for clip in &mut t.clips {
            clip.id = ClipId(Ulid(5000 + i as u128));
        }
        t.kind = TrackKind::Midi;
        tracks.push(t);
    }
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}
