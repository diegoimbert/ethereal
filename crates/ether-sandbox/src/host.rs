//! Host-side controller of a sandboxed plugin ([`SandboxedPlugin`]).

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ether_core::config::PrepareConfig;
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceDescriptor, ParamInfo};
use ether_core::protocol::model::{ParamId, PluginFormat};

use crate::node::{NodeInit, SandboxedNode, Shared};
use crate::shm::{Layout, Region};
use crate::sys::{Semaphore, os_name};
use crate::wire::{self, Notification, Request, Response};

/// Tunables of a sandboxed instance.
#[derive(Clone, Debug)]
pub struct SandboxOptions {
    /// Helper executable (default: [`crate::helper_path`]).
    pub helper: PathBuf,
    /// Format of the plugin (passed as `--format`; the helper loads it through that
    /// format's `PluginFormatHost`). Default: CLAP.
    pub format: PluginFormat,
    /// Max time for the helper to load the plugin and report ready.
    pub startup_timeout: Duration,
    /// Max time for any other control request; a helper that doesn't answer in time is
    /// considered hung, killed and reported as crashed.
    pub request_timeout: Duration,
    /// How long the audio thread may spin-wait for the previous block's result before
    /// outputting silence for it (underrun). `None` = a quarter of a max-size block.
    pub wait_budget: Option<Duration>,
}

impl Default for SandboxOptions {
    fn default() -> Self {
        Self {
            helper: crate::helper_path(),
            format: PluginFormat::Clap,
            startup_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(10),
            wait_budget: None,
        }
    }
}

/// Distinguishes the IPC objects of several instances/activations in one host process.
static NEXT_ID: AtomicU32 = AtomicU32::new(0);

/// A plugin instance running in a helper process (main-thread half). Implements the same
/// [`PluginController`] contract as the in-process `ether_clap::ClapPlugin`; the helper runs
/// the plugin's main-thread work, so `poll` only relays.
pub struct SandboxedPlugin {
    child: Child,
    stdin: Option<ChildStdin>,
    responses: mpsc::Receiver<Response>,
    reader: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
    instance: String,
    options: SandboxOptions,
    descriptor: DeviceDescriptor,
    has_editor: bool,
    active: bool,
    crash_reported: bool,
}

impl std::fmt::Debug for SandboxedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxedPlugin")
            .field("helper_pid", &self.child.id())
            .field("plugin", &self.descriptor.name)
            .field("active", &self.active)
            .finish()
    }
}

fn crashed(msg: impl Into<String>) -> PluginError {
    PluginError::Crashed(msg.into())
}

impl SandboxedPlugin {
    /// Start the helper, which loads `plugin_id` from `bundle` (an AU component id for AUs)
    /// as an `options.format` plugin. `instance` is the dev instance id used in IPC names.
    pub fn spawn(
        bundle: &Path,
        plugin_id: &str,
        instance: &str,
        options: SandboxOptions,
    ) -> Result<Self, PluginError> {
        let mut child = Command::new(&options.helper)
            .arg("--format")
            .arg(options.format.as_str())
            .arg(bundle)
            .arg(plugin_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                PluginError::Ipc(format!(
                    "cannot start sandbox helper {}: {e}",
                    options.helper.display()
                ))
            })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let shared = Arc::new(Shared::default());

        // Reader thread: frames → channel. EOF/garbage = the helper is gone.
        let (tx, responses) = mpsc::channel();
        let reader_shared = shared.clone();
        let reader = std::thread::Builder::new()
            .name("ether-sandbox-reader".into())
            .spawn(move || {
                let mut r = BufReader::new(stdout);
                while let Ok(Some(msg)) = wire::read_frame::<Response>(&mut r) {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                reader_shared.crashed.store(true, Ordering::Release);
            })
            .map_err(|e| PluginError::Ipc(e.to_string()))?;

        let mut plugin = Self {
            child,
            stdin,
            responses,
            reader: Some(reader),
            shared,
            instance: instance.to_owned(),
            descriptor: DeviceDescriptor {
                device_type: ether_core::protocol::devices::DeviceTypeRef::Plugin {
                    plugin_id: plugin_id.to_owned(),
                },
                name: plugin_id.to_owned(),
                category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
                params: Vec::new(),
                audio_inputs: 0,
                audio_outputs: 0,
                midi_input: false,
                sidechain_inputs: 0,
            },
            has_editor: false,
            active: false,
            crash_reported: false,
            options,
        };
        match plugin.receive(plugin.options.startup_timeout)? {
            Response::Ready {
                descriptor,
                has_editor,
            } => {
                plugin.descriptor = descriptor;
                plugin.has_editor = has_editor;
                Ok(plugin)
            }
            Response::Err(e) => Err(e.into()),
            other => Err(PluginError::Ipc(format!("unexpected handshake: {other:?}"))),
        }
    }

