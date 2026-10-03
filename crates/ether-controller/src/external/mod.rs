//! External instrument / audio effect (v0.3, owned by the `external-instrument` node; device
//! `ether_devices::external`, engine `ether_core::hw_io`; CONTRACTS.md §13.7).
//!
//! - [`set_routing`]: `External::SetRouting` (document command from `doc::apply`;
//!   `DeviceChange::Kind` with the new routing; external devices only). The instrument
//!   leaves `audio_send` unset and the effect `midi_out` (`InvalidArgument` otherwise).
//! - [`EtherController::external_command`]: `External::{ListPorts, MeasureLatency}`
//!   (runtime, from `handlers.rs`). `ListPorts` asks `EngineBridge::list_hardware_ports`;
//!   `MeasureLatency` starts `EngineBridge::measure_hw_latency` on the device's node and
//!   replies `Unit`; [`EtherController::external_tick`] then polls the result: the device's
//!   `LATENCY` param is set as one undoable edit and `ExternalEvent::LatencyMeasured` sent,
//!   or `MeasureFailed` when nothing came back (the engine gives up after 2 s; the
//!   controller after [`MEASURE_DEADLINE_MS`] of wall time, e.g. with the audio stopped).
//!   Hosts without hardware I/O (web) reply `Unsupported` (the bridge defaults).
//! - [`hw_io_descs`]: compile hook (`compile.rs`): every external device on a track's own
//!   chain → `TrackDesc::hw_io` (devices inside racks or on drum pads are not routed to
//!   hardware in v0.3: the instrument stays silent there, the effect plays its dry signal).
//! - While a project has external devices, the ports are re-listed every
//!   [`PORTS_POLL_MS`]; a change emits `ExternalEvent::PortsChanged` (the UI marks missing
//!   ports; the engine and the host resolve ports by id, so devices resume by themselves).

use ether_core::NodeKey;
use ether_core::hw_io::HwIoDesc;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::external::{ExternalCommand, ExternalEvent, HardwarePorts};
use ether_core::protocol::message::Event;
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, Device, DeviceChange, DeviceId, DeviceKind, ExternalRouting,
    ParamId, Project, TrackId,
};
use ether_devices::external::{external_audio_effect, external_instrument};

use crate::doc::DocCtx;
use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink, engine::bridge_err};

/// Wall-clock deadline of a measurement (the engine's own timeout is 2 s of audio).
pub const MEASURE_DEADLINE_MS: u64 = 3_000;
/// How often the hardware ports are re-listed while external devices exist.
pub const PORTS_POLL_MS: u64 = 2_000;

/// Runtime state (site-local, never saved).
#[derive(Debug, Default)]
pub(crate) struct ExternalState {
    /// Running measurements: device, its node, start (wall ms).
    pending: Vec<(DeviceId, NodeKey, u64)>,
    /// The last listed ports (`None` until listed, or on hosts without hardware I/O).
    ports: Option<HardwarePorts>,
    last_poll: Option<u64>,
}

/// Whether `d` is an external device; its routing and type.
fn external_of(d: &Device) -> Option<(BuiltinDeviceType, &ExternalRouting)> {
    match &d.kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::ExternalInstrument { routing },
        } => Some((BuiltinDeviceType::ExternalInstrument, routing)),
        DeviceKind::Builtin {
            device: BuiltinDevice::ExternalAudioEffect { routing },
        } => Some((BuiltinDeviceType::ExternalAudioEffect, routing)),
        _ => None,
    }
}

/// The `Latency` param of an external device type.
fn latency_param(ty: BuiltinDeviceType) -> ParamId {
    match ty {
        BuiltinDeviceType::ExternalInstrument => external_instrument::LATENCY,
        _ => external_audio_effect::LATENCY,
    }
}

