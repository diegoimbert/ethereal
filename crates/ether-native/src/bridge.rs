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
    PluginInstance,
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
    instantiate: Instantiate,
    audio: Arc<AudioShared>,
    sources: HashMap<MediaId, Arc<dyn AudioSource>>,
    devices: HashMap<DeviceId, DeviceEntry>,
    /// Instances created for offline export so far (their private registry ids).
    offline_seq: u64,
    /// "Listen on <peer>" native sender (`stream-host`; [`crate::stream`]).
    stream: crate::stream::StreamHost,
    /// GUI-only plugin mirrors while listening (`plugin-mirror`; [`crate::plugin_mirror`]).
    mirrors: crate::plugin_mirror::Mirrors,
}

impl NativeBridge {
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
        Self {
            handle,
            sample_rate: prepare.sample_rate as u32,
            prepare,
            plugins,
            catalog,
            instantiate,
            audio,
            sources: HashMap::new(),
            devices: HashMap::new(),
            offline_seq: 0,
            stream: crate::stream::StreamHost::new(),
            mirrors: Default::default(),
        }
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
        self.mirrors.unpark(device);
        let Some(entry) = self.devices.remove(&device) else {
            return Ok(());
        };
        let r = self.handle.remove_node(entry.key).map_err(engine_err);
        if matches!(entry.kind, DeviceKind::Plugin(_)) {
            self.plugins.destroy(device);
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
        self.replace_device(device);
        // Mirrored (listening): the mirror is the instance; the slot gets a stand-in.
        if let Some(descriptor) = self.mirrors.descriptor(device).cloned() {
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
        let desc = self
            .catalog
            .find_format(plugin.format, &plugin.plugin_id)
            .ok_or_else(|| BridgeError::Other(not_installed(plugin)))?;
        let (node, descriptor) = self
            .plugins
            .instantiate(
                crate::sandbox::instantiator(plugin.sandboxed, &self.instantiate),
                device,
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
                self.plugins.destroy(device);
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
            .find_format(plugin.format, &plugin.plugin_id)
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
        if self.sources.remove(&media).is_some() {
            self.handle.remove_source(media).map_err(engine_err)?;
        }
        Ok(())
    }

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.stream.observe_graph(&graph);
        self.handle.publish(graph).map_err(engine_err)
    }

    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.handle.set_param(change).map_err(engine_err)
    }

    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.stream.observe_transport(&control);
        self.handle.transport(control).map_err(engine_err)
    }

    fn poll(&mut self, out: &mut EngineOutputs) {
        self.handle.poll(out);
        out.cpu_load = self.audio.cpu_load();
    }

    /// v0.2 analysis channel (contracts-3): device frames straight from the engine handle.
    fn poll_analysis(&mut self, out: &mut Vec<ether_core::AnalysisFrame>) {
        self.handle.poll_analysis(|f| out.push(*f));
    }

    fn watch_analysis(&mut self, node: NodeKey, on: bool) -> Result<(), BridgeError> {
        self.handle.watch_analysis(node, on).map_err(engine_err)
    }

    /// `latency-republish`: node latency as last observed by the audio thread.
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
        if !self.mirrors.is_empty()
            || self
                .devices
                .values()
                .any(|d| matches!(d.kind, DeviceKind::Plugin(_)))
        {
            let from = out.len();
            self.plugins.poll(out);
            // Offline (export) instances are private to the export: their notifications
            // (state dirty, GUI edits, latency) must not reach the document.
            let mut i = from;
            while i < out.len() {
                if is_offline_id(out[i].0) {
                    out.remove(i);
                } else {
                    i += 1;
                }
            }
            self.mirrors.observe(&out[from..]);
        }
    }

    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        // A mirrored device's state is its mirror's (COLLAB.md §9.6), or the last state of
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
                .save_state(device)
                .map_err(plugin_err)?
                .map(Base64Bytes)),
            _ => Ok(None),
        }
    }

    fn list_inputs(&mut self) -> Result<InputList, BridgeError> {
        crate::recording::list_inputs(&self.audio)
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
        self.plugins.param_values(device)
    }

    fn poll_midi_input(&mut self, out: &mut Vec<ether_core::protocol::midi_map::MidiInputEvent>) {
        crate::recording::drain_midi_input(&self.audio, out);
    }

    // "Listen on <peer>" native sender (`stream-host`): delegated to `crate::stream`.

    fn stream_capabilities(&self) -> ether_controller::streaming::StreamCapabilities {
        self.stream.capabilities()
    }

    fn start_stream_capture(&mut self) -> Result<(), BridgeError> {
        self.stream
            .start_capture(&mut self.handle, self.sample_rate)
    }

    fn stop_stream_capture(&mut self) -> Result<(), BridgeError> {
        self.stream.stop_capture(&mut self.handle)
    }

    fn stream_open(
        &mut self,
        listener: ether_core::protocol::model::SiteId,
        stream: u32,
        ice: &[ether_core::protocol::collab::IceServer],
    ) -> Result<(), BridgeError> {
        self.stream.open(listener, stream, ice)
    }

    fn stream_signal(
        &mut self,
        listener: ether_core::protocol::model::SiteId,
        stream: u32,
        signal: &ether_core::protocol::collab::StreamSignal,
    ) -> Result<(), BridgeError> {
        self.stream.signal(listener, stream, signal)
    }

    fn stream_close(
        &mut self,
        listener: ether_core::protocol::model::SiteId,
        stream: u32,
    ) -> Result<(), BridgeError> {
        self.stream.close(listener, stream)
    }

    fn poll_stream(&mut self, out: &mut Vec<ether_controller::streaming::StreamOutput>) {
        self.stream.poll(out);
    }

    // Plugin GUI mirrors (`plugin-mirror`): delegated to `crate::plugin_mirror`.

    fn create_plugin_mirror(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<(), BridgeError> {
        let desc = self
            .catalog
            .find_format(plugin.format, &plugin.plugin_id)
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

/// Native [`HostServices`]: wall clock and OS entropy.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeServices;

impl HostServices for NativeServices {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }

    fn random_seed(&mut self) -> u64 {
        getrandom::u64().unwrap_or_else(|_| self.now_ms() ^ u64::from(std::process::id()) << 32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{DedicatedThread, fake};
    use ether_core::protocol::model::{PluginFormat, Ulid};
    use ether_core::protocol::plugins::PluginDescriptor;

    fn bridge() -> (NativeBridge, ether_core::Engine) {
        let ether_core::EngineParts { engine, handle, .. } =
            ether_core::create(ether_core::EngineConfig {
                max_block_size: 128,
                ..Default::default()
            });
        let catalog = PluginCatalog::default();
        catalog.replace(vec![PluginDescriptor {
            sidechain_inputs: Default::default(),
            format: PluginFormat::Clap,
            id: "fake".into(),
            name: "Fake".into(),
            vendor: "T".into(),
            version: "1".into(),
            description: String::new(),
            features: vec![],
            category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
            path: "/fake.clap".into(),
        }]);
        let b = NativeBridge::new(
            handle,
            PrepareConfig {
                sample_rate: 48_000.0,
                max_block_size: 128,
                max_events_per_block: 64,
            },
            PluginHost::new(Arc::new(DedicatedThread::new())),
            catalog,
            fake::instantiate(),
            Arc::new(AudioShared::default()),
        );
        (b, engine)
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
}
