//! Plugin sidechain end to end (`plugin-sidechain`, CONTRACTS §12.14): the CLAP and VST3
//! fixtures (each adds its aux/sidechain input to its output) through the real scanner, the
//! real controller + native bridge + engine (rendered offline on the test thread), in
//! process and sandboxed, plus an offline export.
//!
//! Project: a "Src" track plays an impulse with its output routed nowhere, so it is only
//! heard through its sidechain tap; an empty "FX" track hosts the plugin. With Src as the
//! plugin's sidechain source the impulse comes out of FX; without a source, silence.
#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::ProjectStore;
use ether_controller::{Controller, EtherController};
use ether_core::protocol::export::{ExportEvent, ExportResult};
use ether_core::protocol::model::{IdGen, PluginFormat, ProjectId};
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::protocol::{ClientMessage, Event, ReplyResult, ServerMessage};
use ether_core::{Engine, GarbageCollector, PrepareConfig};
use ether_native::plugins::{PluginCatalog, PluginHost};
use ether_native::rt::AudioShared;
use ether_native::{DedicatedThread, NativeBridge, NativeServices};
use ether_plugin_host::ScanRunner;
use serde_json::{Value, json};

const BLOCK: usize = 256;
const RATE: u32 = 48_000;
/// Samples per beat at 120 BPM.
const BEAT: usize = RATE as usize / 2;
const IMPULSE_AT: usize = 100;
const CLAP_ID: &str = "dev.ethereal.test-plugin";

/// A binary of this workspace: next to this test's target dir when the workspace was built
/// (`cargo test --workspace`), else built into a separate target dir.
fn workspace_bin(package: &str, name: &str) -> PathBuf {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().expect("current exe");
    if let Some(p) = exe
        .ancestors()
        .skip(1)
        .take(2)
        .map(|d| d.join(&file))
        .find(|p| p.is_file())
    {
        return p;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target/ether-native-helper");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args(["build", "-q", "-p", package, "--bin", name])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()
        .unwrap_or_else(|e| panic!("run cargo build for {name}: {e}"));
    assert!(status.success(), "building {name} failed");
    target.join("debug").join(file)
}

fn helper() -> PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| workspace_bin("ether-sandbox", "ether-sandbox-helper"))
        .clone()
}

/// Both fixtures scanned through the real scanner, once per test process.
fn scanned() -> Vec<PluginDescriptor> {
    static S: OnceLock<Vec<PluginDescriptor>> = OnceLock::new();
    S.get_or_init(|| {
        let dir = ether_clap::testing::temp_dir("plugin-sidechain-e2e");
        ether_clap::testing::make_bundle(&dir, "EtherSidechainClap");
        ether_vst3::testing::make_bundle(&dir, "EtherSidechainVst3");
        let targets = ether_native::plugins::formats().discover(Some(&[dir]));
        assert_eq!(targets.len(), 2, "{targets:?}");
        let runner = ScanRunner::new(workspace_bin(
            "ether-plugin-scanner",
            "ether-plugin-scanner",
        ));
        let report = runner.scan_targets(&targets, |_, _, _| {});
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        report.plugins
    })
    .clone()
}

fn find(format: PluginFormat, id: &str) -> PluginDescriptor {
    scanned()
        .into_iter()
        .find(|p| p.format == format && p.id == id)
        .unwrap_or_else(|| panic!("{format} {id} scanned"))
}

/// Mono 48 kHz WAV: silence with one full-scale impulse at [`IMPULSE_AT`].
fn impulse_wav() -> Vec<u8> {
    let mut s = vec![0.0f32; 4_800];
    s[IMPULSE_AT] = 1.0;
    common::wav(RATE, &[s])
}

type Ctl = EtherController<NativeBridge, NativeServices, MemoryStore, MemoryLibrary>;

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
        catalog.replace(scanned());
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
            ids: IdGen::new(0x5C_4Eu64),
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

    fn settle(&mut self) {
        for _ in 0..500 {
            self.render(BLOCK);
            if !self.ctl.media_pending() {
                break;
            }
        }
        self.render(4 * BLOCK);
    }

    fn device(&mut self, id: &str) -> Value {
        self.ok("Project", json!({"type": "Get"}))["project"]["devices"][id].clone()
    }

    /// Peak of the master output over the first beat of playback.
    fn peak(&mut self) -> f32 {
        self.ok("Transport", json!({"type": "Stop"}));
        self.ok("Transport", json!({"type": "Locate", "position": 0.0}));
        self.render(8 * BLOCK);
        self.ok("Transport", json!({"type": "Play"}));
        let out = self.render(BEAT);
        self.ok("Transport", json!({"type": "Stop"}));
        out.iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }

    /// Offline export of the mix (whole project, no normalization): its peak.
    fn export_peak(&mut self, pid: ProjectId, job: &str) -> f32 {
        self.ok(
            "Export",
            json!({"type": "Render", "job": job, "request": {
                "range": {"type": "Project"},
                "format": {"container": "Wav", "bit_depth": "Float32", "sample_rate": null},
                "mode": {"type": "Mix"},
                "normalize": false,
                "tail_seconds": 0.1,
                "name": job,
            }}),
        );
        let files = 'done: {
            for _ in 0..20_000 {
                let from = self.out.len();
                self.tick();
                for m in &self.out[from..] {
                    match m {
                        ServerMessage::Event(Event::Export {
                            event: ExportEvent::Done { job: j, result },
                        }) if j == job => match result {
                            ExportResult::Files { files } => break 'done files.clone(),
                            other => panic!("unexpected export result {other:?}"),
                        },
                        ServerMessage::Event(Event::Export {
                            event: ExportEvent::Failed { job: j, message },
                        }) if j == job => panic!("export failed: {message}"),
                        _ => {}
                    }
                }
            }
            panic!("export did not finish");
        };
        let bytes = self.ctl.store.read(pid, &files[0]).expect("exported file");
        let audio = ether_media::decode(&bytes, Some("wav")).expect("decode");
        audio
            .channels
            .iter()
            .flatten()
            .fold(0.0f32, |m, s| m.max(s.abs()))
    }

    fn notifications(&self) -> Vec<String> {
        self.out
            .iter()
            .filter_map(|m| match m {
                ServerMessage::Event(Event::Notification { message, .. }) => Some(message.clone()),
                _ => None,
            })
            .collect()
    }
}

