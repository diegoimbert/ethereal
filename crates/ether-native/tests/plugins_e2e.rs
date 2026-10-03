//! Plugins end to end with a real CLAP plugin (`ether-clap`'s `ether_test_plugin` fixture,
//! wrapped into a bundle) and the real sandbox helper:
//!
//! - controller + native bridge + engine rendered offline (sample-exact checks): insert,
//!   param edit reaching the plugin, state round trip through save/load (the state blob is
//!   authoritative, params are mirrored back into the document), sandbox toggle keeping
//!   the state, PDC alignment of a sandboxed plugin (+1 block) against an in-process one
//!   and a dry track, helper crash → `Crashed` event + latency-aligned bypass → `Reload`;
//! - the desktop host (null audio backend): the host-handled `List`, `OpenEditor` and
//!   `CloseEditor` commands and the editor-closed event.
//!
//! The fixture reports 64 samples of latency but doesn't actually delay its output; the
//! assertions account for that explicitly.
#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ether_clap::testing;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{Controller, EtherController};
use ether_core::protocol::model::{Base64Bytes, IdGen, PluginFormat};
use ether_core::protocol::plugins::{PluginDescriptor, PluginEvent};
use ether_core::protocol::{ClientMessage, Event, ReplyResult, ServerMessage};
use ether_core::{Engine, GarbageCollector, PrepareConfig};
use ether_native::plugins::{PluginCatalog, PluginHost};
use ether_native::rt::AudioShared;
use ether_native::test_util::TempDir;
use ether_native::{DedicatedThread, NativeBridge, NativeServices};
use serde_json::{Value, json};

const PLUGIN_ID: &str = "dev.ethereal.test-plugin";
/// Latency the fixture reports (without applying it).
const FIXTURE_LATENCY: usize = 64;
const BLOCK: usize = 256;
const RATE: u32 = 48_000;
/// Samples per beat at 120 BPM.
const BEAT: usize = RATE as usize / 2;
/// Position of the impulse inside the test sample.
const IMPULSE_AT: usize = 100;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("native-plugins-e2e");
            testing::make_bundle(&dir, "EtherNativeTest")
        })
        .clone()
}

/// The `ether-sandbox-helper` binary: next to this test's target dir when the workspace
/// was built (`cargo test --workspace`), else built into a separate target dir.
fn helper() -> PathBuf {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER
        .get_or_init(|| {
            let name = format!("ether-sandbox-helper{}", std::env::consts::EXE_SUFFIX);
            let exe = std::env::current_exe().expect("current exe");
            // target/<profile>/deps/<test>
            if let Some(p) = exe
                .ancestors()
                .skip(1)
                .take(2)
                .map(|d| d.join(&name))
                .find(|p| p.is_file())
            {
                return p;
            }
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let target = root.join("target/ether-native-helper");
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let status = std::process::Command::new(cargo)
                .args([
                    "build",
                    "-q",
                    "-p",
                    "ether-sandbox",
                    "--bin",
                    "ether-sandbox-helper",
                ])
                .arg("--manifest-path")
                .arg(root.join("Cargo.toml"))
                .arg("--target-dir")
                .arg(&target)
                .status()
                .expect("run cargo build for the sandbox helper");
            assert!(status.success(), "building ether-sandbox-helper failed");
            target.join("debug").join(name)
        })
        .clone()
}

fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        sidechain_inputs: Default::default(),
        format: PluginFormat::Clap,
        id: PLUGIN_ID.into(),
        name: "Ether Test Plugin".into(),
        vendor: "Ethereal".into(),
        version: "1".into(),
        description: String::new(),
        features: vec!["audio-effect".into()],
        category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
        path: bundle().display().to_string(),
    }
}

/// Mono 48 kHz WAV: silence with one full-scale impulse at [`IMPULSE_AT`].
fn impulse_wav() -> Vec<u8> {
    let mut s = vec![0.0f32; 4_800];
    s[IMPULSE_AT] = 1.0;
    common::wav(RATE, &[s])
}

