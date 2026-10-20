//! [`NativeBridge`]: the controller's [`EngineBridge`] over an in-process `EngineHandle`,
//! plus the native [`HostServices`].
//!
//! Runs on the controller thread. Built-in devices are constructed here with
//! `ether-devices`; plugins are instantiated and activated on the main thread through
//! [`PluginHost`] and only their audio half is sent to the engine. Media arrives decoded;
//! it is resampled here if needed (never on the audio thread) and registered with the
//! engine as an in-memory `AudioSource` (see [`crate::media`] for the no-streaming
//! decision).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ether_controller::{BridgeError, EngineBridge, HostServices, RecordSession, RecordedTakes};
use ether_core::plugin::{PluginError, PluginNotification};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, BuiltinDeviceType, DeviceId, MediaId, MediaRef, ParamId,
    PluginInstance, TrackId,
};
use ether_core::protocol::recording::InputList;
use ether_core::{
    AudioSource, EngineError, EngineHandle, EngineOutputs, NodeKey, ParamChange, PrepareConfig,
    RenderGraphDesc, TransportControl,
};
use ether_media::{DecodedAudio, InMemorySource};

use crate::plugins::{Instantiate, PluginCatalog, PluginHost, PluginSource, format_label};
use crate::rt::AudioShared;

fn engine_err(e: EngineError) -> BridgeError {
    match e {
        EngineError::QueueFull => BridgeError::QueueFull,
        other => BridgeError::Other(other.to_string()),
    }
}

fn plugin_err(e: PluginError) -> BridgeError {
    BridgeError::Other(e.to_string())
}

/// The error for a plugin missing from the scanned catalog (the device stays in the
/// document without an engine node, i.e. bypassed, until a rescan finds it).
fn not_installed(plugin: &PluginInstance) -> String {
    let name = if plugin.name.is_empty() || plugin.name == plugin.plugin_id {
        String::new()
    } else {
        format!("{} ", plugin.name)
    };
    format!(
        "{} plugin {name}({}) is not installed (rescan plugins)",
        format_label(plugin.format),
        plugin.plugin_id
    )
}

enum DeviceKind {
    Builtin(BuiltinDeviceType),
    Plugin(Box<DeviceDescriptor>),
}

struct DeviceEntry {
    key: NodeKey,
    kind: DeviceKind,
}

/// Resolves sampler media against the sources registered with the engine.
struct Sources<'a>(&'a HashMap<MediaId, Arc<dyn AudioSource>>);

impl ether_devices::SampleResolver for Sources<'_> {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

/// Native engine bridge (controller thread).
pub struct NativeBridge {
    handle: EngineHandle,
    prepare: PrepareConfig,
    sample_rate: u32,
    plugins: PluginHost,
    catalog: PluginCatalog,
    /// The host's "a plugin scan is running" flag (`set_scan_flag`).
    scanning: Option<Arc<std::sync::atomic::AtomicBool>>,
    instantiate: Instantiate,
    audio: Arc<AudioShared>,
    sources: HashMap<MediaId, Arc<dyn AudioSource>>,
    devices: HashMap<DeviceId, DeviceEntry>,
    /// Instances created for offline export so far (their private registry ids).
    offline_seq: u64,
    /// An extension's part of the bridge ([`set_bridge_ext`]).
    ext: Option<Box<dyn BridgeExt>>,
    /// GUI-only plugin mirrors while listening ([`crate::plugin_mirror`]).
    mirrors: crate::plugin_mirror::Mirrors,
    /// External Instruments' MIDI (`external-instrument`, [`hw_midi`]): the engine ring until
    /// a graph routes MIDI to hardware, then the output thread.
    hw_midi_io: Option<ether_core::hw_io::HwIoIo>,
    hw_midi: Option<hw_midi::HwMidiOut>,
    hw_routes: Arc<hw_midi::Routes>,
    /// The shared disk reader thread (started with the first stream).
    disk: Option<crate::disk_stream::DiskStreams>,
    /// The last published graph while media are streamed (anchors,
    /// locate targets), and a `SetLoop` override of its loop.
    stream_graph: Option<RenderGraphDesc>,
    /// Audio streaming on (default; `ETHER_DISK_STREAMING=0` turns it off).
    disk_streaming: bool,
    /// 0 = the main engine; otherwise a dedicated engine whose plugin
    /// instances are registered under salted ids ([`Self::reg`]).
    registry_salt: u16,
    /// This engine started dedicated engines (their plugin notifications are drained here).
    spawned_engines: bool,
    /// Dedicated engines' plugin notifications, by registry salt: the main bridge drains the
    /// registry's one queue and files theirs here; each dedicated bridge takes its own.
    dedicated_notes: DedicatedNotes,
}

/// An extension's part of the native bridge (docs/EXTENSIONS.md): made for every bridge (the
/// main engine's and each dedicated engine's) by the factory set with [`set_bridge_ext`], it
/// sees what the bridge sends the engine. The extension reaches it, and the engine handle for
/// taps of its own, with [`NativeBridge::ext_mut`] / [`NativeBridge::ext_and_handle`]. It is
/// dropped before the bridge's plugin instances.
pub trait BridgeExt: std::any::Any + Send {
    /// A graph the bridge published.
    fn observe_graph(&mut self, graph: &RenderGraphDesc) {
        let _ = graph;
    }

    /// A transport control the bridge sent.
    fn observe_transport(&mut self, control: &TransportControl) {
        let _ = control;
    }

    /// A dedicated engine was spawned from this bridge: `child` is its bridge's extension;
    /// `plugins` and `salt` name its plugin instances ([`dedicated_device_id`]).
    fn spawned(&mut self, child: &mut dyn BridgeExt, plugins: &PluginHost, salt: u16) {
        let _ = (child, plugins, salt);
    }
}

/// Makes a bridge's [`BridgeExt`] (it may read the bridge: its plugin host, sample rate).
pub type BridgeExtFactory = fn(&NativeBridge) -> Box<dyn BridgeExt>;

static BRIDGE_EXT: std::sync::OnceLock<BridgeExtFactory> = std::sync::OnceLock::new();

/// Extension point: the extension part of every native bridge made from now on (set once,
/// at start).
pub fn set_bridge_ext(factory: BridgeExtFactory) {
    let _ = BRIDGE_EXT.set(factory);
}

/// Plugin notifications of dedicated engines waiting for their bridge's poll, by registry
/// salt (registry ids). A salt is present while its engine exists.
type DedicatedNotes = Arc<std::sync::Mutex<HashMap<u16, Vec<(DeviceId, PluginNotification)>>>>;

/// At most this many notifications wait for a dedicated engine's poll (oldest dropped).
const MAX_DEDICATED_NOTES: usize = 4096;

impl NativeBridge {
    /// Share the host's "a plugin scan is running" flag, reported by
    /// [`EngineBridge::plugin_catalog`].
    pub fn set_scan_flag(&mut self, scanning: Arc<std::sync::atomic::AtomicBool>) {
        self.scanning = Some(scanning);
    }

    pub fn new(
        handle: EngineHandle,
        prepare: PrepareConfig,
        plugins: PluginHost,
        catalog: PluginCatalog,
        instantiate: Instantiate,
        audio: Arc<AudioShared>,
    ) -> Self {
        let mut handle = handle;
        crate::recording::attach(&mut handle, &audio);
        let hw_midi_io = handle.take_hw_io();
        let mut bridge = Self {
            handle,
            sample_rate: prepare.sample_rate as u32,
            prepare,
            plugins,
            catalog,
            scanning: None,
            instantiate,
            audio,
            sources: HashMap::new(),
            devices: HashMap::new(),
            offline_seq: 0,
            ext: None,
            mirrors: Default::default(),
            hw_midi_io,
            hw_midi: None,
            hw_routes: Default::default(),
            disk: None,
            stream_graph: None,
            registry_salt: 0,
            spawned_engines: false,
            dedicated_notes: Default::default(),
            disk_streaming: !std::env::var("ETHER_DISK_STREAMING")
                .is_ok_and(|v| v == "0" || v == "off"),
        };
        bridge.ext = BRIDGE_EXT.get().map(|make| make(&bridge));
        bridge
    }

    /// The extension part of this bridge ([`set_bridge_ext`]), when it is a `T`.
    pub fn ext_mut<T: BridgeExt>(&mut self) -> Option<&mut T> {
        let ext: &mut dyn std::any::Any = self.ext.as_deref_mut()?;
        ext.downcast_mut()
    }

    /// The extension part of this bridge ([`set_bridge_ext`]), when it is a `T`.
    pub fn ext_ref<T: BridgeExt>(&self) -> Option<&T> {
        let ext: &dyn std::any::Any = self.ext.as_deref()?;
        ext.downcast_ref()
    }

    /// The extension part and the engine handle (an extension's own taps of the engine).
    pub fn ext_and_handle<T: BridgeExt>(&mut self) -> Option<(&mut T, &mut EngineHandle)> {
        let ext: &mut dyn std::any::Any = self.ext.as_deref_mut()?;
        Some((ext.downcast_mut()?, &mut self.handle))
    }

    /// `external-instrument`: the MIDI routes of `graph` (node → port); starts the MIDI
    /// output thread the first time a graph has one.
    fn route_hw_midi(&mut self, graph: &RenderGraphDesc) {
        let routes: HashMap<NodeKey, String> = graph
            .tracks
            .iter()
            .flat_map(|t| t.hw_io.iter())
            .filter_map(|d| Some((d.node, d.routing.midi_out.clone()?)))
            .collect();
        if routes.is_empty() && self.hw_midi.is_none() {
            return;
        }
        self.hw_routes.set(routes);
        if self.hw_midi.is_none()
            && let Some(io) = self.hw_midi_io.take()
        {
            match hw_midi::spawn(
                io,
                self.hw_routes.clone(),
                self.audio.clone(),
                f64::from(self.sample_rate),
                self.prepare.max_block_size as u32,
                Box::new(hw_midi::MidirOut::default()),
            ) {
                Ok(t) => self.hw_midi = Some(t),
                Err(e) => tracing::warn!(error = %e, "could not start the hardware MIDI thread"),
            }
        }
    }

    /// Stream long media from disk (on by default) or decode them whole
    /// (v0.2). Applies to media loaded from now on.
    pub fn set_disk_streaming(&mut self, enabled: bool) {
        self.disk_streaming = enabled;
    }