struct Project {
    pid: ProjectId,
    src: String,
    device: String,
}

/// Src (impulse at beat 0, routed nowhere) + FX (empty, `plugin` inserted).
fn setup(rig: &mut Rig, plugin: &PluginDescriptor) -> Project {
    let pid = rig.ids.next_project_id(1);
    rig.ok(
        "Project",
        json!({"type": "Create", "id": pid.to_string(), "name": "Sidechain"}),
    );
    rig.ok("Transport", json!({"type": "SetTempo", "bpm": 120.0}));
    let media = rig.id();
    rig.ok(
        "Media",
        json!({"type": "Import", "id": media, "source": {"type": "Location",
               "location": {"type": "Library", "id": "lib"}, "path": "impulse.wav"}}),
    );
    let track = |rig: &mut Rig, name: &str, kind: &str| {
        let t = rig.id();
        rig.ok(
            "Track",
            json!({"type": "Create", "id": t, "kind": kind, "name": name, "color": null,
                   "parent": null, "before": null}),
        );
        t
    };
    let src = track(rig, "Src", "Audio");
    let clip = rig.id();
    rig.ok(
        "Clip",
        json!({"type": "CreateAudio", "id": clip, "track": src, "start": 0.0, "media": media}),
    );
    rig.ok(
        "Mixer",
        json!({"type": "SetOutput", "track": src, "output": {"type": "None"}}),
    );
    // Instruments go on MIDI tracks.
    let kind = if plugin.category == ether_core::protocol::devices::DeviceCategory::Instrument {
        "Midi"
    } else {
        "Audio"
    };
    let fx = track(rig, "FX", kind);
    let device = rig.id();
    rig.ok(
        "Device",
        json!({"type": "Insert", "id": device, "track": fx,
               "device": {"type": "Plugin", "plugin_id": plugin.id, "format": plugin.format,
                          "sandboxed": false},
               "before": null}),
    );
    rig.settle();
    Project { pid, src, device }
}

fn set_sidechain(rig: &mut Rig, device: &str, source: Option<&str>) {
    rig.ok(
        "Device",
        json!({"type": "SetSidechain", "device": device, "source": source}),
    );
    rig.render(4 * BLOCK);
}

fn run(plugin: PluginDescriptor) {
    let format = plugin.format;
    assert_eq!(plugin.sidechain_inputs, 2, "{format}: scanned aux bus");
    let mut rig = Rig::new();
    let p = setup(&mut rig, &plugin);

    // Nothing is heard until the plugin takes Src as its sidechain.
    let silent = rig.peak();
    assert!(silent < 1e-6, "{format}: no source {silent}");
    set_sidechain(&mut rig, &p.device, Some(&p.src));
    assert_eq!(rig.device(&p.device)["sidechain"], json!(p.src));
    let wet = rig.peak();
    assert!((wet - 1.0).abs() < 1e-3, "{format}: sidechain heard {wet}");

    // Offline renders route sidechains like playback.
    let exported = rig.export_peak(p.pid, &format!("{format}-mix"));
    assert!(
        (exported - 1.0).abs() < 1e-3,
        "{format}: exported {exported}"
    );

    // Sandboxed: the helper's plugin gets the sidechain through shared memory.
    rig.ok(
        "Plugin",
        json!({"type": "SetSandboxed", "device": p.device, "sandboxed": true}),
    );
    rig.settle();
    assert_eq!(rig.device(&p.device)["kind"]["plugin"]["sandboxed"], true);
    assert_eq!(rig.device(&p.device)["sidechain"], json!(p.src));
    let sandboxed = rig.peak();
    assert!(
        (sandboxed - 1.0).abs() < 1e-3,
        "{format}: sandboxed sidechain {sandboxed}"
    );

    // Clearing the source: the aux bus is silent again (undo brings it back).
    set_sidechain(&mut rig, &p.device, None);
    let cleared = rig.peak();
    assert!(cleared < 1e-6, "{format}: cleared {cleared}");
    rig.ok("Edit", json!({"type": "Undo"}));
    rig.render(4 * BLOCK);
    let undone = rig.peak();
    assert!((undone - 1.0).abs() < 1e-3, "{format}: undo {undone}");

    assert!(
        rig.notifications().is_empty(),
        "{format}: {:?}",
        rig.notifications()
    );
}

#[test]
fn clap_plugin_sidechain() {
    run(find(PluginFormat::Clap, CLAP_ID));
}

#[test]
fn vst3_plugin_sidechain() {
    run(find(PluginFormat::Vst3, ether_vst3::testing::EFFECT_ID));
}

#[test]
fn plugins_without_an_aux_bus_refuse_a_sidechain() {
    let instrument = find(PluginFormat::Vst3, ether_vst3::testing::INSTRUMENT_ID);
    assert_eq!(instrument.sidechain_inputs, 0);
    let mut rig = Rig::new();
    let p = setup(&mut rig, &instrument);
    let err = rig
        .send(
            "Device",
            json!({"type": "SetSidechain", "device": p.device, "source": p.src}),
        )
        .unwrap_err();
    assert!(err.contains("no sidechain input"), "{err}");
}
