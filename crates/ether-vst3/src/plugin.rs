//! Main-thread half of an in-process VST3 plugin ([`Vst3Plugin`]).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use ether_core::config::PrepareConfig;
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo};
use ether_core::protocol::model::ParamId;
use vst3::Steinberg::Vst::BusDirections_::{kInput, kOutput};
use vst3::Steinberg::Vst::BusTypes_::kAux;
use vst3::Steinberg::Vst::MediaTypes_::{kAudio, kEvent};
use vst3::Steinberg::Vst::ProcessModes_::kRealtime;
use vst3::Steinberg::Vst::RestartFlags_::{
    kIoChanged, kLatencyChanged, kParamTitlesChanged, kParamValuesChanged, kReloadComponent,
};
use vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32;
use vst3::Steinberg::Vst::{
    BusInfo, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait,
    IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait, ProcessSetup,
    SpeakerArr, SpeakerArrangement,
};
use vst3::Steinberg::{
    FUnknown, IBStream, IPlugView, IPlugViewTrait, IPluginBaseTrait, IPluginFactoryTrait, TUID,
    ViewRect, kResultOk, kResultTrue,
};
use vst3::{ComPtr, ComWrapper, Interface};

use crate::gui::{DEFAULT_SIZE, HostWindow, PlugFrame, Size};
use crate::host::{ComponentHandler, Edit, HostApplication};
use crate::module::Module;
use crate::node::{BusLayout, NodeInit, NodeShared, ParamMsg, Vst3Node};
use crate::params::{self, StepTable, to_normalized, to_plain};
use crate::scan::{AUDIO_MODULE_CLASS, category_from_features, classes};
use crate::stream::MemoryStream;

/// Capacity of the node ↔ controller rings.
const RING_CAPACITY: usize = 1024;

/// State blob framing: magic, version, then length-prefixed component and controller state.
const STATE_MAGIC: &[u8; 8] = b"EthVST3\0";
const STATE_VERSION: u32 = 1;

struct ActiveLink {
    from_node: rtrb::Consumer<ParamMsg>,
    to_node: rtrb::Producer<ParamMsg>,
    shared: Arc<NodeShared>,
    fault_reported: bool,
}

struct Editor {
    view: ComPtr<IPlugView>,
    frame: ComWrapper<PlugFrame>,
    window: HostWindow,
}

/// An in-process VST3 plugin instance (main-thread half). Not `Send`: create and use it on
/// the thread that will run its UI-thread calls (and, on macOS, its editor).
pub struct Vst3Plugin {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    /// The controller is a separate object (initialized/terminated by us).
    separate_controller: bool,
    /// Component ↔ controller connection points (separate controllers only).
    connection: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    handler: ComWrapper<ComponentHandler>,
    host: ComWrapper<HostApplication>,
    bundle: PathBuf,
    plugin_id: String,
    name: String,
    category: DeviceCategory,
    params: Vec<ParamInfo>,
    steps: StepTable,
    /// Bus layout as reported by `getBusInfo` (before activation).
    layout: BusLayout,
    link: Option<ActiveLink>,
    /// Values set while inactive, delivered to the processor on activation (normalized).
    pending: Vec<ParamMsg>,
    /// Whether the controller has an editor view we can host: probed lazily (on the main
    /// thread, where `createView` may be called), then cached.
    has_editor: Cell<Option<bool>>,
    editor: Option<Editor>,
    /// Dropped last: the library must outlive every object it created.
    module: Arc<Module>,
}

impl std::fmt::Debug for Vst3Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vst3Plugin")
            .field("bundle", &self.bundle)
            .field("plugin_id", &self.plugin_id)
            .field("active", &self.link.is_some())
            .finish()
    }
}

fn load_err(what: &str) -> PluginError {
    PluginError::Load(what.to_owned())
}

