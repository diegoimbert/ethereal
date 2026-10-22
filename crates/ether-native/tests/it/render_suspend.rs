//! A render suspends the live engine (`ether_controller::rendering`, CONTRACTS.md §13.19):
//! the real controller, the native bridge and the engine on the paced `null` backend (real
//! time, with the worker pool), a project of CPU-burning plugins, and an audio export.
//!
//! - `live_plugins_rest_while_rendering`: no live plugin instance processes a block while
//!   the export renders (the offline instances do), and they run again once it is done.
//! - `render_speed` (ignored, a benchmark): the export's wall time with the live engine
//!   suspended and, emulating the engine before #245, kept running. Run in release:
//!
//! ```text
//! cargo test --release -p ether-native --test it render_suspend::render_speed -- --ignored --nocapture
//! taskset -c 0-3 cargo test --release ...   # a 4-core machine
//! ```
//!
//! Environment: `ETHER_BENCH_TRACKS` (default 16), `ETHER_BENCH_PLUGINS` per track (4),
//! `ETHER_BENCH_ITERS` multiply-adds per frame per plugin (300), `ETHER_BENCH_SECONDS` of
//! audio exported (10), `ETHER_BENCH_WORKERS` live worker threads (3).

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::ProjectStore;
use ether_controller::{Controller, EngineBridge, EtherController};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::node::{Device, Node, ProcessContext, ProcessStatus};
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo};
use ether_core::protocol::model::{IdGen, ParamId, PluginFormat};
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::protocol::{ClientMessage, Event, ReplyResult, ServerMessage};
use ether_native::audio::{AudioBackendKind, AudioOutput, AudioSettings};
use ether_native::plugins::{Instantiate, PluginCatalog, PluginHost};
use ether_native::rt::AudioShared;
use ether_native::workers::WorkerPool;
use ether_native::{DedicatedThread, NativeBridge, NativeServices};
use serde_json::{Value, json};

const BLOCK: usize = 256;
const RATE: u32 = 48_000;

/// One test at a time (the counters are global).
static SERIAL: Mutex<()> = Mutex::new(());
/// Blocks processed by live instances (activated at the live block size) and by offline
/// ones (the offline engine's block size).
static LIVE_BLOCKS: AtomicU64 = AtomicU64::new(0);
static OFFLINE_BLOCKS: AtomicU64 = AtomicU64::new(0);
/// Multiply-adds per frame of every instance.
static ITERS: AtomicU64 = AtomicU64::new(0);

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: None,
        device_type: DeviceTypeRef::Plugin {
            plugin_id: "burn".into(),
        },
        name: "Burn".into(),
        category: DeviceCategory::AudioEffect,
        params: Vec::new(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
        implemented: true,
    }
}

/// A fixed amount of floating-point work per frame (a dependent chain: its cost hardly
/// depends on the data), plus a quiet tone so nothing is silent.
struct BurnNode {
    offline: bool,
    state: f32,
    phase: u32,
}

impl Node for BurnNode {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let counter = if self.offline {
            &OFFLINE_BLOCKS
        } else {
            &LIVE_BLOCKS
        };
        counter.fetch_add(1, Ordering::Relaxed);
        let iters = ITERS.load(Ordering::Relaxed);
        audio.pass_through();
        let frames = audio.outputs.first().map_or(0, |o| o.len());
        let mut s = self.state;
        for i in 0..frames {
            for _ in 0..iters {
                s = s.mul_add(0.999_999, 1e-7);
            }
            let tone = ((self.phase.wrapping_add(i as u32) % 96) as f32 / 96.0 - 0.5) * 0.01;
            for out in audio.outputs.iter_mut() {
                out[i] += tone + s * 1e-9;
            }
        }
        self.phase = self.phase.wrapping_add(frames as u32);
        self.state = s;
        ProcessStatus::Continue
    }
}

