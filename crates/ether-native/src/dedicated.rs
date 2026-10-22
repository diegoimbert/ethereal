//! Dedicated engines: a second engine instance of this app that runs a copy of the open
//! project for someone else (an extension streams its output, docs/EXTENSIONS.md).
//!
//! [`NativeBridge::spawn_dedicated`] builds a complete second `NativeBridge` over its own
//! `ether_core` engine, **not connected to the audio device**: the engine runs on the paced
//! `null` audio backend (a thread that renders at real-time speed and discards the output).
//! Plugins are instantiated by the same [`PluginHost`] as the main engine's but registered
//! under ids of their own, so the host's own instances of the same devices are never
//! retired. Their notifications (filed by the main bridge, which drains the registry's one
//! queue) come back from this engine's `poll_plugins`, with the document's ids.
//!
//! [`DedicatedEngine`] is the [`EngineBridge`] a controller holds: it forwards the calls a
//! dedicated engine needs (nodes, media, graph, params, transport, playhead) and owns what
//! keeps the engine running; dropping it stops the engine, releases its plugins and joins
//! its threads. Its bridge's extension ([`crate::bridge::BridgeExt`]) is its own.
//!
//! [`PluginHost`]: crate::plugins::PluginHost

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use ether_controller::{BridgeError, EngineBridge};
use ether_core::plugin::PluginNotification;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, DeviceId, MediaId, MediaRef, MusicalScale, ParamId, PluginInstance,
};
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;

use crate::audio::AudioOutput;
use crate::bridge::NativeBridge;

/// What keeps a dedicated engine alive: its (paced, headless) audio backend and the thread
/// that drops what the audio thread retired.
pub(crate) struct Runner {
    output: Option<AudioOutput>,
    gc_stop: Arc<AtomicBool>,
    gc_thread: Option<JoinHandle<()>>,
}

impl Runner {
    pub(crate) fn new(
        output: AudioOutput,
        gc_stop: Arc<AtomicBool>,
        gc_thread: JoinHandle<()>,
    ) -> Self {
        Self {
            output: Some(output),
            gc_stop,
            gc_thread: Some(gc_thread),
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // Stop the render thread first; the engine it hands back is dropped here (its nodes
        // are retired and returned to the plugin main thread by the GC pass below).
        if let Some(out) = self.output.take() {
            drop(out.stop());
        }
        self.gc_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.gc_thread.take() {
            let _ = t.join();
        }
    }
}

/// The controller's handle on a dedicated engine.
pub struct DedicatedEngine {
    // Field order is drop order: the bridge releases its plugin registrations before the
    // runner stops the engine and collects.
    bridge: NativeBridge,
    _runner: Runner,
}

impl DedicatedEngine {
    pub(crate) fn new(bridge: NativeBridge, runner: Runner) -> Self {
        Self {
            bridge,
            _runner: runner,
        }
    }
}

impl DedicatedEngine {
    /// Its bridge.
    pub fn bridge(&self) -> &NativeBridge {
        &self.bridge
    }

    /// Its bridge (an extension's calls).
    pub fn bridge_mut(&mut self) -> &mut NativeBridge {
        &mut self.bridge
    }
}

impl EngineBridge for DedicatedEngine {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.bridge.create_builtin(device, kind, params)
    }

    fn set_node_scale(
        &mut self,
        device: DeviceId,
        scale: MusicalScale,
    ) -> Result<bool, BridgeError> {
        self.bridge.set_node_scale(device, scale)
    }

    fn update_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
    ) -> Result<bool, BridgeError> {
        self.bridge.update_builtin(device, kind)
    }

    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        self.bridge.create_plugin(device, plugin, state)
    }

    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.bridge.destroy_node(key)
    }

    fn set_always_process(&mut self, device: DeviceId, on: bool) -> Result<(), BridgeError> {
        self.bridge.set_always_process(device, on)
    }

    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.bridge.load_media(media, audio)
    }

    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.bridge.unload_media(media)
    }

    fn stream_media(
        &mut self,
        source: &ether_controller::media_stream::StreamSource,
    ) -> Result<bool, BridgeError> {
        self.bridge.stream_media(source)
    }

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.bridge.publish(graph)
    }

    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.bridge.set_param(change)
    }

    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.bridge.transport(control)
    }

    fn set_suspended(&mut self, suspended: bool) -> Result<(), BridgeError> {
        self.bridge.set_suspended(suspended)
    }

    fn poll(&mut self, out: &mut EngineOutputs) {
        self.bridge.poll(out)
    }

    fn node_latency(&self, key: NodeKey) -> Option<u32> {
        self.bridge.node_latency(key)
    }

    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.bridge.descriptor(device)
    }

    /// Its instances' notifications (their editors' edits), with document ids.
    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        self.bridge.poll_plugins(out)
    }

    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        self.bridge.plugin_state(device)
    }

    fn set_plugin_state(
        &mut self,
        device: DeviceId,
        state: &Base64Bytes,
    ) -> Result<bool, BridgeError> {
        self.bridge.set_plugin_state(device, state)
    }

    fn plugin_catalog(&self) -> Option<ether_controller::PluginCatalogView> {
        self.bridge.plugin_catalog()
    }
}
