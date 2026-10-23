//! A convolution reverb's factory IR is prepared on the Worker (read, normalized,
//! FFT-partitioned) and only assembled on the Worklet, in bounded steps, so switching IRs
//! doesn't stall the audio thread. The same for IRs from the user's own media, whose
//! audio the Worker asks the controller for again.

use std::sync::Arc;
use std::time::Duration;

/// The calling thread's CPU time (Linux and macOS; wall time elsewhere). The comparisons below
/// are about the work a quantum does: on a busy machine (a shared CI runner) a preemption in the
/// middle of a quantum adds wall time but no work, and made them flaky.
fn clock() -> Duration {
    #[cfg(all(
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    {
        #[repr(C)]
        struct Timespec {
            sec: i64,
            nsec: i64,
        }
        unsafe extern "C" {
            fn clock_gettime(clock: i32, ts: *mut Timespec) -> i32;
        }
        #[cfg(target_os = "linux")]
        const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
        #[cfg(target_os = "macos")]
        const CLOCK_THREAD_CPUTIME_ID: i32 = 16;
        let mut ts = Timespec { sec: 0, nsec: 0 };
        // SAFETY: `ts` is a valid, writable `struct timespec` (two 64-bit fields here).
        let rc = unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut ts) };
        assert_eq!(rc, 0, "clock_gettime(CLOCK_THREAD_CPUTIME_ID)");
        Duration::new(ts.sec as u64, ts.nsec as u32)
    }
    #[cfg(not(all(
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    )))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed()
    }
}

use ether_controller::EngineBridge;
use ether_core::AudioSource;
use ether_core::graph::{ChainEntry, ClipContentDesc, ClipDesc, NoteDesc, TrackDesc};
use ether_core::protocol::model::{
    BuiltinDevice, ClipId, DeviceId, IrSource, MediaId, MediaRef, TrackId, TrackKind, Ulid,
};
use ether_core::{EngineOutputs, NodeKey, RenderGraphDesc, TransportControl};
use ether_devices::fx_space;
use ether_devices::{NoSamples, SampleResolver};
use ether_media::{DecodedAudio, InMemorySource};
use ether_wasm::bridge::{self, Shared, WebBridge};
use ether_wasm::proto::{EngineMsg, Frame};
use ether_wasm::ring::{HeapMemory, RingReader};
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM};

fn reverb(id: Option<&str>) -> BuiltinDevice {
    BuiltinDevice::ConvolutionReverb {
        ir: id.map(|id| IrSource::Factory { id: id.into() }),
    }
}

