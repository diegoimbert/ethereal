//! Agent plugins end to end with a real CLAP plugin (`ether-clap`'s `ether_test_plugin`
//! fixture, wrapped into a bundle) behind the native bridge, driven only through the agent
//! tools: `list_plugins` sees the scanned catalog, `add_device` inserts the plugin by id
//! (and by name), `get_device_params` lists its params and `set_device_param` changes the
//! rendered audio (the fixture's output = input × Gain).
#![cfg(any(target_os = "macos", target_os = "linux"))]

use crate::common;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use ether_clap::testing;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{Controller, EtherController};
use ether_core::protocol::model::{IdGen, PluginFormat};
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::protocol::{ClientMessage, ReplyResult, ServerMessage};
use ether_core::{Engine, GarbageCollector, PrepareConfig};
use ether_native::plugins::{PluginCatalog, PluginHost};
use ether_native::rt::AudioShared;
use ether_native::{DedicatedThread, NativeBridge, NativeServices};
use serde_json::{Value, json};

const PLUGIN_ID: &str = "dev.ethereal.test-plugin";
const BLOCK: usize = 256;
const RATE: u32 = 48_000;
const IMPULSE_AT: usize = 100;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("native-agent-plugins-e2e");
            testing::make_bundle(&dir, "EtherAgentTest")
        })
        .clone()
}

fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        sidechain_inputs: 0,
        format: PluginFormat::Clap,
        id: PLUGIN_ID.into(),
        name: "Ethereal Test Plugin".into(),
        vendor: "Ethereal".into(),
        version: "1".into(),
        description: String::new(),
        features: vec!["audio-effect".into()],
        category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
        path: bundle().display().to_string(),
    }
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
        let mut s = vec![0.0f32; 4_800];
        s[IMPULSE_AT] = 0.5;
        library.add_file("lib", "impulse.wav", common::wav(RATE, &[s]));
        Self {
            ctl: EtherController::new(
                bridge,
                NativeServices::default(),
                MemoryStore::new(),
                library,
            ),
            engine: parts.engine,
            gc: parts.gc,
            out: Vec::new(),
            next_id: 1,
            ids: IdGen::new(0xa6e),
            now: 1_000,
        }
    }

    fn id(&mut self) -> String {
        self.now += 1;
        self.ids.next_ulid(self.now).to_string()
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
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
            ReplyResult::Ok { value } => serde_json::to_value(value).unwrap(),
            ReplyResult::Err { error } => panic!("{domain} {command} failed: {}", error.message),
        }
    }

    /// An agent tool call; panics on an `is_error` result.
    fn tool(&mut self, name: &str, input: Value) -> Value {
        let r = self.ok(
            "Agent",
            json!({"type": "CallTool", "name": name, "input": input.to_string()}),
        );
        let content = r["content"].as_str().expect("content").to_string();
        assert_ne!(r["is_error"], true, "{name}: {content}");
        serde_json::from_str(&content).unwrap_or(Value::String(content))
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
            // Loaded: media decoded, plugins in and primed (`ether_controller::loading`).
            if !self.ctl.media_pending() && !self.ctl.loading.active() {
                break;
            }
        }
        self.render(4 * BLOCK);
    }

    /// Peak of the first beat played from the start.
    fn peak(&mut self) -> f32 {
        self.ok("Transport", json!({"type": "Stop"}));
        self.ok("Transport", json!({"type": "Locate", "position": 0.0}));
        self.render(8 * BLOCK);
        self.ok("Transport", json!({"type": "Play"}));
        let out = self.render(RATE as usize / 4);
        self.ok("Transport", json!({"type": "Stop"}));
        out.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }
}

#[test]
fn the_agent_lists_adds_and_drives_a_real_plugin() {
    let mut rig = Rig::new();
    let pid = rig.ids.next_project_id(1).to_string();
    rig.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Agent"}),
    );

    // list_plugins sees the scanned catalog.
    let l = rig.tool("list_plugins", json!({}));
    assert_eq!(l["available"], true);
    assert_eq!(l["plugins"][0]["plugin_id"], PLUGIN_ID, "{l}");
    assert_eq!(l["plugins"][0]["format"], "clap");
    assert_eq!(l["plugins"][0]["category"], "audio_effect");
    let f = rig.tool(
        "find",
        json!({ "query": "ethereal test plugin", "kinds": ["plugin"] }),
    );
    assert_eq!(f["best"]["id"], PLUGIN_ID);

    // An audio track playing an impulse.
    let t = rig.tool("create_track", json!({ "kind": "audio", "name": "Hits" }));
    let track = t["track_id"].as_str().unwrap().to_string();
    let media = rig.id();
    rig.ok(
        "Media",
        json!({"type": "Import", "id": media, "source": {"type": "Location",
               "location": {"type": "Library", "id": "lib"}, "path": "impulse.wav"}}),
    );
    let clip = rig.id();
    rig.ok(
        "Clip",
        json!({"type": "CreateAudio", "id": clip, "track": track, "start": 0.0, "media": media}),
    );
    rig.settle();
    let dry = rig.peak();
    assert!((dry - 0.5).abs() < 0.02, "dry impulse: {dry}");

    // add_device by plugin id.
    let v = rig.tool(
        "add_device",
        json!({ "track_id": "hits", "plugin_id": PLUGIN_ID, "format": "clap" }),
    );
    let device = v["device_id"].as_str().unwrap().to_string();
    assert_eq!(v["name"], "Ethereal Test Plugin", "{v}");
    rig.settle();

    // Its params by name, with display values.
    let p = rig.tool("get_device_params", json!({ "device_id": device }));
    let names: Vec<&str> = p["params"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["name"].as_str())
        .collect();
    assert!(names.contains(&"Gain") && names.contains(&"Mode"), "{p}");
    let unity = rig.peak();
    assert!((unity - dry).abs() < 0.02, "gain 1 = dry: {unity} vs {dry}");

    // set_device_param audibly: Gain 2 doubles the impulse, "a bit less" lowers it, 0 mutes.
    let v = rig.tool(
        "set_device_param",
        json!({ "device_id": "ethereal test plugin", "param": "gain", "value": 2 }),
    );
    assert_eq!(v["value"], 2.0, "{v}");
    rig.render(4 * BLOCK);
    let loud = rig.peak();
    assert!((loud / dry - 2.0).abs() < 0.05, "gain 2: {loud} vs {dry}");
    let v = rig.tool(
        "set_device_param",
        json!({ "device_id": device, "param": "Gain", "delta_percent": -25 }),
    );
    let g = v["value"].as_f64().unwrap();
    assert!((g - 1.5).abs() < 1e-6, "{v}");
    rig.render(4 * BLOCK);
    let less = rig.peak();
    assert!((less / dry - 1.5).abs() < 0.05, "gain 1.5: {less} vs {dry}");
    let v = rig.tool(
        "set_device_param",
        json!({ "device_id": device, "param": "Mode", "value": "hot" }),
    );
    assert_eq!(v["label"], "Hot", "{v}");
    rig.tool(
        "set_device_param",
        json!({ "device_id": device, "param": "Gain", "value": 0 }),
    );
    rig.render(4 * BLOCK);
    let muted = rig.peak();
    assert!(muted < 0.01, "gain 0: {muted}");
    // The document holds the value (saved with the project).
    let project = rig.ok("Project", json!({"type": "Get"}))["project"].clone();
    assert_eq!(project["devices"][&device]["params"]["1"], 0.0);
}