    /// Helper process id.
    pub fn helper_pid(&self) -> u32 {
        self.child.id()
    }

    /// Blocks whose result came too late (replaced by silence) since spawn.
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// True once the helper died (or was killed after hanging).
    pub fn is_crashed(&mut self) -> bool {
        if !self.shared.crashed.load(Ordering::Acquire)
            && matches!(self.child.try_wait(), Ok(Some(_)))
        {
            self.shared.crashed.store(true, Ordering::Release);
        }
        self.shared.crashed.load(Ordering::Acquire)
    }

    /// Kill the helper (tests, or a host-side watchdog). The node goes silent and the next
    /// `poll` reports `Crashed`.
    pub fn kill_helper(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.shared.crashed.store(true, Ordering::Release);
    }

    /// Set a parameter while inactive (like `ClapPlugin::set_param_value`).
    pub fn set_param_value(&mut self, param: ParamId, value: f64) -> Result<(), PluginError> {
        self.expect_ok(Request::SetParam { id: param.0, value })
    }

    fn mark_crashed(&mut self) {
        self.shared.crashed.store(true, Ordering::Release);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn receive(&mut self, timeout: Duration) -> Result<Response, PluginError> {
        match self.responses.recv_timeout(timeout) {
            Ok(r) => Ok(r),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.mark_crashed();
                Err(crashed("plugin helper stopped responding"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.mark_crashed();
                Err(crashed(self.exit_message()))
            }
        }
    }

    fn request(&mut self, req: Request) -> Result<Response, PluginError> {
        if self.is_crashed() {
            return Err(crashed(self.exit_message()));
        }
        let sent = match self.stdin.as_mut() {
            Some(stdin) => wire::write_frame(stdin, &req).is_ok(),
            None => false,
        };
        if !sent {
            self.mark_crashed();
            return Err(crashed(self.exit_message()));
        }
        match self.receive(self.options.request_timeout)? {
            Response::Err(e) => Err(e.into()),
            r => Ok(r),
        }
    }

    fn expect_ok(&mut self, req: Request) -> Result<(), PluginError> {
        match self.request(req)? {
            Response::Ok => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    fn exit_message(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(status)) => format!("plugin helper exited ({status})"),
            _ => "plugin helper is gone".into(),
        }
    }

    /// Second activation step: create the region + semaphore, let the helper map them, then
    /// unlink the names (nothing named survives a later crash of either process).
    fn attach(
        &mut self,
        config: &PrepareConfig,
        channels: (u16, u16),
    ) -> Result<(Region, Semaphore), PluginError> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let shm_name = os_name(&self.instance, pid, &format!("sbx-{id}-shm"));
        let sem_name = os_name(&self.instance, pid, &format!("sbx-{id}-sem"));
        let max_frames = config.max_block_size.max(1);
        let max_in_events = config.max_events_per_block + self.descriptor.params.len().max(16);
        let layout = Layout::new(
            max_frames,
            usize::from(channels.0),
            usize::from(channels.1),
            max_in_events,
            config.max_events_per_block.max(256),
        );
        let mut region =
            Region::create(&shm_name, layout).map_err(|e| PluginError::Ipc(e.to_string()))?;
        let mut sem = Semaphore::create(&sem_name).map_err(|e| PluginError::Ipc(e.to_string()))?;
        let result = self.expect_ok(Request::Attach {
            shm: shm_name,
            sem: sem_name,
        });
        region.unlink();
        sem.unlink();
        result.map(|()| (region, sem))
    }
}

fn unexpected(r: Response) -> PluginError {
    PluginError::Ipc(format!("unexpected response: {r:?}"))
}

impl PluginController for SandboxedPlugin {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn params(&mut self) -> Vec<ParamInfo> {
        if let Ok(Response::Params(p)) = self.request(Request::Params) {
            self.descriptor.params = p;
        }
        self.descriptor.params.clone()
    }

