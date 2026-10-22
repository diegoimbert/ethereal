//! A big project loading on the web host, natively: the real controller (the Worker) and
//! the Worklet's `EngineHost` over heap rings, rendering in step (one controller tick per
//! 16 ms of quanta). The Worklet applies the load's backlog (node creations, graphs,
//! media) a budget per quantum; suspended (`EngineBridge::set_suspended`), it runs no
//! graph while it does, and its output is silent until the project is ready.
//!
//! `cargo test -p ether-wasm --release --test it project_load_quanta -- --ignored --nocapture`
//! prints the quantum times (native: V8 runs the same code a few times slower).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::{ClientMessage, ReplyResult, ServerMessage};
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::ring::HeapMemory;
use ether_wasm::store::{Fs, MemFs, WebLibrary, WebStore};
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM};
use serde_json::{Value, json};

const TRACKS: usize = 24;
/// Audio tracks, each with a clip of its own 20 s, 44.1 kHz sample (decoded and resampled
/// on the Worker, sent to the Worklet in chunks).
const SAMPLES: usize = 12;
const SAMPLE_SECONDS: usize = 20;
const DEVICES: [&str; 5] = ["PolySynth", "Chorus", "Delay", "Reverb", "Compressor"];
/// Quanta per controller tick (16 ms at 48 kHz).
const QUANTA_PER_TICK: usize = 6;
const QUANTUM: Duration = Duration::from_nanos(128 * 1_000_000_000 / 48_000);

static NOW: AtomicU64 = AtomicU64::new(1_750_000_000_000);

struct Clock;

impl HostServices for Clock {
    fn now_ms(&self) -> u64 {
        NOW.load(Ordering::Relaxed)
    }
    fn random_seed(&mut self) -> u64 {
        7
    }
}

type Ctl = EtherController<WebBridge<HeapMemory>, Clock, WebStore<MemFs>, WebLibrary<MemFs>>;

struct Rig {
    ctl: Ctl,
    engine: EngineHost<HeapMemory>,
    next_id: u32,
    ids: ether_core::protocol::model::IdGen,
}

impl Rig {
    fn new() -> Self {
        let control = HeapMemory::new(4 << 20);
        let reports = HeapMemory::new(256 << 10);
        let shared = bridge::shared(control.clone(), reports.clone());
        let mut fs = MemFs::new();
        for i in 0..SAMPLES {
            let frames = 44_100 * SAMPLE_SECONDS;
            let hz = 110.0 * (i + 1) as f32;
            let tone: Vec<f32> = (0..frames)
                .map(|n| 0.3 * (std::f32::consts::TAU * hz * n as f32 / 44_100.0).sin())
                .collect();
            fs.write(
                &format!("library/s{i}.wav"),
                &wav(44_100, &[tone.clone(), tone]),
            )
            .unwrap();
        }
        let ctl = EtherController::with_config(
            WebBridge::new(shared),
            Clock,
            WebStore::new(fs.clone()),
            WebLibrary::new(fs),
            ControllerConfig {
                engine_sample_rate: 48_000,
                ..ControllerConfig::default()
            },
        );
        Self {
            ctl,
            engine: EngineHost::new(48_000, control, reports),
            next_id: 1,
            ids: ether_core::protocol::model::IdGen::new(0x3eb),
        }
    }

    fn id(&mut self) -> String {
        self.ids.next_ulid(NOW.load(Ordering::Relaxed)).to_string()
    }

    fn send(&mut self, domain: &str, command: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let msg: ClientMessage = serde_json::from_value(json!({
            "id": id, "gesture": null, "command": {"domain": domain, "command": command},
        }))
        .unwrap();
        let mut out = Vec::new();
        self.ctl.handle(msg, &mut out);
        match out.into_iter().rev().find_map(|m| match m {
            ServerMessage::Reply(r) if r.id == id => Some(r.result),
            _ => None,
        }) {
            Some(ReplyResult::Ok { value }) => Ok(serde_json::to_value(value).unwrap()),
            Some(ReplyResult::Err { error }) => Err(error.message),
            None => panic!("no reply"),
        }
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
        self.send(domain, command.clone())
            .unwrap_or_else(|e| panic!("{domain} {command}: {e}"))
    }

    /// One controller tick and its 16 ms of quanta; the quanta's render times and peaks.
    fn step(&mut self) -> (Vec<Duration>, f32) {
        NOW.fetch_add(16, Ordering::Relaxed);
        let mut out = Vec::new();
        self.ctl.tick(NOW.load(Ordering::Relaxed), &mut out);
        let mut times = Vec::with_capacity(QUANTA_PER_TICK);
        let mut peak = 0.0f32;
        for _ in 0..QUANTA_PER_TICK {
            let t = Instant::now();
            self.engine.render(RENDER_QUANTUM);
            times.push(t.elapsed());
            for ch in 0..2 {
                peak = self.engine.output(ch)[..RENDER_QUANTUM]
                    .iter()
                    .fold(peak, |m, s| m.max(s.abs()));
            }
        }
        (times, peak)
    }
}