type Ctl = EtherController<NativeBridge, NativeServices, MemoryStore, MemoryLibrary>;

/// Controller + native bridge + engine, rendered offline on the test thread.
struct Rig {
    ctl: Ctl,
    engine: Engine,
    gc: GarbageCollector,
    out: Vec<ServerMessage>,
    next_id: u32,
    ids: IdGen,
    now: u64,
}

impl Rig {
    fn new() -> Self {
        ether_native::sandbox::set_helper_path(Some(helper()));
        // Offline render: wait for every sandboxed block, so a helper starved of CPU on a
        // loaded machine can't turn blocks into silence (underruns) under the assertions.
        ether_native::sandbox::set_wait_budget(Some(std::time::Duration::from_secs(10)));
        let parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: RATE,
            max_block_size: BLOCK,
            ..Default::default()
        });
        let catalog = PluginCatalog::default();
        catalog.replace(vec![descriptor()]);
        let bridge = NativeBridge::new(
            parts.handle,
            PrepareConfig {
                sample_rate: RATE as f32,
                max_block_size: BLOCK,
                max_events_per_block: 256,
            },
            PluginHost::new(Arc::new(DedicatedThread::new())),
            catalog,
            ether_native::plugins::instantiate_any(),
            Arc::new(AudioShared::default()),
        );
        let mut library = MemoryLibrary::new();
        library.add_root("lib", "Library");
        library.add_file("lib", "impulse.wav", impulse_wav());
        let ctl = EtherController::new(bridge, NativeServices, MemoryStore::new(), library);
        Self {
            ctl,
            engine: parts.engine,
            gc: parts.gc,
            out: Vec::new(),
            next_id: 1,
            ids: IdGen::new(0x9_1u64),
            now: 1_000,
        }
    }

    fn id(&mut self) -> String {
        self.now += 1;
        self.ids.next_ulid(self.now).to_string()
    }

    fn send(&mut self, domain: &str, command: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let msg: ClientMessage = serde_json::from_value(json!({
            "id": id, "gesture": null, "command": {"domain": domain, "command": command},
        }))
        .unwrap_or_else(|e| panic!("bad {domain} command {command}: {e}"));
        self.ctl.handle(msg, &mut self.out);
        let reply = self
            .out
            .iter()
            .rev()
            .find_map(|m| match m {
                ServerMessage::Reply(r) if r.id == id => Some(r.result.clone()),
                _ => None,
            })
            .expect("one reply per message");
        match reply {
            ReplyResult::Ok { value } => Ok(serde_json::to_value(value).unwrap()),
            ReplyResult::Err { error } => Err(error.message),
        }
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
        self.send(domain, command.clone())
            .unwrap_or_else(|e| panic!("{domain} {command} failed: {e}"))
    }

    fn tick(&mut self) {
        self.now += 16;
        self.ctl.tick(self.now, &mut self.out);
        self.gc.collect();
    }

    /// Render `frames` (left channel), ticking the controller every block.
    fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut left = Vec::with_capacity(frames);
        let mut l = vec![0.0f32; BLOCK];
        let mut r = vec![0.0f32; BLOCK];
        while left.len() < frames {
            self.tick();
            {
                let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
                self.engine.process(&[], &mut outs, BLOCK);
            }
            left.extend_from_slice(&l);
        }
        left.truncate(frames);
        left
    }

    /// Tick until the controller is idle (media decoded, graph published).
    fn settle(&mut self) {
        for _ in 0..500 {
            self.render(BLOCK);
            if !self.ctl.media_pending() {
                break;
            }
        }
        self.render(4 * BLOCK);
    }

    fn project(&mut self) -> Value {
        self.ok("Project", json!({"type": "Get"}))["project"].clone()
    }

    fn device(&mut self, id: &str) -> Value {
        self.project()["devices"][id].clone()
    }

    fn events(&self) -> impl Iterator<Item = &Event> {
        self.out.iter().filter_map(|m| match m {
            ServerMessage::Event(e) => Some(e),
            _ => None,
        })
    }

    /// Play from the start and return the sample indices (relative to the play command)
    /// of every impulse above `threshold` in the first `frames` samples.
    fn play_onsets(&mut self, frames: usize, threshold: f32) -> Vec<(usize, f32)> {
        self.ok("Transport", json!({"type": "Stop"}));
        self.ok("Transport", json!({"type": "Locate", "position": 0.0}));
        // Flush anything still delayed in PDC lines / plugin FIFOs.
        self.render(8 * BLOCK);
        self.ok("Transport", json!({"type": "Play"}));
        let out = self.render(frames);
        self.ok("Transport", json!({"type": "Stop"}));
        out.iter()
            .enumerate()
            .filter(|(_, x)| x.abs() > threshold)
            .map(|(i, x)| (i, *x))
            .collect()
    }
}

