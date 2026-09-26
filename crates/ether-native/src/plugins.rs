//! CLAP plugin hosting for the native host: main-thread executor, plugin registry, plugin
//! catalog (scanner + on-disk DB).
//!
//! # Threading
//! CLAP main-thread calls (instantiate, activate, state, params, GUI) must run on the thread
//! that owns the `PluginController`, and on macOS AppKit requires that to be the process
//! main thread (floating editors). So every plugin controller lives in a registry that is
//! only ever touched on the [`MainThread`] executor:
//! - the desktop app implements [`MainThread`] with Tauri's `run_on_main_thread`;
//! - headless hosts and tests use [`DedicatedThread`] (one thread owning every plugin).
//!
//! The controller thread calls into the registry synchronously ([`PluginHost::call`]),
//! with a timeout so a stuck main thread can't hang the controller forever.
//!
//! Plugins are activated with the engine's `max_block_size` (the audio backends never call
//! `process` with larger blocks). The audio half ([`PluginNode`]) goes to the engine wrapped
//! in [`HostedPluginNode`], whose `Drop` (on the GC thread, when the engine retires it)
//! sends the node back to the main thread so the controller can `deactivate` it properly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Sender, bounded, unbounded};
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{DeviceId, ParamId};
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::{AudioBuffers, Device, Node, PrepareConfig, ProcessContext, ProcessStatus};

/// Runs closures on the thread that owns plugin controllers.
pub trait MainThread: Send + Sync {
    /// Queue `f` to run on the main thread. Must not run it inline on the caller's thread
    /// unless the caller *is* the main thread.
    fn spawn(&self, f: Box<dyn FnOnce() + Send>);
}

/// A plain thread acting as the "main thread" (headless hosts, tests, Linux/Windows
/// without editors). Exits when dropped.
pub struct DedicatedThread {
    tx: Sender<Box<dyn FnOnce() + Send>>,
}

impl DedicatedThread {
    pub fn new() -> Self {
        let (tx, rx) = unbounded::<Box<dyn FnOnce() + Send>>();
        std::thread::Builder::new()
            .name("ether-plugin-main".into())
            .spawn(move || {
                while let Ok(f) = rx.recv() {
                    f();
                }
            })
            .expect("spawn plugin main thread");
        Self { tx }
    }
}

impl Default for DedicatedThread {
    fn default() -> Self {
        Self::new()
    }
}

impl MainThread for DedicatedThread {
    fn spawn(&self, f: Box<dyn FnOnce() + Send>) {
        let _ = self.tx.send(f);
    }
}

pub(crate) struct Hosted {
    controller: Box<dyn PluginController>,
    /// Set when the device was destroyed while its node was still in the engine: the
    /// controller is dropped once the node comes back.
    destroyed: bool,
    active: bool,
}

thread_local! {
    /// Plugin controllers, keyed by device. Only touched on the main-thread executor.
    static REGISTRY: RefCell<HashMap<DeviceId, Hosted>> = RefCell::new(HashMap::new());
}

/// Handle to the main-thread plugin registry. Cheap to clone.
#[derive(Clone)]
pub struct PluginHost {
    main: Arc<dyn MainThread>,
    timeout: Duration,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHost").finish_non_exhaustive()
    }
}

/// Instantiates a plugin controller on the main thread (`ether_clap::instantiate` in
/// production; fakes in tests).
pub type Instantiate =
    Arc<dyn Fn(&Path, &str) -> Result<Box<dyn PluginController>, PluginError> + Send + Sync>;

impl PluginHost {
    pub fn new(main: Arc<dyn MainThread>) -> Self {
        Self {
            main,
            timeout: Duration::from_secs(30),
        }
    }

