//! External instrument / audio effect (v0.3, owned by the `external-instrument` node; device
//! `ether_devices::external`, engine `ether_core::hw_io`; CONTRACTS.md §13.7).
//!
//! - [`set_routing`]: `External::SetRouting` (document command from `doc::apply`;
//!   `DeviceChange::Kind` with the new routing; external devices only).
//! - [`EtherController::external_command`]: `External::{ListPorts, MeasureLatency}`
//!   (runtime, from `handlers.rs`; `EngineBridge::list_hardware_ports` and the measurement
//!   job, whose result sets the device's `LATENCY` param as one undoable edit).
//! - [`hw_io_descs`]: compile hook (`compile.rs`): every external device of a track →
//!   `TrackDesc::hw_io`.
//!
//! Until the node lands: commands reply `Unsupported` (tests/roadmap_v4.rs), nothing is
//! compiled and the devices are placeholders (the instrument is silent, the effect passes
//! through).

use ether_core::NodeKey;
use ether_core::hw_io::HwIoDesc;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::external::ExternalCommand;
use ether_core::protocol::model::{DeviceId, ExternalRouting, Project, TrackId};

use crate::doc::DocCtx;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// `External::SetRouting`.
pub(crate) fn set_routing(
    ctx: &mut DocCtx,
    device: DeviceId,
    routing: &ExternalRouting,
) -> CmdResult<()> {
    let _ = (ctx, device, routing);
    Err(unsupported(
        "external devices are not implemented yet (external-instrument)",
    ))
}

/// Compile hook: the hardware I/O of `track`'s external devices. Placeholder: none.
pub(crate) fn hw_io_descs(
    project: &Project,
    track: TrackId,
    nodes: &dyn Fn(DeviceId) -> Option<NodeKey>,
) -> Vec<HwIoDesc> {
    let _ = (project, track, nodes);
    Vec::new()
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
        let _ = (command, now, out);
        Err(unsupported(
            "external devices are not implemented yet (external-instrument)",
        ))
    }
}