/// A master track and a MIDI track playing notes through `synth` → `reverb`.
fn graph(synth: NodeKey, reverb: NodeKey) -> RenderGraphDesc {
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
    let notes = (0..64)
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
        length: 64.0,
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

struct Rig {
    bridge: WebBridge<HeapMemory>,
    host: EngineHost<HeapMemory>,
    shared: Shared<HeapMemory>,
    reverb: DeviceId,
    key: NodeKey,
}

const REVERB: DeviceId = DeviceId(Ulid(11));

/// A playing synth → reverb (`ir`) on the Worklet, settled.
fn rig(ir: Option<&str>) -> Rig {
    let control = HeapMemory::new(1 << 20);
    let reports = HeapMemory::new(1 << 16);
    let shared = bridge::shared(control.clone(), reports.clone());
    let mut r = Rig {
        bridge: WebBridge::new(shared.clone()),
        host: EngineHost::new(48_000, control, reports),
        shared,
        reverb: REVERB,
        key: NodeKey {
            index: 0,
            generation: 0,
        },
    };
    let synth = r
        .bridge
        .create_builtin(DeviceId(Ulid(10)), &BuiltinDevice::Synth, &[])
        .unwrap();
    r.key = r.bridge.create_builtin(REVERB, &reverb(ir), &[]).unwrap();
    r.bridge.publish(graph(synth, r.key)).unwrap();
    r.bridge.transport(TransportControl::Play).unwrap();
    r.render(400);
    r
}

impl Rig {
    /// Render `quanta` blocks; the slowest one.
    fn render(&mut self, quanta: usize) -> Duration {
        let mut worst = Duration::ZERO;
        for _ in 0..quanta {
            self.shared.borrow_mut().control.flush();
            let t = clock();
            self.host.render(RENDER_QUANTUM);
            worst = worst.max(clock() - t);
        }
        worst
    }

    fn errors(&mut self) -> Vec<String> {
        let mut out = EngineOutputs::default();
        self.bridge.poll(&mut out);
        std::mem::take(&mut self.shared.borrow_mut().errors)
    }
}

/// The time `ir_swap` takes to build `id` in one go (what the Worklet used to do).
fn direct_build(id: &str) -> Duration {
    (0..3)
        .map(|_| {
            let t = clock();
            let data = fx_space::ir_swap(&reverb(Some(id)), &NoSamples, 48_000.0);
            let e = clock() - t;
            drop(data);
            e
        })
        .min()
        .unwrap()
}

/// The slowest quantum of a switch, over several runs of it: per quantum, the fastest run.
/// The IR is assembled in a fixed amount of work per quantum, so heavy work lands on the same
/// quantum in every run and stays; a quantum made slow by the machine (a neighbour, a stolen
/// vCPU on the shared CI pool: 4.8 ms there for 0.3 ms alone) is not slow in every run.
fn steady_worst(runs: &[Vec<Duration>]) -> Duration {
    (0..runs[0].len())
        .map(|i| runs.iter().map(|run| run[i]).min().unwrap())
        .max()
        .unwrap()
}

/// Runs of each switch for [`steady_worst`].
const RUNS: usize = 5;

#[test]
fn switching_to_the_longest_factory_ir_does_no_heavy_work_per_quantum() {
    let longest = fx_space::FACTORY_IRS
        .iter()
        .max_by(|a, b| a.length_seconds.total_cmp(&b.length_seconds))
        .unwrap()
        .id;
    let direct = direct_build(longest);
    let mut runs = Vec::new();
    for _ in 0..RUNS {
        let mut r = rig(Some("room"));
        assert_eq!(
            r.bridge.update_builtin(r.reverb, &reverb(Some(longest))),
            Ok(true)
        );
        let mut assembling = 0;
        let mut quanta = Vec::new();
        for _ in 0..600 {
            quanta.push(r.render(1));
            assembling += usize::from(r.host.pending_irs() > 0);
        }
        assert_eq!(r.host.pending_irs(), 0, "assembled and swapped in");
        assert!(assembling > 1, "assembled over several quanta");
        assert_eq!(r.errors(), Vec::<String>::new());
        runs.push(quanta);
    }
    let worst = steady_worst(&runs);
    eprintln!("worst quantum {worst:?}; building {longest} in one go: {direct:?}");
    assert!(
        worst * 4 < direct,
        "a quantum took {worst:?}, a quarter of building the IR ({direct:?})"
    );
}

/// Frames the bridge sent (decoded), for a bridge whose control ring nobody else drains.
fn sent(control: &HeapMemory, shared: &Shared<HeapMemory>) -> Vec<String> {
    shared.borrow_mut().control.flush();
    let mut reader = RingReader::new(control.clone());
    let mut out = Vec::new();
    reader.drain(usize::MAX, |bytes| {
        out.push(match Frame::decode(bytes).unwrap() {
            Frame::Msg(EngineMsg::CreateBuiltin { device, .. }) => format!("create {device:?}"),
            Frame::Msg(EngineMsg::UpdateBuiltin { device, .. }) => format!("update {device:?}"),
            Frame::IrBegin { source, .. } => format!("prepared {source:?}"),
            Frame::IrChunk { .. } => "chunk".into(),
            other => format!("{other:?}"),
        });
        true
    });
    out.dedup();
    out
}

#[test]
fn factory_irs_are_prepared_on_the_worker_and_the_rest_left_to_the_worklet() {
    let control = HeapMemory::new(1 << 24);
    let reports = HeapMemory::new(1 << 16);
    let shared = bridge::shared(control.clone(), reports);
    let mut bridge = WebBridge::new(shared.clone());
    bridge
        .create_builtin(REVERB, &reverb(Some("hall")), &[])
        .unwrap();
    let hall = IrSource::Factory { id: "hall".into() };
    assert_eq!(
        sent(&control, &shared),
        [
            format!("create {:?}", reverb(None)),
            format!("prepared {hall:?}"),
            "chunk".into()
        ],
        "created without its IR, which follows prepared"
    );
    for (ir, want) in [
        (reverb(Some("plate")), "prepared"),
        (reverb(Some("no-such-ir")), "update"),
        (reverb(None), "update"),
        (
            BuiltinDevice::ConvolutionReverb {
                ir: Some(IrSource::Media {
                    media: ether_core::protocol::model::MediaId(Ulid(5)),
                }),
            },
            "update",
        ),
    ] {
        assert_eq!(bridge.update_builtin(REVERB, &ir), Ok(true));
        let frames = sent(&control, &shared);
        assert!(frames[0].starts_with(want), "{ir:?}: {frames:?}");
    }
}

#[test]
fn a_newer_change_supersedes_a_prepared_ir_in_flight() {
    // Removing the IR (or deleting the device) while a prepared one is still arriving:
    // the newest change wins, nothing is left assembling, and no errors.
    let mut r = rig(Some("room"));
    r.bridge
        .update_builtin(r.reverb, &reverb(Some("cathedral")))
        .unwrap();
    r.render(2);
    assert_eq!(r.host.pending_irs(), 1);
    r.bridge.update_builtin(r.reverb, &reverb(None)).unwrap();
    r.render(200);
    assert_eq!(r.host.pending_irs(), 0);
    assert_eq!(r.errors(), Vec::<String>::new());

    r.bridge
        .update_builtin(r.reverb, &reverb(Some("hall")))
        .unwrap();
    r.render(2);
    r.bridge.destroy_node(r.key).unwrap();
    r.render(200);
    assert_eq!(r.host.pending_irs(), 0);
    // The graph still references the destroyed node (as it would until the next publish).
    let errors = r.errors();
    assert!(errors.iter().all(|e| !e.contains("IR")), "{errors:?}");
}

// ─── IRs from the user's own media ───

const IR_MEDIA: MediaId = MediaId(Ulid(20));

fn media_reverb(media: MediaId) -> BuiltinDevice {
    BuiltinDevice::ConvolutionReverb {
        ir: Some(IrSource::Media { media }),
    }
}

/// A long user IR: `seconds` of decaying stereo noise at 48 kHz.
fn long_ir(seconds: f64) -> Arc<DecodedAudio> {
    let frames = (seconds * 48_000.0) as usize;
    let mut seed = 0x2545_f491_u32;
    let mut channel = || {
        (0..frames)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
                noise * (-3.0 * i as f32 / frames as f32).exp()
            })
            .collect::<Vec<f32>>()
    };
    let channels = vec![channel(), channel()];
    Arc::new(DecodedAudio {
        sample_rate: 48_000,
        channels,
    })
}

