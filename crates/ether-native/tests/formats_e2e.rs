//! Every plugin format end to end through the native host: one CLAP (`ether-clap`'s
//! fixture), one VST3 (`ether-vst3`'s fixture) and one AU (Apple's built-in AUDelay, macOS
//! only), each:
//!
//! - scanned through the real `ether-plugin-scanner` (targets from the host's
//!   [`ether_native::plugins::formats`] discovery; AUs from the component registry);
//! - inserted with its format (`DeviceSpec::Plugin { format }`), processed (the plugin
//!   audibly changes an impulse, before and after a param edit);
//! - saved and reopened (the state blob restores the edit; params mirrored back);
//! - toggled into the sandbox helper (`--format`) and back, keeping its state, and saved /
//!   reopened while sandboxed.
//!
//! Plus a missing plugin: a project whose VST3 isn't in the catalog opens with a clear
//! "not installed (rescan plugins)" error and a bypassed device, and loads once rescanned.
//!
//! Controller + native bridge + engine are rendered offline on the test thread; plugins
//! live on a [`DedicatedThread`].
#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{Controller, EtherController};
use ether_core::protocol::model::{IdGen, PluginFormat};
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::protocol::{ClientMessage, ReplyResult, ServerMessage};
use ether_core::{Engine, GarbageCollector, PrepareConfig};
use ether_native::plugins::{PluginCatalog, PluginHost};
use ether_native::rt::AudioShared;
use ether_native::{DedicatedThread, NativeBridge, NativeServices};
use ether_plugin_host::{ScanRunner, ScanTarget};
use serde_json::{Value, json};

const BLOCK: usize = 256;
const RATE: u32 = 48_000;
/// Samples per beat at 120 BPM.
const BEAT: usize = RATE as usize / 2;
const IMPULSE_AT: usize = 100;
const CLAP_ID: &str = "dev.ethereal.test-plugin";
/// AUDelay: 1 s delay, 50 % wet/dry by default (equal-power mix: dry gain ≈ 0.707).
#[cfg(target_os = "macos")]
const AU_ID: &str = "aufx:dely:appl";

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

