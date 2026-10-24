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
use ether_core::plugin::{
    NativeWindow, PluginController, PluginError, PluginNode, PluginNotification,
};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{DeviceId, ParamId, PluginFormat, PluginInstance};
use ether_core::protocol::plugins::{PluginDescriptor, ScanFailure};
use ether_core::{AudioBuffers, Device, Node, PrepareConfig, ProcessContext, ProcessStatus};
use ether_plugin_host::Formats;

use crate::health::{Health, Work};

/// Runs closures on the thread that owns plugin controllers.
pub trait MainThread: Send + Sync {
    /// Queue `f` to run on the main thread. Must not run it inline on the caller's thread
    /// unless the caller *is* the main thread.
    fn spawn(&self, f: Box<dyn FnOnce() + Send>);

    /// Run the queued work now if the caller is the main thread, which is blocked waiting
    /// for something that needs it (the host saving on quit while `NativeHost::shutdown`
    /// runs on the main thread). Default: nothing (the main thread serves its own
    /// queue).
    fn run_pending(&self) {}
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

/// Windows: a dedicated thread with its own message loop for plugin main-thread work and
/// editors ([`ether_plugin_host::ui_thread::UiThread`]), so plugins never block the app's
/// window and webview (the desktop app's default there).
#[cfg(windows)]
pub struct PluginUiThread(ether_plugin_host::ui_thread::UiThread);

#[cfg(windows)]
impl PluginUiThread {
    pub fn new() -> std::io::Result<Self> {
        ether_plugin_host::ui_thread::UiThread::new().map(Self)
    }
}

#[cfg(windows)]
impl MainThread for PluginUiThread {
    fn spawn(&self, f: Box<dyn FnOnce() + Send>) {
        self.0.spawn(f);
    }
}

/// Every plugin format the native host loads: CLAP, VST3 and AU (AU is `Unsupported`
/// off macOS), then an extension's (`ether_plugin_host::set_extra_formats`). A format whose
/// `plugins-*` feature is off is not registered, so it is `Unsupported` ("not supported by
/// this build").
pub fn formats() -> Formats {
    let hosts: Vec<Arc<dyn ether_plugin_host::PluginFormatHost>> = vec![
        #[cfg(feature = "plugins-clap")]
        Arc::new(ether_clap::ClapFormat),
        #[cfg(feature = "plugins-vst3")]
        Arc::new(ether_vst3::Vst3Format),
        #[cfg(feature = "plugins-au")]
        Arc::new(ether_au::AuFormat),
    ];
    Formats::with_extras(hosts)
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
        PluginFormat::Vst2 => "VST2",
        PluginFormat::Webclap => "WebCLAP",
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
    /// Its editor was opened (`open_editor` succeeded) and not closed since (`close_editor`,
    /// or the plugin reporting `EditorClosed`). An instance replaced with its editor open
    /// hands the editor over to its replacement (see [`PluginHost::instantiate`]).
    editor_open: bool,
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
    /// Notifications raised outside a plugin's own poll (an editor closed because its
    /// instance was replaced or destroyed), delivered with the next poll.
    notes: Vec<(DeviceId, PluginNotification)>,
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

/// Record whether the editor of `device`'s live instance is open.
fn set_editor_open(device: DeviceId, open: bool) {
    with_registry(|reg| {
        if let Some(h) = reg
            .live
            .get(&device)
            .copied()
            .and_then(|t| reg.instances.get_mut(&t))
        {
            h.editor_open = open;
        }
    });
}

/// Main thread, before `device`'s live instance is retired: close its editor if it is open
/// (a retired instance is no longer polled, so its window would linger, unclosable, until
/// its node comes back) and report whether it was. The UI still shows it open: the caller
/// reopens it on the replacement or reports `EditorClosed`.
fn close_live_editor(device: DeviceId) -> bool {
    let Some(token) = with_registry(|reg| {
        let token = reg.live.get(&device).copied()?;
        reg.instances
            .get(&token)
            .is_some_and(|h| h.editor_open)
            .then_some(token)
    }) else {
        return false;
    };
    // Checked out by a running call: the editor closes when the retired controller drops.
    let _ = with_instance(token, |c| c.close_editor());
    with_registry(|reg| {
        if let Some(h) = reg.instances.get_mut(&token) {
            h.editor_open = false;
        }
    });
    true
}

/// Queue a notification for `device` (delivered by the next poll); `queued` is the host's
/// [`PluginHost::has_queued_notes`] flag.
fn queue_note(queued: &AtomicBool, device: DeviceId, note: PluginNotification) {
    with_registry(|reg| reg.notes.push((device, note)));
    queued.store(true, Ordering::SeqCst);
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

/// How long [`PluginHost::poll_nonblocking`] waits for the main thread before leaving the
/// poll in flight.
const POLL_WAIT: Duration = Duration::from_millis(2);

/// Main thread: poll every live controller and the GUI mirrors.
fn poll_main(
    health: &Health,
    queued: &AtomicBool,
    windows: &EditorWindows,
) -> Vec<(DeviceId, PluginNotification)> {
    queued.store(false, Ordering::SeqCst);
    let live: Vec<(u64, DeviceId)> = with_registry(|reg| {
        reg.instances
            .iter()
            .filter(|(_, h)| !h.retired && h.controller.is_some())
            .map(|(t, h)| (*t, h.device))
            .collect()
    });
    let mut all = Vec::new();
    let mut buf = Vec::new();
    let mut polled = HashMap::new();
    for (token, device) in live {
        health.set_main_device(Some(device));
        let started = Instant::now();
        let window = with_instance(token, |c| {
            c.poll(&mut buf);
            c.editor_window()
        });
        if let Ok(window) = window {
            polled.insert(device, window);
            if buf.iter().any(|n| {
                matches!(
                    n,
                    PluginNotification::EditorClosed | PluginNotification::Crashed { .. }
                )
            }) {
                with_registry(|reg| {
                    if let Some(h) = reg.instances.get_mut(&token) {
                        h.editor_open = false;
                    }
                });
            }
            all.extend(buf.drain(..).map(|n| (device, n)));
        }
        log_slow(health, Work::PollPlugins, Some(device), started.elapsed());
    }
    health.set_main_device(None);
    windows.refresh(polled);
    all.extend(with_registry(|reg| std::mem::take(&mut reg.notes)));
    crate::plugin_mirror::poll_main(&mut all);
    all
}

/// A main-thread plugin call longer than this is logged: on desktop the main thread is
/// also the UI's (window messages, webview IPC), so it is time the UI was frozen.
const SLOW_MAIN_CALL: Duration = Duration::from_millis(50);

fn log_slow(health: &Health, work: Work, device: Option<DeviceId>, took: Duration) {
    if took >= SLOW_MAIN_CALL {
        tracing::warn!(
            work = work.label(),
            plugin = health.device_label(device).as_deref().unwrap_or("-"),
            ms = took.as_millis() as u64,
            "slow plugin call on the main thread"
        );
    }
}

/// The open editors' native windows by device ([`PluginHost::editor_window_for`]): set
/// when an editor opens, refreshed by every poll (a floating editor's window may be found
/// a few polls late), dropped when it closes or its device goes. Live instances only: a
/// listening site's GUI mirrors (`plugin_mirror`) aren't streamed.
#[derive(Clone, Default)]
struct EditorWindows(Arc<Mutex<HashMap<DeviceId, NativeWindow>>>);

impl EditorWindows {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DeviceId, NativeWindow>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn get(&self, device: DeviceId) -> Option<NativeWindow> {
        self.lock().get(&device).copied()
    }

    fn set(&self, device: DeviceId, window: Option<NativeWindow>) {
        let mut map = self.lock();
        match window {
            Some(w) => map.insert(device, w),
            None => map.remove(&device),
        };
    }

    /// After a poll: every polled device's current window. Devices that weren't polled
    /// (checked out by a running call) keep theirs; the others' entries go (their instance
    /// is gone: destroyed or retired).
    fn refresh(&self, polled: HashMap<DeviceId, Option<NativeWindow>>) {
        let live: Vec<DeviceId> = with_registry(|reg| reg.live.keys().copied().collect());
        let mut map = self.lock();
        map.retain(|d, _| !polled.contains_key(d) && live.contains(d));
        map.extend(polled.into_iter().filter_map(|(d, w)| Some((d, w?))));
    }
}

/// The notifications of a poll queued on the main thread.
type PollResult = crossbeam_channel::Receiver<Vec<(DeviceId, PluginNotification)>>;

/// Handle to the main-thread plugin registry. Cheap to clone.
#[derive(Clone)]
pub struct PluginHost {
    main: Arc<dyn MainThread>,
    timeout: Duration,
    /// Set at shutdown: main-thread calls fail fast (the main thread may be busy joining
    /// the host and never run them).
    closing: Arc<AtomicBool>,
    /// What the main thread runs and what the caller waits for (`crate::health`).
    health: Arc<Health>,
    /// The poll still running on the main thread ([`Self::poll_nonblocking`]).
    poll_pending: Arc<Mutex<Option<PollResult>>>,
    /// Notifications wait in the registry for the next poll ([`Self::has_queued_notes`]).
    notes_queued: Arc<AtomicBool>,
    editor_windows: EditorWindows,
    always: AlwaysSwitches,
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
    /// The instance's "always process" switch at creation (`PluginInstance::always_process`;
    /// later changes: [`PluginHost::set_always_process`]).
    pub always_process: bool,
}

/// The "always process" switches of the live plugin nodes, by registry device id
/// (`PluginNode::always_process`).
type AlwaysSwitches = Arc<Mutex<HashMap<DeviceId, Arc<AtomicBool>>>>;

impl PluginHost {
    pub fn new(main: Arc<dyn MainThread>) -> Self {
        Self {
            main,
            timeout: Duration::from_secs(30),
            closing: Arc::new(AtomicBool::new(false)),
            health: Arc::default(),
            poll_pending: Arc::default(),
            notes_queued: Arc::default(),
            editor_windows: EditorWindows::default(),
            always: Arc::default(),
        }
    }

    /// Flip the "always process" switch of `device`'s live node (CONTRACTS.md §13.19).
    /// Any thread; no plugin call.
    pub fn set_always_process(&self, device: DeviceId, on: bool) {
        let switches = self.always.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(switch) = switches.get(&device) {
            switch.store(on, Ordering::Relaxed);
        }
    }

    /// Activity markers of the main-thread calls (watched by `crate::health::Watchdog`).
    pub fn health(&self) -> &Arc<Health> {
        &self.health
    }

    /// [`MainThread::run_pending`]: serve queued main-thread calls if this is the main
    /// thread.
    pub fn run_pending_main(&self) {
        self.main.run_pending();
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
        self.call_as(Work::PluginCall, None, f)
    }

    /// [`Self::call`], recorded in [`Self::health`] as `work` (for `device` when known), so
    /// a stall can say what the main thread is busy with.
    pub(crate) fn call_as<R: Send + 'static>(
        &self,
        work: Work,
        device: Option<DeviceId>,
        f: impl FnOnce() -> R + Send + 'static,
    ) -> Result<R, PluginError> {
        let closed = || PluginError::Ipc("host is shutting down".into());
        if self.closing.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let (tx, rx) = bounded(1);
        let _waiting = self.health.waiting_for_main(work);
        let health = self.health.clone();
        self.main.spawn(Box::new(move || {
            let _running = health.main_call(work, device);
            let started = Instant::now();
            let r = f();
            log_slow(&health, work, device, started.elapsed());
            let _ = tx.send(r);
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
        let health = self.health.clone();
        let queued = self.notes_queued.clone();
        let windows = self.editor_windows.clone();
        let always = self.always.clone();
        if let Some(name) = source.path.file_stem().and_then(|n| n.to_str()) {
            health.name_device(device, name);
        }
        // Set when this call gives up (timeout, shutdown): a load that finishes later on the
        // main thread then drops its instance instead of registering it.
        let abandoned = Arc::new(AtomicBool::new(false));
        let gave_up = abandoned.clone();
        let loaded = self.call_as(Work::LoadPlugin, Some(device), move || {
            // The replaced instance's open editor is reopened on the new one (a reload
            // keeps the plugin's window up), or reported closed if that fails.
            let reopen = close_live_editor(device);
            drop(with_registry(|reg| reg.retire(device)));
            let created = (|| {
                // No registry borrow from here on: the plugin may pump a nested run loop.
                let mut controller = instantiate(source.format, &source.path, &source.plugin_id)?;
                if let Some(state) = state.as_deref().filter(|s| !s.is_empty()) {
                    controller.load_state(state)?;
                }
                let node = controller.activate(&prepare)?;
                let mut switches = always.lock().unwrap_or_else(|e| e.into_inner());
                match node.always_process() {
                    Some(switch) => {
                        switch.store(source.always_process, Ordering::Relaxed);
                        switches.insert(device, switch);
                    }
                    None => {
                        switches.remove(&device);
                    }
                }
                drop(switches);
                Ok::<_, PluginError>((controller, node))
            })();
            let (mut controller, node) = match created {
                Ok(c) => c,
                Err(e) => {
                    if reopen {
                        queue_note(&queued, device, PluginNotification::EditorClosed);
                    }
                    return Err(e);
                }
            };
            if abandoned.load(Ordering::SeqCst) {
                // The caller timed out and reported the load failed: nobody will put
                // this node in the engine, so don't register an orphan live instance.
                controller.deactivate(node);
                drop(controller);
                if reopen {
                    queue_note(&queued, device, PluginNotification::EditorClosed);
                }
                return Err(PluginError::Ipc("load abandoned (timed out)".into()));
            }
            let descriptor = controller.descriptor();
            health.name_device(device, &descriptor.name);
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
                        editor_open: false,
                    },
                );
                // A nested call may have created another instance of the device meanwhile:
                // the latest one wins, the other is retired.
                let old = reg.retire(device);
                reg.live.insert(device, token);
                (token, old)
            });
            drop(old);
            windows.set(device, None);
            if reopen {
                let reopened = with_instance(token, |c| {
                    if c.has_editor() {
                        c.open_editor().map(|()| c.editor_window())
                    } else {
                        Err(PluginError::NoEditor)
                    }
                });
                match reopened {
                    Ok(Ok(window)) => {
                        windows.set(device, window);
                        with_registry(|reg| {
                            if let Some(h) = reg.instances.get_mut(&token) {
                                h.editor_open = true;
                            }
                        })
                    }
                    _ => queue_note(&queued, device, PluginNotification::EditorClosed),
                }
            }
            // If the caller gave up between the check above and now, `Loaded` is dropped
            // undelivered and cleans up after itself.
            Ok::<_, PluginError>(Loaded {
                device,
                loaded: Some((
                    HostedPluginNode::new(token, node, host, &prepare),
                    descriptor,
                )),
            })
        });
        let loaded = match loaded {
            Ok(r) => r?,
            Err(e) => {
                gave_up.store(true, Ordering::SeqCst);
                return Err(e);
            }
        };
        Ok(loaded.deliver())
    }

    /// The device was destroyed: drop its controller now if its node is already back,
    /// otherwise when the node returns from the engine.
    pub fn destroy(&self, device: DeviceId) {
        self.editor_windows.set(device, None);
        self.always
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&device);
        let queued = self.notes_queued.clone();
        let _ = self.call_as(Work::UnloadPlugin, Some(device), move || {
            if close_live_editor(device) {
                queue_note(&queued, device, PluginNotification::EditorClosed);
            }
            drop(with_registry(|reg| reg.retire(device)))
        });
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

    /// Open the device's editor: its live instance, else its GUI mirror
    /// ([`crate::plugin_mirror`]; a listening site's plugins).
    pub fn open_editor(&self, device: DeviceId) -> Result<(), PluginError> {
        let windows = self.editor_windows.clone();
        self.call_as(Work::OpenEditor, Some(device), move || {
            match with_live(device, |c| {
                if !c.has_editor() {
                    return Err(PluginError::NoEditor);
                }
                c.open_editor().map(|()| c.editor_window())
            }) {
                Ok(r) => r.map(|window| {
                    windows.set(device, window);
                    set_editor_open(device, true);
                }),
                // No live instance: a listening site's GUI mirror, else `NotFound` (never a
                // silent success: the UI only shows the editor open on `Ok`).
                Err(PluginError::NotFound(_)) => crate::plugin_mirror::open_editor_main(device),
                Err(e) => Err(e),
            }
        })?
    }

    pub fn close_editor(&self, device: DeviceId) -> Result<(), PluginError> {
        self.editor_windows.set(device, None);
        self.call_as(Work::CloseEditor, Some(device), move || {
            match with_live(device, |c| c.close_editor()) {
                Ok(()) => set_editor_open(device, false),
                Err(PluginError::NotFound(_)) => crate::plugin_mirror::close_editor_main(device),
                Err(_) => {}
            }
        })
    }

    pub fn save_state(&self, device: DeviceId) -> Result<Option<Vec<u8>>, PluginError> {
        self.call_as(Work::SaveState, Some(device), move || {
            match with_live(device, |c| c.save_state()) {
                Ok(r) => r.map(Some),
                Err(PluginError::NotFound(_)) => Ok(None),
                Err(e) => Err(e),
            }
        })?
    }

    /// Load `state` into `device`'s running instance in place (collab: a peer's preset).
    /// Its editor stays open and its node keeps processing. `Ok(false)`: no live instance.
    pub fn load_state_live(&self, device: DeviceId, state: Vec<u8>) -> Result<bool, PluginError> {
        self.call_as(Work::PluginCall, Some(device), move || {
            match with_live(device, |c| c.load_state(&state)) {
                Ok(r) => r.map(|()| true),
                Err(PluginError::NotFound(_)) => Ok(false),
                Err(e) => Err(e),
            }
        })?
    }

    pub fn param_value(&self, device: DeviceId, param: ParamId) -> Option<f64> {
        self.call(move || with_live(device, |c| c.param_value(param)).ok().flatten())
            .ok()
            .flatten()
    }

    /// The plugin's display texts for plain `values` of `device`'s params
    /// (`PluginController::param_text`, one main-thread call): its live instance, else its
    /// GUI mirror. One entry per value; all `None` without an instance.
    pub fn param_texts(
        &self,
        device: DeviceId,
        values: Vec<(ParamId, f64)>,
    ) -> Vec<Option<String>> {
        let n = values.len();
        self.call_as(Work::ReadParams, Some(device), move || {
            let texts = |c: &mut dyn PluginController| -> Vec<Option<String>> {
                values.iter().map(|&(p, v)| c.param_text(p, v)).collect()
            };
            match with_live(device, texts) {
                Ok(t) => Some(t),
                Err(PluginError::NotFound(_)) => {
                    crate::plugin_mirror::param_texts_main(device, &values)
                }
                Err(_) => None,
            }
        })
        .ok()
        .flatten()
        .unwrap_or_else(|| vec![None; n])
    }

    /// Current plain values of every param of `device`'s live instance (one main-thread
    /// call; used to mirror a restored state into the document).
    pub fn param_values(&self, device: DeviceId) -> Vec<(ParamId, f64)> {
        self.call_as(Work::ReadParams, Some(device), move || {
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
        let health = self.health.clone();
        let queued = self.notes_queued.clone();
        let windows = self.editor_windows.clone();
        if let Ok(notes) = self.call_as(Work::PollPlugins, None, move || {
            poll_main(&health, &queued, &windows)
        }) {
            out.extend(notes);
        }
    }

    /// [`Self::poll`] without making the controller thread wait for a busy main thread:
    /// the poll is queued on the main thread, waited for at most [`POLL_WAIT`], and if it
    /// hasn't run by then its notifications are collected by a later call (one poll in
    /// flight at a time). So a main thread busy with the UI, an editor or a slow plugin no
    /// longer holds up the controller's tick (playhead, meters, commands).
    pub fn poll_nonblocking(&self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        let Ok(mut pending) = self.poll_pending.lock() else {
            return;
        };
        let rx = match pending.take() {
            Some(rx) => rx,
            None => {
                if self.closing.load(Ordering::SeqCst) {
                    return;
                }
                let (tx, rx) = bounded(1);
                let health = self.health.clone();
                let queued = self.notes_queued.clone();
                let windows = self.editor_windows.clone();
                self.main.spawn(Box::new(move || {
                    let _running = health.main_call(Work::PollPlugins, None);
                    let _ = tx.send(poll_main(&health, &queued, &windows));
                }));
                rx
            }
        };
        let _waiting = self.health.waiting_for_main(Work::PollPlugins);
        match rx.recv_timeout(POLL_WAIT) {
            Ok(notes) => out.extend(notes),
            Err(RecvTimeoutError::Timeout) => *pending = Some(rx),
            Err(RecvTimeoutError::Disconnected) => {}
        }
    }

    /// The native window of `device`'s open editor (for capturing it and injecting input
    /// into it), from any thread without waiting for the main thread: as of its
    /// opening or the last poll. `None` without an open editor, or while a floating
    /// editor's window isn't found (yet).
    pub fn editor_window_for(&self, device: DeviceId) -> Option<NativeWindow> {
        self.editor_windows.get(device)
    }

    /// Notifications are waiting for a poll although no plugin may be left to poll (an
    /// editor closed with its destroyed instance).
    pub fn has_queued_notes(&self) -> bool {
        self.notes_queued.load(Ordering::SeqCst)
    }

    /// Plugin controllers alive on the main thread, incl. retired ones whose node hasn't
    /// come back yet (tests/diagnostics).
    pub fn live_count(&self) -> usize {
        self.call(|| with_registry(|reg| reg.instances.len()))
            .unwrap_or(0)
    }
}

/// A loaded instance on its way from the main thread to the caller of
/// [`PluginHost::instantiate`]. Dropped undelivered (the caller timed out while the load
/// was finishing), it retires the instance and returns its node, so the controller is
/// deactivated and dropped instead of lingering as the device's live instance.
struct Loaded {
    device: DeviceId,
    /// `None` once delivered.
    loaded: Option<(HostedPluginNode, DeviceDescriptor)>,
}

impl Loaded {
    fn deliver(mut self) -> (Box<dyn Node>, DeviceDescriptor) {
        let (node, descriptor) = self.loaded.take().expect("delivered once");
        (Box::new(node), descriptor)
    }
}

impl Drop for Loaded {
    fn drop(&mut self) {
        let Some((node, _)) = self.loaded.take() else {
            return;
        };
        let (device, token) = (self.device, node.token);
        let queued = node.host.notes_queued.clone();
        // Queued before the node's own return (`HostedPluginNode::drop`), so it is
        // deactivated as a retired instance and dropped.
        node.host.main.spawn(Box::new(move || {
            if with_registry(|reg| reg.live.get(&device) == Some(&token)) {
                if close_live_editor(device) {
                    queue_note(&queued, device, PluginNotification::EditorClosed);
                }
                drop(with_registry(|reg| reg.retire(device)));
            }
        }));
        drop(node);
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
    /// Most latency the plugin may report ([`ether_core::graph::max_latency_samples`]):
    /// what it reports beyond is clamped (issue a plugin answering `-1` must not
    /// size a multi-GB delay line or wrap the engine's PDC sums).
    max_latency: u32,
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
    fn new(
        token: u64,
        node: Box<dyn PluginNode>,
        host: PluginHost,
        prepare: &PrepareConfig,
    ) -> Self {
        let channels = usize::from(node.channels().1).max(2);
        let mut hosted = Self {
            token,
            node: Some(node),
            host,
            bypass: BypassDelay::new(0, 0),
            max_latency: plugin_latency_cap(prepare.sample_rate),
        };
        let reported = hosted.inner().latency();
        if reported > hosted.max_latency {
            tracing::warn!(
                plugin = %hosted.inner().descriptor().name,
                reported,
                clamped = hosted.max_latency,
                "plugin reported an implausible latency; clamped to the delay-compensation limit"
            );
        }
        // Room for the (clamped) reported latency plus a block of headroom for later
        // increases.
        let cap = hosted.latency() as usize + prepare.max_block_size.max(1);
        hosted.bypass = BypassDelay::new(channels, cap);
        hosted
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
            let delay = self.latency() as usize;
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
            let delay = self.latency() as usize;
            self.bypass.process(delay, audio);
            return ProcessStatus::Continue;
        }
        self.inner_mut().process_sidechain(ctx, audio, sidechain)
    }
    /// Clamped to the PDC limit (RT-safe: the engine also reads it on the audio thread).
    fn latency(&self) -> u32 {
        self.inner().latency().min(self.max_latency)
    }
    fn channels(&self) -> (u16, u16) {
        self.inner().channels()
    }
}

/// Most latency (samples) a hosted plugin may report at `sample_rate`: the engine's PDC
/// limit ([`ether_core::graph::MAX_PDC_SECONDS`], 10 s). Longer could never be compensated
/// anyway, and it bounds the bypass delay line (10 s × channels of `f32`).
fn plugin_latency_cap(sample_rate: f32) -> u32 {
    let sr = if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate.round() as u32
    } else {
        48_000
    };
    ether_core::graph::max_latency_samples(sr)
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
    /// Targets that failed to load (last scan, then retries); not persisted here: the scan
    /// cache remembers them across restarts.
    failed: Arc<Mutex<Vec<ScanFailure>>>,
    db_file: Option<PathBuf>,
}

impl PluginCatalog {
    /// Load `<plugin_db_dir>/plugins.json` if present, and the failures remembered by the
    /// scan cache next to it.
    pub fn open(plugin_db_dir: &Path) -> Self {
        let db_file = plugin_db_dir.join("plugins.json");
        let plugins = std::fs::read_to_string(&db_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let failed = ether_plugin_host::ScanCache::open(
            plugin_db_dir.join(ether_plugin_host::SCAN_CACHE_FILE),
        )
        .failures();
        Self {
            inner: Arc::new(Mutex::new(plugins)),
            failed: Arc::new(Mutex::new(failed)),
            db_file: Some(db_file),
        }
    }

    pub fn list(&self) -> Vec<PluginDescriptor> {
        self.inner.lock().map(|v| v.clone()).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |v| v.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn failures(&self) -> Vec<ScanFailure> {
        self.failed.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// Whether `(path, format)` is a target the last scan (or the scan cache) recorded as
    /// failed: the only targets `Plugin::RetryScan` may scan.
    pub fn is_failure(&self, path: &str, format: PluginFormat) -> bool {
        self.failed.lock().is_ok_and(|v| {
            v.iter()
                .any(|f| f.path == path && f.format.is_none_or(|x| x == format))
        })
    }

    pub fn set_failures(&self, failed: Vec<ScanFailure>) {
        if let Ok(mut v) = self.failed.lock() {
            *v = failed;
        }
    }

    /// Apply a retried target ([`ether_plugin_host::ScanRunner::scan_one`]): on success
    /// add its plugins (replacing what that target listed before, and same format + id +
    /// path; another plugin with the same id elsewhere stays) and persist; it leaves the failures,
    /// else its failure is updated (added if it wasn't listed).
    pub fn apply_retry(&self, scan: &ether_plugin_host::TargetScan) {
        let path = scan.target.path.to_string_lossy();
        let same =
            |f: &ScanFailure| f.path == path && f.format.is_none_or(|x| x == scan.target.format);
        match scan.outcome() {
            Ok(found) => {
                let mut plugins = self.list();
                let fmt = scan.target.format;
                plugins.retain(|p| {
                    !(p.format == fmt && p.path == path)
                        && !found
                            .iter()
                            .any(|n| n.format == p.format && n.id == p.id && n.path == p.path)
                });
                plugins.extend(found.iter().cloned());
                self.replace(plugins);
                if let Ok(mut v) = self.failed.lock() {
                    v.retain(|f| !same(f));
                }
            }
            Err(failure) => {
                if let Ok(mut v) = self.failed.lock() {
                    match v.iter_mut().find(|f| same(f)) {
                        Some(f) => *f = failure,
                        None => v.push(failure),
                    }
                }
            }
        }
    }

    /// The first plugin listed as `(format, plugin_id)` (`PluginInstance.format` /
    /// `.plugin_id`). Lookups are always by both: ids are only unique per format.
    pub fn find_format(&self, format: PluginFormat, plugin_id: &str) -> Option<PluginDescriptor> {
        self.find_named(format, plugin_id, None)
    }

    /// The plugin a device loads: by `(format, plugin_id)`, and when several plugins share
    /// them (two VST2 plugins with the same uniqueID, at different paths) the one named like
    /// the device's plugin (`PluginInstance.name`, the listed name it was inserted from), else
    /// the first listed. Documents keep no path (they open on any machine), so the name is
    /// what tells such plugins apart.
    pub fn find_plugin(&self, plugin: &PluginInstance) -> Option<PluginDescriptor> {
        self.find_named(plugin.format, &plugin.plugin_id, Some(&plugin.name))
    }

    fn find_named(
        &self,
        format: PluginFormat,
        plugin_id: &str,
        name: Option<&str>,
    ) -> Option<PluginDescriptor> {
        let all = self.inner.lock().ok()?;
        let mut same = all
            .iter()
            .filter(|p| p.format == format && p.id == plugin_id)
            .peekable();
        let first = *same.peek()?;
        Some(
            same.find(|p| name.is_some_and(|n| p.name == n))
                .unwrap_or(first)
                .clone(),
        )
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

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod fake {
    //! A fake plugin controller for tests (no CLAP binary needed; feature `test-support`
    //! for other crates' tests).
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
            implemented: true,
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

    /// A controller with an editor: logs its editor calls as `"<instance> open|close"`, and
    /// reports `EditorClosed` once `user_closed` is set (the user closed its window).
    pub struct EditorPlugin {
        instance: u32,
        log: Arc<Mutex<Vec<String>>>,
        open: bool,
        user_closed: Arc<AtomicBool>,
        fail_open: bool,
        notes: Arc<Mutex<Vec<PluginNotification>>>,
        state: Vec<u8>,
    }

    impl PluginController for EditorPlugin {
        fn descriptor(&self) -> DeviceDescriptor {
            descriptor()
        }
        fn params(&mut self) -> Vec<ether_core::protocol::devices::ParamInfo> {
            Vec::new()
        }
        fn activate(&mut self, _: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
            Ok(Box::new(FakeNode))
        }
        fn deactivate(&mut self, _node: Box<dyn PluginNode>) {}
        fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
            Ok(self.state.clone())
        }
        fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
            self.state = state.to_vec();
            Ok(())
        }
        fn has_editor(&self) -> bool {
            true
        }
        fn open_editor(&mut self) -> Result<(), PluginError> {
            if self.fail_open {
                return Err(PluginError::Load("editor: no view".into()));
            }
            self.open = true;
            self.log
                .lock()
                .unwrap()
                .push(format!("{} open", self.instance));
            Ok(())
        }
        fn close_editor(&mut self) {
            if std::mem::take(&mut self.open) {
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("{} close", self.instance));
            }
        }
        fn poll(&mut self, out: &mut Vec<PluginNotification>) {
            if self.open {
                out.append(&mut self.notes.lock().unwrap());
            }
            if self.open && self.user_closed.swap(false, Ordering::SeqCst) {
                self.open = false;
                out.push(PluginNotification::EditorClosed);
            }
        }
        /// An X11 window numbered after the instance while open.
        fn editor_window(&self) -> Option<NativeWindow> {
            self.open.then(|| window(self.instance))
        }
    }

    pub fn window(instance: u32) -> NativeWindow {
        NativeWindow {
            pid: 1,
            handle: ether_core::plugin::WindowHandle::X11 {
                window: u64::from(instance),
            },
        }
    }

    impl Drop for EditorPlugin {
        fn drop(&mut self) {
            self.close_editor();
        }
    }

    pub struct EditorProbe {
        pub log: Arc<Mutex<Vec<String>>>,
        pub user_closed: Arc<AtomicBool>,
        /// Instances made from now on fail to open their editor.
        pub fail_open: Arc<AtomicBool>,
        /// Reported by the next poll of an instance whose editor is open (GUI edits).
        #[allow(dead_code)]
        pub notes: Arc<Mutex<Vec<PluginNotification>>>,
    }

    pub fn editor_plugins() -> (Instantiate, EditorProbe) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let user_closed = Arc::new(AtomicBool::new(false));
        let fail_open = Arc::new(AtomicBool::new(false));
        let notes = Arc::new(Mutex::new(Vec::new()));
        let next = Arc::new(std::sync::atomic::AtomicU32::new(1));
        let (l, u, f, n) = (
            log.clone(),
            user_closed.clone(),
            fail_open.clone(),
            notes.clone(),
        );
        let make: Instantiate = Arc::new(move |_, _, _| {
            Ok(Box::new(EditorPlugin {
                instance: next.fetch_add(1, Ordering::SeqCst),
                log: l.clone(),
                open: false,
                user_closed: u.clone(),
                fail_open: f.load(Ordering::SeqCst),
                notes: n.clone(),
                state: Vec::new(),
            }) as Box<dyn PluginController>)
        });
        (
            make,
            EditorProbe {
                log,
                user_closed,
                fail_open,
                notes,
            },
        )
    }

    pub fn take_log(p: &EditorProbe) -> Vec<String> {
        std::mem::take(&mut *p.log.lock().unwrap())
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
            always_process: false,
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

    /// A busy main thread doesn't hold up the controller's poll: the poll stays in flight
    /// and its notifications come with a later one.
    #[test]
    fn nonblocking_poll_does_not_wait_for_a_busy_main_thread() {
        let main = Arc::new(DedicatedThread::new());
        let host = PluginHost::new(main.clone());
        let prepare = PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 256,
            max_events_per_block: 64,
        };
        let d = device(7);
        let _node = host
            .instantiate(fake::instantiate(), d, src("fake"), None, prepare)
            .unwrap();
        let (release, blocked) = bounded::<()>(0);
        main.spawn(Box::new(move || {
            let _ = blocked.recv();
        }));
        let mut notes = Vec::new();
        let started = Instant::now();
        host.poll_nonblocking(&mut notes);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(notes.is_empty());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while notes.is_empty() && Instant::now() < deadline {
            host.poll_nonblocking(&mut notes);
        }
        assert_eq!(notes, vec![(d, PluginNotification::StateDirty)]);
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
    fn only_recorded_failures_can_be_retried() {
        let cat = PluginCatalog::default();
        cat.set_failures(vec![
            ScanFailure {
                path: "/p/bad.clap".into(),
                message: "crashed".into(),
                format: Some(PluginFormat::Clap),
            },
            ScanFailure {
                path: "/p/old.vst3".into(),
                message: "timed out".into(),
                format: None,
            },
        ]);
        assert!(cat.is_failure("/p/bad.clap", PluginFormat::Clap));
        assert!(!cat.is_failure("/p/bad.clap", PluginFormat::Vst3));
        assert!(cat.is_failure("/p/old.vst3", PluginFormat::Vst3));
        assert!(!cat.is_failure("/elsewhere/x.so", PluginFormat::Clap));
        assert!(!cat.is_failure("/p/bad.clap/../x.so", PluginFormat::Clap));
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

    /// Two unrelated VST2 plugins sharing a uniqueID: both stay listed, each loads
    /// (picked by the device's plugin name), and retrying one target leaves the other.
    #[test]
    fn catalog_keeps_vst2_id_collisions_apart() {
        use ether_plugin_host::{ScanTarget, TargetScan};
        let cat = PluginCatalog::default();
        let desc = |name: &str, path: &str| PluginDescriptor {
            sidechain_inputs: 0,
            format: PluginFormat::Vst2,
            id: "41424344".into(),
            name: name.into(),
            vendor: "X".into(),
            version: "1".into(),
            description: String::new(),
            features: vec![],
            category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
            path: path.into(),
        };
        let a = desc("Alpha", "/p/alpha.so");
        let b = desc("Beta", "/p/beta.so");
        cat.replace(vec![a.clone(), b.clone()]);
        assert_eq!(cat.list(), vec![a.clone(), b.clone()]);
        let inst = |name: &str| PluginInstance {
            format: PluginFormat::Vst2,
            plugin_id: "41424344".into(),
            name: name.into(),
            vendor: String::new(),
            version: String::new(),
            sandboxed: false,
            always_process: false,
            state: None,
        };
        assert_eq!(cat.find_plugin(&inst("Alpha")).unwrap().path, a.path);
        assert_eq!(cat.find_plugin(&inst("Beta")).unwrap().path, b.path);
        // An unknown name (renamed plugin, an old document) loads the first one listed.
        assert_eq!(cat.find_plugin(&inst("41424344")).unwrap().path, a.path);
        assert_eq!(
            cat.find_format(PluginFormat::Vst2, "41424344")
                .unwrap()
                .path,
            a.path
        );
        assert!(
            cat.find_plugin(&PluginInstance {
                format: PluginFormat::Vst3,
                ..inst("Beta")
            })
            .is_none()
        );

        // Retrying Beta's target replaces Beta only; Alpha (same id, other path) stays.
        let target = ScanTarget {
            format: PluginFormat::Vst2,
            path: "/p/beta.so".into(),
        };
        let b2 = PluginDescriptor {
            version: "2".into(),
            ..b.clone()
        };
        cat.apply_retry(&TargetScan::found(target.clone(), vec![b2.clone()]));
        assert_eq!(cat.list(), vec![a.clone(), b2.clone()]);
        // A target that now lists another plugin drops what it listed before.
        let b3 = PluginDescriptor {
            id: "45464748".into(),
            ..b2
        };
        cat.apply_retry(&TargetScan::found(target, vec![b3.clone()]));
        assert_eq!(cat.list(), vec![a, b3]);
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
        assert_eq!(
            again
                .find_format(PluginFormat::Clap, "com.x.y")
                .unwrap()
                .path,
            "/p/y.clap"
        );
        assert!(again.find_format(PluginFormat::Vst3, "com.x.y").is_none());
        assert!(again.find_format(PluginFormat::Clap, "nope").is_none());
    }

    #[test]
    fn catalog_lists_remembered_failures_and_applies_retries() {
        use ether_plugin_host::{SCAN_CACHE_FILE, ScanCache, ScanTarget, TargetScan};
        let tmp = crate::test_util::TempDir::new("catalog-retry");
        let target = |p: &str| ScanTarget {
            format: PluginFormat::Vst3,
            path: p.into(),
        };
        // Failures of an earlier session come from the scan cache.
        let mut cache = ScanCache::open(tmp.path().join(SCAN_CACHE_FILE));
        cache.record(&TargetScan::failed(
            target("/p/slow.vst3"),
            "scan timed out after 20s",
        ));
        cache.record(&TargetScan::failed(target("/p/bad.vst3"), "not a plugin"));
        cache.save().unwrap();
        let cat = PluginCatalog::open(tmp.path());
        let failed = cat.failures();
        assert_eq!(
            failed.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["/p/bad.vst3", "/p/slow.vst3"]
        );
        assert_eq!(failed[1].message, "scan timed out after 20s");
        assert_eq!(failed[1].format, Some(PluginFormat::Vst3));

        // A retry that loads: its plugins join (and persist), the failure goes.
        let desc = PluginDescriptor {
            sidechain_inputs: 0,
            format: PluginFormat::Vst3,
            id: "0123".into(),
            name: "Slow".into(),
            vendor: "X".into(),
            version: "7".into(),
            description: String::new(),
            features: vec![],
            category: ether_core::protocol::devices::DeviceCategory::Instrument,
            path: "/p/slow.vst3".into(),
        };
        cat.apply_retry(&TargetScan::found(
            target("/p/slow.vst3"),
            vec![desc.clone()],
        ));
        assert_eq!(cat.list(), vec![desc.clone()]);
        assert_eq!(cat.failures().len(), 1);
        assert_eq!(
            PluginCatalog::open(tmp.path())
                .find_format(PluginFormat::Vst3, "0123")
                .unwrap()
                .name,
            "Slow"
        );
        // Retrying it again doesn't duplicate it.
        cat.apply_retry(&TargetScan::found(target("/p/slow.vst3"), vec![desc]));
        assert_eq!(cat.len(), 1);

        // A retry that fails again keeps the row, with the new reason.
        cat.apply_retry(&TargetScan::failed(
            target("/p/bad.vst3"),
            "scan timed out after 90s",
        ));
        assert_eq!(
            cat.failures()
                .iter()
                .map(|f| (f.path.as_str(), f.message.as_str()))
                .collect::<Vec<_>>(),
            [("/p/bad.vst3", "scan timed out after 90s")]
        );
    }

    const PREPARE: PrepareConfig = PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: 128,
        max_events_per_block: 64,
    };

    /// A device without a live instance nor a GUI mirror: `OpenEditor` fails (`NotFound`),
    /// never a silent success the UI would show as an open editor.
    #[test]
    fn open_editor_without_an_instance_or_mirror_fails() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        assert!(matches!(
            host.open_editor(device(40)),
            Err(PluginError::NotFound(_))
        ));
        // Once its instance is destroyed too.
        let (make, _probe) = fake::editor_plugins();
        let d = device(41);
        let (node, _) = host.instantiate(make, d, src("e"), None, PREPARE).unwrap();
        host.destroy(d);
        drop(node);
        assert!(matches!(host.open_editor(d), Err(PluginError::NotFound(_))));
    }

    /// The editor opens on the live instance, closes, and opens again: after the user
    /// closes its window (`EditorClosed`) as after `CloseEditor`.
    #[test]
    fn editor_reopens_after_closing() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let (make, probe) = fake::editor_plugins();
        let d = device(42);
        let _node = host.instantiate(make, d, src("e"), None, PREPARE).unwrap();
        host.open_editor(d).unwrap();
        host.close_editor(d).unwrap();
        host.open_editor(d).unwrap();
        probe.user_closed.store(true, Ordering::SeqCst);
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert_eq!(notes, vec![(d, PluginNotification::EditorClosed)]);
        host.open_editor(d).unwrap();
        assert_eq!(
            fake::take_log(&probe),
            ["1 open", "1 close", "1 open", "1 open"]
        );
    }

    /// A device re-instantiated with its editor open (reload, sandbox toggle, a plugin's
    /// restart request, a preset): the old instance's window closes (a retired instance
    /// is not polled: its window would linger, deaf to its close button, until its node
    /// comes back) and the editor opens on the new instance, so the window the UI shows as
    /// open is the live one's.
    #[test]
    fn replacing_an_instance_hands_its_open_editor_over() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let (make, probe) = fake::editor_plugins();
        let d = device(43);
        let (old_node, _) = host
            .instantiate(make.clone(), d, src("e"), None, PREPARE)
            .unwrap();
        host.open_editor(d).unwrap();
        let _new_node = host
            .instantiate(make.clone(), d, src("e"), None, PREPARE)
            .unwrap();
        assert_eq!(fake::take_log(&probe), ["1 open", "1 close", "2 open"]);
        drop(old_node);
        // The open editor is the new instance's: closing it by hand is reported, and
        // `CloseEditor` / `OpenEditor` reach it.
        probe.user_closed.store(true, Ordering::SeqCst);
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert_eq!(notes, vec![(d, PluginNotification::EditorClosed)]);
        host.open_editor(d).unwrap();
        host.close_editor(d).unwrap();
        assert_eq!(fake::take_log(&probe), ["2 open", "2 close"]);

        // Replaced with its editor closed: nothing opens.
        let _third = host
            .instantiate(make.clone(), d, src("e"), None, PREPARE)
            .unwrap();
        assert!(fake::take_log(&probe).is_empty());

        // The replacement can't open it: the UI hears the editor closed.
        host.open_editor(d).unwrap();
        probe.fail_open.store(true, Ordering::SeqCst);
        let _fourth = host.instantiate(make, d, src("e"), None, PREPARE).unwrap();
        assert_eq!(fake::take_log(&probe), ["3 open", "3 close"]);
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert_eq!(notes, vec![(d, PluginNotification::EditorClosed)]);
    }

    /// The open editor's native window is looked up by device: as of its opening, then
    /// each poll; gone once it closes (by hand or by the user), moved to a replacement
    /// instance, gone with its device.
    #[test]
    fn editor_windows_follow_open_editors() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let (make, probe) = fake::editor_plugins();
        let d = device(46);
        let (old_node, _) = host
            .instantiate(make.clone(), d, src("e"), None, PREPARE)
            .unwrap();
        assert_eq!(host.editor_window_for(d), None);
        host.open_editor(d).unwrap();
        assert_eq!(host.editor_window_for(d), Some(fake::window(1)));
        host.close_editor(d).unwrap();
        assert_eq!(host.editor_window_for(d), None);
        host.open_editor(d).unwrap();
        probe.user_closed.store(true, Ordering::SeqCst);
        host.poll(&mut Vec::new());
        assert_eq!(host.editor_window_for(d), None);

        host.open_editor(d).unwrap();
        let (_new_node, _) = host.instantiate(make, d, src("e"), None, PREPARE).unwrap();
        assert_eq!(host.editor_window_for(d), Some(fake::window(2)));
        drop(old_node);
        host.poll(&mut Vec::new());
        assert_eq!(host.editor_window_for(d), Some(fake::window(2)));
        host.destroy(d);
        assert_eq!(host.editor_window_for(d), None);
        host.poll(&mut Vec::new());
        assert_eq!(host.editor_window_for(d), None);
    }

