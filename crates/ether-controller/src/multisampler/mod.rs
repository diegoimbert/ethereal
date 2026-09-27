//! Multisampler zones (v0.2, owned by the `multisampler` node; model
//! `ether_model::multisampler`, CONTRACTS.md §12.4).
//!
//! - [`set_zones`]: `Device::SetZones` (document command from `doc/devices.rs`): one
//!   `DeviceChange::Kind` op, then an in-place node update (`EngineBridge::update_builtin`,
//!   falling back to re-creating the node like the sampler's slices). Placeholder:
//!   `Unsupported`.
//! - Zone media load like the sampler's sample: rebuild/update multisamplers when their
//!   media finishes loading (shared touch in `handlers.rs::tick_impl`, next to samplers).

use ether_core::protocol::model::{DeviceId, SampleZone};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

pub(crate) fn set_zones(ctx: &mut DocCtx, device: DeviceId, zones: &[SampleZone]) -> CmdResult<()> {
    let _ = (ctx, device, zones);
    Err(unsupported(
        "multisampler zones are not implemented yet (multisampler)",
    ))
}
