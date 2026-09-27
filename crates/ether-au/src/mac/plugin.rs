//! Main-thread half of an AU instance ([`AuPlugin`]).

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use ether_core::config::PrepareConfig;
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo};
use ether_core::protocol::model::ParamId;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, msg_send};
use objc2_audio_toolbox::{
    AUAudioUnit, AUAudioUnitBus, AUAudioUnitBusArray, AUParameterAutomationEvent,
    AUParameterAutomationEventType, AUParameterObserverToken,
};
use objc2_avf_audio::AVAudioFormat;
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSPropertyListFormat, NSPropertyListSerialization, NSString,
};

use super::editor::{self, EditorWindow};
use super::node::{
    AuNode, MAX_CHANNELS, NodeInit, NodeShared, ParamLinks, RenderFn, ScheduleMidiFn,
    ScheduleParamFn, rt_parts,
};
use super::params::ParamSet;
use crate::AuComponentId;
use crate::ids::{decode_state, encode_state};

/// Notifications produced by the parameter observer (any thread) for `poll`.
#[derive(Default)]
struct Observed {
    queue: Mutex<VecDeque<PluginNotification>>,
    /// Links of the active node (echo suppression of host-scheduled values), else `None`.
    links: Mutex<Option<Arc<ParamLinks>>>,
    /// Address → id, refreshed with the params.
    ids: Mutex<Vec<(u64, u32)>>,
}

struct ActiveLink {
    shared: Arc<NodeShared>,
    sample_rate: f64,
    fault_reported: bool,
}

/// An AU instance (main-thread half). Not `Send`: create and use it on the plugin main
/// thread (on macOS, the process main thread for editors).
pub struct AuPlugin {
    au: Retained<AUAudioUnit>,
    id: AuComponentId,
    name: String,
    category: DeviceCategory,
    params: ParamSet,
    observed: Arc<Observed>,
    observer_token: AUParameterObserverToken,
    /// Tree the observer is installed on (to detect a replaced tree).
    observed_tree: Option<usize>,
    io: (u16, u16, bool),
    link: Option<ActiveLink>,
    latency: u32,
    editor: Option<EditorWindow>,
    /// Custom editor available (cached at load).
    has_view: bool,
}

impl std::fmt::Debug for AuPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuPlugin")
            .field("id", &self.id.to_string())
            .field("active", &self.link.is_some())
            .finish()
    }
}

fn ns_error(e: &NSError) -> String {
    e.localizedDescription().to_string()
}

/// Channel count of bus 0 of `busses` (0 if none).
fn bus0(busses: &AUAudioUnitBusArray) -> Option<Retained<AUAudioUnitBus>> {
    // SAFETY: bounds checked by `count`.
    unsafe { (busses.count() > 0).then(|| busses.objectAtIndexedSubscript(0)) }
}

fn bus_channels(bus: &AUAudioUnitBus) -> u32 {
    // `format` is declared in AVFAudio's AUAudioUnitBus category (not bound by objc2).
    // SAFETY: `-[AUAudioUnitBus format]` returns a non-null AVAudioFormat.
    let format: Retained<AVAudioFormat> = unsafe { msg_send![bus, format] };
    unsafe { format.channelCount() }
}

/// Set a standard (deinterleaved f32) format on `bus`: `preferred` channels if accepted,
/// else the bus' current channel count, at `sample_rate`. Returns the channel count.
fn set_bus_format(bus: &AUAudioUnitBus, sample_rate: f64, preferred: u32) -> Result<u32, String> {
    let current = bus_channels(bus);
    let mut last_err = String::new();
    for ch in [preferred, current] {
        if ch == 0 || ch as usize > MAX_CHANNELS {
            continue;
        }
        // SAFETY: plain initializer.
        let Some(format) = (unsafe {
            AVAudioFormat::initStandardFormatWithSampleRate_channels(
                AVAudioFormat::alloc(),
                sample_rate,
                ch,
            )
        }) else {
            continue;
        };
        // SAFETY: `-[AUAudioUnitBus setFormat:error:]` (AVFAudio category).
        let ok: Result<(), Retained<NSError>> =
            unsafe { msg_send![bus, setFormat: &*format, error: _] };
        match ok {
            Ok(()) => return Ok(ch),
            Err(e) => last_err = ns_error(&e),
        }
    }
    Err(format!("unsupported stream format: {last_err}"))
}