    /// Media currently streamed from disk (tests/diagnostics).
    pub fn streamed_media(&self) -> Vec<MediaId> {
        self.disk.as_ref().map_or_else(Vec::new, |d| {
            self.sources
                .keys()
                .copied()
                .filter(|m| d.get(*m).is_some())
                .collect()
        })
    }

    /// The stream cache of `media`, if streamed (tests/diagnostics).
    pub fn stream_cache(&self, media: MediaId) -> Option<Arc<ether_media::stream::StreamCache>> {
        self.disk.as_ref()?.get(media).map(|s| s.cache.clone())
    }

    /// Re-derive the anchors of every stream from the last graph.
    fn update_stream_anchors(&mut self) {
        let (Some(disk), Some(graph)) = (self.disk.as_mut(), self.stream_graph.as_ref()) else {
            return;
        };
        let frames = |m: MediaId| disk.frames(m);
        let anchors = ether_media::stream::anchors::anchors(graph, self.sample_rate, &frames);
        disk.set_anchors(anchors);
    }

    /// Make sure what plays at `beat` is decoded before the engine jumps
    /// there (bounded wait on the controller thread).
    fn prime_streams_at(&mut self, beat: f64) {
        let (Some(disk), Some(graph)) = (self.disk.as_ref(), self.stream_graph.as_ref()) else {
            return;
        };
        if disk.is_empty() {
            return;
        }
        let frames = |m: MediaId| disk.frames(m);
        let targets =
            ether_media::stream::anchors::positions_at(graph, beat, self.sample_rate, &frames);
        if !disk.prime(targets, crate::disk_stream::PRIME_TIMEOUT) {
            tracing::debug!(beat, "stream prime timed out");
        }
    }

    /// A second, independent engine for a participant: its own
    /// `ether_core` engine on the paced `null` backend (no audio device), its own bridge
    /// (own stream sender and tap), the same plugin host (plugins registered under ids of
    /// their own). The result is dropped to stop it.
    pub fn spawn_dedicated(&mut self) -> Result<crate::dedicated::DedicatedEngine, BridgeError> {
        use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
        static SALT: AtomicU16 = AtomicU16::new(1);
        // Never 0 (the main engine's).
        let mut salt = SALT.fetch_add(1, Ordering::Relaxed);
        if salt == 0 {
            salt = SALT.fetch_add(1, Ordering::Relaxed);
        }
        let config = ether_core::EngineConfig {
            sample_rate: self.prepare.sample_rate as u32,
            max_block_size: self.prepare.max_block_size,
            max_events_per_block: self.prepare.max_events_per_block,
            // One thread renders it: it must not take cores from the host's own engine.
            worker_threads: 0,
            ..Default::default()
        };
        #[allow(unused_mut)] // the stretcher needs it with `warp-complex`
        let mut parts = ether_core::create(config);
        #[cfg(feature = "warp-complex")]
        parts
            .handle
            .set_stretcher_factory(Arc::new(ether_stretch::SignalsmithFactory::default()));
        let shared = Arc::new(AudioShared::default());
        if let Some(root) = self.audio.recording.projects_root() {
            shared.recording.set_projects_root(root);
        }
        let settings = crate::audio::AudioSettings {
            backend: crate::audio::AudioBackendKind::Null,
            sample_rate: Some(self.sample_rate),
            max_block_size: self.prepare.max_block_size,
            ..Default::default()
        };
        let output =
            crate::audio::AudioOutput::start(Box::new(parts.engine), &settings, shared.clone())
                .map_err(|(e, _)| {
                    BridgeError::Other(format!("could not start another engine: {e}"))
                })?;
        let gc_stop = Arc::new(AtomicBool::new(false));
        let gc_thread = {
            let stop = gc_stop.clone();
            let mut gc = parts.gc;
            std::thread::Builder::new()
                .name("ether-gc-dedicated".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        gc.collect();
                        std::thread::sleep(crate::host::GC_INTERVAL);
                    }
                    gc.collect();
                })
                .map_err(|e| BridgeError::Other(e.to_string()))?
        };
        let mut bridge = NativeBridge::new(
            parts.handle,
            self.prepare,
            self.plugins.clone(),
            self.catalog.clone(),
            self.instantiate.clone(),
            shared,
        );
        bridge.registry_salt = salt;
        bridge.dedicated_notes = self.dedicated_notes.clone();
        if let Ok(mut notes) = self.dedicated_notes.lock() {
            notes.insert(salt, Vec::new());
        }
        if let (Some(ext), Some(child)) = (self.ext.as_deref_mut(), bridge.ext.as_deref_mut()) {
            ext.spawned(child, &self.plugins, salt);
        }
        self.spawned_engines = true;
        Ok(crate::dedicated::DedicatedEngine::new(
            bridge,
            crate::dedicated::Runner::new(output, gc_stop, gc_thread),
        ))
    }

    /// The plugin registry id of `device`: itself for the main engine; for a dedicated one an
    /// id of its own, so its instance never retires the host's instance of the same device.
    fn reg(&self, device: DeviceId) -> DeviceId {
        if self.registry_salt == 0 {
            return device;
        }
        dedicated_device_id(self.registry_salt, device)
    }

    /// The registry salt of this bridge: 0 for the main engine, else a dedicated engine's.
    pub fn registry_salt(&self) -> u16 {
        self.registry_salt
    }

    /// A dedicated engine's notifications, filed by the main bridge, with document ids.
    fn take_dedicated_notes(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        let notes = match self.dedicated_notes.lock() {
            Ok(mut m) => m
                .get_mut(&self.registry_salt)
                .map(std::mem::take)
                .unwrap_or_default(),
            Err(_) => return,
        };
        if notes.is_empty() {
            return;
        }
        let devices: HashMap<DeviceId, DeviceId> = self
            .devices
            .iter()
            .filter(|(_, e)| matches!(e.kind, DeviceKind::Plugin(_)))
            .map(|(d, _)| (self.reg(*d), *d))
            .collect();
        out.extend(
            notes
                .into_iter()
                .filter_map(|(reg, n)| Some((*devices.get(&reg)?, n))),
        );
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn plugins(&self) -> &PluginHost {
        &self.plugins
    }

    /// Engine node of a device (tests/diagnostics).
    pub fn node_of(&self, device: DeviceId) -> Option<NodeKey> {
        self.devices.get(&device).map(|d| d.key)
    }

    fn replace_device(&mut self, device: DeviceId) {
        if self.devices.contains_key(&device) {
            let _ = self.destroy_device(device);
        }
    }

    fn destroy_device(&mut self, device: DeviceId) -> Result<(), BridgeError> {
        self.remove_device(device, true)
    }

    /// Remove `device`'s engine node; its plugin instance too unless `retire_plugin` is
    /// false (a plugin about to be re-instantiated: [`PluginHost::instantiate`] retires the
    /// old instance itself and hands its open editor over to the new one).
    fn remove_device(&mut self, device: DeviceId, retire_plugin: bool) -> Result<(), BridgeError> {
        self.mirrors.unpark(device);
        let Some(entry) = self.devices.remove(&device) else {
            return Ok(());
        };
        let r = self.handle.remove_node(entry.key).map_err(engine_err);
        if retire_plugin && matches!(entry.kind, DeviceKind::Plugin(_)) {
            self.plugins.destroy(self.reg(device));
        }
        r
    }
}

/// Registry ids of offline (export) plugin instances: a fixed high prefix plus a counter,
/// so they never retire the live instance of the same device in the [`PluginHost`].
const OFFLINE_DEVICE_PREFIX: u128 = 0xE0F1_0000_0000_0000_0000_0000_0000_0000;

/// A registry id minted by [`NativeBridge::create_offline_plugin`].
fn is_offline_id(device: DeviceId) -> bool {
    device.0.0 >> 112 == OFFLINE_DEVICE_PREFIX >> 112
}

/// Registry ids of a dedicated engine's plugins: its devices are the
/// document's, so they share the main thread's registry with the host's own instances
/// only through ids of this form (a 16-bit prefix, the engine's 16-bit salt, the low 96 bits
/// of the device id).
const DEDICATED_DEVICE_PREFIX: u128 = 0xD3D1;

/// A registry id minted for a dedicated engine's plugin ([`NativeBridge::reg`]).
fn is_dedicated_id(device: DeviceId) -> bool {
    device.0.0 >> 112 == DEDICATED_DEVICE_PREFIX
}

/// The registry id of `device`'s instance on the dedicated engine of `salt`.
pub fn dedicated_device_id(salt: u16, device: DeviceId) -> DeviceId {
    let low = device.0.0 & ((1u128 << 96) - 1);
    DeviceId(ether_core::protocol::model::Ulid(
        DEDICATED_DEVICE_PREFIX << 112 | u128::from(salt) << 96 | low,
    ))
}

/// A registry id of the main engine's own plugin instances (never an export's or a
/// dedicated engine's).
pub fn is_main_instance_id(device: DeviceId) -> bool {
    !is_dedicated_id(device) && !is_offline_id(device)
}

/// An offline plugin instance (export): the hosted node plus the registry entry it owns,
/// released when the offline engine drops the node.
struct OfflinePluginNode {
    node: Option<Box<dyn ether_core::Node>>,
    host: PluginHost,
    registry_id: DeviceId,
}

impl OfflinePluginNode {
    fn inner(&self) -> &dyn ether_core::Node {
        self.node.as_deref().expect("present until drop")
    }
    fn inner_mut(&mut self) -> &mut dyn ether_core::Node {
        self.node.as_deref_mut().expect("present until drop")
    }
}

impl ether_core::Node for OfflinePluginNode {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.inner_mut().prepare(config);
    }
    fn reset(&mut self) {
        self.inner_mut().reset();
    }
    fn process(
        &mut self,
        ctx: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
    ) -> ether_core::ProcessStatus {
        self.inner_mut().process(ctx, audio)
    }
    fn latency(&self) -> u32 {
        self.inner().latency()
    }
    fn channels(&self) -> (u16, u16) {
        self.inner().channels()
    }
    fn sidechain_inputs(&self) -> u16 {
        self.inner().sidechain_inputs()
    }
    fn process_sidechain(
        &mut self,
        ctx: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ether_core::ProcessStatus {
        self.inner_mut().process_sidechain(ctx, audio, sidechain)
    }
}