    /// A device destroyed with its editor open (removed, undo of its insert, project
    /// switch): the window closes now, not when the node comes back, and the UI hears it.
    #[test]
    fn destroying_a_device_closes_its_editor_and_reports_it() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let (make, probe) = fake::editor_plugins();
        let d = device(44);
        let (node, _) = host.instantiate(make, d, src("e"), None, PREPARE).unwrap();
        host.open_editor(d).unwrap();
        host.destroy(d);
        assert_eq!(fake::take_log(&probe), ["1 open", "1 close"]);
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert_eq!(notes, vec![(d, PluginNotification::EditorClosed)]);
        drop(node);
        // Destroyed with its editor closed: nothing to report.
        let (make, _probe) = fake::editor_plugins();
        let e = device(45);
        let (_node, _) = host.instantiate(make, e, src("e"), None, PREPARE).unwrap();
        host.destroy(e);
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// A controller for the timeout / latency regression tests: optionally slow to load
    /// or to open its editor, its node reports `latency`, and it counts drops/deactivations.
    struct Probe {
        open_sleep: Duration,
        latency: u32,
        counts: Arc<ProbeCounts>,
    }

    #[derive(Default)]
    struct ProbeCounts {
        made: std::sync::atomic::AtomicU32,
        deactivated: std::sync::atomic::AtomicU32,
        dropped: std::sync::atomic::AtomicU32,
    }

