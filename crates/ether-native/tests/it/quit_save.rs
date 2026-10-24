//! Quitting saves the live state of every plugin, even when the host is shut down ON
//! the plugins' main thread (the desktop app stops the host from Tauri's exit handler, on
//! the process main thread): the save runs before plugin calls are closed, and the
//! stopping thread serves the main-thread calls while it waits.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo};
use ether_core::protocol::message::{Reply, ReplyResult};
use ether_core::protocol::model::{Base64Bytes, IdGen, ParamId, PluginFormat};
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_core::{AudioBuffers, Device, Node, PrepareConfig, ProcessContext, ProcessStatus};
use ether_native::MainThread;
use ether_native::audio::{AudioBackendKind, AudioSettings};
use ether_native::host::{HostConfig, HostOptions, NativeHost};
use ether_native::test_util::TempDir;
use serde_json::{Value, json};

const PLUGIN_ID: &str = "test.quit-save";

type Job = Box<dyn FnOnce() + Send>;

/// Like the desktop app's main thread: work runs only when the owning thread serves it
/// (here the test thread, which also shuts the host down).
struct OwnerThread {
    owner: ThreadId,
    queue: Mutex<VecDeque<Job>>,
}

impl MainThread for OwnerThread {
    fn spawn(&self, f: Job) {
        self.queue.lock().unwrap().push_back(f);
    }

    fn run_pending(&self) {
        if std::thread::current().id() != self.owner {
            return;
        }
        loop {
            let job = self.queue.lock().unwrap().pop_front();
            match job {
                Some(job) => job(),
                None => break,
            }
        }
    }
}

/// The plugin's live state, changed by the test as a user would in the plugin's window.
#[derive(Default)]
struct Live {
    state: Mutex<Vec<u8>>,
    /// Report `StateDirty` at the next poll.
    changed: AtomicBool,
    /// `StateDirty` was reported.
    reported: AtomicBool,
}

struct Plugin(Arc<Live>);
struct Silent;

fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: None,
        device_type: DeviceTypeRef::Plugin {
            plugin_id: PLUGIN_ID.into(),
        },
        name: "Quit Save".into(),
        category: DeviceCategory::AudioEffect,
        params: Vec::new(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
        implemented: true,
    }
}

impl Node for Silent {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        _: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        ProcessStatus::Silent
    }
}

impl Device for Silent {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }
    fn param(&self, _: ParamId) -> Option<f64> {
        None
    }
    fn set_param(&mut self, _: ParamId, _: f64) {}
}

impl PluginNode for Silent {
    fn is_faulted(&self) -> bool {
        false
    }
}

impl PluginController for Plugin {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }
    fn params(&mut self) -> Vec<ParamInfo> {
        Vec::new()
    }
    fn activate(&mut self, _: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
        Ok(Box::new(Silent))
    }
    fn deactivate(&mut self, _: Box<dyn PluginNode>) {}
    fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
        Ok(self.0.state.lock().unwrap().clone())
    }
    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        *self.0.state.lock().unwrap() = state.to_vec();
        Ok(())
    }
    fn has_editor(&self) -> bool {
        false
    }
    fn open_editor(&mut self) -> Result<(), PluginError> {
        Err(PluginError::NoEditor)
    }
    fn close_editor(&mut self) {}
    fn poll(&mut self, out: &mut Vec<PluginNotification>) {
        if self.0.changed.swap(false, Ordering::SeqCst) {
            out.push(PluginNotification::StateDirty);
            self.0.reported.store(true, Ordering::SeqCst);
        }
    }
}

struct Rig {
    host: NativeHost,
    main: Arc<OwnerThread>,
    replies: Arc<Mutex<Vec<Reply>>>,
    next_id: u32,
}

impl Rig {
    /// Serve the main thread until `done` (bounded).
    fn pump_until(&self, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            self.main.run_pending();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg: ClientMessage = serde_json::from_value(json!({
            "id": id,
            "command": { "domain": domain, "command": command },
        }))
        .unwrap();
        self.host.send(msg).unwrap();
        let replies = self.replies.clone();
        let mut reply = None;
        self.pump_until("a reply", || {
            reply = replies.lock().unwrap().iter().find(|r| r.id == id).cloned();
            reply.is_some()
        });
        match reply.unwrap().result {
            ReplyResult::Ok { value } => serde_json::to_value(value).unwrap(),
            ReplyResult::Err { error } => panic!("{domain} {command} failed: {error:?}"),
        }
    }
}

