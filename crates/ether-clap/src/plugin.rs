//! Main-thread half of an in-process CLAP plugin ([`ClapPlugin`]).

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use clack_extensions::audio_ports::{AudioPortFlags, AudioPortInfoBuffer};
use clack_extensions::gui::{GuiApiType, GuiConfiguration};
use clack_host::events::event_types::ParamValueEvent;
use clack_host::events::spaces::CoreEventSpace;
use clack_host::prelude::*;
use ether_core::config::PrepareConfig;
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo};
use ether_core::protocol::model::ParamId;

use crate::host::{EtherHost, HostMainThread, HostShared, PluginExts, host_info};
use crate::node::{ClapNode, NodeInit, NodeMsg, NodeShared, PortLayout};
use crate::scan::{category_from_features, load_entry};

/// Capacity of the node → controller ring (param edits and gestures from the plugin).
const RING_CAPACITY: usize = 1024;

struct ActiveLink {
    from_node: rtrb::Consumer<NodeMsg>,
    shared: Arc<NodeShared>,
    fault_reported: bool,
}

/// An in-process CLAP plugin instance (main-thread half). Not `Send`: create and use it on
/// the thread that will run its main-thread callbacks (and, on macOS, its GUI).
pub struct ClapPlugin {
    // Field order matters: the instance must drop before the entry (library) it came from.
    instance: PluginInstance<EtherHost>,
    _entry: PluginEntry,
    bundle: PathBuf,
    plugin_id: String,
    name: String,
    category: DeviceCategory,
    params: Vec<ParamInfo>,
    /// Main audio input/output channel counts + note input (cached).
    io: (u16, u16, bool),
    link: Option<ActiveLink>,
    editor: Option<GuiConfiguration<'static>>,
    editor_open: bool,
}

impl std::fmt::Debug for ClapPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClapPlugin")
            .field("bundle", &self.bundle)
            .field("plugin_id", &self.plugin_id)
            .field("active", &self.link.is_some())
            .finish()
    }
}