    struct ProbeNode(u32);

    impl Node for ProbeNode {
        fn prepare(&mut self, _: &PrepareConfig) {}
        fn reset(&mut self) {}
        fn process(
            &mut self,
            _: &mut ProcessContext<'_>,
            _: &mut AudioBuffers<'_, '_>,
        ) -> ProcessStatus {
            ProcessStatus::Silent
        }
        fn latency(&self) -> u32 {
            self.0
        }
    }
    impl Device for ProbeNode {
        fn descriptor(&self) -> DeviceDescriptor {
            fake::descriptor()
        }
        fn param(&self, _: ParamId) -> Option<f64> {
            None
        }
        fn set_param(&mut self, _: ParamId, _: f64) {}
    }
    impl PluginNode for ProbeNode {
        fn is_faulted(&self) -> bool {
            false
        }
    }

    impl PluginController for Probe {
        fn descriptor(&self) -> DeviceDescriptor {
            fake::descriptor()
        }
        fn params(&mut self) -> Vec<ether_core::protocol::devices::ParamInfo> {
            Vec::new()
        }
        fn activate(&mut self, _: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
            Ok(Box::new(ProbeNode(self.latency)))
        }
        fn deactivate(&mut self, _: Box<dyn PluginNode>) {
            self.counts.deactivated.fetch_add(1, Ordering::SeqCst);
        }
        fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
            Ok(Vec::new())
        }
        fn load_state(&mut self, _: &[u8]) -> Result<(), PluginError> {
            Ok(())
        }
        fn has_editor(&self) -> bool {
            true
        }
        fn open_editor(&mut self) -> Result<(), PluginError> {
            std::thread::sleep(self.open_sleep);
            Ok(())
        }
        fn close_editor(&mut self) {}
        fn poll(&mut self, _: &mut Vec<PluginNotification>) {}
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.counts.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn probe_plugins(
        load_sleep: Duration,
        open_sleep: Duration,
        latency: u32,
    ) -> (Instantiate, Arc<ProbeCounts>) {
        let counts = Arc::new(ProbeCounts::default());
        let c = counts.clone();
        let make: Instantiate = Arc::new(move |_, _, _| {
            std::thread::sleep(load_sleep);
            c.made.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Probe {
                open_sleep,
                latency,
                counts: c.clone(),
            }) as Box<dyn PluginController>)
        });
        (make, counts)
    }

    fn wait_for(what: &str, f: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Issue a load still running on the main thread when the caller times out is
    /// cleaned up when it finishes (deactivated, dropped, not left as the live instance).
    #[test]
    fn a_load_that_times_out_is_freed_when_it_finishes() {
        let mut host = PluginHost::new(Arc::new(DedicatedThread::new()));
        host.timeout = Duration::from_millis(100);
        let (make, counts) = probe_plugins(Duration::from_millis(500), Duration::ZERO, 0);
        let d = device(40);
        let err = host
            .instantiate(make, d, src("slow"), None, PREPARE)
            .err()
            .unwrap();
        assert!(matches!(err, PluginError::Ipc(_)), "{err:?}");
        wait_for("the late load to be dropped", || {
            counts.dropped.load(Ordering::SeqCst) == 1
        });
        host.timeout = Duration::from_secs(30);
        assert_eq!(counts.made.load(Ordering::SeqCst), 1);
        assert_eq!(counts.deactivated.load(Ordering::SeqCst), 1);
        assert_eq!(host.live_count(), 0);
        assert!(matches!(host.save_state(d), Ok(None)));
    }

    /// Issue the late race: the caller gives up after the load registered its
    /// instance (here: while reopening the editor of the instance it replaces). The
    /// undelivered result retires the orphan and returns its node.
    #[test]
    fn an_undelivered_load_retires_its_instance() {
        let mut host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let (make, counts) = probe_plugins(Duration::ZERO, Duration::from_millis(400), 0);
        let d = device(41);
        let (first, _) = host
            .instantiate(make.clone(), d, src("p"), None, PREPARE)
            .unwrap();
        host.open_editor(d).unwrap();
        host.timeout = Duration::from_millis(100);
        assert!(host.instantiate(make, d, src("p"), None, PREPARE).is_err());
        wait_for("the orphan to be dropped", || {
            counts.dropped.load(Ordering::SeqCst) == 1
        });
        host.timeout = Duration::from_secs(30);
        assert_eq!(counts.deactivated.load(Ordering::SeqCst), 1);
        // Only the first (retired) instance is left, until its node comes back.
        assert_eq!(host.live_count(), 1);
        assert!(matches!(host.save_state(d), Ok(None)));
        drop(first);
        assert_eq!(host.live_count(), 0);
        assert_eq!(counts.dropped.load(Ordering::SeqCst), 2);
    }

    /// Issue a plugin reporting `u32::MAX` latency is clamped to the PDC limit (10 s)
    /// instead of sizing a multi-GB bypass delay line.
    #[test]
    fn absurd_plugin_latency_is_clamped() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let (make, _) = probe_plugins(Duration::ZERO, Duration::ZERO, u32::MAX);
        let (node, _) = host
            .instantiate(make, device(42), src("p"), None, PREPARE)
            .unwrap();
        assert_eq!(node.latency(), 480_000);
        assert_eq!(
            plugin_latency_cap(96_000.0),
            ether_core::graph::max_latency_samples(96_000)
        );
        assert_eq!(plugin_latency_cap(f32::NAN), 480_000);
        let (make, _) = probe_plugins(Duration::ZERO, Duration::ZERO, 512);
        let (node, _) = host
            .instantiate(make, device(43), src("p"), None, PREPARE)
            .unwrap();
        assert_eq!(node.latency(), 512);
    }
}