impl Drop for OfflinePluginNode {
    fn drop(&mut self) {
        // The hosted node goes back to the main thread for `deactivate` first, then the
        // controller is released (both run in order on the main thread).
        drop(self.node.take());
        self.host.destroy(self.registry_id);
    }
}

impl EngineBridge for NativeBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.replace_device(device);
        let mut node = ether_devices::create(kind, &Sources(&self.sources));
        for (id, value) in params {
            node.set_param(*id, *value);
        }
        let key = self.handle.add_node(node).map_err(engine_err)?;
        self.devices.insert(
            device,
            DeviceEntry {
                key,
                kind: DeviceKind::Builtin(kind.device_type()),
            },
        );
        Ok(key)
    }

    /// v0.2 (`midi-fx`): the resolved scale reaches the live node (`Node::set_data`).
    fn set_node_scale(
        &mut self,
        device: DeviceId,
        scale: ether_core::protocol::model::MusicalScale,
    ) -> Result<bool, BridgeError> {
        let Some(entry) = self.devices.get(&device) else {
            return Ok(false);
        };
        self.handle
            .set_node_data(entry.key, Box::new(scale))
            .map_err(engine_err)?;
        Ok(true)
    }

    /// Sampler slice edits reach the live node in place (`Node::set_data` with the new
    /// `SliceSettings`), so sounding notes aren't cut. Anything else: re-create.
    fn update_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
    ) -> Result<bool, BridgeError> {
        // External devices (`external-instrument`): the routing is compiled into the graph's
        // `hw_io`, the node holds none, so a routing edit keeps the node as is.
        if let BuiltinDevice::ExternalInstrument { .. }
        | BuiltinDevice::ExternalAudioEffect { .. } = kind
        {
            return Ok(self.devices.get(&device).is_some_and(
                |e| matches!(e.kind, DeviceKind::Builtin(t) if t == kind.device_type()),
            ));
        }
        // Multisampler zone edits: the resolved zone set, swapped in by `Node::set_data`.
        if let BuiltinDevice::MultiSampler { .. } = kind {
            let Some(entry) = self.devices.get(&device) else {
                return Ok(false);
            };
            if !matches!(
                entry.kind,
                DeviceKind::Builtin(BuiltinDeviceType::MultiSampler)
            ) {
                return Ok(false);
            }
            let set = ether_devices::multisampler::zone_set(kind, &Sources(&self.sources));
            self.handle
                .set_node_data(entry.key, Box::new(set))
                .map_err(engine_err)?;
            return Ok(true);
        }
        // A Volume LFO's drawn curve (`fx-modulation`): a fixed-size copy, swapped in.
        if let BuiltinDevice::VolumeLfo { .. } = kind {
            let Some(entry) = self.devices.get(&device) else {
                return Ok(false);
            };
            if !matches!(
                entry.kind,
                DeviceKind::Builtin(BuiltinDeviceType::VolumeLfo)
            ) {
                return Ok(false);
            }
            let Some(data) = ether_devices::fx_modulation::volume_lfo_curve(kind) else {
                return Ok(false);
            };
            self.handle
                .set_node_data(entry.key, data)
                .map_err(engine_err)?;
            return Ok(true);
        }
        // v0.3 (`fx-space`): a new IR, read and partitioned here (off the audio thread);
        // the node crossfades to it.
        if let BuiltinDevice::ConvolutionReverb { .. } = kind {
            let Some(entry) = self.devices.get(&device) else {
                return Ok(false);
            };
            if !matches!(
                entry.kind,
                DeviceKind::Builtin(BuiltinDeviceType::ConvolutionReverb)
            ) {
                return Ok(false);
            }
            let Some(data) = ether_devices::fx_space::ir_swap(
                kind,
                &Sources(&self.sources),
                self.prepare.sample_rate,
            ) else {
                return Ok(false);
            };
            self.handle
                .set_node_data(entry.key, data)
                .map_err(engine_err)?;
            return Ok(true);
        }
        let BuiltinDevice::Sampler { slices, .. } = kind else {
            return Ok(false);
        };
        let Some(entry) = self.devices.get(&device) else {
            return Ok(false);
        };
        if !matches!(entry.kind, DeviceKind::Builtin(BuiltinDeviceType::Sampler)) {
            return Ok(false);
        }
        self.handle
            .set_node_data(entry.key, Box::new(slices.clone()))
            .map_err(engine_err)?;
        Ok(true)
    }

    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        // The live instance being replaced (reload, sandbox toggle, restart request, preset)
        // is retired by `PluginHost::instantiate`, which reopens its editor on the new one;
        // on every path that doesn't reach it, it is destroyed here.
        let _ = self.remove_device(device, false);
        // Mirrored (listening): the mirror is the instance; the slot gets a stand-in.
        if let Some(descriptor) = self.mirrors.descriptor(device).cloned() {
            self.plugins.destroy(self.reg(device));
            let node = crate::plugin_mirror::MirrorStandIn::new(&descriptor);
            let key = self.handle.add_node(Box::new(node)).map_err(engine_err)?;
            self.devices.insert(
                device,
                DeviceEntry {
                    key,
                    kind: DeviceKind::Plugin(Box::new(descriptor)),
                },
            );
            return Ok(key);
        }
        // Ids are unique per format only: look the plugin up by (format, id).
        let Some(desc) = self.catalog.find_plugin(plugin) else {
            self.plugins.destroy(self.reg(device));
            return Err(BridgeError::Other(not_installed(plugin)));
        };
        let (node, descriptor) = self
            .plugins
            .instantiate(
                crate::sandbox::instantiator(plugin.sandboxed, &self.instantiate),
                self.reg(device),
                PluginSource {
                    format: desc.format,
                    // AUs have no bundle: their path is the component id.
                    path: PathBuf::from(desc.path),
                    plugin_id: plugin.plugin_id.clone(),
                },
                state.map(|s| s.0.clone()),
                // Activated with the engine's max block size: backends never exceed it.
                self.prepare,
            )
            .map_err(plugin_err)?;
        let key = match self.handle.add_node(node) {
            Ok(key) => key,
            Err(e) => {
                // The node was dropped (and returned to the main thread): drop the controller.
                self.plugins.destroy(self.reg(device));
                return Err(engine_err(e));
            }
        };
        self.devices.insert(
            device,
            DeviceEntry {
                key,
                kind: DeviceKind::Plugin(Box::new(descriptor)),
            },
        );
        Ok(key)
    }

    fn create_offline_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
        sample_rate: u32,
    ) -> Result<Box<dyn ether_core::Node>, BridgeError> {
        let _ = device;
        let desc = self
            .catalog
            .find_plugin(plugin)
            .ok_or_else(|| BridgeError::Other(not_installed(plugin)))?;
        self.offline_seq += 1;
        let registry_id = DeviceId(ether_core::protocol::model::Ulid(
            OFFLINE_DEVICE_PREFIX | u128::from(self.offline_seq),
        ));
        let (node, _) = self
            .plugins
            .instantiate(
                crate::sandbox::instantiator(plugin.sandboxed, &self.instantiate),
                registry_id,
                PluginSource {
                    format: desc.format,
                    path: PathBuf::from(desc.path),
                    plugin_id: plugin.plugin_id.clone(),
                },
                state.map(|s| s.0.clone()),
                // Activated for the offline engine's blocks (never larger).
                PrepareConfig {
                    sample_rate: sample_rate as f32,
                    max_block_size: ether_core::offline::OFFLINE_MAX_BLOCK,
                    max_events_per_block: self.prepare.max_events_per_block,
                },
            )
            .map_err(plugin_err)?;
        Ok(Box::new(OfflinePluginNode {
            node: Some(node),
            host: self.plugins.clone(),
            registry_id,
        }))
    }

    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        match self
            .devices
            .iter()
            .find(|(_, e)| e.key == key)
            .map(|(d, _)| *d)
        {
            Some(device) => self.destroy_device(device),
            None => self.handle.remove_node(key).map_err(engine_err),
        }
    }

    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        let audio = if audio.sample_rate != self.sample_rate && audio.frames() > 0 {
            // Contract: the controller resamples; be lenient and do it here (off-RT).
            Arc::new(
                ether_media::resample(&audio, self.sample_rate)
                    .map_err(|e| BridgeError::Other(e.to_string()))?,
            )
        } else {
            audio
        };
        let source: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio));
        self.handle
            .add_source(media.id, source.clone())
            .map_err(engine_err)?;
        self.sources.insert(media.id, source);
        Ok(())
    }

    fn preview(
        &mut self,
        id: u64,
        audio: Option<Arc<DecodedAudio>>,
        gain: f32,
    ) -> Result<(), BridgeError> {
        use ether_core::preview::PreviewControl;
        let control = match audio {
            Some(audio) => {
                let audio = if audio.sample_rate != self.sample_rate && audio.frames() > 0 {
                    // Contract: the controller resamples; be lenient (off-RT), as load_media.
                    Arc::new(
                        ether_media::resample(&audio, self.sample_rate)
                            .map_err(|e| BridgeError::Other(e.to_string()))?,
                    )
                } else {
                    audio
                };
                PreviewControl::Play {
                    id,
                    source: Arc::new(InMemorySource::new(audio)),
                    gain,
                }
            }
            None => PreviewControl::Stop,
        };
        self.handle.preview(control).map_err(engine_err)
    }

    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        if let Some(disk) = self.disk.as_mut() {
            disk.remove(media);
        }
        if self.sources.remove(&media).is_some() {
            self.handle.remove_source(media).map_err(engine_err)?;
        }
        Ok(())
    }

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        if let Some(ext) = self.ext.as_deref_mut() {
            ext.observe_graph(&graph);
        }
        self.route_hw_midi(&graph);
        if self.disk.as_ref().is_some_and(|d| !d.is_empty()) {
            self.stream_graph = Some(graph.clone());
            self.update_stream_anchors();
        } else {
            self.stream_graph = None;
        }
        self.handle.publish(graph).map_err(engine_err)
    }

    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.handle.set_param(change).map_err(engine_err)
    }

    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        if let Some(ext) = self.ext.as_deref_mut() {
            ext.observe_transport(&control);
        }
        // Decode the jump target first; keep the loop anchor current.
        match &control {
            TransportControl::Locate { position } => self.prime_streams_at(position.0),
            TransportControl::Play => {
                let at = self.handle.playhead().position.0;
                self.prime_streams_at(at);
            }
            TransportControl::SetLoop { enabled, region } => {
                if let Some(g) = self.stream_graph.as_mut() {
                    g.loop_enabled = *enabled;
                    g.loop_start = region.start.0;
                    g.loop_end = region.end.0;
                    self.update_stream_anchors();
                }
            }
            _ => {}
        }
        self.handle.transport(control).map_err(engine_err)
    }

    /// Audio streaming (CONTRACTS.md §13.1): stream long media from disk
    /// ([`crate::disk_stream`]). `ETHER_DISK_STREAMING=0` turns it off (whole-file decode,
    /// as v0.2).
    fn stream_media(
        &mut self,
        source: &ether_controller::media_stream::StreamSource,
    ) -> Result<bool, BridgeError> {
        if !self.disk_streaming {
            return Ok(false);
        }
        let id = source.media.id;
        // Replacing a stream (reload at another rate, relink): drop the old one first.
        let _ = self.unload_media(id);
        let root = self.audio.recording.projects_root();
        let disk = self
            .disk
            .get_or_insert_with(crate::disk_stream::DiskStreams::new);
        let info = disk
            .open(source, root.as_deref(), self.sample_rate)
            .map_err(|e| BridgeError::Other(e.to_string()))?;
        let src: Arc<dyn AudioSource> = info.cache;
        if let Err(e) = self.handle.add_source(id, src.clone()) {
            disk.remove(id);
            return Err(engine_err(e));
        }
        self.sources.insert(id, src);
        self.update_stream_anchors();
        Ok(true)
    }

    fn poll(&mut self, out: &mut EngineOutputs) {
        self.handle.poll(out);
        out.cpu_load = self.audio.cpu_load();
    }

    /// v0.2 analysis channel: device frames straight from the engine handle.
    fn poll_analysis(&mut self, out: &mut Vec<ether_core::AnalysisFrame>) {
        self.handle.poll_analysis(|f| out.push(*f));
    }

    fn watch_analysis(&mut self, node: NodeKey, on: bool) -> Result<(), BridgeError> {
        self.handle.watch_analysis(node, on).map_err(engine_err)
    }

    /// Node latency as last observed by the audio thread.
    fn node_latency(&self, key: NodeKey) -> Option<u32> {
        self.handle.node_latency(key)
    }

    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        match &self.devices.get(&device)?.kind {
            DeviceKind::Builtin(t) => Some(ether_devices::descriptor(*t)),
            DeviceKind::Plugin(d) => Some((**d).clone()),
        }
    }

    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        // The registry has one notification queue: the main engine's bridge drains it and
        // files a dedicated engine's notifications for that engine's own poll (below).
        if self.registry_salt != 0 {
            return self.take_dedicated_notes(out);
        }
        if !self.mirrors.is_empty()
            || self.spawned_engines
            || self.plugins.has_queued_notes()
            || self
                .devices
                .values()
                .any(|d| matches!(d.kind, DeviceKind::Plugin(_)))
        {
            let from = out.len();
            self.plugins.poll_nonblocking(out);
            // Offline (export) instances are private to the export, and a dedicated engine's
            // belong to its requester, not to the document: their notifications (state dirty,
            // GUI edits, latency) must not reach it here. A dedicated engine's go to its own
            // poll (its requester commits the edits).
            let mut dedicated = Vec::new();
            let mut i = from;
            while i < out.len() {
                if is_offline_id(out[i].0) {
                    out.remove(i);
                } else if is_dedicated_id(out[i].0) {
                    dedicated.push(out.remove(i));
                } else {
                    i += 1;
                }
            }
            if !dedicated.is_empty()
                && let Ok(mut notes) = self.dedicated_notes.lock()
            {
                for (reg, n) in dedicated {
                    let salt = (reg.0.0 >> 96) as u16;
                    if let Some(q) = notes.get_mut(&salt) {
                        if q.len() >= MAX_DEDICATED_NOTES {
                            q.remove(0);
                        }
                        q.push((reg, n));
                    }
                }
            }
            self.mirrors.observe(&out[from..]);
        }
    }

    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        // A mirrored device's state is its mirror's, or the last state of
        // a dropped mirror until the live instance replacing it exists.
        if self.mirrors.contains(device) {
            return Ok(self
                .plugins
                .mirror_state(device)
                .map_err(plugin_err)?
                .map(Base64Bytes));
        }
        if let Some(state) = self.mirrors.parked(device) {
            return Ok(Some(Base64Bytes(state.clone())));
        }
        match self.devices.get(&device).map(|d| &d.kind) {
            Some(DeviceKind::Plugin(_)) => Ok(self
                .plugins
                .save_state(self.reg(device))
                .map_err(plugin_err)?
                .map(Base64Bytes)),
            _ => Ok(None),
        }
    }

    fn set_plugin_state(
        &mut self,
        device: DeviceId,
        state: &Base64Bytes,
    ) -> Result<bool, BridgeError> {
        // A listening site's GUI mirror is re-created from the document instead.
        if self.mirrors.contains(device) {
            return Ok(false);
        }
        match self.devices.get(&device).map(|d| &d.kind) {
            Some(DeviceKind::Plugin(_)) => self
                .plugins
                .load_state_live(self.reg(device), state.0.clone())
                .map_err(plugin_err),
            _ => Ok(false),
        }
    }

    fn list_inputs(&mut self) -> Result<InputList, BridgeError> {
        crate::recording::list_inputs(&self.audio)
    }

    /// `external-instrument`: MIDI outputs (midir), the audio input channels the engine
    /// gets (as for recording) and the running stream's output channels.
    fn list_hardware_ports(
        &mut self,
    ) -> Result<ether_core::protocol::external::HardwarePorts, BridgeError> {
        use ether_core::protocol::recording::AudioInputChannel;
        let inputs = crate::recording::list_inputs(&self.audio)?.audio;
        let outputs = self
            .audio
            .output_channels
            .load(std::sync::atomic::Ordering::Relaxed)
            .min(crate::rt::MAX_OUTPUT_CHANNELS as u32) as u16;
        Ok(ether_core::protocol::external::HardwarePorts {
            midi_outputs: hw_midi::list_outputs(),
            audio_inputs: inputs,
            audio_outputs: (0..outputs)
                .map(|i| AudioInputChannel {
                    index: i,
                    name: format!("Out {}", i + 1),
                })
                .collect(),
        })
    }

    fn measure_hw_latency(&mut self, node: NodeKey) -> Result<(), BridgeError> {
        self.handle.measure_hw_latency(node).map_err(engine_err)
    }

    fn poll_hw_latency(&mut self) -> Option<ether_core::hw_io::HwLatencyResult> {
        self.handle.poll_hw_latency()
    }

    fn start_recording(&mut self, session: &RecordSession) -> Result<(), BridgeError> {
        crate::recording::start(&self.audio, session)
    }

    fn poll_recording(
        &mut self,
        audio: &mut Vec<ether_core::protocol::recording::LiveAudioChunk>,
        midi: &mut Vec<ether_core::protocol::recording::LiveMidiNote>,
    ) {
        crate::recording::poll_live(&self.audio, audio, midi);
    }

    fn stop_recording(&mut self) -> Result<RecordedTakes, BridgeError> {
        crate::recording::stop(&self.audio, &self.handle)
    }

    fn plugin_param_values(&mut self, device: DeviceId) -> Vec<(ParamId, f64)> {
        self.plugins.param_values(self.reg(device))
    }

    fn probe_plugin(&mut self, plugin: &PluginInstance) -> Option<DeviceDescriptor> {
        let desc = self.catalog.find_plugin(plugin)?;
        self.offline_seq += 1;
        // An offline id: its notifications never reach the document (`poll_plugins`).
        let registry_id = DeviceId(ether_core::protocol::model::Ulid(
            OFFLINE_DEVICE_PREFIX | u128::from(self.offline_seq),
        ));
        let (node, descriptor) = self
            .plugins
            .instantiate(
                crate::sandbox::instantiator(plugin.sandboxed, &self.instantiate),
                registry_id,
                PluginSource {
                    format: desc.format,
                    path: PathBuf::from(desc.path),
                    plugin_id: plugin.plugin_id.clone(),
                },
                None,
                self.prepare,
            )
            .ok()?;
        // Released right away (deactivated, then the controller dropped, on the main thread).
        drop(OfflinePluginNode {
            node: Some(node),
            host: self.plugins.clone(),
            registry_id,
        });
        Some(descriptor)
    }

    fn plugin_catalog(&self) -> Option<ether_controller::PluginCatalogView> {
        Some(ether_controller::PluginCatalogView {
            plugins: self.catalog.list(),
            scanning: self
                .scanning
                .as_ref()
                .is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed)),
        })
    }

    fn poll_midi_input(&mut self, out: &mut Vec<ether_core::protocol::midi_map::MidiInputEvent>) {
        crate::recording::drain_midi_input(&self.audio, out);
    }

    fn live_midi(&mut self, data: [u8; 3]) -> Result<(), BridgeError> {
        if crate::recording::inject_midi(&self.audio, data) {
            Ok(())
        } else {
            Err(BridgeError::QueueFull)
        }
    }

    fn preview_note(&mut self, track: TrackId, data: [u8; 3]) -> Result<(), BridgeError> {
        self.handle.preview_note(track, data).map_err(engine_err)
    }

    // Plugin GUI mirrors: delegated to `crate::plugin_mirror`.

    fn create_plugin_mirror(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<(), BridgeError> {
        let desc = self
            .catalog
            .find_plugin(plugin)
            .ok_or_else(|| BridgeError::Other(not_installed(plugin)))?;
        let descriptor = self
            .plugins
            .create_mirror(
                crate::sandbox::instantiator(plugin.sandboxed, &self.instantiate),
                device,
                PluginSource {
                    format: desc.format,
                    path: PathBuf::from(desc.path),
                    plugin_id: plugin.plugin_id.clone(),
                },
                state.map(|s| s.0.clone()),
            )
            .map_err(plugin_err)?;
        self.mirrors.insert(device, descriptor);
        Ok(())
    }

    fn destroy_plugin_mirror(&mut self, device: DeviceId) -> Result<(), BridgeError> {
        if !self.mirrors.contains(device) {
            return Ok(());
        }
        self.mirrors.remove(device);
        let state = self.plugins.destroy_mirror(device).map_err(plugin_err)?;
        // Its stand-in still holds the slot: the live instance replacing it starts here.
        if let Some(state) = state
            && self.devices.contains_key(&device)
        {
            self.mirrors.park(device, state);
        }
        Ok(())
    }

    fn set_plugin_mirror_param(
        &mut self,
        device: DeviceId,
        param: ParamId,
        value: f64,
    ) -> Result<(), BridgeError> {
        if !self.mirrors.contains(device) {
            return Err(BridgeError::Other(format!("device {device} has no mirror")));
        }
        if !self.mirrors.push(device, param, value) {
            return Ok(());
        }
        self.plugins
            .set_mirror_param(device, param, value)
            .map_err(|e| {
                self.mirrors.forget(device, param);
                plugin_err(e)
            })
    }
}