    /// Run `f` on the main thread with the registry and wait for its result.
    pub(crate) fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut HashMap<DeviceId, Hosted>) -> R + Send + 'static,
    ) -> Result<R, PluginError> {
        let (tx, rx) = bounded(1);
        self.main.spawn(Box::new(move || {
            let r = REGISTRY.with(|reg| f(&mut reg.borrow_mut()));
            let _ = tx.send(r);
        }));
        rx.recv_timeout(self.timeout)
            .map_err(|_| PluginError::Ipc("main thread did not respond".into()))
    }

    /// Instantiate, restore state, and activate a plugin on the main thread. Returns the
    /// engine node (wrapped so it comes back to the main thread when retired) and the
    /// descriptor.
    pub fn instantiate(
        &self,
        instantiate: Instantiate,
        device: DeviceId,
        bundle: PathBuf,
        plugin_id: String,
        state: Option<Vec<u8>>,
        prepare: PrepareConfig,
    ) -> Result<(Box<dyn Node>, DeviceDescriptor), PluginError> {
        let host = self.clone();
        let (node, descriptor) = self.call(move |reg| {
            // Replacing a live instance of the same device: drop the old one first.
            reg.remove(&device);
            let mut controller = instantiate(&bundle, &plugin_id)?;
            if let Some(state) = state.as_deref().filter(|s| !s.is_empty()) {
                controller.load_state(state)?;
            }
            let node = controller.activate(&prepare)?;
            let descriptor = controller.descriptor();
            reg.insert(
                device,
                Hosted {
                    controller,
                    destroyed: false,
                    active: true,
                },
            );
            Ok::<_, PluginError>((node, descriptor))
        })??;
        Ok((
            Box::new(HostedPluginNode {
                device,
                node: Some(node),
                host,
            }),
            descriptor,
        ))
    }

    /// The device was destroyed: drop its controller now if its node is already back,
    /// otherwise when the node returns from the engine.
    pub fn destroy(&self, device: DeviceId) {
        let _ = self.call(move |reg| {
            if let Some(h) = reg.get_mut(&device) {
                if h.active {
                    h.destroyed = true;
                } else {
                    reg.remove(&device);
                }
            }
        });
    }

    /// Called from [`HostedPluginNode::drop`] (GC thread): deactivate on the main thread.
    fn node_returned(&self, device: DeviceId, node: Box<dyn PluginNode>) {
        self.main.spawn(Box::new(move || {
            REGISTRY.with(|reg| {
                let mut reg = reg.borrow_mut();
                if let Some(h) = reg.get_mut(&device) {
                    h.controller.deactivate(node);
                    h.active = false;
                    if h.destroyed {
                        reg.remove(&device);
                    }
                }
                // Unknown device: the controller is gone already; `node` drops here.
            });
        }));
    }

    pub fn open_editor(&self, device: DeviceId) -> Result<(), PluginError> {
        self.call(move |reg| {
            let h = reg
                .get_mut(&device)
                .ok_or_else(|| PluginError::NotFound(device.to_string()))?;
            if !h.controller.has_editor() {
                return Err(PluginError::NoEditor);
            }
            h.controller.open_editor()
        })?
    }

    pub fn close_editor(&self, device: DeviceId) -> Result<(), PluginError> {
        self.call(move |reg| {
            if let Some(h) = reg.get_mut(&device) {
                h.controller.close_editor();
            }
        })
    }

    pub fn save_state(&self, device: DeviceId) -> Result<Option<Vec<u8>>, PluginError> {
        self.call(move |reg| match reg.get_mut(&device) {
            Some(h) if !h.destroyed => h.controller.save_state().map(Some),
            _ => Ok(None),
        })?
    }

    pub fn param_value(&self, device: DeviceId, param: ParamId) -> Option<f64> {
        self.call(move |reg| reg.get_mut(&device)?.controller.param_value(param))
            .ok()
            .flatten()
    }

    /// Poll every live controller (CLAP `on_main_thread`, timers, GUI) and collect their
    /// notifications.
    pub fn poll(&self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        if let Ok(notes) = self.call(|reg| {
            let mut all = Vec::new();
            let mut buf = Vec::new();
            for (id, h) in reg.iter_mut().filter(|(_, h)| !h.destroyed) {
                h.controller.poll(&mut buf);
                all.extend(buf.drain(..).map(|n| (*id, n)));
            }
            all
        }) {
            out.extend(notes);
        }
    }

    /// Number of plugin controllers alive on the main thread (tests/diagnostics).
    pub fn live_count(&self) -> usize {
        self.call(|reg| reg.len()).unwrap_or(0)
    }
}