impl AuPlugin {
    /// Instantiate the component `plugin_id` (main thread).
    pub fn load(plugin_id: &str) -> Result<Self, PluginError> {
        let id = AuComponentId::parse(plugin_id)
            .ok_or_else(|| PluginError::NotFound(format!("not an AU id: {plugin_id}")))?;
        let (au, name, _vendor) = super::instantiate(&id)?;
        let mut plugin = Self {
            au,
            id,
            name,
            category: id.category(),
            params: ParamSet::empty(),
            observed: Arc::new(Observed::default()),
            observer_token: std::ptr::null_mut(),
            observed_tree: None,
            io: (0, 0, false),
            link: None,
            latency: 0,
            editor: None,
            has_view: false,
        };
        plugin.refresh_params(None);
        plugin.refresh_io();
        plugin.latency = plugin.query_latency(48_000.0);
        plugin.has_view = editor::has_custom_view(&plugin.au);
        Ok(plugin)
    }

    /// Test support: change a parameter the way the unit's own GUI would (no originator,
    /// so every observer is told).
    #[doc(hidden)]
    pub fn simulate_gui_edit(&mut self, param: ParamId, value: f64) {
        if let Some(p) = self.params.object(param.0) {
            unsafe { p.setValue_originator(value as f32, std::ptr::null_mut()) };
        }
    }

    pub fn is_active(&self) -> bool {
        self.link.is_some()
    }

    fn refresh_io(&mut self) {
        // SAFETY: plain getters.
        let (inputs, outputs) = unsafe { (self.au.inputBusses(), self.au.outputBusses()) };
        let i = bus0(&inputs).map_or(0, |b| bus_channels(&b)) as u16;
        let o = bus0(&outputs).map_or(0, |b| bus_channels(&b)) as u16;
        let midi = matches!(&self.id.component_type, b"aumu" | b"aumf" | b"aumi")
            || unsafe { !self.au.scheduleMIDIEventBlock().is_null() };
        self.io = (i.min(2), o.min(2), midi);
    }

    fn query_latency(&self, sample_rate: f64) -> u32 {
        // SAFETY: plain getter.
        let secs = unsafe { self.au.latency() };
        if secs.is_finite() && secs > 0.0 {
            (secs * sample_rate).round() as u32
        } else {
            0
        }
    }

    /// Re-read the parameter tree and (re)install the observer if the tree changed.
    fn refresh_params(&mut self, saved: Option<&[(u64, u32)]>) {
        self.params.refresh(&self.au, saved);
        if let Ok(mut ids) = self.observed.ids.lock() {
            *ids = self.params.map.entries().to_vec();
        }
        let tree_ptr = self
            .params
            .tree
            .as_ref()
            .map(|t| Retained::as_ptr(t) as usize);
        if tree_ptr != self.observed_tree {
            self.install_observer();
            self.observed_tree = tree_ptr;
        }
    }

    fn install_observer(&mut self) {
        self.remove_observer();
        let Some(tree) = self.params.tree.clone() else {
            return;
        };
        let observed = self.observed.clone();
        let block = RcBlock::new(
            move |n: isize, events: std::ptr::NonNull<AUParameterAutomationEvent>| {
                // SAFETY: `events` holds `n` events.
                let events =
                    unsafe { std::slice::from_raw_parts(events.as_ptr(), n.max(0) as usize) };
                let ids = match observed.ids.lock() {
                    Ok(ids) => ids,
                    Err(_) => return,
                };
                let links = observed.links.lock().ok().and_then(|l| l.clone());
                let mut out = Vec::new();
                for e in events {
                    let Ok(i) = ids.binary_search_by_key(&e.address, |x| x.0) else {
                        continue;
                    };
                    let param = ParamId(ids[i].1);
                    match e.eventType {
                        AUParameterAutomationEventType::Touch => {
                            out.push(PluginNotification::GestureBegin { param })
                        }
                        AUParameterAutomationEventType::Release => {
                            out.push(PluginNotification::GestureEnd { param })
                        }
                        _ => {
                            // Skip echoes of values the host itself scheduled.
                            let echo = links.as_ref().is_some_and(|l| {
                                l.index_of_address(e.address).is_some_and(|k| {
                                    l.last_host[k].load(Ordering::Relaxed) == e.value.to_bits()
                                })
                            });
                            if !echo {
                                out.push(PluginNotification::ParamEdited {
                                    param,
                                    value: f64::from(e.value),
                                });
                            }
                        }
                    }
                }
                drop(ids);
                if !out.is_empty()
                    && let Ok(mut q) = observed.queue.lock()
                {
                    q.extend(out);
                }
            },
        );
        // SAFETY: the tree copies the block; the token is removed in `remove_observer`.
        self.observer_token =
            unsafe { tree.tokenByAddingParameterAutomationObserver(RcBlock::as_ptr(&block)) };
    }