    fn activate(&mut self, config: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
        if self.active {
            return Err(PluginError::Activation("plugin is already active".into()));
        }
        let (descriptor, latency, channels) = match self.request(Request::Activate {
            sample_rate: config.sample_rate,
            max_block_size: config.max_block_size as u32,
            max_events_per_block: config.max_events_per_block as u32,
        })? {
            Response::Activated {
                descriptor,
                latency,
                inputs,
                outputs,
            } => (descriptor, latency, (inputs, outputs)),
            other => return Err(unexpected(other)),
        };
        self.descriptor = descriptor;
        let (region, sem) = match self.attach(config, channels) {
            Ok(v) => v,
            Err(e) => {
                let _ = self.request(Request::Deactivate);
                return Err(e);
            }
        };
        self.active = true;
        self.shared.plugin_latency.store(latency, Ordering::Relaxed);
        self.shared
            .block_latency
            .store(config.max_block_size.max(1) as u32, Ordering::Relaxed);

        let mut values = Vec::with_capacity(self.descriptor.params.len());
        for p in self.descriptor.params.clone() {
            let v = self.param_value(p.id).unwrap_or(p.default);
            values.push((p.id.0, v));
        }
        let wait_budget = self.options.wait_budget.unwrap_or_else(|| {
            Duration::from_secs_f64(
                config.max_block_size as f64 / f64::from(config.sample_rate.max(1.0)) / 4.0,
            )
        });
        Ok(Box::new(SandboxedNode::new(NodeInit {
            region,
            sem,
            shared: self.shared.clone(),
            descriptor: self.descriptor.clone(),
            values,
            channels,
            wait_budget,
        })))
    }

    fn deactivate(&mut self, node: Box<dyn PluginNode>) {
        drop(node);
        if self.active {
            self.active = false;
            if let Err(e) = self.expect_ok(Request::Deactivate) {
                tracing::warn!("sandbox deactivate failed: {e}");
            }
        }
    }

    fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
        match self.request(Request::SaveState)? {
            Response::State(s) => wire::decode_state(&s),
            other => Err(unexpected(other)),
        }
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        self.expect_ok(Request::LoadState(wire::encode_state(state)))
    }

    fn param_value(&mut self, param: ParamId) -> Option<f64> {
        match self.request(Request::ParamValue(param.0)) {
            Ok(Response::ParamValue(v)) => v,
            _ => None,
        }
    }

    fn set_param_value(&mut self, param: ParamId, value: f64) -> Result<(), PluginError> {
        SandboxedPlugin::set_param_value(self, param, value)
    }

    fn has_editor(&self) -> bool {
        self.has_editor
    }

    fn open_editor(&mut self) -> Result<(), PluginError> {
        if !self.has_editor {
            return Err(PluginError::NoEditor);
        }
        self.expect_ok(Request::OpenEditor)
    }

    fn close_editor(&mut self) {
        if self.has_editor && !self.is_crashed() {
            let _ = self.expect_ok(Request::CloseEditor);
        }
    }

    fn poll(&mut self, out: &mut Vec<PluginNotification>) {
        if !self.is_crashed() {
            match self.request(Request::Poll) {
                Ok(Response::Notifications(list)) => {
                    let mut rescan = false;
                    for n in list {
                        match n {
                            // Report the node's total latency (plugin + the sandbox block),
                            // i.e. what `Node::latency` now returns.
                            Notification::LatencyChanged { samples } => {
                                self.shared.plugin_latency.store(samples, Ordering::Relaxed);
                                let block = self.shared.block_latency.load(Ordering::Relaxed);
                                out.push(PluginNotification::LatencyChanged {
                                    samples: samples + block,
                                });
                                continue;
                            }
                            Notification::ParamsChanged => rescan = true,
                            _ => {}
                        }
                        out.push(n.into());
                    }
                    if rescan {
                        self.params();
                    }
                }
                Ok(other) => tracing::warn!("sandbox poll: {:?}", unexpected(other)),
                Err(_) => {} // crash reported below
            }
        }
        if self.is_crashed() && !self.crash_reported {
            self.crash_reported = true;
            let message = self.exit_message();
            out.push(PluginNotification::Crashed { message });
        }
    }
}

impl Drop for SandboxedPlugin {
    fn drop(&mut self) {
        // Closing stdin tells the helper to deactivate, destroy the plugin and exit.
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        // Bounded join: a daemon spawned by the plugin could keep the pipe open.
        if let Some(reader) = self.reader.take() {
            let deadline = Instant::now() + Duration::from_secs(1);
            while !reader.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }
}