/// Engine node for a hosted plugin: forwards to the plugin's audio half and, when dropped
/// (GC thread), returns it to the main thread for `deactivate`.
pub struct HostedPluginNode {
    device: DeviceId,
    node: Option<Box<dyn PluginNode>>,
    host: PluginHost,
}

impl HostedPluginNode {
    fn inner(&self) -> &dyn PluginNode {
        self.node
            .as_deref()
            .expect("plugin node present until drop")
    }
    fn inner_mut(&mut self) -> &mut dyn PluginNode {
        self.node
            .as_deref_mut()
            .expect("plugin node present until drop")
    }
}

impl Node for HostedPluginNode {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.inner_mut().prepare(config);
    }
    fn reset(&mut self) {
        self.inner_mut().reset();
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.inner_mut().process(ctx, audio)
    }
    fn latency(&self) -> u32 {
        self.inner().latency()
    }
    fn channels(&self) -> (u16, u16) {
        self.inner().channels()
    }
}

impl Device for HostedPluginNode {
    fn descriptor(&self) -> DeviceDescriptor {
        self.inner().descriptor()
    }
    fn param(&self, id: ParamId) -> Option<f64> {
        self.inner().param(id)
    }
    fn set_param(&mut self, id: ParamId, value: f64) {
        self.inner_mut().set_param(id, value);
    }
}

impl Drop for HostedPluginNode {
    fn drop(&mut self) {
        if let Some(node) = self.node.take() {
            self.host.node_returned(self.device, node);
        }
    }
}

/// Known plugins (from the out-of-process scanner), persisted as JSON in the per-instance
/// plugin DB folder. Thread-safe; cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct PluginCatalog {
    inner: Arc<Mutex<Vec<PluginDescriptor>>>,
    db_file: Option<PathBuf>,
}