impl Device for BurnNode {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }
    fn param(&self, _: ParamId) -> Option<f64> {
        None
    }
    fn set_param(&mut self, _: ParamId, _: f64) {}
}

impl PluginNode for BurnNode {
    fn is_faulted(&self) -> bool {
        false
    }
}

struct BurnPlugin;

impl PluginController for BurnPlugin {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }
    fn params(&mut self) -> Vec<ParamInfo> {
        Vec::new()
    }
    fn activate(&mut self, config: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
        Ok(Box::new(BurnNode {
            offline: config.max_block_size == ether_core::offline::OFFLINE_MAX_BLOCK,
            state: 0.5,
            phase: 0,
        }))
    }
    fn deactivate(&mut self, _: Box<dyn PluginNode>) {}
    fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
        Ok(Vec::new())
    }
    fn load_state(&mut self, _: &[u8]) -> Result<(), PluginError> {
        Ok(())
    }
    fn has_editor(&self) -> bool {
        false
    }
    fn open_editor(&mut self) -> Result<(), PluginError> {
        Err(PluginError::NoEditor)
    }
    fn close_editor(&mut self) {}
    fn poll(&mut self, _: &mut Vec<PluginNotification>) {}
}

fn instantiate() -> Instantiate {
    Arc::new(|_: PluginFormat, _: &Path, _: &str| {
        Ok(Box::new(BurnPlugin) as Box<dyn PluginController>)
    })
}

type Ctl = EtherController<NativeBridge, NativeServices, MemoryStore, MemoryLibrary>;

struct Rig {
    ctl: Ctl,
    output: Option<AudioOutput>,
    gc: ether_core::GarbageCollector,
    out: Vec<ServerMessage>,
    next_id: u32,
    ids: IdGen,
    /// Emulate the engine before #245: resume it after every controller call.
    never_suspend: bool,
    pid: Option<ether_core::protocol::model::ProjectId>,
}