fn track(rig: &mut Rig, name: &str) -> String {
    let id = rig.id();
    rig.ok(
        "Track",
        json!({"type": "Create", "id": id, "kind": "Audio", "name": name, "color": null,
               "parent": null, "before": null}),
    );
    id
}

fn clip(rig: &mut Rig, track: &str, media: &str, beat: f64) {
    let id = rig.id();
    rig.ok(
        "Clip",
        json!({"type": "CreateAudio", "id": id, "track": track, "start": beat, "media": media}),
    );
}

fn insert_plugin(rig: &mut Rig, track: &str, sandboxed: bool) -> String {
    let id = rig.id();
    rig.ok(
        "Device",
        json!({"type": "Insert", "id": id, "track": track,
               "device": {"type": "Plugin", "plugin_id": PLUGIN_ID, "sandboxed": sandboxed},
               "before": null}),
    );
    id
}

/// Gain stored in the fixture's state blob (f64 LE gain, f64 LE mode).
fn state_gain(device: &Value) -> Option<f64> {
    let state = &device["kind"]["plugin"]["state"];
    let bytes: Base64Bytes = serde_json::from_value(state.clone()).ok()?;
    Some(f64::from_le_bytes(bytes.0.get(..8)?.try_into().ok()?))
}

fn saved_file(rig: &Rig, pid: &str) -> Value {
    let pid = serde_json::from_value(json!(pid)).unwrap();
    let bytes = rig
        .ctl
        .store
        .file(pid, "project.ether")
        .expect("project saved");
    serde_json::from_slice(bytes).unwrap()
}

/// Onsets of the three test tracks, in clip order (A at beat 0, B at beat 1, C at beat 2).
fn onsets(rig: &mut Rig) -> Vec<usize> {
    let hits = rig.play_onsets(3 * BEAT + 8 * BLOCK, 0.05);
    let starts: Vec<usize> = hits.iter().map(|(i, _)| *i).collect();
    // One impulse per clip.
    assert_eq!(starts.len(), 3, "expected 3 impulses, got {hits:?}");
    starts
}