    fn remove_observer(&mut self) {
        if self.observer_token.is_null() {
            return;
        }
        if let Some(tree) = &self.params.tree {
            // SAFETY: token from `tokenByAddingParameterAutomationObserver` on this tree.
            unsafe { tree.removeParameterObserver(self.observer_token) };
        }
        self.observer_token = std::ptr::null_mut();
    }

    fn plist_state(&self) -> Result<Vec<u8>, PluginError> {
        // SAFETY: plain getters.
        let dict = unsafe {
            self.au
                .fullStateForDocument()
                .or_else(|| self.au.fullState())
        };
        let Some(dict) = dict else {
            return Ok(Vec::new());
        };
        // SAFETY: serializing a property-list dictionary.
        let data = unsafe {
            NSPropertyListSerialization::dataWithPropertyList_format_options_error(
                &dict,
                NSPropertyListFormat::BinaryFormat_v1_0,
                0,
            )
        }
        .map_err(|e| PluginError::State(ns_error(&e)))?;
        Ok(data.to_vec())
    }

    fn apply_plist(&self, plist: &[u8]) -> Result<(), PluginError> {
        if plist.is_empty() {
            return Ok(());
        }
        let data = NSData::with_bytes(plist);
        // SAFETY: parsing untrusted bytes is what this API is for.
        let obj: Retained<AnyObject> = unsafe {
            NSPropertyListSerialization::propertyListWithData_options_format_error(
                &data,
                objc2_foundation::NSPropertyListMutabilityOptions::Immutable,
                std::ptr::null_mut(),
            )
        }
        .map_err(|e| PluginError::State(ns_error(&e)))?;
        let dict = obj
            .downcast::<NSDictionary>()
            .map_err(|_| PluginError::State("AU state is not a dictionary".into()))?;
        // SAFETY: the unit validates its own state dictionary.
        // (An NSDictionary<NSString, AnyObject> as the API expects: plist keys are strings.)
        let dict: Retained<NSDictionary<NSString, AnyObject>> =
            unsafe { Retained::cast_unchecked(dict) };
        unsafe { self.au.setFullStateForDocument(Some(&dict)) };
        Ok(())
    }
}

impl PluginController for AuPlugin {
    fn descriptor(&self) -> DeviceDescriptor {
        DeviceDescriptor {
            layout: None,
            device_type: DeviceTypeRef::Plugin {
                plugin_id: self.id.to_string(),
            },
            name: self.name.clone(),
            category: self.category,
            params: self.params.infos.clone(),
            audio_inputs: self.io.0,
            audio_outputs: self.io.1,
            midi_input: self.io.2,
            // Extra input busses (AU sidechains) are not routed yet.
            sidechain_inputs: 0,
        }
    }

    fn params(&mut self) -> Vec<ParamInfo> {
        self.refresh_params(None);
        self.params.infos.clone()
    }