impl ClapPlugin {
    /// Load `bundle` and instantiate `plugin_id` (main thread).
    pub fn load(bundle: &Path, plugin_id: &str) -> Result<Self, PluginError> {
        let entry = load_entry(bundle)?;
        let factory = entry
            .get_plugin_factory()
            .ok_or_else(|| PluginError::Load("bundle has no plugin factory".into()))?;
        let descriptor = factory
            .plugin_descriptors()
            .find(|d| d.id().and_then(|id| id.to_str().ok()) == Some(plugin_id))
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_owned()))?;
        let name = descriptor
            .name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| plugin_id.to_owned());
        let features: Vec<String> = descriptor
            .features()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();

        let cid = CString::new(plugin_id).map_err(|_| PluginError::NotFound(plugin_id.into()))?;
        let instance = PluginInstance::<EtherHost>::new(
            |_| HostShared::default(),
            |shared| HostMainThread::new(shared),
            &entry,
            &cid,
            &host_info(),
        )
        .map_err(|e| PluginError::Load(e.to_string()))?;

        let mut plugin = Self {
            instance,
            _entry: entry,
            bundle: bundle.to_path_buf(),
            plugin_id: plugin_id.to_owned(),
            name,
            category: category_from_features(&features),
            params: Vec::new(),
            io: (0, 0, false),
            link: None,
            editor: None,
            editor_open: false,
        };
        plugin.params = plugin.query_params();
        plugin.refresh_io();
        plugin.editor = plugin.negotiate_editor();
        Ok(plugin)
    }

    pub fn bundle(&self) -> &Path {
        &self.bundle
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn is_active(&self) -> bool {
        self.link.is_some()
    }

    fn exts(&self) -> PluginExts {
        self.instance.access_handler(|h| h.exts.get())
    }

    fn query_params(&mut self) -> Vec<ParamInfo> {
        match self.exts().params {
            Some(ext) => crate::params::list(&ext, &self.instance.plugin_handle()),
            None => Vec::new(),
        }
    }

    fn port_layout(&mut self) -> PortLayout {
        let Some(ext) = self.exts().audio_ports else {
            // No audio-ports extension: assume a plain stereo effect/instrument.
            return PortLayout {
                inputs: vec![2],
                outputs: vec![2],
                main_in: Some(0),
                main_out: Some(0),
            };
        };
        let handle = self.instance.plugin_handle();
        let mut buffer = AudioPortInfoBuffer::new();
        let mut side = |is_input: bool| {
            let mut channels = Vec::new();
            let mut main = None;
            for i in 0..ext.count(&handle, is_input) {
                let info = ext.get(&handle, i, is_input, &mut buffer);
                let (ch, is_main) = info
                    .map(|p| (p.channel_count, p.flags.contains(AudioPortFlags::IS_MAIN)))
                    .unwrap_or((0, false));
                if is_main && main.is_none() {
                    main = Some(channels.len());
                }
                channels.push(ch);
            }
            if main.is_none() && !channels.is_empty() {
                main = Some(0);
            }
            (channels, main)
        };
        let (inputs, main_in) = side(true);
        let (outputs, main_out) = side(false);
        PortLayout {
            inputs,
            outputs,
            main_in,
            main_out,
        }
    }

    fn refresh_io(&mut self) {
        let (i, o) = self.port_layout().main_channels();
        let midi = self.has_note_input();
        self.io = (i, o, midi);
    }

    fn has_note_input(&mut self) -> bool {
        match self.exts().note_ports {
            Some(ext) => ext.count(&self.instance.plugin_handle(), true) > 0,
            None => false,
        }
    }

    fn query_latency(&mut self) -> u32 {
        match self.exts().latency {
            Some(ext) => ext.get(&self.instance.plugin_handle()),
            None => 0,
        }
    }

    /// Current plain value of a parameter (main thread; works active or inactive).
    pub fn param_value(&mut self, param: ParamId) -> Option<f64> {
        let ext = self.exts().params?;
        ext.get_value(&self.instance.plugin_handle(), ClapId::new(param.0))
    }

    /// Set a parameter while the plugin is NOT active (CLAP `params.flush`). While active,
    /// send `EventKind::Param` to the node instead (engine param queue / automation).
    pub fn set_param_value(&mut self, param: ParamId, value: f64) -> Result<(), PluginError> {
        if self.link.is_some() {
            return Err(PluginError::State(
                "plugin is active: send param events to its node".into(),
            ));
        }
        let Some(ext) = self.exts().params else {
            return Err(PluginError::State("plugin has no params extension".into()));
        };
        let mut input = EventBuffer::with_capacity(1);
        input.push(&ParamValueEvent::new(
            0,
            ClapId::new(param.0),
            Pckn::match_all(),
            value,
        ));
        let mut output = EventBuffer::with_capacity(16);
        if let Some(mut handle) = self.instance.inactive_plugin_handle() {
            ext.flush(&mut handle, &input.as_input(), &mut output.as_output());
        }
        Ok(())
    }

    /// Pick a GUI configuration we can host: the platform API as a CLAP floating window.
    /// Embedded-only plugins need a host-created window (macOS: pending AppKit bindings, see
    /// the clap node BCR), so they report no editor for now.
    fn negotiate_editor(&mut self) -> Option<GuiConfiguration<'static>> {
        let gui = self.exts().gui?;
        let api_type: GuiApiType<'static> = GuiApiType::default_for_current_platform()?;
        let config = GuiConfiguration {
            api_type,
            is_floating: true,
        };
        gui.is_api_supported(&self.instance.plugin_handle(), config)
            .then_some(config)
    }

    fn drain_flush(&mut self, out: &mut Vec<PluginNotification>) {
        let Some(ext) = self.exts().params else {
            return;
        };
        let input = EventBuffer::new();
        let mut output = EventBuffer::with_capacity(64);
        if let Some(mut handle) = self.instance.inactive_plugin_handle() {
            ext.flush(&mut handle, &input.as_input(), &mut output.as_output());
        }
        for event in output.iter() {
            let msg = match event.as_core_event() {
                Some(CoreEventSpace::ParamValue(e)) => e.param_id().map(|id| NodeMsg::Param {
                    id: id.get(),
                    value: e.value(),
                }),
                Some(CoreEventSpace::ParamGestureBegin(e)) => {
                    e.param_id().map(|id| NodeMsg::GestureBegin(id.get()))
                }
                Some(CoreEventSpace::ParamGestureEnd(e)) => {
                    e.param_id().map(|id| NodeMsg::GestureEnd(id.get()))
                }
                _ => None,
            };
            if let Some(msg) = msg {
                out.push(notification(msg));
            }
        }
    }
}