impl PluginCatalog {
    /// Load `<plugin_db_dir>/plugins.json` if present.
    pub fn open(plugin_db_dir: &Path) -> Self {
        let db_file = plugin_db_dir.join("plugins.json");
        let plugins = std::fs::read_to_string(&db_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(plugins)),
            db_file: Some(db_file),
        }
    }

    pub fn list(&self) -> Vec<PluginDescriptor> {
        self.inner.lock().map(|v| v.clone()).unwrap_or_default()
    }

    pub fn find(&self, plugin_id: &str) -> Option<PluginDescriptor> {
        self.inner
            .lock()
            .ok()?
            .iter()
            .find(|p| p.id == plugin_id)
            .cloned()
    }

    /// Replace the catalog (after a scan) and persist it.
    pub fn replace(&self, plugins: Vec<PluginDescriptor>) {
        if let Some(file) = &self.db_file
            && let Ok(json) = serde_json::to_string_pretty(&plugins)
            && let Err(e) = crate::store::atomic_write(file, json.as_bytes())
        {
            tracing::warn!(%e, "failed to write plugin DB");
        }
        if let Ok(mut v) = self.inner.lock() {
            *v = plugins;
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A fake plugin controller for tests (no CLAP binary needed).
    use super::*;
    use ether_core::protocol::devices::{DeviceCategory, DeviceTypeRef};
    use std::sync::atomic::{AtomicU32, Ordering};

    pub static DEACTIVATED: AtomicU32 = AtomicU32::new(0);

    pub struct FakeController {
        pub state: Vec<u8>,
        pub main_thread: std::thread::ThreadId,
        pub max_block: usize,
    }

    pub struct FakeNode;

    impl Node for FakeNode {
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
    impl Device for FakeNode {
        fn descriptor(&self) -> DeviceDescriptor {
            descriptor()
        }
        fn param(&self, _: ParamId) -> Option<f64> {
            None
        }
        fn set_param(&mut self, _: ParamId, _: f64) {}
    }
    impl PluginNode for FakeNode {
        fn is_faulted(&self) -> bool {
            false
        }
    }

    pub fn descriptor() -> DeviceDescriptor {
        DeviceDescriptor {
            device_type: DeviceTypeRef::Plugin {
                plugin_id: "fake".into(),
            },
            name: "Fake".into(),
            category: DeviceCategory::AudioEffect,
            params: Vec::new(),
            audio_inputs: 2,
            audio_outputs: 2,
            midi_input: false,
        }
    }

    impl PluginController for FakeController {
        fn descriptor(&self) -> DeviceDescriptor {
            assert_eq!(std::thread::current().id(), self.main_thread);
            descriptor()
        }
        fn params(&mut self) -> Vec<ether_core::protocol::devices::ParamInfo> {
            Vec::new()
        }
        fn activate(&mut self, config: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
            assert_eq!(std::thread::current().id(), self.main_thread);
            self.max_block = config.max_block_size;
            Ok(Box::new(FakeNode))
        }
        fn deactivate(&mut self, _node: Box<dyn PluginNode>) {
            assert_eq!(std::thread::current().id(), self.main_thread);
            DEACTIVATED.fetch_add(1, Ordering::SeqCst);
        }
        fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
            Ok(self.state.clone())
        }
        fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
            self.state = state.to_vec();
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
            out.push(PluginNotification::StateDirty);
        }
    }

    pub fn instantiate() -> Instantiate {
        Arc::new(|_bundle, id| {
            if id == "missing" {
                return Err(PluginError::NotFound(id.into()));
            }
            Ok(Box::new(FakeController {
                state: Vec::new(),
                main_thread: std::thread::current().id(),
                max_block: 0,
            }) as Box<dyn PluginController>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn device(n: u128) -> DeviceId {
        DeviceId(ether_core::protocol::model::Ulid(n))
    }

    #[test]
    fn plugin_lifecycle_on_main_thread() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let prepare = PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 256,
            max_events_per_block: 64,
        };
        let d = device(1);
        let (node, desc) = host
            .instantiate(
                fake::instantiate(),
                d,
                PathBuf::from("/x.clap"),
                "fake".into(),
                Some(b"state".to_vec()),
                prepare,
            )
            .unwrap();
        assert_eq!(desc.name, "Fake");
        assert_eq!(host.save_state(d).unwrap().as_deref(), Some(&b"state"[..]));
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert_eq!(notes, vec![(d, PluginNotification::StateDirty)]);
        assert!(matches!(host.open_editor(d), Err(PluginError::NoEditor)));

        // Destroy while the node is still "in the engine": controller kept until it returns.
        let before = fake::DEACTIVATED.load(Ordering::SeqCst);
        host.destroy(d);
        assert_eq!(host.live_count(), 1);
        drop(node); // GC thread drops the node → back to main thread → deactivate + drop.
        assert_eq!(host.live_count(), 0);
        assert_eq!(fake::DEACTIVATED.load(Ordering::SeqCst), before + 1);

        let err = host
            .instantiate(
                fake::instantiate(),
                device(2),
                PathBuf::from("/x.clap"),
                "missing".into(),
                None,
                prepare,
            )
            .err()
            .unwrap();
        assert!(matches!(err, PluginError::NotFound(_)));
    }

    #[test]
    fn catalog_persists() {
        let tmp = crate::test_util::TempDir::new("catalog");
        let cat = PluginCatalog::open(tmp.path());
        assert!(cat.list().is_empty());
        cat.replace(vec![PluginDescriptor {
            format: ether_core::protocol::model::PluginFormat::Clap,
            id: "com.x.y".into(),
            name: "Y".into(),
            vendor: "X".into(),
            version: "1".into(),
            description: String::new(),
            features: vec![],
            category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
            path: "/p/y.clap".into(),
        }]);
        let again = PluginCatalog::open(tmp.path());
        assert_eq!(again.find("com.x.y").unwrap().path, "/p/y.clap");
        assert!(again.find("nope").is_none());
    }
}