/// `External::SetRouting`.
pub(crate) fn set_routing(
    ctx: &mut DocCtx,
    device: DeviceId,
    routing: &ExternalRouting,
) -> CmdResult<()> {
    let d = ctx.device(device)?;
    let Some((ty, _)) = external_of(&d) else {
        return Err(invalid(format!(
            "device {} is not an external device",
            d.id
        )));
    };
    ether_core::protocol::model::check_routing(routing).map_err(invalid)?;
    let kind = match ty {
        BuiltinDeviceType::ExternalInstrument => {
            if routing.audio_send.is_some() {
                return Err(invalid("an External Instrument has no audio send"));
            }
            BuiltinDevice::ExternalInstrument {
                routing: routing.clone(),
            }
        }
        _ => {
            if routing.midi_out.is_some() {
                return Err(invalid("an External Audio Effect has no MIDI output"));
            }
            BuiltinDevice::ExternalAudioEffect {
                routing: routing.clone(),
            }
        }
    };
    ctx.set_device(
        d.id,
        DeviceChange::Kind(DeviceKind::Builtin { device: kind }),
    )
}

/// Compile hook: the hardware I/O of the external devices on `track`'s own chain, in chain
/// order (devices without a live node are skipped).
pub(crate) fn hw_io_descs(
    project: &Project,
    track: TrackId,
    nodes: &dyn Fn(DeviceId) -> Option<NodeKey>,
) -> Vec<HwIoDesc> {
    let mut devices: Vec<&Device> = project
        .devices
        .values()
        .filter(|d| d.track == track && d.pad.is_none() && d.chain.is_none())
        .filter(|d| external_of(d).is_some())
        .collect();
    devices.sort_by(|a, b| a.order.cmp(&b.order));
    devices
        .into_iter()
        .filter_map(|d| {
            let (_, routing) = external_of(d)?;
            Some(HwIoDesc {
                node: nodes(d.id)?,
                routing: routing.clone(),
            })
        })
        .collect()
}