impl Rig {
    /// `offline_workers`: threads of the export's render (with the engine suspended).
    fn new(never_suspend: bool, workers: usize, offline_workers: usize) -> Self {
        let mut parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: RATE,
            max_block_size: BLOCK,
            worker_threads: workers,
            ..Default::default()
        });
        if workers > 0 {
            parts.engine.set_executor(Box::new(WorkerPool::new(
                workers,
                Duration::from_micros(BLOCK as u64 * 1_000_000 / u64::from(RATE)),
            )));
        }
        let catalog = PluginCatalog::default();
        catalog.replace(vec![PluginDescriptor {
            sidechain_inputs: Default::default(),
            format: PluginFormat::Clap,
            id: "burn".into(),
            name: "Burn".into(),
            vendor: "T".into(),
            version: "1".into(),
            description: String::new(),
            features: vec![],
            category: DeviceCategory::AudioEffect,
            path: "/burn.clap".into(),
        }]);
        let shared = Arc::new(AudioShared::default());
        let output = AudioOutput::start(
            Box::new(parts.engine),
            &AudioSettings {
                backend: AudioBackendKind::Null,
                sample_rate: Some(RATE),
                max_block_size: BLOCK,
                buffer_size: Some(BLOCK as u32),
                ..Default::default()
            },
            shared.clone(),
        )
        .map_err(|(e, _)| e)
        .expect("null backend starts");
        let bridge = NativeBridge::new(
            parts.handle,
            PrepareConfig {
                sample_rate: RATE as f32,
                max_block_size: BLOCK,
                max_events_per_block: 256,
            },
            PluginHost::new(Arc::new(DedicatedThread::new())),
            catalog,
            instantiate(),
            shared,
        );
        let mut bridge = bridge;
        bridge.set_offline_workers(offline_workers);
        let ctl = EtherController::new(
            bridge,
            NativeServices::default(),
            MemoryStore::new(),
            MemoryLibrary::new(),
        );
        Self {
            ctl,
            output: Some(output),
            gc: parts.gc,
            out: Vec::new(),
            next_id: 1,
            ids: IdGen::new(0x5e7d),
            never_suspend,
            pid: None,
        }
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    fn id(&mut self) -> String {
        self.ids.next_ulid(Self::now()).to_string()
    }

    fn send(&mut self, domain: &str, command: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let msg: ClientMessage = serde_json::from_value(json!({
            "id": id, "gesture": null, "command": {"domain": domain, "command": command},
        }))
        .unwrap();
        self.ctl.handle(msg, &mut self.out);
        if self.never_suspend {
            let _ = self.ctl.bridge.set_suspended(false);
        }
        let reply = self
            .out
            .iter()
            .rev()
            .find_map(|m| match m {
                ServerMessage::Reply(r) if r.id == id => Some(r.result.clone()),
                _ => None,
            })
            .expect("a reply");
        match reply {
            ReplyResult::Ok { value } => Ok(serde_json::to_value(value).unwrap()),
            ReplyResult::Err { error } => Err(error.message),
        }
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
        self.send(domain, command.clone())
            .unwrap_or_else(|e| panic!("{domain} {command}: {e}"))
    }

    /// One controller tick, paced like the hosts (a tick every 16 ms, the tick's own time
    /// included).
    fn tick(&mut self) {
        let started = Instant::now();
        self.ctl.tick(Self::now(), &mut self.out);
        if self.never_suspend {
            let _ = self.ctl.bridge.set_suspended(false);
        }
        self.gc.collect();
        if let Some(rest) = Duration::from_millis(16).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }

    /// The export's files once it is done, from the events since the last call.
    fn export_ended(&mut self) -> Option<Vec<String>> {
        use ether_core::protocol::export::{ExportEvent, ExportResult};
        let ended = self.out.iter().find_map(|m| match m {
            ServerMessage::Event(Event::Export { event }) => match event {
                ExportEvent::Done { result, .. } => Some(match result {
                    ExportResult::Files { files } => files.clone(),
                    ExportResult::Download { .. } => Vec::new(),
                }),
                ExportEvent::Failed { message, .. } => panic!("export failed: {message}"),
                _ => None,
            },
            _ => None,
        });
        self.out.clear();
        ended
    }

    /// A project of `tracks` tracks with `plugins` plugins each, loaded and settled.
    fn project(&mut self, tracks: usize, plugins: usize) {
        let pid = self.ids.next_project_id(Self::now());
        self.pid = Some(pid);
        let pid = pid.to_string();
        self.ok(
            "Project",
            json!({"type": "Create", "id": pid, "name": "Heavy"}),
        );
        for _ in 0..tracks {
            let t = self.id();
            self.ok(
                "Track",
                json!({"type": "Create", "id": t, "kind": "Audio", "parent": null}),
            );
            for _ in 0..plugins {
                let id = self.id();
                self.ok(
                    "Device",
                    json!({"type": "Insert", "id": id, "track": t,
                           "device": {"type": "Plugin", "plugin_id": "burn"}}),
                );
            }
        }
        for _ in 0..30 {
            self.tick();
        }
    }

    /// Export `seconds` of the project (WAV, mix): the wall time it took and the file.
    fn export(&mut self, seconds: f64) -> (Duration, Vec<u8>) {
        let beats = seconds * 2.0; // 120 bpm
        let started = Instant::now();
        self.ok(
            "Export",
            json!({"type": "Render", "job": "bench", "request": {
                "range": {"type": "Custom", "start": 0.0, "end": beats},
                "format": {"container": "Wav", "bit_depth": "Int16", "sample_rate": null},
                "mode": {"type": "Mix"}, "normalize": false, "tail_seconds": 0.0, "name": null,
            }}),
        );
        self.out.clear();
        loop {
            self.tick();
            if let Some(files) = self.export_ended() {
                let took = started.elapsed();
                let bytes = self
                    .ctl
                    .store
                    .read(self.pid.expect("a project"), &files[0])
                    .expect("the exported file");
                return (took, bytes);
            }
            assert!(
                started.elapsed() < Duration::from_secs(600),
                "the export ends"
            );
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(o) = self.output.take() {
            let _ = o.stop();
        }
    }
}

