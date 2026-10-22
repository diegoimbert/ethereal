//! A big project loading while the audio thread runs (the real controller, the native
//! bridge and the engine on the paced `null` backend, in real time): late callbacks
//! (`AudioShared::overruns`, a callback longer than its buffer) during and after the load.
//!
//! The plugins model what makes a real load stall the audio: each instance takes 25 ms to
//! instantiate on the main thread, holding a lock its library shares with the audio
//! thread (as plugin frameworks' process-wide singletons do), and its first block is slow
//! (lazy setup). Before the engine was suspended during loads, every partial graph ran
//! while the next plugins instantiated: callbacks waited on that lock for tens of ms, over
//! and over. Suspended, no plugin runs before the last is in, and their first blocks run
//! while the load primes, before the transport is available.
//!
//! `cargo test -p ether-native --test it project_load_xruns -- --ignored --nocapture`
//! prints the numbers with and without the suspension.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{Controller, EngineBridge, EtherController};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::node::{Device, Node, ProcessContext, ProcessStatus};
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo};
use ether_core::protocol::model::{IdGen, ParamId, PluginFormat};
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::protocol::{ClientMessage, ReplyResult, ServerMessage};
use ether_native::audio::{AudioBackendKind, AudioOutput, AudioSettings};
use ether_native::plugins::{Instantiate, PluginCatalog, PluginHost};
use ether_native::rt::AudioShared;
use ether_native::{DedicatedThread, NativeBridge, NativeServices};
use serde_json::{Value, json};

const TRACKS: usize = 8;
const PLUGINS_PER_TRACK: usize = 6;
const BLOCK: usize = 256;
const RATE: u32 = 48_000;
const INSTANTIATE: Duration = Duration::from_millis(25);
const FIRST_BLOCK: Duration = Duration::from_millis(3);
const BLOCK_COST: Duration = Duration::from_micros(15);

/// One test at a time uses the fake plugin library (its lock and counters are global).
static SERIAL: Mutex<()> = Mutex::new(());
/// The plugin library's process-wide lock.
static LIBRARY: Mutex<()> = Mutex::new(());
/// Blocks processed by the plugins.
static PROCESSED: AtomicU32 = AtomicU32::new(0);

fn spin(d: Duration) {
    let t = Instant::now();
    while t.elapsed() < d {
        std::hint::spin_loop();
    }
}

fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: None,
        device_type: DeviceTypeRef::Plugin {
            plugin_id: "slow".into(),
        },
        name: "Slow".into(),
        category: DeviceCategory::AudioEffect,
        params: Vec::new(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
        implemented: true,
    }
}

struct SlowNode {
    first: bool,
}

impl Node for SlowNode {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let _library = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
        PROCESSED.fetch_add(1, Ordering::Relaxed);
        if std::mem::take(&mut self.first) {
            spin(FIRST_BLOCK);
        }
        spin(BLOCK_COST);
        audio.pass_through();
        ProcessStatus::Continue
    }
}

impl Device for SlowNode {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }
    fn param(&self, _: ParamId) -> Option<f64> {
        None
    }
    fn set_param(&mut self, _: ParamId, _: f64) {}
}

impl PluginNode for SlowNode {
    fn is_faulted(&self) -> bool {
        false
    }
}

struct SlowPlugin;

impl PluginController for SlowPlugin {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }
    fn params(&mut self) -> Vec<ParamInfo> {
        Vec::new()
    }
    fn activate(&mut self, _: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
        Ok(Box::new(SlowNode { first: true }))
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
        let _library = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
        std::thread::sleep(INSTANTIATE);
        Ok(Box::new(SlowPlugin) as Box<dyn PluginController>)
    })
}

type Ctl = EtherController<NativeBridge, NativeServices, MemoryStore, MemoryLibrary>;

struct Rig {
    ctl: Ctl,
    shared: Arc<AudioShared>,
    output: Option<AudioOutput>,
    gc: ether_core::GarbageCollector,
    out: Vec<ServerMessage>,
    next_id: u32,
    ids: IdGen,
    /// Emulate the engine before suspended loads: resume it after every controller call.
    never_suspend: bool,
}

