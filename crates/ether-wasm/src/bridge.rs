//! Controller side (Worker): [`WebBridge`] implements `EngineBridge` by serializing every
//! call into the control ring for the AudioWorklet, and reads [`EngineReport`]s back.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use ether_controller::{BridgeError, EngineBridge};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, BuiltinDeviceType, DeviceId, MediaId, MediaRef, ParamId,
    PluginInstance,
};
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;

use crate::proto::{EngineMsg, EngineReport, REPORT_ERROR, REPORT_STATE};
use crate::ring::{RingMemory, RingReader, RingWriter};

/// Bytes of reports drained per poll (reports are small; this only bounds a backlog).
const REPORT_BUDGET: usize = 256 * 1024;

/// Shared between the bridge (owned by the controller) and the host wrapper, which flushes
/// the ring after every call and surfaces engine errors as notifications.
pub struct BridgeShared<M: RingMemory> {
    pub control: RingWriter<M>,
    pub reports: RingReader<M>,
    /// Errors reported by the Worklet (compile errors, unknown nodes, ...).
    pub errors: Vec<String>,
    /// Blocks rendered by the Worklet as of the last report (0 = not running yet).
    pub blocks: u64,
}

impl<M: RingMemory> BridgeShared<M> {
    /// Drain reports into `out` (latest playhead, max-held meters, summed diagnostics).
    pub fn poll_reports(&mut self, out: &mut EngineOutputs) {
        let Self {
            reports,
            errors,
            blocks,
            ..
        } = self;
        reports.drain(REPORT_BUDGET, |bytes| {
            match bytes.first() {
                Some(&REPORT_STATE) => match EngineReport::decode(bytes) {
                    Ok(r) => {
                        out.playhead = Some(r.playhead);
                        for m in r.meters {
                            match out.meters.iter_mut().find(|e| e.track == m.track) {
                                Some(e) => {
                                    for ch in 0..2 {
                                        e.peak[ch] = e.peak[ch].max(m.peak[ch]);
                                        e.rms[ch] = e.rms[ch].max(m.rms[ch]);
                                    }
                                    e.clipped |= m.clipped;
                                }
                                None => out.meters.push(m),
                            }
                        }
                        out.event_overflow |= r.event_overflow;
                        out.underruns += r.underruns;
                        *blocks = r.blocks;
                    }
                    Err(e) => errors.push(format!("bad engine report: {e}")),
                },
                Some(&REPORT_ERROR) => {
                    errors.push(String::from_utf8_lossy(&bytes[1..]).into_owned())
                }
                _ => errors.push("unknown engine report".into()),
            }
            true
        });
        let skipped = reports.take_skipped();
        if skipped > 0 {
            errors.push(format!("report ring corrupt: skipped {skipped} bytes"));
        }
    }
}

pub type Shared<M> = Rc<RefCell<BridgeShared<M>>>;

pub fn shared<M: RingMemory>(control: M, reports: M) -> Shared<M> {
    Rc::new(RefCell::new(BridgeShared {
        control: RingWriter::new(control),
        reports: RingReader::new(reports),
        errors: Vec::new(),
        blocks: 0,
    }))
}

/// `EngineBridge` over the SharedArrayBuffer rings. Node keys are virtual (see
/// [`EngineMsg`]); the Worklet maps them. No plugins on the web.
pub struct WebBridge<M: RingMemory> {
    shared: Shared<M>,
    next_node: u32,
    devices: BTreeMap<DeviceId, (NodeKey, BuiltinDeviceType)>,
}

impl<M: RingMemory> WebBridge<M> {
    pub fn new(shared: Shared<M>) -> Self {
        Self {
            shared,
            next_node: 0,
            devices: BTreeMap::new(),
        }
    }

    fn send(&mut self, msg: EngineMsg) {
        let mut shared = self.shared.borrow_mut();
        for frame in msg.encode() {
            shared.control.send(&frame);
        }
    }
}

impl<M: RingMemory> EngineBridge for WebBridge<M> {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.next_node = self.next_node.wrapping_add(1);
        let key = NodeKey {
            index: self.next_node,
            generation: 1,
        };
        self.devices.insert(device, (key, kind.device_type()));
        self.send(EngineMsg::CreateBuiltin {
            key,
            device: kind.clone(),
            params: params.to_vec(),
        });
        Ok(key)
    }

    fn create_plugin(
        &mut self,
        _device: DeviceId,
        _plugin: &PluginInstance,
        _state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        Err(BridgeError::Unsupported(
            "plugins are not available in the browser".into(),
        ))
    }

    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.devices.retain(|_, (k, _)| *k != key);
        self.send(EngineMsg::DestroyNode { key });
        Ok(())
    }

    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.send(EngineMsg::LoadMedia {
            media: media.id,
            audio,
        });
        Ok(())
    }

    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.send(EngineMsg::UnloadMedia { media });
        Ok(())
    }

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.send(EngineMsg::Publish {
            graph: Box::new(graph),
        });
        Ok(())
    }

    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.send(EngineMsg::SetParam { change });
        Ok(())
    }

    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.send(EngineMsg::Transport { control });
        Ok(())
    }

    fn poll(&mut self, out: &mut EngineOutputs) {
        out.clear();
        let mut shared = self.shared.borrow_mut();
        shared.control.flush();
        shared.poll_reports(out);
    }

    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        let (_, kind) = self.devices.get(&device)?;
        Some(ether_devices::descriptor(*kind))
    }
}
