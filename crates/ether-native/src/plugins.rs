//! Plugin hosting for the native host (CLAP, VST3, AU through `ether-plugin-host`'s
//! [`Formats`] registry): main-thread executor, plugin registry, plugin catalog (scanner +
//! on-disk DB).
//!
//! # Threading
//! Plugin main-thread calls (instantiate, activate, state, params, GUI) must run on the
//! thread that owns the `PluginController`, and on macOS AppKit requires that to be the
//! process main thread (floating editors). So every plugin controller lives in a registry
//! that is only ever touched on the [`MainThread`] executor:
//! - the desktop app implements [`MainThread`] with Tauri's `run_on_main_thread`;
//! - headless hosts and tests use [`DedicatedThread`] (one thread owning every plugin).
//!
//! The controller thread calls into the registry synchronously ([`PluginHost::call`]),
//! with a timeout so a stuck main thread can't hang the controller forever.
//!
//! # Re-entrancy
//! Some plugin calls run a **nested run loop** on the main thread: AU `instantiate` (async
//! units) and `open_editor` pump `CFRunLoopRunInMode` until their completion arrives, and
//! anything queued with `run_on_main_thread` (other registry calls, returning nodes) can
//! run inside them. So the registry is never borrowed across a plugin call: a controller is
//! **checked out** of the registry (its slot left empty) for the duration of the call and
//! checked back in afterwards. A nested call that needs the same controller sees it busy
//! (`poll` skips it; a node returning from the engine is parked and deactivated at
//! check-in; other calls fail with an `Ipc` "busy" error).
//!
//! Plugins are activated with the engine's `max_block_size` (the audio backends never call
//! `process` with larger blocks). The audio half ([`PluginNode`]) goes to the engine wrapped
//! in [`HostedPluginNode`], whose `Drop` (on the GC thread, when the engine retires it)
//! sends the node back to the main thread so the controller can `deactivate` it properly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{RecvTimeoutError, Sender, bounded, unbounded};
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{DeviceId, ParamId, PluginFormat};
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::{AudioBuffers, Device, Node, PrepareConfig, ProcessContext, ProcessStatus};
use ether_plugin_host::Formats;

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

/// Every plugin format the native host loads: CLAP, VST3 and AU (AU is `Unsupported` off
/// macOS).
pub fn formats() -> Formats {
    Formats::new(vec![
        Arc::new(ether_clap::ClapFormat),
        Arc::new(ether_vst3::Vst3Format),
        Arc::new(ether_au::AuFormat),
    ])
}

/// The in-process instantiator of the native host: [`formats`], by format.
pub fn instantiate_any() -> Instantiate {
    let formats = formats();
    Arc::new(move |format, path, id| formats.instantiate(format, path, id))
}

/// Display name of a format (`CLAP`, `VST3`, `AU`) for user-facing messages.
pub fn format_label(format: PluginFormat) -> &'static str {
    match format {
        PluginFormat::Clap => "CLAP",
        PluginFormat::Vst3 => "VST3",
        PluginFormat::Au => "AU",
    }
}

pub(crate) struct Hosted {
    device: DeviceId,
    /// `None` while checked out for a plugin call (see the module docs on re-entrancy).
    controller: Option<Box<dyn PluginController>>,
    /// The device was destroyed or replaced while this instance's node was still in the
    /// engine: the controller is dropped once the node comes back.
    retired: bool,
    active: bool,
    /// The node came back from the engine while the controller was checked out: it is
    /// deactivated when the controller is checked back in.
    returned: Option<Box<dyn PluginNode>>,
}

/// Main-thread plugin registry. Instances are keyed by a per-instance token, so an old
/// instance of a device (retired, node still in the engine) and its replacement never get
/// confused: a returning node always finds its own controller.
#[derive(Default)]
pub(crate) struct Registry {
    instances: HashMap<u64, Hosted>,
    /// Current (non-retired) instance of each device.
    live: HashMap<DeviceId, u64>,
    next_token: u64,
}