impl Drop for NativeBridge {
    fn drop(&mut self) {
        // A dedicated engine releases its plugins (registered under its own ids); the main
        // engine's are the host's to release at shutdown.
        if self.registry_salt == 0 {
            return;
        }
        // The extension first (what it opened stops with it), then the instances, whose open
        // editors close with them (`PluginHost::destroy`).
        self.ext = None;
        if let Ok(mut notes) = self.dedicated_notes.lock() {
            notes.remove(&self.registry_salt);
        }
        let plugins: Vec<DeviceId> = self
            .devices
            .iter()
            .filter(|(_, e)| matches!(e.kind, DeviceKind::Plugin(_)))
            .map(|(d, _)| *d)
            .collect();
        for d in plugins {
            self.plugins.destroy(self.reg(d));
        }
    }
}

/// Native [`HostServices`]: wall clock, OS entropy and (feature `net`) the sample pack's
/// downloads (`samples_http`).
#[derive(Debug, Default, Clone)]
pub struct NativeServices {
    #[cfg(feature = "net")]
    http: crate::samples_http::Http,
}

impl HostServices for NativeServices {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }

    fn random_seed(&mut self) -> u64 {
        getrandom::u64().unwrap_or_else(|_| self.now_ms() ^ u64::from(std::process::id()) << 32)
    }

    fn samples_url(&self) -> Option<String> {
        std::env::var("ETHER_SAMPLES_URL").ok()
    }

    #[cfg(feature = "net")]
    fn http_get(&mut self, id: u64, url: &str, max_bytes: u64) -> Result<(), String> {
        self.http.get(id, url, max_bytes)
    }

    #[cfg(feature = "net")]
    fn poll_http(&mut self, out: &mut Vec<ether_controller::HttpEvent>) {
        self.http.poll(out);
    }
}