/// Whether a live node built from `old` takes `new` in place (`engine.rs`, next to the
/// sampler slices): only the routing of an external device changed. The node holds no
/// routing (it is compiled into `TrackDesc::hw_io`), so the bridge accepts it as a no-op
/// and the node keeps its state (no dropout on a routing edit).
pub(crate) fn updatable_in_place(old: &BuiltinDevice, new: &BuiltinDevice) -> bool {
    matches!(
        (old, new),
        (
            BuiltinDevice::ExternalInstrument { .. },
            BuiltinDevice::ExternalInstrument { .. }
        ) | (
            BuiltinDevice::ExternalAudioEffect { .. },
            BuiltinDevice::ExternalAudioEffect { .. }
        )
    )
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Runtime `External` commands (`SetRouting` goes through `doc::apply`).
    pub(crate) fn external_command(
        &mut self,
        command: &ExternalCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match command {
            ExternalCommand::ListPorts => {
                let ports = self.bridge.list_hardware_ports().map_err(bridge_err)?;
                self.external.ports = Some(ports.clone());
                self.external.last_poll = Some(now);
                Ok(ReplyValue::HardwarePorts { ports })
            }
            ExternalCommand::MeasureLatency { device } => {
                let project = self
                    .doc
                    .as_ref()
                    .map(|d| &d.project)
                    .ok_or_else(crate::handlers::no_project)?;
                let d = project
                    .devices
                    .get(device)
                    .ok_or_else(|| not_found(format!("device {device}")))?;
                let Some((ty, routing)) = external_of(d) else {
                    return Err(invalid(format!(
                        "device {device} is not an external device"
                    )));
                };
                // Hosts without hardware I/O (web) reply `Unsupported` whatever the routing.
                if let Err(e @ crate::BridgeError::Unsupported(_)) =
                    self.bridge.list_hardware_ports()
                {
                    return Err(bridge_err(e));
                }
                let out_ok = match ty {
                    BuiltinDeviceType::ExternalInstrument => routing.midi_out.is_some(),
                    _ => routing.audio_send.is_some(),
                };
                if !out_ok || routing.audio_return.is_none() {
                    return Err(invalid_state(
                        "choose the hardware output and return before measuring",
                    ));
                }
                if self.external.pending.iter().any(|p| p.0 == *device) {
                    return Err(invalid_state("already measuring this device"));
                }
                // A routing edit just before may have re-created the node: publish now.
                self.publish_if_due(now, true, out);
                let node = self
                    .engine
                    .node(*device)
                    .ok_or_else(|| invalid_state("the device is not running"))?;
                self.bridge.measure_hw_latency(node).map_err(bridge_err)?;
                self.external.pending.push((*device, node, now));
                Ok(ReplyValue::Unit)
            }
            ExternalCommand::SetRouting { .. } => Err(invalid("SetRouting is a document command")),
        }
    }

    /// Called every tick (`lib.rs`): measurement results and port changes.
    pub(crate) fn external_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        while let Some(r) = self.bridge.poll_hw_latency() {
            let Some(i) = self.external.pending.iter().position(|p| p.1 == r.node) else {
                continue;
            };
            let (device, _, _) = self.external.pending.remove(i);
            self.measured(device, r.samples, now, out);
        }
        let mut expired = Vec::new();
        self.external.pending.retain(|&(device, _, at)| {
            let keep = now.saturating_sub(at) < MEASURE_DEADLINE_MS;
            if !keep {
                expired.push(device);
            }
            keep
        });
        for device in expired {
            self.measured(device, None, now, out);
        }
        self.poll_ports(now, out);
    }

    fn measured(
        &mut self,
        device: DeviceId,
        samples: Option<u32>,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let fail = |out: &mut dyn MessageSink, message: &str| {
            event(
                out,
                Event::External {
                    event: ExternalEvent::MeasureFailed {
                        device,
                        message: message.into(),
                    },
                },
            );
        };
        let Some(ty) = self
            .doc
            .as_ref()
            .map(|d| &d.project)
            .and_then(|p| p.devices.get(&device))
            .and_then(|d| external_of(d).map(|(ty, _)| ty))
        else {
            return;
        };
        let Some(samples) = samples else {
            fail(
                out,
                "nothing came back within 2 s: check the cables, the routing and the levels",
            );
            return;
        };
        let sr = self.config().engine_sample_rate as f32;
        let ms = f64::from(samples) * 1000.0 / f64::from(sr.max(1.0));
        if ms > ether_devices::external::MAX_LATENCY_MS {
            fail(out, "the round trip is above 500 ms");
            return;
        }
        let param = latency_param(ty);
        let r = self.edit_with("Measure Latency", None, now, out, |ctx| {
            ctx.set_device(
                device,
                DeviceChange::Param {
                    param,
                    value: Some(ms),
                },
            )
        });
        match r {
            Ok(()) => event(
                out,
                Event::External {
                    event: ExternalEvent::LatencyMeasured {
                        device,
                        latency_ms: ms,
                    },
                },
            ),
            Err(e) => fail(out, &e.message),
        }
    }

    /// Re-list the ports every [`PORTS_POLL_MS`] while the project has external devices.
    fn poll_ports(&mut self, now: u64, out: &mut dyn MessageSink) {
        let any = self
            .doc
            .as_ref()
            .map(|d| &d.project)
            .is_some_and(|p| p.devices.values().any(|d| external_of(d).is_some()));
        if !any {
            return;
        }
        if self
            .external
            .last_poll
            .is_some_and(|t| now.saturating_sub(t) < PORTS_POLL_MS)
        {
            return;
        }
        self.external.last_poll = Some(now);
        let Ok(ports) = self.bridge.list_hardware_ports() else {
            return;
        };
        if self.external.ports.as_ref() != Some(&ports) {
            let first = self.external.ports.is_none();
            self.external.ports = Some(ports.clone());
            if !first {
                event(
                    out,
                    Event::External {
                        event: ExternalEvent::PortsChanged { ports },
                    },
                );
            }
        }
    }
}