impl Registry {
    /// Retire the current instance of `device`: dropped now if its node is back, else when
    /// its node returns (or at check-in, if a call has it checked out). Returns a controller
    /// for the caller to drop outside the registry borrow.
    fn retire(&mut self, device: DeviceId) -> Option<Box<dyn PluginController>> {
        let token = self.live.remove(&device)?;
        let h = self.instances.get_mut(&token)?;
        if h.active {
            h.retired = true;
            None
        } else {
            self.instances.remove(&token)?.controller
        }
    }
}

thread_local! {
    /// Plugin controllers. Only touched on the main-thread executor, and only borrowed for
    /// bookkeeping: never across a call into a plugin.
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Run `f` with the registry (main thread). `f` must not call into a plugin.
fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|reg| f(&mut reg.borrow_mut()))
}

fn busy() -> PluginError {
    PluginError::Ipc("plugin is busy (another main-thread call is running)".into())
}

/// Take the controller of instance `token` out of the registry. `None` if the instance is
/// unknown or already checked out.
fn checkout(token: u64) -> Option<Box<dyn PluginController>> {
    with_registry(|reg| reg.instances.get_mut(&token)?.controller.take())
}

/// Put a checked-out controller back: deactivates a node that returned meanwhile, and drops
/// the instance if it was retired and is inactive (or was removed meanwhile).
fn checkin(token: u64, mut controller: Box<dyn PluginController>) {
    while let Some(node) = with_registry(|reg| reg.instances.get_mut(&token)?.returned.take()) {
        controller.deactivate(node);
        with_registry(|reg| {
            if let Some(h) = reg.instances.get_mut(&token) {
                h.active = false;
            }
        });
    }
    let dropped = with_registry(|reg| match reg.instances.get_mut(&token) {
        Some(h) if h.retired && !h.active => {
            reg.instances.remove(&token);
            Some(controller)
        }
        Some(h) => {
            h.controller = Some(controller);
            None
        }
        None => Some(controller),
    });
    drop(dropped);
}

/// Run `f` on the (checked-out) controller of instance `token`.
fn with_instance<R>(
    token: u64,
    f: impl FnOnce(&mut dyn PluginController) -> R,
) -> Result<R, PluginError> {
    let mut controller = checkout(token).ok_or_else(busy)?;
    let r = f(&mut *controller);
    checkin(token, controller);
    Ok(r)
}

/// Run `f` on the live instance of `device`; `NotFound` without one.
fn with_live<R>(
    device: DeviceId,
    f: impl FnOnce(&mut dyn PluginController) -> R,
) -> Result<R, PluginError> {
    let token = with_registry(|reg| reg.live.get(&device).copied())
        .ok_or_else(|| PluginError::NotFound(device.to_string()))?;
    with_instance(token, f)
}

/// Handle to the main-thread plugin registry. Cheap to clone.
#[derive(Clone)]
pub struct PluginHost {
    main: Arc<dyn MainThread>,
    timeout: Duration,
    /// Set at shutdown: main-thread calls fail fast (the main thread may be busy joining
    /// the host and never run them).
    closing: Arc<AtomicBool>,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHost").finish_non_exhaustive()
    }
}

/// Instantiates a plugin controller on the main thread ([`instantiate_any`] in production;
/// fakes in tests). Arguments: the format, `PluginDescriptor.path` (the component id for
/// AUs) and the plugin id.
pub type Instantiate = Arc<
    dyn Fn(PluginFormat, &Path, &str) -> Result<Box<dyn PluginController>, PluginError>
        + Send
        + Sync,
>;

/// What [`PluginHost::instantiate`] loads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSource {
    pub format: PluginFormat,
    /// `PluginDescriptor.path`: the bundle, or the component id for AUs.
    pub path: PathBuf,
    pub plugin_id: String,
}