/// Hardware MIDI out of External Instruments (`external-instrument`, CONTRACTS.md §13.7):
/// a thread drains the engine's [`ether_core::hw_io::HwMidiEvent`] ring and sends each
/// message to its device's `midi_out` port when its engine sample reaches the speakers.
///
/// - **Timing.** [`crate::rt::EngineClock`] maps engine samples to wall time (the renderer
///   publishes it before every chunk); a message for engine sample `f` goes out at the wall
///   time of `f` plus the output latency (`RecordingShared::output_latency_or(block)`), so
///   it is jitter-free (the audio thread renders ahead by about that much). Whatever offset
///   remains between the note and its returned audio is the hardware round trip that
///   `External::MeasureLatency` measures and PDC compensates.
/// - **Routing.** `node → port id` comes from every published graph (`TrackDesc::hw_io`).
///   Unresolved ports drop their messages (the device is silent) and are retried at most
///   every [`RETRY_MS`], so a device that comes back resumes. When a node's port changes or
///   the node goes away, its sounding notes get note-offs on the old port.
/// - midir has no timestamped output, so the thread sleeps until each message is due
///   (≈1 ms resolution).
pub mod hw_midi {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use ether_core::NodeKey;
    use ether_core::hw_io::{HwIoIo, HwMidiEvent};
    use ether_core::protocol::recording::MidiPort;

    use crate::rt::AudioShared;

    /// Client name of the MIDI output connections.
    const CLIENT: &str = "Ethereal";
    /// Minimum time between two attempts to open a missing port.
    pub const RETRY_MS: u64 = 1000;
    /// Messages kept waiting for their time (beyond: the oldest are sent late rather than
    /// growing memory without bound).
    const MAX_PENDING: usize = 16_384;