fn start(dir: &Path, live: Arc<Live>) -> Rig {
    // The plugin is in the scanned list.
    let db = dir.join("plugin-db");
    std::fs::create_dir_all(&db).unwrap();
    let listed = vec![PluginDescriptor {
        sidechain_inputs: Default::default(),
        format: PluginFormat::Clap,
        id: PLUGIN_ID.into(),
        name: "Quit Save".into(),
        vendor: "Ethereal".into(),
        version: "1".into(),
        description: String::new(),
        features: vec!["audio-effect".into()],
        category: DeviceCategory::AudioEffect,
        path: "/nonexistent/quit-save.clap".into(),
    }];
    std::fs::write(
        db.join("plugins.json"),
        serde_json::to_vec(&listed).unwrap(),
    )
    .unwrap();

    let main = Arc::new(OwnerThread {
        owner: std::thread::current().id(),
        queue: Mutex::default(),
    });
    let host = NativeHost::start(
        HostConfig {
            audio: Some(AudioSettings {
                backend: AudioBackendKind::Null,
                max_block_size: 256,
                ..Default::default()
            }),
            data_dir: dir.to_path_buf(),
            instance: "test".into(),
            projects_root: dir.join("projects"),
            library_roots: Vec::new(),
        },
        HostOptions {
            main_thread: main.clone(),
            instantiate: Arc::new(move |_, _, _| {
                Ok(Box::new(Plugin(live.clone())) as Box<dyn PluginController>)
            }),
            ..HostOptions::default()
        },
    )
    .expect("host starts");
    let replies: Arc<Mutex<Vec<Reply>>> = Arc::default();
    let sink = replies.clone();
    host.subscribe(Arc::new(move |m| {
        if let ServerMessage::Reply(r) = m {
            sink.lock().unwrap().push(r);
        }
    }));
    Rig {
        host,
        main,
        replies,
        next_id: 1,
    }
}

#[test]
fn quitting_on_the_main_thread_saves_the_live_plugin_state() {
    let dir = TempDir::new("quit-save");
    let live = Arc::new(Live::default());
    *live.state.lock().unwrap() = vec![1];
    let mut rig = start(dir.path(), live.clone());
    let mut ids = IdGen::new(0x566);

    let project = ids.next_project_id(1).to_string();
    rig.ok(
        "Project",
        json!({"type": "Create", "id": project, "name": "Quit"}),
    );
    let track = ids.next_ulid(2).to_string();
    rig.ok(
        "Track",
        json!({"type": "Create", "id": track, "kind": "Audio", "name": null, "color": null,
               "parent": null, "before": null}),
    );
    let device = ids.next_ulid(3).to_string();
    rig.ok(
        "Device",
        json!({"type": "Insert", "id": device, "track": track,
               "device": {"type": "Plugin", "plugin_id": PLUGIN_ID, "sandboxed": false},
               "before": null}),
    );
    rig.ok("Project", json!({"type": "Save", "autosave": false}));

    // A change made in the plugin's own window (a preset loaded), reported as `StateDirty`;
    // quit before the slow timer saves it.
    *live.state.lock().unwrap() = vec![42, 7];
    live.changed.store(true, Ordering::SeqCst);
    rig.pump_until("the change reported", || {
        live.reported.load(Ordering::SeqCst)
    });
    // A few ticks for the controller to take the notification.
    let settle = Instant::now() + Duration::from_millis(200);
    rig.pump_until("a few ticks", || Instant::now() >= settle);

    let Rig { host, .. } = rig;
    // On the "main thread": nothing else serves the plugin calls of the save.
    host.shutdown();

    let file =
        ether_native::project_dirs::dir(&dir.path().join("projects"), project.parse().unwrap())
            .join("project.ether");
    let doc: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let state = &doc["project"]["devices"][&device]["kind"]["plugin"]["state"];
    let state: Base64Bytes = serde_json::from_value(state.clone())
        .unwrap_or_else(|e| panic!("no plugin state saved ({e}): {doc:#}"));
    assert_eq!(
        state.0,
        vec![42, 7],
        "the live state, not the one saved before"
    );
}