    fn activate(&mut self, config: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
        if self.link.is_some() {
            return Err(PluginError::Activation("plugin is already active".into()));
        }
        let act = |e: String| PluginError::Activation(e);
        let sample_rate = f64::from(config.sample_rate);
        let max_frames = config.max_block_size.max(1);

        // Engine format on the main busses: non-interleaved f32 at the engine rate.
        // SAFETY: plain getters/setters on the main thread, unit not rendering.
        let (inputs, outputs) = unsafe { (self.au.inputBusses(), self.au.outputBusses()) };
        let out_bus = bus0(&outputs).ok_or_else(|| act("the unit has no output bus".into()))?;
        let out_channels = set_bus_format(&out_bus, sample_rate, 2).map_err(act)? as usize;
        let mut in_channels = 0usize;
        if let Some(in_bus) = bus0(&inputs) {
            in_channels = set_bus_format(&in_bus, sample_rate, 2).map_err(act)? as usize;
            unsafe { in_bus.setEnabled(true) };
        }
        // Other busses keep their defaults but must share the sample rate; extra input
        // busses (sidechains) are disabled: they are not routed yet.
        for (is_input, busses) in [(true, &inputs), (false, &outputs)] {
            for i in 1..unsafe { busses.count() } {
                let bus = unsafe { busses.objectAtIndexedSubscript(i) };
                let ch = bus_channels(&bus);
                let _ = set_bus_format(&bus, sample_rate, ch);
                if is_input {
                    unsafe { bus.setEnabled(false) };
                }
            }
        }
        // Sent as a 64-bit value: `AUAudioUnitV2Bridge` declares the setter with an
        // NSUInteger (`Q`) argument while `AUAudioUnit` uses `AUAudioFrameCount`; objc2's
        // encoding check would reject the generated binding. The low 32 bits are what a
        // `UInt32` implementation reads either way (arm64/x86_64 register passing).
        let frames = max_frames as u64;
        let _: () = unsafe { msg_send![&*self.au, setMaximumFramesToRender: frames] };

        let parts = rt_parts(in_channels, max_frames);
        // Host blocks must be installed before allocating render resources.
        unsafe {
            self.au
                .setMusicalContextBlock(RcBlock::as_ptr(&parts.host.musical));
            self.au
                .setTransportStateBlock(RcBlock::as_ptr(&parts.host.transport));
            self.au
                .setMIDIOutputEventBlock(RcBlock::as_ptr(&parts.host.midi_out));
        }
        // Fetch and cache the RT blocks BEFORE allocating render resources: AUAudioUnit.h
        // asks hosts that schedule events to do so, and some v3 units only set up
        // parameter/MIDI scheduling if the blocks were fetched first (otherwise events are
        // dropped silently). The copies stay valid across allocate/deallocate.
        let render: Option<RcBlock<RenderFn>> = unsafe { RcBlock::copy(self.au.renderBlock()) };
        let schedule_param: Option<RcBlock<ScheduleParamFn>> =
            unsafe { RcBlock::copy(self.au.scheduleParameterBlock()) };
        let schedule_midi: Option<RcBlock<ScheduleMidiFn>> =
            unsafe { RcBlock::copy(self.au.scheduleMIDIEventBlock()) };

        unsafe { self.au.allocateRenderResourcesAndReturnError() }
            .map_err(|e| act(ns_error(&e)))?;
        let Some(render) = render else {
            unsafe { self.au.deallocateRenderResources() };
            return Err(act("the unit has no render block".into()));
        };

        self.refresh_params(None);
        self.refresh_io();
        let links = Arc::new(ParamLinks::new(self.params.map.entries().to_vec()));
        if let Ok(mut l) = self.observed.links.lock() {
            *l = Some(links.clone());
        }
        let values: Vec<(u32, u64, f64)> = self
            .params
            .objects
            .iter()
            .filter_map(|(id, p)| {
                let addr = self.params.map.address(*id)?;
                Some((*id, addr, f64::from(unsafe { p.value() })))
            })
            .collect();
        let shared = Arc::new(NodeShared::default());
        self.latency = self.query_latency(sample_rate);
        shared.latency.store(self.latency, Ordering::Relaxed);
        self.link = Some(ActiveLink {
            shared: shared.clone(),
            sample_rate,
            fault_reported: false,
        });
        let has_input = in_channels > 0;
        Ok(Box::new(AuNode::new(
            NodeInit {
                au: self.au.clone(),
                render,
                schedule_param,
                schedule_midi,
                links,
                shared,
                descriptor: self.descriptor(),
                has_input,
                in_channels,
                out_channels,
                max_frames,
                sample_rate,
                values,
            },
            parts,
        )))
    }