/// Create an instance of class `cid` with interface `I` from the module's factory.
fn create<I: Interface>(module: &Module, cid: &TUID) -> Option<ComPtr<I>> {
    let mut obj = std::ptr::null_mut();
    let iid = I::IID.map(|b| b as std::ffi::c_char);
    // SAFETY: valid factory; `cid`/`iid` are 16-byte ids; on success `obj` holds one ref.
    let r = unsafe {
        module
            .factory()
            .createInstance(cid.as_ptr(), iid.as_ptr(), &mut obj)
    };
    if r != kResultOk {
        return None;
    }
    // SAFETY: `obj` is an owned `I*` (or null).
    unsafe { ComPtr::from_raw(obj.cast::<I>()) }
}

/// `(channels, is a kAux bus)` of every audio bus of `dir`.
fn bus_infos(component: &ComPtr<IComponent>, dir: i32) -> Vec<(u32, bool)> {
    // SAFETY (all calls): valid component; `BusInfo` is plain C data.
    let count = unsafe { component.getBusCount(kAudio as i32, dir) }.max(0);
    (0..count)
        .map(|i| {
            let mut info: BusInfo = unsafe { std::mem::zeroed() };
            if unsafe { component.getBusInfo(kAudio as i32, dir, i, &mut info) } == kResultOk {
                (
                    info.channelCount.max(0) as u32,
                    info.busType as i64 == kAux as i64,
                )
            } else {
                (0, false)
            }
        })
        .collect()
}

fn bus_channels(component: &ComPtr<IComponent>, dir: i32) -> Vec<u32> {
    bus_infos(component, dir)
        .into_iter()
        .map(|(c, _)| c)
        .collect()
}

/// Scanner: the sidechain channels of audio-module class `cid` of `module` (its first `kAux`
/// input bus, capped at 2; 0 = none or the component can't be created). Only creates and
/// initializes the component (no controller, no activation).
pub(crate) fn probe_sidechain_inputs(module: &Module, cid: &[u8; 16]) -> u16 {
    let host = ComWrapper::new(HostApplication);
    let Some(host_ctx) = host.as_com_ref::<FUnknown>().map(|r| r.as_ptr()) else {
        return 0;
    };
    let tuid: TUID = cid.map(|b| b as std::ffi::c_char);
    let Some(component) = create::<IComponent>(module, &tuid) else {
        return 0;
    };
    // SAFETY (all calls): valid component on this (the scanner's main) thread.
    if unsafe { component.initialize(host_ctx) } != kResultOk {
        return 0;
    }
    let inputs = bus_infos(&component, kInput as i32);
    let layout = BusLayout {
        aux_in: BusLayout::find_aux(&inputs),
        inputs: inputs.into_iter().map(|(c, _)| c).collect(),
        ..BusLayout::default()
    };
    unsafe { component.terminate() };
    layout.sidechain_channels()
}