fn notification(msg: NodeMsg) -> PluginNotification {
    match msg {
        NodeMsg::Param { id, value } => PluginNotification::ParamEdited {
            param: ParamId(id),
            value,
        },
        NodeMsg::GestureBegin(id) => PluginNotification::GestureBegin { param: ParamId(id) },
        NodeMsg::GestureEnd(id) => PluginNotification::GestureEnd { param: ParamId(id) },
    }
}

impl PluginController for ClapPlugin {
    fn descriptor(&self) -> DeviceDescriptor {
        // `&self` only: port info and params are cached (refreshed by `params()`, `activate`
        // and `poll` on rescan), since querying needs the mutable main-thread handle.
        let io = self.io;
        DeviceDescriptor {
            device_type: DeviceTypeRef::Plugin {
                plugin_id: self.plugin_id.clone(),
            },
            name: self.name.clone(),
            category: self.category,
            params: self.params.clone(),
            audio_inputs: io.0,
            audio_outputs: io.1,
            midi_input: io.2,
        }
    }

    fn params(&mut self) -> Vec<ParamInfo> {
        self.params = self.query_params();
        self.params.clone()
    }

    fn activate(&mut self, config: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
        if self.link.is_some() {
            return Err(PluginError::Activation("plugin is already active".into()));
        }
        let layout = self.port_layout();
        self.refresh_io();
        let latency = self.query_latency();
        self.params = self.query_params();
        let values: Vec<(u32, f64)> = self
            .params
            .clone()
            .iter()
            .map(|p| (p.id.0, self.param_value(p.id).unwrap_or(p.default)))
            .collect();
        let descriptor = self.descriptor();

        let max_frames = config.max_block_size.max(1);
        let processor = self
            .instance
            .activate(
                |_, _| (),
                PluginAudioConfiguration {
                    sample_rate: f64::from(config.sample_rate),
                    min_frames_count: 1,
                    max_frames_count: max_frames as u32,
                },
            )
            .map_err(|e| PluginError::Activation(e.to_string()))?;

        let (to_main, from_node) = rtrb::RingBuffer::new(RING_CAPACITY);
        let shared = Arc::new(NodeShared::default());
        shared.latency.store(latency, Ordering::Relaxed);
        self.link = Some(ActiveLink {
            from_node,
            shared: shared.clone(),
            fault_reported: false,
        });
        Ok(Box::new(ClapNode::new(NodeInit {
            processor,
            shared,
            to_main,
            descriptor,
            layout,
            max_frames,
            max_events: config.max_events_per_block,
            values,
        })))
    }

    fn deactivate(&mut self, node: Box<dyn PluginNode>) {
        // Dropping the node drops the audio processor handle; clack then stops processing
        // (if started) and deactivates.
        drop(node);
        if self.link.take().is_some()
            && let Err(e) = self.instance.try_deactivate()
        {
            tracing::warn!("CLAP deactivate failed: {e}");
        }
    }

    fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
        let Some(ext) = self.exts().state else {
            return Ok(Vec::new());
        };
        let mut data = Vec::new();
        ext.save(&self.instance.plugin_handle(), &mut data)
            .map_err(|e| PluginError::State(e.to_string()))?;
        Ok(data)
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        let Some(ext) = self.exts().state else {
            return if state.is_empty() {
                Ok(())
            } else {
                Err(PluginError::State("plugin has no state extension".into()))
            };
        };
        let mut reader = state;
        ext.load(&self.instance.plugin_handle(), &mut reader)
            .map_err(|e| PluginError::State(e.to_string()))
    }

    fn has_editor(&self) -> bool {
        self.editor.is_some()
    }

    fn open_editor(&mut self) -> Result<(), PluginError> {
        let config = self.editor.ok_or(PluginError::NoEditor)?;
        let gui = self.exts().gui.ok_or(PluginError::NoEditor)?;
        let handle = self.instance.plugin_handle();
        if !self.editor_open {
            gui.create(&handle, config)
                .map_err(|e| PluginError::Load(format!("editor: {e}")))?;
            if let Ok(title) = CString::new(self.name.clone()) {
                gui.suggest_title(&handle, &title);
            }
            self.editor_open = true;
        }
        gui.show(&handle)
            .map_err(|e| PluginError::Load(format!("editor: {e}")))
    }

    fn close_editor(&mut self) {
        if !self.editor_open {
            return;
        }
        if let Some(gui) = self.exts().gui {
            let handle = self.instance.plugin_handle();
            let _ = gui.hide(&handle);
            gui.destroy(&handle);
        }
        self.editor_open = false;
    }

    fn poll(&mut self, out: &mut Vec<PluginNotification>) {
        let take = HostShared::take;
        if self
            .instance
            .access_shared_handler(|s| take(&s.callback_requested))
        {
            self.instance.call_on_main_thread_callback();
        }

        let exts = self.exts();
        if let Some(timer) = exts.timer {
            let due = self
                .instance
                .access_handler(|h| h.due_timers(Instant::now()));
            let handle = self.instance.plugin_handle();
            for id in due {
                timer.on_timer(&handle, id);
            }
        }

        if self
            .instance
            .access_shared_handler(|s| take(&s.flush_requested))
            && self.link.is_none()
        {
            self.drain_flush(out);
        }

        if let Some(link) = self.link.as_mut() {
            while let Ok(msg) = link.from_node.pop() {
                out.push(notification(msg));
            }
            if link.shared.faulted.load(Ordering::Acquire) && !link.fault_reported {
                link.fault_reported = true;
                out.push(PluginNotification::Crashed {
                    message: "plugin process() failed".into(),
                });
            }
        }

        if self
            .instance
            .access_shared_handler(|s| take(&s.restart_requested))
        {
            out.push(PluginNotification::RestartRequested);
        }
        if self
            .instance
            .access_handler(|h| h.latency_changed.replace(false))
        {
            let samples = self.query_latency();
            if let Some(link) = &self.link {
                link.shared.latency.store(samples, Ordering::Relaxed);
            }
            out.push(PluginNotification::LatencyChanged { samples });
        }
        if self
            .instance
            .access_handler(|h| h.params_rescan.replace(false))
        {
            self.params = self.query_params();
            out.push(PluginNotification::ParamsChanged);
        }
        if self
            .instance
            .access_handler(|h| h.state_dirty.replace(false))
        {
            out.push(PluginNotification::StateDirty);
        }
        if self.instance.access_shared_handler(|s| take(&s.gui_closed)) && self.editor_open {
            if let Some(gui) = exts.gui {
                gui.destroy(&self.instance.plugin_handle());
            }
            self.editor_open = false;
            out.push(PluginNotification::EditorClosed);
        }
    }
}

impl Drop for ClapPlugin {
    fn drop(&mut self) {
        self.close_editor();
    }
}