    fn deactivate(&mut self, node: Box<dyn PluginNode>) {
        // Stop the unit from using the node's host blocks before they are freed.
        unsafe {
            self.au.deallocateRenderResources();
            self.au.setMusicalContextBlock(std::ptr::null_mut());
            self.au.setTransportStateBlock(std::ptr::null_mut());
            self.au.setMIDIOutputEventBlock(std::ptr::null_mut());
        }
        drop(node);
        self.link = None;
        if let Ok(mut l) = self.observed.links.lock() {
            *l = None;
        }
    }

    fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
        let plist = self.plist_state()?;
        Ok(encode_state(self.params.map.entries(), &plist))
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        if state.is_empty() {
            return Ok(());
        }
        let (table, plist) = decode_state(state).map_err(PluginError::State)?;
        self.apply_plist(plist)?;
        self.refresh_params(Some(&table));
        Ok(())
    }

    fn param_value(&mut self, param: ParamId) -> Option<f64> {
        let p = self.params.object(param.0)?;
        Some(f64::from(unsafe { p.value() }))
    }

    fn set_param_value(&mut self, param: ParamId, value: f64) -> Result<(), PluginError> {
        let p = self
            .params
            .object(param.0)
            .ok_or_else(|| PluginError::State(format!("unknown param {}", param.0)))?;
        // Originator = our observer token, so the change is not echoed back as an edit.
        unsafe { p.setValue_originator(value as f32, self.observer_token) };
        Ok(())
    }

    fn has_editor(&self) -> bool {
        self.has_view
    }

    fn open_editor(&mut self) -> Result<(), PluginError> {
        if let Some(w) = &self.editor {
            w.show();
            return Ok(());
        }
        if !self.has_editor() {
            return Err(PluginError::NoEditor);
        }
        let window = EditorWindow::open(&self.au, &self.name)
            .map_err(|e| PluginError::Load(format!("editor: {e}")))?;
        self.editor = Some(window);
        Ok(())
    }

    fn close_editor(&mut self) {
        if let Some(w) = self.editor.take() {
            w.close();
        }
    }

    fn poll(&mut self, out: &mut Vec<PluginNotification>) {
        // Hosts without a running run loop on the plugin main thread (the sandbox helper,
        // tests) still get AU work delivered to it (observers, main-queue callbacks). Never
        // re-entered when a run loop is already running (the app's main thread).
        super::service_idle_run_loop();
        if let Ok(mut q) = self.observed.queue.lock() {
            out.extend(q.drain(..));
        }
        if let Some(link) = &self.link
            && link.shared.reset_requested.swap(false, Ordering::AcqRel)
        {
            // Deferred from the audio thread (out-of-process unit, see `AuNode::reset`).
            unsafe { self.au.reset() };
        }
        if let Some(link) = self.link.as_mut()
            && link.shared.faulted.load(Ordering::Acquire)
            && !link.fault_reported
        {
            link.fault_reported = true;
            out.push(PluginNotification::Crashed {
                message: "Audio Unit render failed repeatedly".into(),
            });
        }
        // Latency (AUAudioUnit.latency, polled).
        let sr = self.link.as_ref().map_or(48_000.0, |l| l.sample_rate);
        let latency = self.query_latency(sr);
        if latency != self.latency {
            self.latency = latency;
            if let Some(link) = &self.link {
                link.shared.latency.store(latency, Ordering::Relaxed);
            }
            out.push(PluginNotification::LatencyChanged { samples: latency });
        }
        // Parameter tree replaced (v3 units may rebuild it).
        let tree = unsafe { self.au.parameterTree() }.map(|t| Retained::as_ptr(&t) as usize);
        if tree != self.observed_tree {
            self.refresh_params(None);
            out.push(PluginNotification::ParamsChanged);
        }
        if let Some(w) = &self.editor
            && !w.is_visible()
        {
            if let Some(w) = self.editor.take() {
                w.close();
            }
            out.push(PluginNotification::EditorClosed);
        }
    }
}

impl Drop for AuPlugin {
    fn drop(&mut self) {
        self.close_editor();
        self.remove_observer();
    }
}