impl Vst3Plugin {
    /// Load `bundle` and instantiate the audio-module class `plugin_id` (main thread).
    pub fn load(bundle: &Path, plugin_id: &str) -> Result<Self, PluginError> {
        let cid = crate::parse_class_id(plugin_id)
            .ok_or_else(|| PluginError::NotFound(format!("invalid VST3 class id {plugin_id}")))?;
        let module = Arc::new(Module::load(bundle)?);
        let host = ComWrapper::new(HostApplication);
        module.set_host_context(&host);
        let class = classes(&module)
            .into_iter()
            .find(|c| c.cid == cid && c.category == AUDIO_MODULE_CLASS)
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_owned()))?;
        let tuid: TUID = cid.map(|b| b as std::ffi::c_char);
        let host_ctx = host
            .as_com_ref::<FUnknown>()
            .map(|r| r.as_ptr())
            .ok_or_else(|| load_err("host context"))?;

        let component = create::<IComponent>(&module, &tuid)
            .ok_or_else(|| load_err("factory could not create the component"))?;
        // SAFETY (all calls below): valid COM objects used on their main thread.
        if unsafe { component.initialize(host_ctx) } != kResultOk {
            return Err(load_err("component initialize failed"));
        }
        let Some(processor) = component.cast::<IAudioProcessor>() else {
            unsafe { component.terminate() };
            return Err(load_err("component has no IAudioProcessor"));
        };

        // Controller: the component itself (single component) or a separate class.
        let (controller, separate) = match component.cast::<IEditController>() {
            Some(c) => (Some(c), false),
            None => {
                let mut ctrl_cid: TUID = [0; 16];
                let separate = (unsafe { component.getControllerClassId(&mut ctrl_cid) }
                    == kResultOk)
                    .then(|| create::<IEditController>(&module, &ctrl_cid))
                    .flatten()
                    .filter(|c| unsafe { c.initialize(host_ctx) } == kResultOk);
                (separate, true)
            }
        };

        let mut plugin = Self {
            component,
            processor,
            controller,
            separate_controller: separate,
            connection: None,
            handler: ComWrapper::new(ComponentHandler::default()),
            host,
            bundle: bundle.to_path_buf(),
            plugin_id: plugin_id.to_owned(),
            name: class.name.clone(),
            category: category_from_features(&class.features()),
            params: Vec::new(),
            steps: StepTable::default(),
            layout: BusLayout::default(),
            link: None,
            pending: Vec::new(),
            has_editor: Cell::new(None),
            editor: None,
            module: module.clone(),
        };

        if let Some(ctrl) = plugin.controller.clone() {
            if plugin.separate_controller
                && let (Some(a), Some(b)) = (
                    plugin.component.cast::<IConnectionPoint>(),
                    ctrl.cast::<IConnectionPoint>(),
                )
            {
                unsafe {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                }
                plugin.connection = Some((a, b));
            }
            // Controller starts from the component's state.
            let stream = MemoryStream::new();
            if let Some(s) = stream.as_com_ref::<IBStream>()
                && unsafe { plugin.component.getState(s.as_ptr()) } == kResultOk
            {
                stream.rewind();
                unsafe { ctrl.setComponentState(s.as_ptr()) };
            }
            if let Some(h) = plugin.handler.as_com_ref::<IComponentHandler>() {
                unsafe { ctrl.setComponentHandler(h.as_ptr()) };
            }
        }
        plugin.refresh_layout();
        plugin.refresh_params();
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

    fn refresh_params(&mut self) {
        (self.params, self.steps) = match &self.controller {
            Some(c) => params::list(c),
            None => (Vec::new(), StepTable::default()),
        };
    }

    fn refresh_layout(&mut self) {
        // SAFETY: valid component.
        let events = unsafe { self.component.getBusCount(kEvent as i32, kInput as i32) };
        let inputs = bus_infos(&self.component, kInput as i32);
        self.layout = BusLayout {
            aux_in: BusLayout::find_aux(&inputs),
            inputs: inputs.into_iter().map(|(c, _)| c).collect(),
            outputs: bus_channels(&self.component, kOutput as i32),
            event_input: events > 0,
        };
    }

    /// `Some(has editor)`, probing (`createView` + platform check) at most once, and only on
    /// the main thread; `None` = unknown (called off the main thread before any probe).
    fn probe_editor(&self) -> Option<bool> {
        if let Some(known) = self.has_editor.get() {
            return Some(known);
        }
        let known = if !HostWindow::SUPPORTED {
            false
        } else if HostWindow::on_main_thread() {
            self.create_view().is_some()
        } else {
            return None;
        };
        self.has_editor.set(Some(known));
        Some(known)
    }

    fn create_view(&self) -> Option<ComPtr<IPlugView>> {
        let ctrl = self.controller.as_ref()?;
        let name = c"editor".as_ptr(); // Vst::ViewType::kEditor
        // SAFETY: valid controller; the returned view is owned (one reference).
        let view = unsafe { ComPtr::from_raw(ctrl.createView(name)) }?;
        let supported = unsafe { view.isPlatformTypeSupported(HostWindow::PLATFORM_TYPE.as_ptr()) };
        (supported == kResultTrue).then_some(view)
    }

    fn query_latency(&self) -> u32 {
        // SAFETY: valid processor (main thread).
        unsafe { self.processor.getLatencySamples() }
    }

    /// Negotiate bus arrangements: stereo main buses when the plugin accepts them, the
    /// plugin's own arrangements otherwise. Returns the resulting channel counts.
    fn setup_buses(&mut self) -> BusLayout {
        let current = |dir: i32, n: usize| -> Vec<SpeakerArrangement> {
            (0..n as i32)
                .map(|i| {
                    let mut arr: SpeakerArrangement = 0;
                    // SAFETY: valid processor; `arr` is an out value.
                    unsafe { self.processor.getBusArrangement(dir, i, &mut arr) };
                    arr
                })
                .collect()
        };
        let (n_in, n_out) = (self.layout.inputs.len(), self.layout.outputs.len());
        let mut ins = current(kInput as i32, n_in);
        let mut outs = current(kOutput as i32, n_out);
        if let Some(a) = ins.first_mut() {
            *a = SpeakerArr::kStereo;
        }
        if let Some(a) = outs.first_mut() {
            *a = SpeakerArr::kStereo;
        }
        // SAFETY: arrays sized as passed; the plugin may reject (then keep its defaults).
        let ok = unsafe {
            self.processor.setBusArrangements(
                ins.as_mut_ptr(),
                n_in as i32,
                outs.as_mut_ptr(),
                n_out as i32,
            )
        };
        if ok != kResultOk {
            tracing::debug!("VST3 plugin rejected stereo main buses; using its defaults");
        }
        let channels = |dir| {
            current(dir, if dir == kInput as i32 { n_in } else { n_out })
                .into_iter()
                .map(|a| a.count_ones())
                .collect()
        };
        BusLayout {
            inputs: channels(kInput as i32),
            outputs: channels(kOutput as i32),
            event_input: self.layout.event_input,
            aux_in: self.layout.aux_in,
        }
    }

    fn set_bus_active(&self, active: bool) {
        let state = u8::from(active);
        // SAFETY (all calls): valid component; indices within the reported bus counts.
        unsafe {
            if !self.layout.inputs.is_empty() {
                self.component
                    .activateBus(kAudio as i32, kInput as i32, 0, state);
            }
            if !self.layout.outputs.is_empty() {
                self.component
                    .activateBus(kAudio as i32, kOutput as i32, 0, state);
            }
            if self.layout.event_input {
                self.component
                    .activateBus(kEvent as i32, kInput as i32, 0, state);
            }
            // The sidechain bus is active whenever the plugin is: without a source it gets
            // silence (the bus state can only change while inactive, a source can change
            // any time).
            if let Some(aux) = self.layout.aux_in {
                self.component
                    .activateBus(kAudio as i32, kInput as i32, aux as i32, state);
            }
        }
    }

    fn controller_normalized(&self, id: u32) -> Option<f64> {
        let ctrl = self.controller.as_ref()?;
        self.steps.steps(id)?;
        // SAFETY: valid controller (main thread).
        Some(unsafe { ctrl.getParamNormalized(id) })
    }

    fn set_controller_normalized(&self, id: u32, normalized: f64) {
        if let Some(ctrl) = &self.controller {
            // SAFETY: valid controller (main thread).
            unsafe { ctrl.setParamNormalized(id, normalized) };
        }
    }

    fn set_pending(&mut self, id: u32, normalized: f64) {
        match self.pending.iter_mut().find(|(k, _)| *k == id) {
            Some(p) => p.1 = normalized,
            None => self.pending.push((id, normalized)),
        }
    }

    fn close_editor_inner(&mut self) -> bool {
        let Some(editor) = self.editor.take() else {
            return false;
        };
        // SAFETY: the view is attached to `editor.window`'s content view, which is still alive.
        unsafe {
            editor.view.removed();
            editor.view.setFrame(std::ptr::null_mut());
        }
        drop(editor.view);
        drop(editor.frame);
        // The plugin view is gone; now the host window can go.
        editor.window.close();
        true
    }

    fn activate_node(&mut self, config: &PrepareConfig) -> Result<Vst3Node, PluginError> {
        if self.link.is_some() {
            return Err(PluginError::Activation("plugin is already active".into()));
        }
        self.refresh_layout();
        self.refresh_params();
        let layout = self.setup_buses();
        self.set_bus_active(true);
        let max_frames = config.max_block_size.max(1);
        let mut setup = ProcessSetup {
            processMode: kRealtime as i32,
            symbolicSampleSize: kSample32 as i32,
            maxSamplesPerBlock: max_frames as i32,
            sampleRate: f64::from(config.sample_rate),
        };
        // SAFETY (all calls): valid component/processor on the main thread.
        unsafe {
            if self.processor.canProcessSampleSize(kSample32 as i32) != kResultOk {
                return Err(PluginError::Activation(
                    "plugin cannot process 32-bit float".into(),
                ));
            }
            if self.processor.setupProcessing(&mut setup) != kResultOk {
                return Err(PluginError::Activation("setupProcessing failed".into()));
            }
            if self.component.setActive(1) != kResultOk {
                return Err(PluginError::Activation("setActive failed".into()));
            }
            self.processor.setProcessing(1);
        }
        let latency = self.query_latency();

        let values: Vec<(u32, f64)> = self
            .steps
            .ids()
            .map(|id| {
                let steps = self.steps.steps(id).unwrap_or(0);
                let norm = self.controller_normalized(id).unwrap_or(0.0);
                (id, to_plain(norm, steps))
            })
            .collect();
        let (to_main, from_node) = rtrb::RingBuffer::new(RING_CAPACITY);
        let (to_node, from_main) = rtrb::RingBuffer::new(RING_CAPACITY);
        let shared = Arc::new(NodeShared::default());
        shared.latency.store(latency, Ordering::Relaxed);
        self.link = Some(ActiveLink {
            from_node,
            to_node,
            shared: shared.clone(),
            fault_reported: false,
        });
        let descriptor = self.descriptor();
        Ok(Vst3Node::new(NodeInit {
            processor: self.processor.clone(),
            shared,
            to_main,
            from_main,
            descriptor,
            layout,
            config: *config,
            module: self.module.clone(),
            steps: self.steps.clone(),
            values,
            pending: std::mem::take(&mut self.pending),
            midi_map: self
                .controller
                .as_ref()
                .map(crate::node::MidiMap::query)
                .unwrap_or_default(),
        }))
    }

    fn deactivate_inner(&mut self) {
        let Some(mut link) = self.link.take() else {
            return;
        };
        // Values the processor saw last block → controller.
        while let Ok((id, v)) = link.from_node.pop() {
            self.set_controller_normalized(id, v);
        }
        // SAFETY: valid component/processor on the main thread; the node (audio side) is gone.
        unsafe {
            self.processor.setProcessing(0);
            self.component.setActive(0);
        }
        self.set_bus_active(false);
    }

    /// Values set while inactive only reached the controller: flush them into the processor
    /// (activate, zero-sample `process`, deactivate) so its state includes them.
    fn flush_pending(&mut self) -> Result<(), PluginError> {
        if self.link.is_some() || self.pending.is_empty() {
            return Ok(());
        }
        let config = PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 64,
            max_events_per_block: 16,
        };
        let mut node = self.activate_node(&config)?;
        node.flush_params();
        drop(node);
        self.deactivate_inner();
        Ok(())
    }

    /// Host-window housekeeping: user closed it, plugin asked for a resize, user resized it.
    fn poll_editor(&mut self, out: &mut Vec<PluginNotification>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if !editor.window.is_visible() {
            self.close_editor_inner();
            out.push(PluginNotification::EditorClosed);
            return;
        }
        if let Some(size) = editor.frame.take_resize() {
            editor.window.resize(size);
            let mut rect = size.to_rect();
            // SAFETY: attached view (main thread).
            unsafe { editor.view.onSize(&mut rect) };
            return;
        }
        let current = editor.window.content_size();
        if current != editor.window.known_size() {
            let mut rect = current.to_rect();
            // SAFETY (all calls): attached view (main thread).
            let size = unsafe {
                if editor.view.canResize() == kResultTrue {
                    editor.view.checkSizeConstraint(&mut rect);
                    Size::from_rect(&rect)
                } else {
                    let mut r: ViewRect = std::mem::zeroed();
                    if editor.view.getSize(&mut r) == kResultOk {
                        Size::from_rect(&r)
                    } else {
                        current
                    }
                }
            };
            let mut rect = size.to_rect();
            unsafe { editor.view.onSize(&mut rect) };
            if size != current {
                editor.window.resize(size);
            } else {
                editor.window.set_known_size(size);
            }
        }
    }
}