impl PluginHost {
    pub fn new(main: Arc<dyn MainThread>) -> Self {
        Self {
            main,
            timeout: Duration::from_secs(30),
            closing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Make every pending and future main-thread call fail immediately (shutdown).
    ///
    /// Retired controllers whose node returns after this point are only dropped if the
    /// main loop still runs the queued `node_returned` work; if it has already exited
    /// they are left behind with the thread-local registry (reclaimed at process exit,
    /// without a `deactivate`). That is acceptable at quit.
    pub fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
    }

    /// Run `f` on the main thread and wait for its result.
    pub(crate) fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce() -> R + Send + 'static,
    ) -> Result<R, PluginError> {
        let closed = || PluginError::Ipc("host is shutting down".into());
        if self.closing.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let (tx, rx) = bounded(1);
        self.main.spawn(Box::new(move || {
            let _ = tx.send(f());
        }));
        let deadline = Instant::now() + self.timeout;
        loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(r) => return Ok(r),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(PluginError::Ipc("main thread dropped the call".into()));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self.closing.load(Ordering::SeqCst) {
                        return Err(closed());
                    }
                    if Instant::now() >= deadline {
                        return Err(PluginError::Ipc("main thread did not respond".into()));
                    }
                }
            }
        }
    }

    /// Instantiate, restore state, and activate a plugin on the main thread. Returns the
    /// engine node (wrapped so it comes back to the main thread when retired) and the
    /// descriptor. A live instance of the same device is retired (never dropped while its
    /// node may still be in the engine).
    pub fn instantiate(
        &self,
        instantiate: Instantiate,
        device: DeviceId,
        source: PluginSource,
        state: Option<Vec<u8>>,
        prepare: PrepareConfig,
    ) -> Result<(Box<dyn Node>, DeviceDescriptor), PluginError> {
        let host = self.clone();
        let (token, node, descriptor) = self.call(move || {
            drop(with_registry(|reg| reg.retire(device)));
            // No registry borrow from here on: the plugin may pump a nested run loop.
            let mut controller = instantiate(source.format, &source.path, &source.plugin_id)?;
            if let Some(state) = state.as_deref().filter(|s| !s.is_empty()) {
                controller.load_state(state)?;
            }
            let node = controller.activate(&prepare)?;
            let descriptor = controller.descriptor();
            let (token, old) = with_registry(|reg| {
                let token = reg.next_token;
                reg.next_token += 1;
                reg.instances.insert(
                    token,
                    Hosted {
                        device,
                        controller: Some(controller),
                        retired: false,
                        active: true,
                        returned: None,
                    },
                );
                // A nested call may have created another instance of the device meanwhile:
                // the latest one wins, the other is retired.
                let old = reg.retire(device);
                reg.live.insert(device, token);
                (token, old)
            });
            drop(old);
            Ok::<_, PluginError>((token, node, descriptor))
        })??;
        Ok((
            Box::new(HostedPluginNode::new(
                token,
                node,
                host,
                prepare.max_block_size,
            )),
            descriptor,
        ))
    }

    /// The device was destroyed: drop its controller now if its node is already back,
    /// otherwise when the node returns from the engine.
    pub fn destroy(&self, device: DeviceId) {
        let _ = self.call(move || drop(with_registry(|reg| reg.retire(device))));
    }

    /// Called from [`HostedPluginNode::drop`] (GC thread): deactivate on the main thread.
    fn node_returned(&self, token: u64, node: Box<dyn PluginNode>) {
        self.main.spawn(Box::new(move || {
            // Checked out (a call is running, maybe further up this stack): park the node
            // for the check-in. Unknown instance: the controller is gone, `node` drops here.
            let taken = with_registry(|reg| {
                let Some(h) = reg.instances.get_mut(&token) else {
                    return Err(Some(node));
                };
                match h.controller.take() {
                    Some(c) => Ok((c, node)),
                    None => {
                        h.returned = Some(node);
                        Err(None)
                    }
                }
            });
            if let Ok((mut controller, node)) = taken {
                controller.deactivate(node);
                with_registry(|reg| {
                    if let Some(h) = reg.instances.get_mut(&token) {
                        h.active = false;
                    }
                });
                checkin(token, controller);
            }
        }));
    }

    pub fn open_editor(&self, device: DeviceId) -> Result<(), PluginError> {
        self.call(move || {
            with_live(device, |c| {
                if !c.has_editor() {
                    return Err(PluginError::NoEditor);
                }
                c.open_editor()
            })?
        })?
    }

    pub fn close_editor(&self, device: DeviceId) -> Result<(), PluginError> {
        self.call(move || {
            let _ = with_live(device, |c| c.close_editor());
        })
    }

    pub fn save_state(&self, device: DeviceId) -> Result<Option<Vec<u8>>, PluginError> {
        self.call(move || match with_live(device, |c| c.save_state()) {
            Ok(r) => r.map(Some),
            Err(PluginError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        })?
    }

    pub fn param_value(&self, device: DeviceId, param: ParamId) -> Option<f64> {
        self.call(move || with_live(device, |c| c.param_value(param)).ok().flatten())
            .ok()
            .flatten()
    }

    /// Current plain values of every param of `device`'s live instance (one main-thread
    /// call; used to mirror a restored state into the document).
    pub fn param_values(&self, device: DeviceId) -> Vec<(ParamId, f64)> {
        self.call(move || {
            with_live(device, |c| {
                let params = c.params();
                params
                    .iter()
                    .filter_map(|p| Some((p.id, c.param_value(p.id)?)))
                    .collect()
            })
            .unwrap_or_default()
        })
        .unwrap_or_default()
    }

    /// Poll every live controller (CLAP `on_main_thread`, timers, GUI, AU run loop work)
    /// and collect their notifications. Controllers checked out by a running call are
    /// skipped this time.
    pub fn poll(&self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        if let Ok(notes) = self.call(|| {
            let live: Vec<(u64, DeviceId)> = with_registry(|reg| {
                reg.instances
                    .iter()
                    .filter(|(_, h)| !h.retired && h.controller.is_some())
                    .map(|(t, h)| (*t, h.device))
                    .collect()
            });
            let mut all = Vec::new();
            let mut buf = Vec::new();
            for (token, device) in live {
                if with_instance(token, |c| c.poll(&mut buf)).is_ok() {
                    all.extend(buf.drain(..).map(|n| (device, n)));
                }
            }
            all
        }) {
            out.extend(notes);
        }
    }

    /// Plugin controllers alive on the main thread, incl. retired ones whose node hasn't
    /// come back yet (tests/diagnostics).
    pub fn live_count(&self) -> usize {
        self.call(|| with_registry(|reg| reg.instances.len()))
            .unwrap_or(0)
    }
}