impl Rig {
    fn new(never_suspend: bool) -> Self {
        let parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: RATE,
            max_block_size: BLOCK,
            worker_threads: 0,
            ..Default::default()
        });
        let catalog = PluginCatalog::default();
        catalog.replace(vec![PluginDescriptor {
            sidechain_inputs: Default::default(),
            format: PluginFormat::Clap,
            id: "slow".into(),
            name: "Slow".into(),
            vendor: "T".into(),
            version: "1".into(),
            description: String::new(),
            features: vec![],
            category: DeviceCategory::AudioEffect,
            path: "/slow.clap".into(),
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
            shared.clone(),
        );
        let ctl = EtherController::new(
            bridge,
            NativeServices::default(),
            MemoryStore::new(),
            MemoryLibrary::new(),
        );
        Self {
            ctl,
            shared,
            output: Some(output),
            gc: parts.gc,
            out: Vec::new(),
            next_id: 1,
            ids: IdGen::new(0x10ad),
            never_suspend,
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
        self.out.clear();
        match reply {
            ReplyResult::Ok { value } => Ok(serde_json::to_value(value).unwrap()),
            ReplyResult::Err { error } => Err(error.message),
        }
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
        self.send(domain, command.clone())
            .unwrap_or_else(|e| panic!("{domain} {command}: {e}"))
    }

    /// One controller tick, then wait for the next (16 ms, like the hosts).
    fn tick(&mut self) {
        self.ctl.tick(Self::now(), &mut self.out);
        if self.never_suspend {
            let _ = self.ctl.bridge.set_suspended(false);
        }
        self.out.clear();
        self.gc.collect();
        std::thread::sleep(Duration::from_millis(16));
    }

    fn overruns(&self) -> u32 {
        self.shared.overruns.load(Ordering::Relaxed)
    }

    fn plugins_loaded(&self) -> bool {
        self.ctl
            .transport_state()
            .and_then(|s| s.loading)
            .is_none_or(|l| l.plugins_loaded == l.plugins_total)
    }

    fn loading(&self) -> bool {
        self.ctl.loading.active()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(o) = self.output.take() {
            let _ = o.stop();
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)] // printed
struct Numbers {
    /// Late callbacks from the open until the last plugin is in.
    during_load: u32,
    /// Plugin blocks processed in that time.
    processed_during_load: u32,
    /// Late callbacks from then until the transport is available (the priming).
    priming: u32,
    /// Late callbacks in the 2 s of playback that follow.
    after_load: u32,
    /// Play refused while loading.
    play_refused: bool,
    load_ms: u128,
}

fn measure(never_suspend: bool) -> Numbers {
    let mut rig = Rig::new(never_suspend);
    let pid = rig.ids.next_project_id(Rig::now()).to_string();
    rig.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Big"}),
    );
    let mut tracks = Vec::new();
    for _ in 0..TRACKS {
        let id = rig.id();
        rig.ok(
            "Track",
            json!({"type": "Create", "id": id, "kind": "Audio", "parent": null}),
        );
        tracks.push(id);
    }
    for t in &tracks {
        for _ in 0..PLUGINS_PER_TRACK {
            let id = rig.id();
            rig.ok(
                "Device",
                json!({"type": "Insert", "id": id, "track": t,
                       "device": {"type": "Plugin", "plugin_id": "slow"}}),
            );
        }
    }
    rig.ok("Project", json!({"type": "Save", "autosave": false}));
    // Settle, then reopen the saved project.
    for _ in 0..20 {
        rig.tick();
    }
    let started = Instant::now();
    let base = rig.overruns();
    rig.ok("Project", json!({"type": "Open", "id": pid}));
    let play_refused = rig.send("Transport", json!({"type": "Play"})).is_err();
    // A callback in flight when the load began may still run the old project's graph.
    std::thread::sleep(Duration::from_millis(10));
    let processed = PROCESSED.load(Ordering::Relaxed);
    let mut processed_during_load = 0;
    while !rig.plugins_loaded() {
        processed_during_load = PROCESSED.load(Ordering::Relaxed) - processed;
        rig.tick();
        assert!(started.elapsed() < Duration::from_secs(30), "the load ends");
    }
    let during_load = rig.overruns() - base;
    let base = rig.overruns();
    while rig.loading() {
        rig.tick();
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the priming ends"
        );
    }
    let load_ms = started.elapsed().as_millis();
    let priming = rig.overruns() - base;
    let base = rig.overruns();
    rig.ok("Transport", json!({"type": "Play"}));
    for _ in 0..125 {
        rig.tick();
    }
    rig.ok("Transport", json!({"type": "Stop"}));
    Numbers {
        during_load,
        processed_during_load,
        priming,
        after_load: rig.overruns() - base,
        play_refused,
        load_ms,
    }
}

#[test]
fn a_big_project_loads_without_late_callbacks() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let n = measure(false);
    eprintln!("suspended load: {n:?}");
    assert!(n.play_refused, "play is refused while the project loads");
    assert_eq!(
        n.processed_during_load, 0,
        "no plugin runs before the last one is in"
    );
    // A suspended callback does nothing: only a preempted thread could be late.
    assert!(n.during_load <= 1, "{n:?}");
    // The first blocks' cost is paid while priming; playback after it is steady. The null
    // backend's thread runs at normal priority, so a shared runner can still preempt it past
    // a buffer now and then: a few of the ~375 callbacks may be late (nextest runs this test
    // alone, `.config/nextest.toml`; with the suite beside it, 12-15 were).
    assert!(n.after_load <= 3, "{n:?}");
}

/// The repro: the same load with the engine never suspended (as before), then suspended.
#[test]
#[ignore = "timing measurement; run by hand"]
fn repro_late_callbacks_with_and_without_the_suspension() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = measure(true);
    let after = measure(false);
    eprintln!("never suspended: {before:?}");
    eprintln!("suspended:       {after:?}");
}