impl PluginController for Vst3Plugin {
    fn descriptor(&self) -> DeviceDescriptor {
        let (i, o) = self.layout.main_channels();
        DeviceDescriptor {
            layout: None,
            device_type: DeviceTypeRef::Plugin {
                plugin_id: self.plugin_id.clone(),
            },
            name: self.name.clone(),
            category: self.category,
            params: self.params.clone(),
            audio_inputs: i,
            audio_outputs: o,
            midi_input: self.layout.event_input,
            sidechain_inputs: self.layout.sidechain_channels(),
        }
    }

    fn params(&mut self) -> Vec<ParamInfo> {
        self.refresh_params();
        self.params.clone()
    }

    fn activate(&mut self, config: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
        Ok(Box::new(self.activate_node(config)?))
    }

    fn deactivate(&mut self, node: Box<dyn PluginNode>) {
        drop(node);
        self.deactivate_inner();
    }

    fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
        self.flush_pending()?;
        let component = MemoryStream::new();
        let s = component
            .as_com_ref::<IBStream>()
            .ok_or_else(|| PluginError::State("stream".into()))?;
        // SAFETY: valid component and stream (main thread).
        if unsafe { self.component.getState(s.as_ptr()) } != kResultOk {
            return Err(PluginError::State("component getState failed".into()));
        }
        let controller = MemoryStream::new();
        if let (Some(ctrl), Some(s)) = (&self.controller, controller.as_com_ref::<IBStream>()) {
            // Optional: many controllers keep no state of their own.
            // SAFETY: valid controller and stream (main thread).
            unsafe { ctrl.getState(s.as_ptr()) };
        }
        Ok(encode_state(&component.bytes(), &controller.bytes()))
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        if state.is_empty() {
            return Ok(());
        }
        let (component, controller) = decode_state(state)?;
        let stream = MemoryStream::from_bytes(component);
        let s = stream
            .as_com_ref::<IBStream>()
            .ok_or_else(|| PluginError::State("stream".into()))?;
        // SAFETY (all calls): valid objects and streams (main thread).
        if unsafe { self.component.setState(s.as_ptr()) } != kResultOk {
            return Err(PluginError::State("component setState failed".into()));
        }
        if let Some(ctrl) = &self.controller {
            stream.rewind();
            unsafe { ctrl.setComponentState(s.as_ptr()) };
            if !controller.is_empty() {
                let cs = MemoryStream::from_bytes(controller);
                if let Some(c) = cs.as_com_ref::<IBStream>() {
                    unsafe { ctrl.setState(c.as_ptr()) };
                }
            }
        }
        // The state now defines every value.
        self.pending.clear();
        Ok(())
    }

    fn param_value(&mut self, param: ParamId) -> Option<f64> {
        let steps = self.steps.steps(param.0)?;
        Some(to_plain(self.controller_normalized(param.0)?, steps))
    }

    fn set_param_value(&mut self, param: ParamId, value: f64) -> Result<(), PluginError> {
        if self.link.is_some() {
            return Err(PluginError::State(
                "plugin is active: send param events to its node".into(),
            ));
        }
        let steps = self
            .steps
            .steps(param.0)
            .ok_or_else(|| PluginError::State(format!("unknown param {}", param.0)))?;
        let norm = to_normalized(value, steps);
        self.set_controller_normalized(param.0, norm);
        self.set_pending(param.0, norm);
        Ok(())
    }

    fn has_editor(&self) -> bool {
        self.probe_editor().unwrap_or(false)
    }

    fn open_editor(&mut self) -> Result<(), PluginError> {
        match self.probe_editor() {
            Some(true) => {}
            Some(false) => return Err(PluginError::NoEditor),
            None => {
                return Err(PluginError::Load(
                    "editor: plugin editors must be opened on the main thread".into(),
                ));
            }
        }
        if let Some(e) = &self.editor {
            e.window.show();
            return Ok(());
        }
        let view = self.create_view().ok_or(PluginError::NoEditor)?;
        let editor_err = |e: String| PluginError::Load(format!("editor: {e}"));
        // SAFETY (all calls): valid view (main thread).
        let size = unsafe {
            let mut r: ViewRect = std::mem::zeroed();
            match view.getSize(&mut r) {
                x if x == kResultOk && r.right > r.left && r.bottom > r.top => Size::from_rect(&r),
                _ => DEFAULT_SIZE,
            }
        };
        let resizable = unsafe { view.canResize() } == kResultTrue;
        let window = HostWindow::open(&self.name, size, resizable).map_err(editor_err)?;
        let frame = PlugFrame::new();
        if let Some(f) = frame.as_com_ref::<vst3::Steinberg::IPlugFrame>() {
            unsafe { view.setFrame(f.as_ptr()) };
        }
        let attached = window.view_ptr().is_some_and(|parent| {
            // SAFETY: `parent` is the content view of `window`, kept alive until after
            // `removed()` (see `close_editor_inner`).
            unsafe { view.attached(parent, HostWindow::PLATFORM_TYPE.as_ptr()) == kResultOk }
        });
        if !attached {
            unsafe { view.setFrame(std::ptr::null_mut()) };
            window.close();
            return Err(editor_err("IPlugView::attached failed".into()));
        }
        window.show();
        self.editor = Some(Editor {
            view,
            frame,
            window,
        });
        Ok(())
    }

    fn close_editor(&mut self) {
        self.close_editor_inner();
    }

    fn poll(&mut self, out: &mut Vec<PluginNotification>) {
        // Values the processor received (automation, UI) → controller, so its GUI follows.
        let mut synced = Vec::new();
        if let Some(link) = self.link.as_mut() {
            while let Ok(msg) = link.from_node.pop() {
                synced.push(msg);
            }
            if link.shared.faulted.load(Ordering::Acquire) && !link.fault_reported {
                link.fault_reported = true;
                out.push(PluginNotification::Crashed {
                    message: "plugin process() failed".into(),
                });
            }
        }
        for (id, v) in synced {
            self.set_controller_normalized(id, v);
        }

        // Edits from the plugin's GUI (IComponentHandler).
        for edit in self.handler.take_edits() {
            match edit {
                Edit::Begin(id) => {
                    out.push(PluginNotification::GestureBegin { param: ParamId(id) })
                }
                Edit::End(id) => out.push(PluginNotification::GestureEnd { param: ParamId(id) }),
                Edit::Perform(id, norm) => {
                    let steps = self.steps.steps(id).unwrap_or(0);
                    // The processor learns about GUI edits only through the host.
                    match self.link.as_mut() {
                        Some(link) => {
                            let _ = link.to_node.push((id, norm));
                        }
                        None => self.set_pending(id, norm),
                    }
                    out.push(PluginNotification::ParamEdited {
                        param: ParamId(id),
                        value: to_plain(norm, steps),
                    });
                }
            }
        }

        let flags = self.handler.take_restart();
        if flags & kLatencyChanged != 0 {
            let samples = self.query_latency();
            if let Some(link) = &self.link {
                link.shared.latency.store(samples, Ordering::Relaxed);
            }
            out.push(PluginNotification::LatencyChanged { samples });
        }
        if flags & (kReloadComponent | kIoChanged) != 0
            || (flags & kLatencyChanged != 0 && self.link.is_some())
        {
            // VST3 applies bus and latency changes on re-activation.
            out.push(PluginNotification::RestartRequested);
        }
        if flags & (kParamTitlesChanged | kParamValuesChanged) != 0 {
            self.refresh_params();
            out.push(PluginNotification::ParamsChanged);
        }
        if self.handler.dirty.swap(false, Ordering::AcqRel) {
            out.push(PluginNotification::StateDirty);
        }
        self.poll_editor(out);
    }
}