#[test]
fn live_plugins_rest_while_rendering() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    ITERS.store(2, Ordering::Relaxed);
    let mut rig = Rig::new(false, 1, 1);
    rig.project(4, 2);
    let live = LIVE_BLOCKS.load(Ordering::Relaxed);
    assert!(live > 0, "the live plugins run");
    // A long render, so a few ticks see it running.
    rig.ok(
        "Export",
        json!({"type": "Render", "job": "j", "request": {
            "range": {"type": "Custom", "start": 0.0, "end": 240.0},
            "format": {"container": "Wav", "bit_depth": "Int16", "sample_rate": null},
            "mode": {"type": "Mix"}, "normalize": false, "tail_seconds": 0.0, "name": null,
        }}),
    );
    // A callback in flight when the render began may still run the graph.
    std::thread::sleep(Duration::from_millis(30));
    let live = LIVE_BLOCKS.load(Ordering::Relaxed);
    let offline = OFFLINE_BLOCKS.load(Ordering::Relaxed);
    for _ in 0..10 {
        rig.tick();
        assert_eq!(rig.export_ended(), None);
    }
    assert_eq!(LIVE_BLOCKS.load(Ordering::Relaxed), live, "suspended");
    assert!(
        OFFLINE_BLOCKS.load(Ordering::Relaxed) > offline,
        "rendering"
    );
    rig.ok("Export", json!({"type": "Cancel", "job": "j"}));
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        LIVE_BLOCKS.load(Ordering::Relaxed) > live,
        "resumed after the render"
    );
}

#[test]
fn renders_on_workers_equal_renders_on_one_thread() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    ITERS.store(2, Ordering::Relaxed);
    let file = |offline_workers| {
        let mut rig = Rig::new(false, 1, offline_workers);
        rig.project(6, 2);
        rig.export(2.0).1
    };
    let one = file(0);
    assert!(one.len() > 2 * 48_000 * 4, "a 2 s file");
    assert!(one == file(3), "bit-identical with 3 workers");
}

#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn render_speed() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tracks = env("ETHER_BENCH_TRACKS", 16) as usize;
    let plugins = env("ETHER_BENCH_PLUGINS", 4) as usize;
    let seconds = env("ETHER_BENCH_SECONDS", 10) as f64;
    let workers = env("ETHER_BENCH_WORKERS", 3) as usize;
    ITERS.store(env("ETHER_BENCH_ITERS", 300), Ordering::Relaxed);
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    eprintln!(
        "{tracks} tracks x {plugins} plugins, {} iters/frame, {seconds} s exported, \
         {workers} live workers, {cores} cores available",
        ITERS.load(Ordering::Relaxed)
    );
    // Before #245 (the live engine runs, the render has one thread), suspended with one
    // thread, suspended with the live engine's workers; twice each.
    let modes = [
        ("before: live engine running, 1 render thread ", true, 0),
        ("live engine suspended, 1 render thread       ", false, 0),
        (
            "live engine suspended, render on its workers ",
            false,
            workers,
        ),
    ];
    for (name, never_suspend, offline) in modes.iter().chain(&modes) {
        let mut rig = Rig::new(*never_suspend, workers, *offline);
        rig.project(tracks, plugins);
        let live = LIVE_BLOCKS.load(Ordering::Relaxed);
        let (took, _) = rig.export(seconds);
        let live = LIVE_BLOCKS.load(Ordering::Relaxed) - live;
        eprintln!(
            "{name}: {:.2} s ({:.2}x real time), {live} live plugin blocks meanwhile",
            took.as_secs_f64(),
            seconds / took.as_secs_f64(),
        );
    }
}