/// Every fixture scanned through the real scanner, once per test process.
fn scanned() -> Vec<PluginDescriptor> {
    static S: OnceLock<Vec<PluginDescriptor>> = OnceLock::new();
    S.get_or_init(|| {
        let dir = ether_clap::testing::temp_dir("formats-e2e");
        let clap = ether_clap::testing::make_bundle(&dir, "EtherFormatsClap");
        let vst3 = ether_vst3::testing::make_bundle(&dir, "EtherFormatsVst3");
        let targets = ether_native::plugins::formats().discover(Some(&[dir]));
        assert_eq!(
            targets,
            vec![
                ScanTarget {
                    format: PluginFormat::Clap,
                    path: clap
                },
                ScanTarget {
                    format: PluginFormat::Vst3,
                    path: vst3
                },
            ],
            "bundles discovered by format"
        );
        // AUs only exist on macOS, so only there does the target list grow.
        #[cfg(target_os = "macos")]
        let targets = {
            let mut targets = targets;
            // AUs come from the component registry (what a rescan scans).
            let all = ether_native::plugins::formats().discover(None);
            let au = ScanTarget {
                format: PluginFormat::Au,
                path: AU_ID.into(),
            };
            assert!(all.contains(&au), "AUDelay in the registry targets");
            targets.push(au);
            targets
        };
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
    catalog: PluginCatalog,
    out: Vec<ServerMessage>,
    next_id: u32,
    ids: IdGen,
    now: u64,
}

impl Rig {
    fn new(plugins: Vec<PluginDescriptor>) -> Self {
        ether_native::sandbox::set_helper_path(Some(helper()));
        let parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: RATE,
            max_block_size: BLOCK,
            ..Default::default()
        });
        let catalog = PluginCatalog::default();
        catalog.replace(plugins);
        let bridge = NativeBridge::new(
            parts.handle,
            PrepareConfig {
                sample_rate: RATE as f32,
                max_block_size: BLOCK,
                max_events_per_block: 256,
            },
            PluginHost::new(Arc::new(DedicatedThread::new())),
            catalog.clone(),
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
            catalog,
            out: Vec::new(),
            next_id: 1,
            ids: IdGen::new(0xF0_4Au64),
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

    /// Peak of the plugin track's impulse (beat 0) over the dry track's (beat 1). The window
    /// ends before AUDelay's 1 s echo.
    fn ratio(&mut self) -> f32 {
        self.ok("Transport", json!({"type": "Stop"}));
        self.ok("Transport", json!({"type": "Locate", "position": 0.0}));
        self.render(8 * BLOCK);
        self.ok("Transport", json!({"type": "Play"}));
        let out = self.render(BEAT + BEAT / 2);
        self.ok("Transport", json!({"type": "Stop"}));
        let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let (wet, dry) = out.split_at(BEAT / 2);
        let dry = peak(dry);
        assert!(dry > 0.5, "dry impulse present: {dry}");
        peak(wet) / dry
    }

    fn save_and_reopen(&mut self, pid: &str) {
        self.ok("Project", json!({"type": "Save"}));
        self.ok("Project", json!({"type": "Open", "id": pid}));
        self.settle();
    }

    fn notifications(&self) -> Vec<String> {
        self.out
            .iter()
            .filter_map(|m| match m {
                ServerMessage::Event(ether_core::protocol::Event::Notification {
                    message, ..
                }) => Some(message.clone()),
                _ => None,
            })
            .collect()
    }
}

/// New project: an impulse on a plugin track at beat 0 and on a dry track at beat 1.
/// Returns (project id, plugin track id).
fn setup(rig: &mut Rig) -> (String, String) {
    let pid = rig.ids.next_project_id(1).to_string();
    rig.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Formats"}),
    );
    rig.ok("Transport", json!({"type": "SetTempo", "bpm": 120.0}));
    let media = rig.id();
    rig.ok(
        "Media",
        json!({"type": "Import", "id": media, "source": {"type": "Location",
               "location": {"type": "Library", "id": "lib"}, "path": "impulse.wav"}}),
    );
    let track = |rig: &mut Rig, name: &str, beat: f64| {
        let t = rig.id();
        rig.ok(
            "Track",
            json!({"type": "Create", "id": t, "kind": "Audio", "name": name, "color": null,
                   "parent": null, "before": null}),
        );
        let c = rig.id();
        rig.ok(
            "Clip",
            json!({"type": "CreateAudio", "id": c, "track": t, "start": beat, "media": media}),
        );
        t
    };
    let fx = track(rig, "FX", 0.0);
    track(rig, "Dry", 1.0);
    (pid, fx)
}

fn insert(rig: &mut Rig, track: &str, plugin: &PluginDescriptor) -> String {
    let id = rig.id();
    rig.ok(
        "Device",
        json!({"type": "Insert", "id": id, "track": track,
               "device": {"type": "Plugin", "plugin_id": plugin.id, "format": plugin.format,
                          "sandboxed": false},
               "before": null}),
    );
    id
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

/// How a format's fixture is exercised: `param` set to `value` changes the plugin track's
/// level from `before` to `after` (relative to the dry track).
struct Case {
    plugin: PluginDescriptor,
    param: u32,
    value: f64,
    before: f32,
    after: f32,
}

fn run(case: Case) {
    let format = case.plugin.format;
    let mut rig = Rig::new(scanned());
    let (pid, fx) = setup(&mut rig);
    let dev = insert(&mut rig, &fx, &case.plugin);
    rig.settle();
    let d = rig.device(&dev);
    assert_eq!(d["kind"]["plugin"]["format"], json!(format), "{d}");
    assert_eq!(d["kind"]["plugin"]["plugin_id"], case.plugin.id);
    assert_eq!(d["name"], case.plugin.name, "named after the plugin");

    // --- Process.
    let r = rig.ratio();
    assert!(close(r, case.before), "{format}: before the edit {r}");
    rig.ok(
        "Device",
        json!({"type": "SetParam", "device": dev, "param": case.param, "value": case.value}),
    );
    rig.render(4 * BLOCK);
    let r = rig.ratio();
    assert!(close(r, case.after), "{format}: after the edit {r}");

    // --- Save / reopen: the state blob restores the edit.
    rig.save_and_reopen(&pid);
    let d = rig.device(&dev);
    assert!(
        d["kind"]["plugin"]["state"].is_string(),
        "{format}: state saved: {d}"
    );
    let mirrored = d["params"][case.param.to_string()].as_f64().unwrap();
    assert!(
        (mirrored - case.value).abs() < 1e-3,
        "{format}: param mirrored from the state: {d}"
    );
    let r = rig.ratio();
    assert!(close(r, case.after), "{format}: after reopening {r}");

    // --- Sandbox toggle: the helper loads it with its format and state.
    let before = ether_native::sandbox::last_helper_pid();
    rig.ok(
        "Plugin",
        json!({"type": "SetSandboxed", "device": dev, "sandboxed": true}),
    );
    rig.settle();
    assert_eq!(rig.device(&dev)["kind"]["plugin"]["sandboxed"], true);
    let helper = ether_native::sandbox::last_helper_pid();
    assert!(
        helper.is_some() && helper != before,
        "{format}: a helper runs it"
    );
    let r = rig.ratio();
    assert!(close(r, case.after), "{format}: sandboxed {r}");
    rig.save_and_reopen(&pid);
    let r = rig.ratio();
    assert!(close(r, case.after), "{format}: sandboxed, reopened {r}");
    rig.ok(
        "Plugin",
        json!({"type": "SetSandboxed", "device": dev, "sandboxed": false}),
    );
    rig.settle();
    assert_eq!(rig.device(&dev)["kind"]["plugin"]["sandboxed"], false);
    let r = rig.ratio();
    assert!(close(r, case.after), "{format}: back in-process {r}");
    assert!(
        rig.notifications().is_empty(),
        "{format}: {:?}",
        rig.notifications()
    );
}

#[test]
fn clap_end_to_end() {
    // Gain (param 1, plain gain).
    run(Case {
        plugin: find(PluginFormat::Clap, CLAP_ID),
        param: 1,
        value: 0.5,
        before: 1.0,
        after: 0.5,
    });
}

#[test]
fn vst3_end_to_end() {
    // Gain (param 1, continuous: output gain = 2 × normalized, default 0.5).
    run(Case {
        plugin: find(PluginFormat::Vst3, ether_vst3::testing::EFFECT_ID),
        param: 1,
        value: 0.25,
        before: 1.0,
        after: 0.5,
    });
}

#[cfg(target_os = "macos")]
#[test]
fn au_end_to_end() {
    // Wet/dry mix (param 0, percent): 50 % → dry gain ≈ 0.707; 0 % → dry only.
    run(Case {
        plugin: find(PluginFormat::Au, AU_ID),
        param: 0,
        value: 0.0,
        before: std::f32::consts::FRAC_1_SQRT_2,
        after: 1.0,
    });
}

#[test]
fn missing_plugins_are_bypassed_until_rescanned() {
    let vst3 = find(PluginFormat::Vst3, ether_vst3::testing::EFFECT_ID);
    let mut rig = Rig::new(scanned());
    let (pid, fx) = setup(&mut rig);
    let dev = insert(&mut rig, &fx, &vst3);
    rig.settle();
    rig.ok(
        "Device",
        json!({"type": "SetParam", "device": dev, "param": 1, "value": 0.25}),
    );
    rig.render(4 * BLOCK);

    // The catalog lost it (uninstalled, or a machine without it): the project still opens,
    // with a clear error, and the device is bypassed.
    rig.ok("Project", json!({"type": "Save"}));
    let others: Vec<_> = scanned()
        .into_iter()
        .filter(|p| p.format != PluginFormat::Vst3)
        .collect();
    rig.catalog.replace(others);
    rig.out.clear();
    rig.ok("Project", json!({"type": "Open", "id": pid}));
    rig.settle();
    let notes = rig.notifications();
    assert!(
        notes.iter().any(|n| n.contains(&format!(
            "VST3 plugin {} ({}) is not installed (rescan plugins)",
            vst3.name, vst3.id
        ))),
        "{notes:?}"
    );
    assert_eq!(rig.device(&dev)["kind"]["plugin"]["format"], "Vst3");
    let r = rig.ratio();
    assert!(close(r, 1.0), "bypassed: dry signal {r}");
    // A CLAP with the same id would not stand in for it: lookups are by (format, id).
    assert!(
        rig.catalog
            .find_format(PluginFormat::Vst3, &vst3.id)
            .is_none()
    );

    // Rescanned: reopening loads it with its saved state.
    rig.catalog.replace(scanned());
    rig.ok("Project", json!({"type": "Open", "id": pid}));
    rig.settle();
    let r = rig.ratio();
    assert!(close(r, 0.5), "restored after the rescan {r}");
}