fn media_ref(audio: &DecodedAudio) -> MediaRef {
    MediaRef {
        location: Default::default(),
        id: IR_MEDIA,
        name: "my-ir".into(),
        file: "media/my-ir.wav".into(),
        sample_rate: 48_000,
        channels: audio.channels.len() as u16,
        frames: audio.frames() as u64,
        hash: None,
    }
}

struct OneMedia(Arc<dyn AudioSource>);

impl SampleResolver for OneMedia {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        (media == IR_MEDIA).then(|| self.0.clone())
    }
}

/// The time the Worklet took to build `audio` as an IR in one go (as it used to).
fn direct_media_build(audio: &Arc<DecodedAudio>) -> Duration {
    let samples = OneMedia(Arc::new(InMemorySource::new(audio.clone())));
    (0..3)
        .map(|_| {
            let t = clock();
            let data = fx_space::ir_swap(&media_reverb(IR_MEDIA), &samples, 48_000.0);
            let e = clock() - t;
            assert!(data.is_some());
            drop(data);
            e
        })
        .min()
        .unwrap()
}

/// Send `msg` straight to the Worklet (what the bridge used to send).
fn send_raw(shared: &Shared<HeapMemory>, msg: EngineMsg) {
    for frame in msg.encode() {
        shared.borrow_mut().control.send(&frame);
    }
}

impl Rig {
    /// Load `audio` as [`IR_MEDIA`] into the Worklet (the controller's `load_media`).
    fn load(&mut self, audio: &Arc<DecodedAudio>) {
        self.bridge
            .load_media(&media_ref(audio), audio.clone())
            .unwrap();
        self.render(200);
    }

    /// The controller's side of `take_ir_media_requests`: decode each again (here, the
    /// same audio) and hand it over.
    fn serve_requests(&mut self, audio: &Arc<DecodedAudio>) -> usize {
        let requests = self.bridge.take_ir_media_requests();
        for &m in &requests {
            assert_eq!(m, IR_MEDIA);
            self.bridge.ir_media(m, audio.clone()).unwrap();
        }
        requests.len()
    }
}