impl Drop for Vst3Plugin {
    fn drop(&mut self) {
        self.close_editor_inner();
        if let Some(link) = self.link.take() {
            // The node should have been deactivated; don't leave the plugin processing.
            drop(link);
            // SAFETY: valid component/processor (main thread).
            unsafe {
                self.processor.setProcessing(0);
                self.component.setActive(0);
            }
        }
        // SAFETY (all calls): teardown order of the SDK: disconnect, drop the handler,
        // terminate the controller, then the component; the module is unloaded last.
        unsafe {
            if let Some((a, b)) = self.connection.take() {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(ctrl) = self.controller.take() {
                ctrl.setComponentHandler(std::ptr::null_mut());
                if self.separate_controller {
                    ctrl.terminate();
                }
            }
            self.component.terminate();
        }
        let _ = &self.host;
        // `processor`/`component` are released when the fields drop, before `module`
        // (declared last) drops its reference; the library unloads with the last node.
    }
}

/// Pack component + controller state into one blob (see [`STATE_MAGIC`]).
fn encode_state(component: &[u8], controller: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(STATE_MAGIC.len() + 12 + component.len() + controller.len());
    out.extend_from_slice(STATE_MAGIC);
    out.extend_from_slice(&STATE_VERSION.to_le_bytes());
    for part in [component, controller] {
        out.extend_from_slice(&(part.len() as u32).to_le_bytes());
        out.extend_from_slice(part);
    }
    out
}

fn decode_state(state: &[u8]) -> Result<(&[u8], &[u8]), PluginError> {
    let bad = |what: &str| PluginError::State(format!("invalid VST3 state blob: {what}"));
    let rest = state
        .strip_prefix(STATE_MAGIC)
        .ok_or_else(|| bad("magic"))?;
    let take_u32 = |r: &[u8]| -> Option<u32> {
        r.get(..4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let version = take_u32(rest).ok_or_else(|| bad("truncated"))?;
    if version != STATE_VERSION {
        return Err(bad(&format!("unknown version {version}")));
    }
    let mut rest = &rest[4..];
    let mut part = || -> Result<&[u8], PluginError> {
        let len = take_u32(rest).ok_or_else(|| bad("truncated"))? as usize;
        let data = rest.get(4..4 + len).ok_or_else(|| bad("truncated"))?;
        rest = &rest[4 + len..];
        Ok(data)
    };
    let component = part()?;
    let controller = part()?;
    Ok((component, controller))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_framing_round_trip() {
        let blob = encode_state(b"component", b"");
        assert_eq!(decode_state(&blob).unwrap(), (&b"component"[..], &b""[..]));
        let blob = encode_state(b"", b"ctrl");
        assert_eq!(decode_state(&blob).unwrap(), (&b""[..], &b"ctrl"[..]));
        assert!(decode_state(b"nope").is_err());
        assert!(decode_state(&blob[..blob.len() - 1]).is_err());
        let mut v2 = blob.clone();
        v2[8] = 2;
        assert!(decode_state(&v2).is_err());
    }
}