#[test]
fn plugins_in_the_device_chain() {
    let mut rig = Rig::new();
    let pid = rig.ids.next_project_id(1).to_string();
    rig.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Plugins"}),
    );
    rig.ok("Transport", json!({"type": "SetTempo", "bpm": 120.0}));
    let media = rig.id();
    rig.ok(
        "Media",
        json!({"type": "Import", "id": media, "source": {"type": "Location",
               "location": {"type": "Library", "id": "lib"}, "path": "impulse.wav"}}),
    );

    // A: sandboxed plugin; B: the same plugin in-process; C: dry. One impulse each, a beat
    // apart.
    let a = track(&mut rig, "Sandboxed");
    let b = track(&mut rig, "In-process");
    let c = track(&mut rig, "Dry");
    clip(&mut rig, &a, &media, 0.0);
    clip(&mut rig, &b, &media, 1.0);
    clip(&mut rig, &c, &media, 2.0);
    let pa = insert_plugin(&mut rig, &a, true);
    let pb = insert_plugin(&mut rig, &b, false);
    rig.settle();

    // Insert: named after the plugin, params mirrored from the live instance.
    let dev = rig.device(&pa);
    assert_eq!(dev["name"], "Ethereal Test Plugin");
    assert_eq!(dev["kind"]["plugin"]["sandboxed"], true);
    assert_eq!(dev["params"]["1"], 1.0, "gain mirrored: {dev}");
    let helper_pid = ether_native::sandbox::last_helper_pid().expect("a helper was spawned");

    // --- PDC: the sandboxed track (+1 block of real delay) lines up with the in-process
    // one; the dry track is compensated for the full reported latency (block + 64), which
    // the fixture doesn't actually apply, hence exactly +64.
    let t = onsets(&mut rig);
    assert_eq!(t[1] - t[0], BEAT, "sandboxed vs in-process: {t:?}");
    assert_eq!(
        t[2] - t[1],
        BEAT + FIXTURE_LATENCY,
        "dry vs plugin tracks: {t:?}"
    );

    // --- A param edit reaches the plugin; its state is saved with the project.
    rig.ok(
        "Device",
        json!({"type": "SetParam", "device": pa, "param": 1, "value": 0.5}),
    );
    rig.render(4 * BLOCK);
    let hits = rig.play_onsets(3 * BEAT + 8 * BLOCK, 0.05);
    let amp = |hits: &[(usize, f32)], i: usize| hits[i].1.abs();
    assert!(
        (amp(&hits, 0) / amp(&hits, 1) - 0.5).abs() < 0.01,
        "gain 0.5 on the sandboxed plugin: {hits:?}"
    );
    rig.ok("Project", json!({"type": "Save"}));
    let saved = saved_file(&rig, &pid);
    assert_eq!(state_gain(&saved["project"]["devices"][&pa]), Some(0.5));

    // --- Sandbox toggle: re-instantiated in-process with its state (and back).
    rig.ok(
        "Plugin",
        json!({"type": "SetSandboxed", "device": pa, "sandboxed": false}),
    );
    rig.settle();
    let dev = rig.device(&pa);
    assert_eq!(dev["kind"]["plugin"]["sandboxed"], false);
    assert_eq!(state_gain(&dev), Some(0.5));
    assert_eq!(dev["params"]["1"], 0.5);
    let t = onsets(&mut rig);
    assert_eq!(t[1] - t[0], BEAT, "both in-process now: {t:?}");
    rig.ok(
        "Plugin",
        json!({"type": "SetSandboxed", "device": pa, "sandboxed": true}),
    );
    rig.settle();
    let hits = rig.play_onsets(3 * BEAT + 8 * BLOCK, 0.05);
    assert!(
        (amp(&hits, 0) / amp(&hits, 1) - 0.5).abs() < 0.01,
        "state survives the round trip through the sandbox: {hits:?}"
    );
    let new_pid = ether_native::sandbox::last_helper_pid().unwrap();
    assert_ne!(new_pid, helper_pid, "a new helper runs the instance");

    // --- Crash: the helper dies → Crashed event, the device is bypassed (dry signal,
    // delayed by its reported latency so it stays aligned with the dry track).
    let status = std::process::Command::new("kill")
        .args(["-9", &new_pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let mut crashed = false;
    for _ in 0..200 {
        rig.render(BLOCK);
        crashed = rig.events().any(|e| {
            matches!(e, Event::Plugin { event: PluginEvent::Crashed { device, .. } }
                if device.to_string() == pa)
        });
        if crashed {
            break;
        }
    }
    assert!(crashed, "PluginEvent::Crashed for the sandboxed device");
    let hits = rig.play_onsets(3 * BEAT + 8 * BLOCK, 0.05);
    assert_eq!(hits.len(), 3, "bypassed, not silent: {hits:?}");
    assert!(
        (amp(&hits, 0) - amp(&hits, 2)).abs() < 0.01,
        "bypass = dry signal: {hits:?}"
    );
    assert_eq!(
        hits[2].0 - hits[0].0,
        2 * BEAT,
        "bypass is aligned with the dry track: {hits:?}"
    );

    // --- Reload: a fresh helper from the last saved state.
    rig.out.clear();
    rig.ok("Plugin", json!({"type": "Reload", "device": pa}));
    rig.settle();
    assert_ne!(ether_native::sandbox::last_helper_pid().unwrap(), new_pid);
    let t = onsets(&mut rig);
    assert_eq!(t[1] - t[0], BEAT, "sandboxed again after reload: {t:?}");

    // --- Save/load: the state blob is authoritative; the document mirror is refreshed
    // from it on load.
    rig.ok("Project", json!({"type": "Save"}));
    let mut file = saved_file(&rig, &pid);
    assert_eq!(state_gain(&file["project"]["devices"][&pa]), Some(0.5));
    file["project"]["devices"][&pa]["params"]["1"] = json!(1.7);
    let id = serde_json::from_value(json!(pid)).unwrap();
    ether_controller::store::ProjectStore::save(
        &mut rig.ctl.store,
        id,
        &serde_json::to_string(&file).unwrap(),
    )
    .unwrap();
    rig.ok("Project", json!({"type": "Open", "id": pid}));
    rig.settle();
    let dev = rig.device(&pa);
    assert_eq!(dev["kind"]["plugin"]["sandboxed"], true);
    assert_eq!(
        dev["params"]["1"], 0.5,
        "params mirrored from the restored state"
    );
    let hits = rig.play_onsets(3 * BEAT + 8 * BLOCK, 0.05);
    assert!(
        (amp(&hits, 0) / amp(&hits, 1) - 0.5).abs() < 0.01,
        "restored state is audible: {hits:?}"
    );
    let _ = pb;
}

#[test]
fn host_lists_plugins_and_opens_editors() {
    use common::{Client, Paths};
    let tmp = TempDir::new("plugins-host");
    let db = tmp.path().join("plugin-db");
    std::fs::create_dir_all(&db).unwrap();
    std::fs::write(
        db.join("plugins.json"),
        serde_json::to_string(&vec![descriptor()]).unwrap(),
    )
    .unwrap();
    let mut c = Client::start(&Paths::new(tmp.path()));

    let listed = c.ok("Plugin", json!({"type": "List"}));
    assert_eq!(listed["plugins"][0]["id"], PLUGIN_ID);

    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Editors"}),
    );
    let t = c.id();
    c.ok(
        "Track",
        json!({"type": "Create", "id": t, "kind": "Audio", "name": "FX", "color": null,
               "parent": null, "before": null}),
    );
    let d = c.id();
    c.ok(
        "Device",
        json!({"type": "Insert", "id": d, "track": t,
               "device": {"type": "Plugin", "plugin_id": PLUGIN_ID, "sandboxed": false},
               "before": null}),
    );
    // The fixture's (headless, floating) editor reports itself closed after 3 timer ticks.
    c.ok("Plugin", json!({"type": "OpenEditor", "device": d}));
    c.wait(
        |r| {
            r.events
                .iter()
                .find(|e| {
                    matches!(e, Event::Plugin { event: PluginEvent::EditorClosed { device } }
                    if device.to_string() == d)
                })
                .map(|_| ())
        },
        "EditorClosed",
    );
    c.ok("Plugin", json!({"type": "OpenEditor", "device": d}));
    c.ok("Plugin", json!({"type": "CloseEditor", "device": d}));
    // Unknown devices fail cleanly.
    let other = c.id();
    let id = c.post(
        "Plugin",
        json!({"type": "OpenEditor", "device": other}),
        None,
    );
    assert!(c.reply(id).is_err());
    c.quit();
}