fn wav(rate: u32, channels: &[Vec<f32>]) -> Vec<u8> {
    let n_ch = channels.len() as u16;
    let frames = channels[0].len();
    let data_len = (frames * n_ch as usize * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&n_ch.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * n_ch as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(n_ch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for ch in channels {
            b.extend_from_slice(&((ch[i].clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    b
}

/// A saved project: [`TRACKS`] MIDI tracks of [`DEVICES`], each with a busy clip, and
/// [`SAMPLES`] audio tracks.
fn big_project(rig: &mut Rig) -> String {
    let pid = rig
        .ids
        .next_project_id(NOW.load(Ordering::Relaxed))
        .to_string();
    rig.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Big"}),
    );
    for i in 0..SAMPLES {
        let t = rig.id();
        rig.ok(
            "Track",
            json!({"type": "Create", "id": t, "kind": "Audio", "parent": null}),
        );
        let media = rig.id();
        rig.ok(
            "Media",
            json!({"type": "Import", "id": media, "source": {"type": "Location",
                   "location": {"type": "Library", "id": "browser"}, "path": format!("s{i}.wav")}}),
        );
        let clip = rig.id();
        rig.ok(
            "Clip",
            json!({"type": "CreateAudio", "id": clip, "track": t, "start": 0.0, "media": media}),
        );
    }
    for _ in 0..TRACKS {
        let t = rig.id();
        rig.ok(
            "Track",
            json!({"type": "Create", "id": t, "kind": "Midi", "parent": null}),
        );
        for d in DEVICES {
            let id = rig.id();
            rig.ok(
                "Device",
                json!({"type": "Insert", "id": id, "track": t,
                       "device": {"type": "Builtin", "device": {"type": d}}}),
            );
        }
        let clip = rig.id();
        rig.ok(
            "Clip",
            json!({"type": "CreateMidi", "id": clip, "track": t, "start": 0.0,
                   "length": 16.0, "name": null}),
        );
        let notes: Vec<Value> = (0..32)
            .map(|i| {
                json!({"id": rig.id(), "pitch": 48 + (i % 12), "velocity": 0.8,
                       "start": i as f64 * 0.5, "duration": 0.4})
            })
            .collect();
        rig.ok("Note", json!({"type": "Add", "clip": clip, "notes": notes}));
    }
    rig.ok("Project", json!({"type": "Save", "autosave": false}));
    pid
}

#[derive(Debug)]
#[allow(dead_code)] // printed
struct Numbers {
    /// Quanta from the open until the Worklet applied the whole load (and play works).
    load_quanta: usize,
    /// Controller ticks of the load.
    load_ticks: usize,
    /// Of those, the ones that took longer than a quantum lasts, the longest and the sum.
    late_during_load: usize,
    worst_during_load: Duration,
    cpu_during_load: Duration,
    /// Loudest sample while loading.
    peak_during_load: f32,
    /// Ticks play was refused, and the loudest sample meanwhile.
    refused_ticks: usize,
    peak_while_refused: f32,
    /// Late quanta in the 2 s of playback that follow.
    late_after_load: usize,
    worst_after_load: Duration,
}

fn measure(rig: &mut Rig) -> Numbers {
    let pid = big_project(rig);
    while rig.ctl.media_pending() {
        rig.step();
    }
    let other = rig
        .ids
        .next_project_id(NOW.load(Ordering::Relaxed))
        .to_string();
    rig.ok(
        "Project",
        json!({"type": "Create", "id": other, "name": "Other"}),
    );
    for _ in 0..40 {
        rig.step();
    }
    rig.ok("Project", json!({"type": "Open", "id": pid}));
    let (mut quanta, mut late, mut worst, mut cpu, mut peak) =
        (0, 0, Duration::ZERO, Duration::ZERO, 0.0f32);
    let mut ticks = 0;
    let (mut refused_ticks, mut peak_refused) = (0, 0.0f32);
    let mut refused = false;
    let mut loading = true;
    while loading {
        let (times, p) = rig.step();
        ticks += 1;
        quanta += times.len();
        late += times.iter().filter(|t| **t > QUANTUM).count();
        worst = times.iter().copied().fold(worst, Duration::max);
        cpu += times.iter().sum::<Duration>();
        peak = peak.max(p);
        if refused {
            refused_ticks += 1;
            peak_refused = peak_refused.max(p);
        }
        // Play is pressed every tick until it is accepted (at once without the loading
        // state: the partial graphs then play while the backlog drains).
        let backlog = rig.engine.control_backlog() > 0;
        refused = rig.send("Transport", json!({"type": "Play"})).is_err();
        loading = refused || backlog || rig.ctl.media_pending();
        assert!(quanta < 200_000, "the load ends");
    }
    let (mut late_after, mut worst_after) = (0, Duration::ZERO);
    for _ in 0..125 {
        let (times, _) = rig.step();
        late_after += times.iter().filter(|t| **t > QUANTUM).count();
        worst_after = times.iter().copied().fold(worst_after, Duration::max);
    }
    Numbers {
        load_quanta: quanta,
        load_ticks: ticks,
        late_during_load: late,
        worst_during_load: worst,
        cpu_during_load: cpu,
        peak_during_load: peak,
        refused_ticks,
        peak_while_refused: peak_refused,
        late_after_load: late_after,
        worst_after_load: worst_after,
    }
}

#[test]
fn a_big_project_loads_silently_on_the_web_host() {
    let mut rig = Rig::new();
    let n = measure(&mut rig);
    eprintln!("web load: {n:?}");
    assert!(n.refused_ticks > 10, "play waits for the media: {n:?}");
    assert_eq!(n.peak_while_refused, 0.0, "silent while loading");
}

#[test]
#[ignore = "timing measurement; run by hand"]
fn repro_quantum_times_during_a_big_load() {
    let mut rig = Rig::new();
    let n = measure(&mut rig);
    eprintln!("web load: {n:?}");
}