/// Engine node for a hosted plugin: forwards to the plugin's audio half and, when dropped
/// (GC thread), returns it to the main thread for `deactivate`.
///
/// Once the plugin is faulted (sandbox helper crashed/hung, or a fatal in-process error) the
/// node is **bypassed**: the dry input is passed through, delayed by the latency the node
/// reports, so plugin delay compensation keeps the track aligned. It stays bypassed until
/// the device is re-instantiated (`PluginCommand::Reload`).
pub struct HostedPluginNode {
    token: u64,
    node: Option<Box<dyn PluginNode>>,
    host: PluginHost,
    bypass: BypassDelay,
}

/// Pre-allocated delay line used while the plugin is faulted (RT-safe: no allocation).
struct BypassDelay {
    /// One ring per output channel, `cap + 1` samples each.
    rings: Vec<Vec<f32>>,
    /// Longest delay the rings can apply.
    cap: usize,
    write: usize,
}

impl BypassDelay {
    fn new(channels: usize, cap: usize) -> Self {
        Self {
            rings: vec![vec![0.0; cap + 1]; channels],
            cap,
            write: 0,
        }
    }

    fn clear(&mut self) {
        for r in &mut self.rings {
            r.fill(0.0);
        }
        self.write = 0;
    }

    /// Outputs = inputs delayed by `delay` samples (clamped to the capacity).
    fn process(&mut self, delay: usize, audio: &mut AudioBuffers<'_, '_>) {
        let delay = delay.min(self.cap);
        let len = self.cap + 1;
        let frames = audio.outputs.first().map_or(0, |o| o.len());
        for i in 0..frames {
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                let x = audio.inputs.get(c).map_or(0.0, |ch| ch[i]);
                match self.rings.get_mut(c) {
                    Some(ring) => {
                        ring[self.write] = x;
                        out[i] = ring[(self.write + len - delay) % len];
                    }
                    None => out[i] = if delay == 0 { x } else { 0.0 },
                }
            }
            self.write = (self.write + 1) % len;
        }
    }
}