#[test]
fn switching_to_a_long_media_ir_does_no_heavy_work_per_quantum() {
    let audio = long_ir(6.0);
    let direct = direct_media_build(&audio);
    let mut before = Duration::MAX;
    let mut runs = Vec::new();
    for _ in 0..RUNS {
        // Before the Worklet reads and partitions the media IR itself.
        let mut r = rig(Some("room"));
        r.load(&audio);
        send_raw(
            &r.shared,
            EngineMsg::UpdateBuiltin {
                key: r.key,
                device: media_reverb(IR_MEDIA),
            },
        );
        before = before.min(r.render(50));
        assert_eq!(r.errors(), Vec::<String>::new());

        // Now: the Worker asks for the audio again, prepares it, the Worklet assembles it.
        let mut r = rig(Some("room"));
        r.load(&audio);
        assert_eq!(r.bridge.ir_audio_kept(), 0, "loaded media are not kept");
        assert_eq!(
            r.bridge.update_builtin(r.reverb, &media_reverb(IR_MEDIA)),
            Ok(true)
        );
        assert_eq!(r.serve_requests(&audio), 1, "the audio is asked for");
        assert_eq!(r.bridge.ir_audio_kept(), 0, "dropped once the IR is sent");
        let mut assembling = 0;
        let mut quanta = Vec::new();
        for _ in 0..600 {
            quanta.push(r.render(1));
            assembling += usize::from(r.host.pending_irs() > 0);
        }
        assert_eq!(r.host.pending_irs(), 0, "assembled and swapped in");
        assert!(assembling > 1, "assembled over several quanta");
        assert_eq!(r.errors(), Vec::<String>::new());
        runs.push(quanta);
    }
    let after = steady_worst(&runs);
    eprintln!(
        "6 s stereo media IR: worst quantum before {before:?}, after {after:?}; \
         building it in one go: {direct:?}"
    );
    // The old path's one quantum did most of the build (about 70% of the direct build alone;
    // under load the two drift apart, 4.8 ms against 13.9 ms on the CI pool).
    assert!(
        before * 4 > direct,
        "the old path built it in one quantum: {before:?} of {direct:?}"
    );
    assert!(
        after * 4 < direct,
        "a quantum took {after:?}, a quarter of building the IR ({direct:?})"
    );
}

/// A playing synth → reverb whose media IR is there from the start (`prepared` on the
/// Worker, or built by the Worklet as it used to be), settled before playing; both output
/// channels of `quanta` played quanta.
fn played_with_media_ir(audio: &Arc<DecodedAudio>, prepared: bool, quanta: usize) -> Vec<f32> {
    let control = HeapMemory::new(1 << 20);
    let reports = HeapMemory::new(1 << 16);
    let shared = bridge::shared(control.clone(), reports.clone());
    let mut r = Rig {
        bridge: WebBridge::new(shared.clone()),
        host: EngineHost::new(48_000, control, reports),
        shared,
        reverb: REVERB,
        key: NodeKey {
            index: 0,
            generation: 0,
        },
    };
    r.load(audio);
    let synth = r
        .bridge
        .create_builtin(DeviceId(Ulid(10)), &BuiltinDevice::Synth, &[])
        .unwrap();
    let params = [(fx_space::convolution_reverb::SIZE, 120.0)];
    if prepared {
        r.key = r
            .bridge
            .create_builtin(REVERB, &media_reverb(IR_MEDIA), &params)
            .unwrap();
        assert_eq!(r.serve_requests(audio), 1);
    } else {
        r.key = NodeKey {
            index: 99,
            generation: 1,
        };
        send_raw(
            &r.shared,
            EngineMsg::CreateBuiltin {
                key: r.key,
                device: media_reverb(IR_MEDIA),
                params: params.to_vec(),
            },
        );
    }
    r.bridge.publish(graph(synth, r.key)).unwrap();
    for _ in 0..2_000 {
        r.render(1);
        if r.host.pending_irs() == 0 && r.shared.borrow().control.pending_bytes() == 0 {
            break;
        }
    }
    // The swap's crossfade (over silence) is done.
    r.render(200);
    assert_eq!(r.host.pending_irs(), 0);
    r.bridge.transport(TransportControl::Play).unwrap();
    let mut out = Vec::new();
    for _ in 0..quanta {
        r.render(1);
        out.extend_from_slice(r.host.output(0));
        out.extend_from_slice(r.host.output(1));
    }
    assert_eq!(r.errors(), Vec::<String>::new());
    out
}