    /// Where the thread sends messages: midir, or a test double.
    pub trait MidiOutSink: Send + 'static {
        /// Send `data` to `port`; `false` = the port is not available (now).
        fn send(&mut self, port: &str, data: &[u8]) -> bool;
    }

    /// Hardware MIDI outputs (`MidiPort::id` = the port name, like inputs).
    pub fn list_outputs() -> Vec<MidiPort> {
        let Ok(probe) = midir::MidiOutput::new(CLIENT) else {
            return Vec::new();
        };
        probe
            .ports()
            .iter()
            .filter_map(|p| probe.port_name(p).ok())
            .map(|name| MidiPort {
                id: name.clone(),
                name,
            })
            .collect()
    }

    /// midir output connections, opened on first use and re-opened after a failure.
    #[derive(Default)]
    pub struct MidirOut {
        conns: HashMap<String, midir::MidiOutputConnection>,
        failed: HashMap<String, Instant>,
    }

    impl MidirOut {
        fn connect(&mut self, port: &str) -> Option<&mut midir::MidiOutputConnection> {
            if !self.conns.contains_key(port) {
                if self
                    .failed
                    .get(port)
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(RETRY_MS))
                {
                    return None;
                }
                let conn = midir::MidiOutput::new(CLIENT).ok().and_then(|out| {
                    let p = out
                        .ports()
                        .into_iter()
                        .find(|p| out.port_name(p).is_ok_and(|n| n == port))?;
                    out.connect(&p, "ethereal-out").ok()
                });
                match conn {
                    Some(c) => {
                        self.failed.remove(port);
                        self.conns.insert(port.to_string(), c);
                    }
                    None => {
                        self.failed.insert(port.to_string(), Instant::now());
                        return None;
                    }
                }
            }
            self.conns.get_mut(port)
        }
    }

    impl MidiOutSink for MidirOut {
        fn send(&mut self, port: &str, data: &[u8]) -> bool {
            let ok = self.connect(port).is_some_and(|c| c.send(data).is_ok());
            if !ok && self.conns.remove(port).is_some() {
                // The port went away: retry later.
                self.failed.insert(port.to_string(), Instant::now());
            }
            ok
        }
    }

    /// `node → port id` of the published graph, shared with the thread.
    #[derive(Debug, Default)]
    pub struct Routes {
        inner: Mutex<(u64, HashMap<NodeKey, String>)>,
    }

    impl Routes {
        /// Replace the routes (a publish); no-op when unchanged.
        pub fn set(&self, routes: HashMap<NodeKey, String>) {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if g.1 != routes {
                g.0 += 1;
                g.1 = routes;
            }
        }

        fn get_if_newer(&self, version: u64) -> Option<(u64, HashMap<NodeKey, String>)> {
            let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            (g.0 != version).then(|| (g.0, g.1.clone()))
        }

        pub fn is_empty(&self) -> bool {
            self.inner
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .1
                .is_empty()
        }
    }

    /// The thread's state: due-time ordering, routing, sounding notes.
    #[derive(Default)]
    pub struct Scheduler {
        pending: Vec<HwMidiEvent>,
        routes: HashMap<NodeKey, String>,
        version: u64,
        /// Sounding notes: node, port, status channel nibble, key.
        sounding: Vec<(NodeKey, String, u8, u8)>,
    }

    impl Scheduler {
        pub fn push(&mut self, m: HwMidiEvent) {
            if self.pending.len() >= MAX_PENDING {
                self.pending.remove(0);
            }
            // Keep sorted by frame (stable: equal frames keep their order).
            let at = self.pending.partition_point(|p| p.frame <= m.frame);
            self.pending.insert(at, m);
        }

        /// New routes: release notes whose node is gone or moved to another port.
        pub fn set_routes(
            &mut self,
            version: u64,
            routes: HashMap<NodeKey, String>,
            sink: &mut dyn MidiOutSink,
        ) {
            self.version = version;
            self.routes = routes;
            let routes = &self.routes;
            self.sounding.retain(|(node, port, ch, key)| {
                let keep = routes.get(node) == Some(port);
                if !keep {
                    sink.send(port, &[0x80 | ch, *key, 0]);
                }
                keep
            });
        }

        /// Send every message due at engine sample `now` (its frame `+ offset <= now`);
        /// returns the frame of the next pending one.
        pub fn step(&mut self, now: f64, offset: f64, sink: &mut dyn MidiOutSink) -> Option<u64> {
            let due = self
                .pending
                .partition_point(|m| m.frame as f64 + offset <= now);
            for m in self.pending.drain(..due) {
                let Some(port) = self.routes.get(&m.node) else {
                    continue;
                };
                if !sink.send(port, &m.data) {
                    continue;
                }
                let (status, ch, key) = (m.data[0] & 0xf0, m.data[0] & 0x0f, m.data[1]);
                let same = |s: &(NodeKey, String, u8, u8)| s.0 == m.node && s.2 == ch;
                match status {
                    0x90 if m.data[2] > 0 => {
                        if !self.sounding.iter().any(|s| same(s) && s.3 == key) {
                            self.sounding.push((m.node, port.clone(), ch, key));
                        }
                    }
                    0x80 | 0x90 => self.sounding.retain(|s| !(same(s) && s.3 == key)),
                    0xb0 if matches!(key, 120 | 123) => self.sounding.retain(|s| !same(s)),
                    _ => {}
                }
            }
            self.pending.first().map(|m| m.frame)
        }

        pub fn version(&self) -> u64 {
            self.version
        }
    }

    /// The running thread; dropping it stops and joins it.
    pub struct HwMidiOut {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Drop for HwMidiOut {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Start the MIDI output thread.
    pub fn spawn(
        mut io: HwIoIo,
        routes: Arc<Routes>,
        audio: Arc<AudioShared>,
        sample_rate: f64,
        block: u32,
        mut sink: Box<dyn MidiOutSink>,
    ) -> std::io::Result<HwMidiOut> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("ether-hw-midi".into())
            .spawn(move || {
                let mut s = Scheduler::default();
                while !flag.load(Ordering::Relaxed) {
                    if let Some((v, r)) = routes.get_if_newer(s.version()) {
                        s.set_routes(v, r, sink.as_mut());
                    }
                    while let Ok(m) = io.midi.pop() {
                        s.push(m);
                    }
                    let offset = f64::from(audio.recording.output_latency_or(block));
                    let wait = match audio.clock.sample_now(sample_rate) {
                        Some(now) => match s.step(now, offset, sink.as_mut()) {
                            Some(next) => {
                                ((next as f64 + offset - now) / sample_rate).clamp(0.0, 0.002)
                            }
                            None => 0.002,
                        },
                        None => 0.01,
                    };
                    std::thread::sleep(Duration::from_secs_f64(wait.max(0.0002)));
                }
            })?;
        Ok(HwMidiOut {
            stop,
            thread: Some(thread),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{DedicatedThread, fake};
    use ether_core::protocol::model::{PluginFormat, Ulid};
    use ether_core::protocol::plugins::PluginDescriptor;

    fn bridge() -> (NativeBridge, ether_core::Engine) {
        bridge_with(fake::instantiate())
    }

    fn bridge_with(instantiate: Instantiate) -> (NativeBridge, ether_core::Engine) {
        crate::test_util::fake_bridge(instantiate)
    }

    fn media_ref(id: u128) -> MediaRef {
        MediaRef {
            location: Default::default(),
            id: MediaId(Ulid(id)),
            name: "a.wav".into(),
            file: "media/a.wav".into(),
            sample_rate: 44_100,
            channels: 1,
            frames: 441,
            hash: None,
        }
    }

    #[test]
    fn builtin_devices() {
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(3));
        let key = b.create_builtin(d, &BuiltinDevice::Synth, &[]).unwrap();
        assert_eq!(
            b.descriptor(d).unwrap().device_type,
            ether_core::protocol::devices::DeviceTypeRef::Builtin {
                device: BuiltinDeviceType::Synth
            }
        );
        // Re-creating the same device replaces its node.
        let key2 = b.create_builtin(d, &BuiltinDevice::Delay, &[]).unwrap();
        assert_ne!(key, key2);
        assert!(b.destroy_node(key).is_err());
        b.destroy_node(key2).unwrap();
        assert!(b.descriptor(d).is_none());
    }

    #[test]
    fn sampler_slices_update_in_place() {
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(4));
        let sampler = |markers: Vec<f64>| BuiltinDevice::Sampler {
            sample: None,
            slices: ether_core::protocol::model::SliceSettings {
                enabled: true,
                base_note: 36,
                markers: markers
                    .into_iter()
                    .map(ether_core::protocol::model::Seconds)
                    .collect(),
            },
        };
        let key = b.create_builtin(d, &sampler(vec![0.0]), &[]).unwrap();
        assert_eq!(b.update_builtin(d, &sampler(vec![0.0, 0.5])), Ok(true));
        assert_eq!(b.node_of(d), Some(key), "same node");
        // Other devices (and unknown ones) are re-created by the controller.
        let rack = DeviceId(Ulid(5));
        b.create_builtin(rack, &BuiltinDevice::DrumRack, &[])
            .unwrap();
        assert_eq!(b.update_builtin(rack, &BuiltinDevice::DrumRack), Ok(false));
        assert_eq!(
            b.update_builtin(DeviceId(Ulid(6)), &sampler(vec![])),
            Ok(false)
        );
    }

    #[test]
    fn multisampler_zones_update_in_place() {
        use ether_core::protocol::model::SampleZone;
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(7));
        let ms = |n: usize| BuiltinDevice::MultiSampler {
            zones: vec![SampleZone::default(); n],
        };
        let key = b.create_builtin(d, &ms(1), &[]).unwrap();
        assert_eq!(b.update_builtin(d, &ms(2)), Ok(true));
        assert_eq!(b.node_of(d), Some(key), "same node");
        // A multisampler kind sent for another device type is refused.
        let s = DeviceId(Ulid(8));
        b.create_builtin(s, &BuiltinDevice::Compressor, &[])
            .unwrap();
        assert_eq!(b.update_builtin(s, &ms(1)), Ok(false));
    }

    #[test]
    fn convolution_ir_updates_in_place() {
        use ether_core::protocol::model::IrSource;
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(9));
        let reverb = |id: Option<&str>| BuiltinDevice::ConvolutionReverb {
            ir: id.map(|id| IrSource::Factory { id: id.into() }),
        };
        let key = b.create_builtin(d, &reverb(Some("room")), &[]).unwrap();
        assert_eq!(b.update_builtin(d, &reverb(Some("hall"))), Ok(true));
        assert_eq!(b.update_builtin(d, &reverb(None)), Ok(true));
        assert_eq!(b.node_of(d), Some(key), "same node");
        // A reverb kind sent for another device type is refused.
        let s = DeviceId(Ulid(10));
        b.create_builtin(s, &BuiltinDevice::Compressor, &[])
            .unwrap();
        assert_eq!(b.update_builtin(s, &reverb(Some("hall"))), Ok(false));
    }

    #[test]
    fn midi_input_reaches_the_controller_with_its_port() {
        let (mut b, _engine) = bridge();
        let mut out = Vec::new();
        b.poll_midi_input(&mut out);
        assert!(out.is_empty());
        crate::recording::inject_midi_from(&b.audio, "Knobs", [0xb0, 21, 99]);
        b.poll_midi_input(&mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(
            (out[0].port.as_str(), out[0].data),
            ("Knobs", [0xb0, 21, 99])
        );
    }

    /// `keyboard-midi`: computer keyboard notes go to the engine's live MIDI ring (the
    /// path `recording::tests` plays and records) but not back to the controller's port
    /// queue (the controller feeds MIDI capture itself: counted once).
    #[test]
    fn live_midi_goes_to_the_engine_only() {
        let (mut b, _engine) = bridge();
        b.live_midi([0x90, 60, 100]).unwrap();
        b.live_midi([0x80, 60, 0]).unwrap();
        let mut out = Vec::new();
        b.poll_midi_input(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn media_is_resampled_and_registered() {
        let (mut b, _engine) = bridge();
        let audio = Arc::new(DecodedAudio {
            sample_rate: 44_100,
            channels: vec![vec![0.25; 441]],
        });
        b.load_media(&media_ref(1), audio).unwrap();
        let src = b.sources.get(&MediaId(Ulid(1))).unwrap();
        assert_eq!(
            src.frames(),
            ether_media::resampled_len(441, 44_100, 48_000) as u64
        );
        b.unload_media(MediaId(Ulid(1))).unwrap();
        assert!(b.sources.is_empty());
        // Unknown media: no-op.
        b.unload_media(MediaId(Ulid(9))).unwrap();
    }

    /// A plugin device re-created (reload, sandbox toggle, a restart request, a preset)
    /// with its editor open: the editor is the new instance's (the old window closes, the
    /// new one opens), so the window the UI shows as open is live and closes and reopens
    /// normally. A device whose plugin can't be re-created reports its editor closed.
    #[test]
    fn recreating_a_plugin_keeps_its_editor_on_the_new_instance() {
        let (make, probe) = fake::editor_plugins();
        let (mut b, _engine) = bridge_with(make);
        let d = DeviceId(Ulid(5));
        let inst = PluginInstance {
            format: PluginFormat::Clap,
            plugin_id: "fake".into(),
            name: "Fake".into(),
            vendor: "T".into(),
            version: "1".into(),
            sandboxed: false,
            state: None,
        };
        b.create_plugin(d, &inst, None).unwrap();
        b.plugins().open_editor(d).unwrap();
        b.create_plugin(d, &inst, None).unwrap();
        assert_eq!(fake::take_log(&probe), ["1 open", "1 close", "2 open"]);
        let mut notes = Vec::new();
        b.poll_plugins(&mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        b.plugins().close_editor(d).unwrap();
        b.plugins().open_editor(d).unwrap();
        assert_eq!(fake::take_log(&probe), ["2 close", "2 open"]);

        // Not installed any more: the old instance goes, its editor reported closed.
        let missing = PluginInstance {
            plugin_id: "not-installed".into(),
            ..inst
        };
        assert!(b.create_plugin(d, &missing, None).is_err());
        assert_eq!(fake::take_log(&probe), ["2 close"]);
        // (Polled although no plugin is left; a poll may stay in flight a few ticks.)
        notes.clear();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while notes.is_empty() && std::time::Instant::now() < deadline {
            b.poll_plugins(&mut notes);
        }
        assert_eq!(notes, vec![(d, PluginNotification::EditorClosed)]);
        assert!(matches!(
            b.plugins().open_editor(d),
            Err(ether_core::plugin::PluginError::NotFound(_))
        ));
    }

    #[test]
    fn plugins_via_catalog_and_main_thread() {
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(5));
        let inst = PluginInstance {
            format: PluginFormat::Clap,
            plugin_id: "fake".into(),
            name: "Fake".into(),
            vendor: "T".into(),
            version: "1".into(),
            sandboxed: false,
            state: None,
        };
        let key = b
            .create_plugin(d, &inst, Some(&Base64Bytes(b"abc".to_vec())))
            .unwrap();
        assert_eq!(b.node_of(d), Some(key));
        assert_eq!(b.descriptor(d).unwrap().name, "Fake");
        assert_eq!(
            b.plugin_state(d).unwrap(),
            Some(Base64Bytes(b"abc".to_vec()))
        );
        assert_eq!(b.plugin_state(DeviceId(Ulid(6))).unwrap(), None);
        let mut notes = Vec::new();
        b.poll_plugins(&mut notes);
        assert_eq!(notes, vec![(d, PluginNotification::StateDirty)]);

        let missing = PluginInstance {
            plugin_id: "not-installed".into(),
            ..inst.clone()
        };
        match b.create_plugin(DeviceId(Ulid(7)), &missing, None) {
            Err(BridgeError::Other(m)) => assert_eq!(
                m,
                "CLAP plugin Fake (not-installed) is not installed (rescan plugins)"
            ),
            other => panic!("{other:?}"),
        }
        // Looked up by (format, id): the same id in another format is another plugin.
        let other_format = PluginInstance {
            format: PluginFormat::Vst3,
            name: "fake".into(),
            ..inst.clone()
        };
        match b.create_plugin(DeviceId(Ulid(8)), &other_format, None) {
            Err(BridgeError::Other(m)) => {
                assert_eq!(m, "VST3 plugin (fake) is not installed (rescan plugins)")
            }
            other => panic!("{other:?}"),
        }
        assert!(b.descriptor(DeviceId(Ulid(8))).is_none());

        b.destroy_node(key).unwrap();
        assert!(b.descriptor(d).is_none());
    }

    /// A device whose plugin isn't in the catalog gets no instance; once a scan (here a
    /// retried target) lists it, re-creating the device ("found · reload") loads it with the
    /// state saved in the project.
    #[test]
    fn a_missing_plugin_loads_with_its_saved_state_once_a_scan_finds_it() {
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(5));
        let inst = PluginInstance {
            format: PluginFormat::Vst3,
            plugin_id: "later".into(),
            name: "Later".into(),
            vendor: "T".into(),
            version: "1".into(),
            sandboxed: false,
            state: None,
        };
        let saved = Base64Bytes(b"saved".to_vec());
        assert!(b.create_plugin(d, &inst, Some(&saved)).is_err());
        assert_eq!(b.node_of(d), None);

        let target = ether_plugin_host::ScanTarget {
            format: PluginFormat::Vst3,
            path: "/later.vst3".into(),
        };
        b.catalog.apply_retry(&ether_plugin_host::TargetScan::found(
            target,
            vec![PluginDescriptor {
                sidechain_inputs: Default::default(),
                format: PluginFormat::Vst3,
                id: "later".into(),
                name: "Later".into(),
                vendor: "T".into(),
                version: "1".into(),
                description: String::new(),
                features: vec![],
                category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
                path: "/later.vst3".into(),
            }],
        ));
        let key = b.create_plugin(d, &inst, Some(&saved)).unwrap();
        assert_eq!(b.node_of(d), Some(key));
        assert_eq!(b.plugin_state(d).unwrap(), Some(saved));
    }

    #[test]
    fn offline_plugins_are_independent_instances() {
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(5));
        let inst = PluginInstance {
            format: PluginFormat::Clap,
            plugin_id: "fake".into(),
            name: "Fake".into(),
            vendor: "T".into(),
            version: "1".into(),
            sandboxed: false,
            state: None,
        };
        let state = Base64Bytes(b"abc".to_vec());
        let key = b.create_plugin(d, &inst, Some(&state)).unwrap();
        let live = b.plugins().live_count();
        let offline = b
            .create_offline_plugin(d, &inst, Some(&state), 44_100)
            .unwrap();
        assert_eq!(b.plugins().live_count(), live + 1);
        // The offline instance's notifications (its state load) never reach the controller.
        let mut notes = Vec::new();
        b.poll_plugins(&mut notes);
        assert!(notes.iter().all(|(dev, _)| *dev == d), "{notes:?}");
        // The live instance is untouched.
        assert_eq!(b.node_of(d), Some(key));
        assert_eq!(b.plugin_state(d).unwrap(), Some(state.clone()));
        // Dropping the offline node releases its instance.
        drop(offline);
        assert_eq!(b.plugins().live_count(), live);
        assert_eq!(b.plugin_state(d).unwrap(), Some(state));

        let missing = PluginInstance {
            plugin_id: "not-installed".into(),
            ..inst
        };
        match b.create_offline_plugin(d, &missing, None, 48_000) {
            Err(BridgeError::Other(m)) => assert!(m.contains("is not installed"), "{m}"),
            other => panic!("{:?}", other.map(|_| ())),
        }
    }

    /// A dedicated engine's instances report their edits from its own poll, with document
    /// ids (never from the main engine's); tearing it down closes the editor it opened and
    /// leaves the host's instance alone.
    #[test]
    fn a_dedicated_engines_editor_edits_come_from_its_own_poll() {
        let (make, probe) = fake::editor_plugins();
        let (mut b, _engine) = bridge_with(make);
        let d = DeviceId(Ulid(5));
        let inst = PluginInstance {
            format: PluginFormat::Clap,
            plugin_id: "fake".into(),
            name: "Fake".into(),
            vendor: "T".into(),
            version: "1".into(),
            sandboxed: false,
            state: None,
        };
        b.create_plugin(d, &inst, None).unwrap(); // editor instance 1
        let mut e = b.spawn_dedicated().unwrap();
        e.create_plugin(d, &inst, None).unwrap(); // instance 2
        let salt = e.bridge().registry_salt();
        assert_ne!(salt, 0);
        b.plugins()
            .open_editor(dedicated_device_id(salt, d))
            .unwrap();
        assert_eq!(fake::take_log(&probe), ["2 open"]);
        assert!(!is_main_instance_id(dedicated_device_id(salt, d)));
        assert!(is_main_instance_id(d));

        // Edits in its editor: from its own poll, with the document's id.
        probe.notes.lock().unwrap().extend([
            PluginNotification::GestureBegin { param: ParamId(1) },
            PluginNotification::ParamEdited {
                param: ParamId(1),
                value: 0.25,
            },
        ]);
        let mut theirs = Vec::new();
        for _ in 0..200 {
            let mut main = Vec::new();
            b.poll_plugins(&mut main);
            assert!(
                main.iter().all(|(_, n)| !matches!(
                    n,
                    PluginNotification::ParamEdited { .. }
                        | PluginNotification::GestureBegin { .. }
                )),
                "{main:?}"
            );
            e.poll_plugins(&mut theirs);
            if theirs.len() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            theirs,
            [
                (d, PluginNotification::GestureBegin { param: ParamId(1) }),
                (
                    d,
                    PluginNotification::ParamEdited {
                        param: ParamId(1),
                        value: 0.25
                    }
                ),
            ]
        );

        // Tearing the engine down closes the editor; the host's instance is untouched.
        let live = b.plugins().live_count();
        drop(e);
        for _ in 0..200 {
            if b.plugins().live_count() < live {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(fake::take_log(&probe), ["2 close"]);
        assert!(b.dedicated_notes.lock().unwrap().is_empty());
    }

    /// A replicated state (the host's preset) is loaded into a dedicated engine's
    /// instance in place: same instance, its open editor stays open, the host's own
    /// instance untouched.
    #[test]
    fn a_dedicated_engines_instance_loads_a_state_live_with_its_editor_open() {
        let (make, probe) = fake::editor_plugins();
        let (mut b, _engine) = bridge_with(make);
        let d = DeviceId(Ulid(5));
        let inst = PluginInstance {
            format: PluginFormat::Clap,
            plugin_id: "fake".into(),
            name: "Fake".into(),
            vendor: "T".into(),
            version: "1".into(),
            sandboxed: false,
            state: None,
        };
        let mine = Base64Bytes(b"host".to_vec());
        b.create_plugin(d, &inst, Some(&mine)).unwrap(); // instance 1
        let mut e = b.spawn_dedicated().unwrap();
        e.create_plugin(d, &inst, None).unwrap(); // instance 2
        let salt = e.bridge().registry_salt();
        b.plugins()
            .open_editor(dedicated_device_id(salt, d))
            .unwrap();
        assert_eq!(fake::take_log(&probe), ["2 open"]);
        let live = b.plugins().live_count();

        let preset = Base64Bytes(b"preset".to_vec());
        assert_eq!(e.set_plugin_state(d, &preset).ok(), Some(true));
        assert_eq!(e.plugin_state(d).unwrap(), Some(preset));
        assert!(fake::take_log(&probe).is_empty(), "the editor stayed open");
        assert_eq!(b.plugins().live_count(), live, "not re-created");
        assert_eq!(b.plugin_state(d).unwrap(), Some(mine));
        // No instance: the caller re-creates from the document.
        let none = e.set_plugin_state(DeviceId(Ulid(6)), &Base64Bytes(Vec::new()));
        assert_eq!(none.ok(), Some(false));
    }

    /// Dedicated remote engines: a second engine beside the main one,
    /// with its own plugin instances of the same devices, running headless.
    #[test]
    fn a_dedicated_engine_runs_beside_the_main_one() {
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(5));
        let inst = PluginInstance {
            format: PluginFormat::Clap,
            plugin_id: "fake".into(),
            name: "Fake".into(),
            vendor: "T".into(),
            version: "1".into(),
            sandboxed: false,
            state: None,
        };
        let mine = Base64Bytes(b"main".to_vec());
        let theirs = Base64Bytes(b"remote".to_vec());
        let main_key = b.create_plugin(d, &inst, Some(&mine)).unwrap();
        let live = b.plugins().live_count();

        let mut e = b.spawn_dedicated().expect("a second engine starts");
        // The same device: a second instance with its own state; the main one stays.
        let k = e.create_plugin(d, &inst, Some(&theirs)).unwrap();
        assert_eq!(b.plugins().live_count(), live + 1);
        assert_eq!(b.node_of(d), Some(main_key));
        assert_eq!(b.plugin_state(d).unwrap(), Some(mine.clone()));
        assert_eq!(e.plugin_state(d).unwrap(), Some(theirs));
        // Its notifications never reach the document through the main bridge.
        // The main instance's notification can arrive after the first poll on a loaded machine.
        let mut notes = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while notes.is_empty() && std::time::Instant::now() < deadline {
            b.poll_plugins(&mut notes);
            if notes.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert!(notes.iter().all(|(dev, _)| *dev == d), "{notes:?}");
        assert_eq!(notes.len(), 1, "only the main instance's: {notes:?}");

        // It renders on its own (paced, no audio device): its playhead moves, the main
        // engine's (never started here) does not.
        e.publish(RenderGraphDesc {
            version: 1,
            ..Default::default()
        })
        .unwrap();
        e.transport(TransportControl::Play).unwrap();
        let mut moved = false;
        for _ in 0..200 {
            let mut out = EngineOutputs::default();
            e.poll(&mut out);
            if out
                .playhead
                .is_some_and(|p| p.playing && p.position.0 > 0.0)
            {
                moved = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(moved, "the dedicated engine's playhead advanced");
        let mut main = EngineOutputs::default();
        b.poll(&mut main);
        assert!(!main.playhead.is_some_and(|p| p.playing));
        e.destroy_node(k).unwrap();

        // Dropping it stops the engine and releases its plugins; the main one is untouched.
        drop(e);
        for _ in 0..200 {
            if b.plugins().live_count() == live {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(b.plugins().live_count(), live);
        assert_eq!(b.plugin_state(d).unwrap(), Some(mine));
    }

    // ─── external-instrument ───

    /// Records what the MIDI thread sends, with the time.
    type Sent = Vec<(std::time::Instant, String, Vec<u8>)>;
    #[derive(Clone, Default)]
    struct RecordingSink(Arc<std::sync::Mutex<Sent>>);

    impl hw_midi::MidiOutSink for RecordingSink {
        fn send(&mut self, port: &str, data: &[u8]) -> bool {
            if port == "Gone" {
                return false;
            }
            self.0
                .lock()
                .unwrap()
                .push((std::time::Instant::now(), port.into(), data.to_vec()));
            true
        }
    }

    fn key(i: u32) -> NodeKey {
        NodeKey {
            index: i,
            generation: 1,
        }
    }

    fn ev(node: u32, frame: u64, data: [u8; 3]) -> ether_core::hw_io::HwMidiEvent {
        ether_core::hw_io::HwMidiEvent {
            node: key(node),
            frame,
            data,
        }
    }

    #[test]
    fn external_routing_updates_in_place() {
        let (mut b, _engine) = bridge();
        let d = DeviceId(Ulid(9));
        let kind = BuiltinDevice::new(BuiltinDeviceType::ExternalAudioEffect);
        b.create_builtin(d, &kind, &[]).unwrap();
        assert!(b.update_builtin(d, &kind).unwrap());
        let other = BuiltinDevice::new(BuiltinDeviceType::ExternalInstrument);
        assert!(
            !b.update_builtin(d, &other).unwrap(),
            "another type is re-created"
        );
        assert!(!b.update_builtin(DeviceId(Ulid(10)), &kind).unwrap());
    }

    #[test]
    fn hw_midi_scheduler_orders_routes_and_releases_notes() {
        let sink = RecordingSink::default();
        let mut out = sink.clone();
        let mut s = hw_midi::Scheduler::default();
        s.set_routes(
            1,
            [(key(1), "A".to_string()), (key(2), "Gone".to_string())].into(),
            &mut out,
        );
        s.push(ev(1, 200, [0x90, 60, 100]));
        s.push(ev(1, 100, [0x90, 62, 100]));
        s.push(ev(2, 100, [0x90, 64, 100]));
        s.push(ev(3, 100, [0x90, 65, 100]));
        // Nothing due before frame + offset.
        assert_eq!(s.step(149.0, 50.0, &mut out), Some(100));
        assert!(sink.0.lock().unwrap().is_empty());
        // Frame 100 is due: node 1 sends, node 2's port is gone (dropped), node 3 has no
        // route (dropped).
        assert_eq!(s.step(150.0, 50.0, &mut out), Some(200));
        assert_eq!(sink.0.lock().unwrap().len(), 1);
        assert_eq!(s.step(250.0, 50.0, &mut out), None);
        let sent: Vec<Vec<u8>> = sink.0.lock().unwrap().iter().map(|e| e.2.clone()).collect();
        assert_eq!(sent, vec![vec![0x90, 62, 100], vec![0x90, 60, 100]]);
        // A note-off releases one; the port changing releases the rest on the old port.
        s.push(ev(1, 300, [0x80, 62, 0]));
        s.step(400.0, 0.0, &mut out);
        sink.0.lock().unwrap().clear();
        s.set_routes(2, [(key(1), "B".to_string())].into(), &mut out);
        let sent: Vec<(String, Vec<u8>)> = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|e| (e.1.clone(), e.2.clone()))
            .collect();
        assert_eq!(sent, vec![("A".to_string(), vec![0x80, 60, 0])]);
    }

    /// Messages leave when their engine sample plays: clock time of the frame plus the
    /// output latency (here the block, no device latency reported).
    #[test]
    fn hw_midi_thread_sends_on_time() {
        let audio = Arc::new(AudioShared::default());
        let (mut tx, rx) = ether_core::recording::rtrb::RingBuffer::new(64);
        let routes = Arc::new(hw_midi::Routes::default());
        routes.set([(key(1), "A".to_string())].into());
        let sink = RecordingSink::default();
        let t = hw_midi::spawn(
            ether_core::hw_io::HwIoIo { midi: rx },
            routes,
            audio.clone(),
            48_000.0,
            480,
            Box::new(sink.clone()),
        )
        .unwrap();
        // The engine starts rendering sample 0 now; a note at 100 ms (4800).
        audio.clock.publish(0);
        let start = std::time::Instant::now();
        tx.push(ev(1, 4800, [0x90, 60, 100])).unwrap();
        tx.push(ev(1, 9600, [0x80, 60, 0])).unwrap();
        let deadline = start + std::time::Duration::from_secs(5);
        while sink.0.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        drop(t);
        let sent = sink.0.lock().unwrap().clone();
        assert_eq!(sent.len(), 2);
        // Due at (4800 + 480) / 48 kHz = 110 ms and 210 ms. Never early; late by at most 50 ms
        // (a shared CI runner can stall a thread for ~15 ms; a broken schedule is off by 100s).
        for ((at, _, _), want) in sent.iter().zip([110.0, 210.0]) {
            let ms = at.duration_since(start).as_secs_f64() * 1000.0;
            assert!(
                ms >= want - 1.0 && ms < want + 50.0,
                "sent at {ms} ms, due {want}"
            );
        }
    }

    /// Acceptance with the null backend's loopback input: the controller measures an
    /// External Audio Effect's round trip (send → loopback → return) through the native
    /// bridge, and sets its `Latency` to it.
    #[test]
    fn measure_latency_through_the_null_loopback() {
        use ether_controller::memory::{MemoryLibrary, MemoryStore};
        use ether_controller::{Controller, EtherController};
        use ether_core::protocol::Command;
        use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
        use ether_core::protocol::external::{ExternalCommand, ExternalEvent};
        use ether_core::protocol::message::{ClientMessage, Event, ServerMessage};
        use ether_core::protocol::model::{
            ExternalRouting, HwChannels, ProjectId, TrackId, TrackKind,
        };
        use ether_core::protocol::project::ProjectCommand;
        use ether_core::protocol::tracks::TrackCommand;

        let delay = 2048usize;
        let block = 256usize;
        let parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: 48_000,
            max_block_size: block,
            max_nodes: 32,
            ..Default::default()
        });
        let shared = Arc::new(AudioShared::default());
        let settings = crate::audio::AudioSettings {
            backend: crate::audio::AudioBackendKind::Null,
            input_device: Some(format!("loopback:{delay}")),
            max_block_size: block,
            ..Default::default()
        };
        let out =
            crate::audio::AudioOutput::start(Box::new(parts.engine), &settings, shared.clone())
                .map_err(|(e, _)| e)
                .expect("null backend starts");
        let mut gc = parts.gc;
        let gc_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gc_flag = gc_stop.clone();
        let gc_thread = std::thread::spawn(move || {
            while !gc_flag.load(std::sync::atomic::Ordering::Relaxed) {
                gc.collect();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        let b = NativeBridge::new(
            parts.handle,
            PrepareConfig {
                sample_rate: 48_000.0,
                max_block_size: block,
                max_events_per_block: 256,
            },
            PluginHost::new(Arc::new(DedicatedThread::new())),
            PluginCatalog::default(),
            fake::instantiate(),
            shared.clone(),
        );
        let mut ctl = EtherController::new(
            b,
            NativeServices::default(),
            MemoryStore::new(),
            MemoryLibrary::new(),
        );
        let mut n = 0;
        let mut send = |ctl: &mut EtherController<_, _, _, _>, command: Command| {
            n += 1;
            let mut out = Vec::new();
            ctl.handle(
                ClientMessage {
                    id: n,
                    gesture: None,
                    command,
                },
                &mut out,
            );
            out
        };
        send(
            &mut ctl,
            Command::Project(ProjectCommand::Create {
                id: ProjectId::v7(1_750_000_000_000, [7; 10]),
                name: "Loop".into(),
            }),
        );
        let track = TrackId(Ulid(10));
        send(
            &mut ctl,
            Command::Track(TrackCommand::Create {
                id: track,
                kind: TrackKind::Audio,
                name: None,
                color: None,
                parent: None,
                before: None,
            }),
        );
        let fx = DeviceId(Ulid(11));
        send(
            &mut ctl,
            Command::Device(DeviceCommand::Insert {
                id: fx,
                track,
                device: DeviceSpec::Builtin {
                    device: BuiltinDevice::new(BuiltinDeviceType::ExternalAudioEffect),
                },
                before: None,
            }),
        );
        // The loopback returns the stereo output on inputs 1/2.
        send(
            &mut ctl,
            Command::External(ExternalCommand::SetRouting {
                device: fx,
                routing: ExternalRouting {
                    audio_send: Some(HwChannels { first: 0, count: 2 }),
                    audio_return: Some(HwChannels { first: 0, count: 2 }),
                    ..ExternalRouting::default()
                },
            }),
        );
        let ports = send(&mut ctl, Command::External(ExternalCommand::ListPorts));
        assert!(
            format!("{ports:?}").contains("Loopback L"),
            "the loopback input is listed: {ports:?}"
        );
        send(
            &mut ctl,
            Command::External(ExternalCommand::MeasureLatency { device: fx }),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut result = None;
        while result.is_none() && std::time::Instant::now() < deadline {
            let mut out = Vec::new();
            ctl.tick(NativeServices::default().now_ms(), &mut out);
            for m in out {
                if let ServerMessage::Event(Event::External { event }) = m {
                    result = Some(event);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let want = delay as f64 * 1000.0 / 48_000.0;
        match result {
            Some(ExternalEvent::LatencyMeasured { device, latency_ms }) => {
                assert_eq!(device, fx);
                assert!((latency_ms - want).abs() < 1e-6, "{latency_ms} vs {want}");
            }
            other => panic!("no measurement: {other:?}"),
        }
        let p = ctl.project().unwrap();
        assert_eq!(
            p.devices[&fx]
                .params
                .get(&ether_devices::external::external_audio_effect::LATENCY)
                .copied(),
            Some(want)
        );
        drop(ctl);
        drop(out.stop());
        gc_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = gc_thread.join();
    }
}