impl HostedPluginNode {
    fn new(token: u64, node: Box<dyn PluginNode>, host: PluginHost, max_block: usize) -> Self {
        let channels = usize::from(node.channels().1).max(2);
        // Room for the reported latency plus a block of headroom for later increases.
        let cap = node.latency() as usize + max_block.max(1);
        Self {
            token,
            node: Some(node),
            host,
            bypass: BypassDelay::new(channels, cap),
        }
    }

    /// The plugin faulted: the node is bypassed.
    pub fn is_bypassed(&self) -> bool {
        self.inner().is_faulted()
    }

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
        self.bypass.clear();
        self.inner_mut().reset();
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        if self.inner().is_faulted() {
            let delay = self.inner().latency() as usize;
            self.bypass.process(delay, audio);
            return ProcessStatus::Continue;
        }
        self.inner_mut().process(ctx, audio)
    }
    fn sidechain_inputs(&self) -> u16 {
        self.inner().sidechain_inputs()
    }
    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        if self.inner().is_faulted() {
            let delay = self.inner().latency() as usize;
            self.bypass.process(delay, audio);
            return ProcessStatus::Continue;
        }
        self.inner_mut().process_sidechain(ctx, audio, sidechain)
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
            self.host.node_returned(self.token, node);
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

    /// The first plugin with id `plugin_id`, whatever its format. Ids are only unique per
    /// format: to load a plugin use [`PluginCatalog::find_format`].
    pub fn find(&self, plugin_id: &str) -> Option<PluginDescriptor> {
        self.inner
            .lock()
            .ok()?
            .iter()
            .find(|p| p.id == plugin_id)
            .cloned()
    }

    /// The plugin `(format, plugin_id)` (`PluginInstance.format` / `.plugin_id`).
    pub fn find_format(&self, format: PluginFormat, plugin_id: &str) -> Option<PluginDescriptor> {
        self.inner
            .lock()
            .ok()?
            .iter()
            .find(|p| p.format == format && p.id == plugin_id)
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
    /// State blobs of deactivated instances (identifies which instance was deactivated).
    pub static DEACTIVATED_STATES: std::sync::Mutex<Vec<Vec<u8>>> =
        std::sync::Mutex::new(Vec::new());

    pub struct FakeController {
        pub state: Vec<u8>,
        pub main_thread: std::thread::ThreadId,
        pub max_block: usize,
        /// Run inside `save_state` (tests of nested run loops).
        pub on_save: Option<Box<dyn FnMut()>>,
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
            layout: None,
            device_type: DeviceTypeRef::Plugin {
                plugin_id: "fake".into(),
            },
            name: "Fake".into(),
            category: DeviceCategory::AudioEffect,
            params: Vec::new(),
            audio_inputs: 2,
            audio_outputs: 2,
            midi_input: false,
            sidechain_inputs: 0,
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
            DEACTIVATED_STATES.lock().unwrap().push(self.state.clone());
        }
        fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
            if let Some(f) = &mut self.on_save {
                f();
            }
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
        Arc::new(|_format, _path, id| {
            if id == "missing" {
                return Err(PluginError::NotFound(id.into()));
            }
            Ok(Box::new(FakeController {
                state: Vec::new(),
                main_thread: std::thread::current().id(),
                max_block: 0,
                on_save: None,
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

    fn src(id: &str) -> PluginSource {
        PluginSource {
            format: PluginFormat::Clap,
            path: PathBuf::from("/x.clap"),
            plugin_id: id.into(),
        }
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
                src("fake"),
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
        assert!(fake::DEACTIVATED.load(Ordering::SeqCst) > before);

        let err = host
            .instantiate(
                fake::instantiate(),
                device(2),
                src("missing"),
                None,
                prepare,
            )
            .err()
            .unwrap();
        assert!(matches!(err, PluginError::NotFound(_)));
    }

    #[test]
    fn replacing_a_device_keeps_old_instance_until_its_node_returns() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let prepare = PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 128,
            max_events_per_block: 64,
        };
        let d = device(10);
        let make = |state: &[u8]| {
            host.instantiate(
                fake::instantiate(),
                d,
                src("fake"),
                Some(state.to_vec()),
                prepare,
            )
            .unwrap()
            .0
        };
        let deactivated = |s: &[u8]| {
            fake::DEACTIVATED_STATES
                .lock()
                .unwrap()
                .iter()
                .any(|x| x == s)
        };
        let old = make(b"replace-v1");
        let new = make(b"replace-v2");
        // Old instance retired but alive (its node is still "in the engine").
        assert_eq!(host.live_count(), 2);
        assert_eq!(
            host.save_state(d).unwrap().as_deref(),
            Some(&b"replace-v2"[..])
        );
        // The old node returns: the OLD controller is deactivated and dropped.
        drop(old);
        assert_eq!(host.live_count(), 1);
        assert!(deactivated(b"replace-v1"));
        assert!(!deactivated(b"replace-v2"));
        assert_eq!(
            host.save_state(d).unwrap().as_deref(),
            Some(&b"replace-v2"[..])
        );
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert_eq!(notes.len(), 1);
        // Then the new one goes away normally.
        host.destroy(d);
        drop(new);
        assert_eq!(host.live_count(), 0);
        assert!(deactivated(b"replace-v2"));
    }

    #[test]
    fn calls_fail_fast_after_close() {
        /// A main thread that never runs anything (e.g. blocked joining the host) but
        /// keeps the queued closures alive, so a call really stays pending.
        #[derive(Default)]
        struct Stuck(std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>>);
        impl MainThread for Stuck {
            fn spawn(&self, f: Box<dyn FnOnce() + Send>) {
                self.0.lock().unwrap().push(f);
            }
        }
        let stuck = Arc::new(Stuck::default());
        let host = PluginHost::new(stuck.clone());
        let h2 = host.clone();
        let t = std::thread::spawn(move || {
            let r = h2.save_state(device(11));
            (r, std::time::Instant::now())
        });
        std::thread::sleep(Duration::from_millis(150));
        // Still pending: the closure is queued, not run, not dropped.
        assert!(!t.is_finished());
        assert_eq!(stuck.0.lock().unwrap().len(), 1);
        let closed_at = std::time::Instant::now();
        host.close();
        let (r, returned_at) = t.join().unwrap();
        match r {
            Err(PluginError::Ipc(msg)) => assert_eq!(msg, "host is shutting down"),
            other => panic!("{other:?}"),
        }
        let after_close = returned_at.duration_since(closed_at);
        assert!(after_close < Duration::from_millis(500), "{after_close:?}");
        // New calls fail immediately without queueing anything.
        let start = std::time::Instant::now();
        match host.save_state(device(11)) {
            Err(PluginError::Ipc(msg)) => assert_eq!(msg, "host is shutting down"),
            other => panic!("{other:?}"),
        }
        assert!(start.elapsed() < Duration::from_millis(50));
        assert_eq!(stuck.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn bypass_delays_the_dry_signal() {
        let run = |d: &mut BypassDelay, delay: usize, input: &[f32]| {
            let mut out = vec![9.0f32; input.len()];
            let inputs: [&[f32]; 1] = [input];
            let mut outputs: [&mut [f32]; 1] = [&mut out];
            let mut audio = AudioBuffers {
                inputs: &inputs,
                outputs: &mut outputs,
            };
            d.process(delay, &mut audio);
            out
        };
        let mut d = BypassDelay::new(2, 4);
        assert_eq!(run(&mut d, 0, &[1.0, 2.0]), [1.0, 2.0]);
        // Delay 3 across block boundaries.
        let mut d = BypassDelay::new(2, 4);
        assert_eq!(run(&mut d, 3, &[1.0, 0.0]), [0.0, 0.0]);
        assert_eq!(run(&mut d, 3, &[0.0, 0.0]), [0.0, 1.0]);
        // Longer than the capacity: clamped.
        let mut d = BypassDelay::new(1, 2);
        assert_eq!(run(&mut d, 10, &[1.0, 0.0, 0.0, 0.0]), [0.0, 0.0, 1.0, 0.0]);
        d.clear();
        assert_eq!(run(&mut d, 2, &[0.0, 0.0, 0.0]), [0.0; 3]);
    }

    /// A main thread whose queued jobs can also be run from inside a job ([`pump`]), like
    /// Tauri's `run_on_main_thread` jobs running inside an AU's nested `CFRunLoopRunInMode`.
    type Job = Box<dyn FnOnce() + Send>;

    struct Pumping {
        tx: crossbeam_channel::Sender<Job>,
    }

    thread_local! {
        static PUMP_RX: RefCell<Option<crossbeam_channel::Receiver<Job>>> =
            const { RefCell::new(None) };
    }

    impl Pumping {
        fn new() -> Self {
            let (tx, rx) = unbounded::<Job>();
            std::thread::spawn(move || {
                PUMP_RX.with(|r| *r.borrow_mut() = Some(rx.clone()));
                while let Ok(f) = rx.recv() {
                    f();
                }
            });
            Self { tx }
        }
    }

    impl MainThread for Pumping {
        fn spawn(&self, f: Box<dyn FnOnce() + Send>) {
            let _ = self.tx.send(f);
        }
    }

    /// Nested "run loop": run queued main-thread jobs until `done` (or a timeout).
    fn pump(done: &AtomicBool) {
        let rx = PUMP_RX
            .with(|r| r.borrow().clone())
            .expect("on the pumping thread");
        let start = Instant::now();
        while !done.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(10) {
            if let Ok(f) = rx.recv_timeout(Duration::from_millis(5)) {
                f();
            }
        }
    }

    #[test]
    fn registry_is_not_borrowed_across_plugin_calls() {
        let host = PluginHost::new(Arc::new(Pumping::new()));
        let prepare = PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 64,
            max_events_per_block: 64,
        };
        let a = device(20);
        let b = device(21);
        let (node_a, _) = host
            .instantiate(
                fake::instantiate(),
                a,
                src("fake"),
                Some(b"a".to_vec()),
                prepare,
            )
            .unwrap();

        // While B instantiates (and pumps a nested run loop), other registry calls and A's
        // returning node run inside it.
        let entered = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (e2, d2) = (entered.clone(), done.clone());
        let nested: Instantiate = Arc::new(move |_, _, _| {
            e2.store(true, Ordering::SeqCst);
            pump(&d2);
            Ok(Box::new(fake::FakeController {
                state: Vec::new(),
                main_thread: std::thread::current().id(),
                max_block: 0,
                on_save: None,
            }) as Box<dyn PluginController>)
        });
        let h2 = host.clone();
        let (e3, d3) = (entered.clone(), done.clone());
        let other = std::thread::spawn(move || {
            while !e3.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            let mut notes = Vec::new();
            h2.poll(&mut notes);
            let state = h2.save_state(a).unwrap();
            let b_state = h2.save_state(b).unwrap();
            drop(node_a); // back to the main thread → deactivated inside the nested loop
            h2.destroy(a);
            let count = h2.live_count();
            d3.store(true, Ordering::SeqCst);
            (notes, state, b_state, count)
        });
        let (node_b, _) = host
            .instantiate(nested, b, src("fake"), None, prepare)
            .unwrap();
        let (notes, state, b_state, count) = other.join().unwrap();
        assert_eq!(notes, vec![(a, PluginNotification::StateDirty)]);
        assert_eq!(state.as_deref(), Some(&b"a"[..]));
        assert_eq!(b_state, None, "B is not live until its instantiate returns");
        assert_eq!(count, 0, "A deactivated and dropped inside the nested loop");
        assert_eq!(host.live_count(), 1);

        // A call running a nested loop keeps its controller checked out: a node returning
        // meanwhile is parked and deactivated at check-in; other calls see it busy.
        let c = device(22);
        let done = Arc::new(AtomicBool::new(false));
        let d2 = done.clone();
        let pumping_save: Instantiate = Arc::new(move |_, _, _| {
            let d = d2.clone();
            Ok(Box::new(fake::FakeController {
                state: b"pump-save".to_vec(),
                main_thread: std::thread::current().id(),
                max_block: 0,
                on_save: Some(Box::new(move || pump(&d))),
            }) as Box<dyn PluginController>)
        });
        let (node_c, _) = host
            .instantiate(
                pumping_save,
                c,
                src("fake"),
                Some(b"pump-save".to_vec()),
                prepare,
            )
            .unwrap();
        let h2 = host.clone();
        let d3 = done.clone();
        let saver = std::thread::spawn(move || h2.save_state(c));
        std::thread::sleep(Duration::from_millis(50));
        // Inside C's save_state: C is busy, B still answers.
        assert!(matches!(host.save_state(c), Err(PluginError::Ipc(m)) if m.contains("busy")));
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert!(notes.iter().all(|(d, _)| *d == b), "{notes:?}");
        host.destroy(c);
        let before = fake::DEACTIVATED_STATES
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.as_slice() == b"pump-save")
            .count();
        drop(node_c);
        assert_eq!(
            host.live_count(),
            2,
            "C parked, not dropped, while checked out"
        );
        d3.store(true, Ordering::SeqCst);
        assert_eq!(
            saver.join().unwrap().unwrap().as_deref(),
            Some(&b"pump-save"[..])
        );
        assert_eq!(
            host.live_count(),
            1,
            "C deactivated and dropped at check-in"
        );
        let after = fake::DEACTIVATED_STATES
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.as_slice() == b"pump-save")
            .count();
        assert_eq!(after, before + 1);
        host.destroy(b);
        drop(node_b);
        assert_eq!(host.live_count(), 0);
    }

    #[test]
    fn catalog_finds_by_format() {
        let cat = PluginCatalog::default();
        let desc = |format, path: &str| PluginDescriptor {
            sidechain_inputs: Default::default(),
            format,
            id: "same.id".into(),
            name: "Y".into(),
            vendor: "X".into(),
            version: "1".into(),
            description: String::new(),
            features: vec![],
            category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
            path: path.into(),
        };
        cat.replace(vec![
            desc(PluginFormat::Clap, "/p/y.clap"),
            desc(PluginFormat::Vst3, "/p/y.vst3"),
        ]);
        assert_eq!(
            cat.find_format(PluginFormat::Vst3, "same.id").unwrap().path,
            "/p/y.vst3"
        );
        assert_eq!(
            cat.find_format(PluginFormat::Clap, "same.id").unwrap().path,
            "/p/y.clap"
        );
        assert!(cat.find_format(PluginFormat::Au, "same.id").is_none());
        assert_eq!(format_label(PluginFormat::Vst3), "VST3");
    }

    #[test]
    fn catalog_persists() {
        let tmp = crate::test_util::TempDir::new("catalog");
        let cat = PluginCatalog::open(tmp.path());
        assert!(cat.list().is_empty());
        cat.replace(vec![PluginDescriptor {
            sidechain_inputs: Default::default(),
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