#[test]
fn a_prepared_media_ir_sounds_like_one_built_in_one_go() {
    let audio = long_ir(1.5);
    let prepared = played_with_media_ir(&audio, true, 800);
    let direct = played_with_media_ir(&audio, false, 800);
    let peak = direct.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.01, "the reverb plays ({peak})");
    let diff = prepared
        .iter()
        .zip(&direct)
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(diff <= 1e-5 * peak, "differs by {diff} (peak {peak})");
}

#[test]
fn media_ir_audio_is_kept_only_until_delivered() {
    let audio = long_ir(0.5);
    let control = HeapMemory::new(1 << 24);
    let reports = HeapMemory::new(1 << 16);
    let shared = bridge::shared(control.clone(), reports);
    let mut bridge = WebBridge::new(shared.clone());
    let ir = IrSource::Media { media: IR_MEDIA };
    let second = DeviceId(Ulid(12));

    // Created before its media loaded: the Worklet has no IR to read (nothing is built).
    let first = bridge
        .create_builtin(REVERB, &media_reverb(IR_MEDIA), &[])
        .unwrap();
    assert_eq!(
        sent(&control, &shared),
        [format!("create {:?}", media_reverb(IR_MEDIA))]
    );
    // The media loads: its audio is kept for the reverb, which the controller re-creates
    // now, prepared from it without decoding it again.
    bridge
        .load_media(&media_ref(&audio), audio.clone())
        .unwrap();
    assert_eq!(bridge.ir_audio_kept(), 1);
    let _ = sent(&control, &shared);
    bridge
        .create_builtin(second, &media_reverb(IR_MEDIA), &[])
        .unwrap();
    assert_eq!(bridge.take_ir_media_requests(), []);
    assert_eq!(
        sent(&control, &shared),
        [
            format!("create {:?}", reverb(None)),
            format!("prepared {ir:?}"),
            "chunk".into()
        ]
    );
    bridge.destroy_node(first).unwrap();
    assert_eq!(
        bridge.ir_audio_kept(),
        0,
        "released once no reverb waits for it"
    );

    // A later change asks for it again; a newer change before it arrives wins.
    bridge
        .update_builtin(second, &reverb(Some("hall")))
        .unwrap();
    let _ = sent(&control, &shared);
    bridge
        .update_builtin(second, &media_reverb(IR_MEDIA))
        .unwrap();
    assert_eq!(
        sent(&control, &shared),
        Vec::<String>::new(),
        "the current IR plays on"
    );
    assert_eq!(bridge.take_ir_media_requests(), [IR_MEDIA]);
    bridge.update_builtin(second, &reverb(None)).unwrap();
    let _ = sent(&control, &shared);
    bridge.ir_media(IR_MEDIA, audio.clone()).unwrap();
    assert_eq!(sent(&control, &shared), Vec::<String>::new(), "superseded");
    assert_eq!(bridge.ir_audio_kept(), 0);

    // Its audio arrives: prepared, then dropped.
    bridge
        .update_builtin(second, &media_reverb(IR_MEDIA))
        .unwrap();
    assert_eq!(bridge.take_ir_media_requests(), [IR_MEDIA]);
    bridge.ir_media(IR_MEDIA, audio.clone()).unwrap();
    assert_eq!(
        sent(&control, &shared),
        [format!("prepared {ir:?}"), "chunk".into()]
    );
    assert_eq!(bridge.ir_audio_kept(), 0);

    // Unloaded media: nothing is prepared from it (the Worklet can't resolve it).
    bridge.update_builtin(second, &reverb(None)).unwrap();
    bridge
        .update_builtin(second, &media_reverb(IR_MEDIA))
        .unwrap();
    assert_eq!(bridge.take_ir_media_requests(), [IR_MEDIA]);
    bridge.unload_media(IR_MEDIA).unwrap();
    let _ = sent(&control, &shared);
    bridge.ir_media(IR_MEDIA, audio).unwrap();
    assert_eq!(sent(&control, &shared), Vec::<String>::new());
    assert_eq!(bridge.ir_audio_kept(), 0);
}
