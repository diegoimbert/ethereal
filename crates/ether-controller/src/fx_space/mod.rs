//! Convolution reverb IRs (v0.3, owned by the `fx-space` node; device
//! `ether_devices::fx_space`, model `ether_model::IrSource`; CONTRACTS.md §13.6).
//!
//! - [`set_ir`]: `Device::SetIr` (document command from `doc/devices.rs`): validates the
//!   media (exists, audio) and replaces the device kind (`DeviceChange::Kind`); the live node
//!   swaps its IR through `EngineBridge::update_builtin` (fallback: re-create).
//! - [`list_factory_irs`]: `Device::ListFactoryIrs` (from `handlers.rs`).
//! - IR media are ordinary media: `BuiltinDevice::media()` lists them, so they load, resolve
//!   (references in place), go missing and relink like samples. Rebuilding a reverb when its
//!   IR media finishes loading is a shared touch in `handlers.rs` (next to samplers).
//!
//! Until the node lands: both reply `Unsupported` (tests/roadmap_v4.rs).

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::{DeviceId, IrSource};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

/// `Device::SetIr`.
pub(crate) fn set_ir(ctx: &mut DocCtx, device: DeviceId, ir: Option<&IrSource>) -> CmdResult<()> {
    let _ = (ctx, device, ir);
    Err(unsupported(
        "the convolution reverb is not implemented yet (fx-space)",
    ))
}

/// `Device::ListFactoryIrs`.
pub(crate) fn list_factory_irs() -> CmdResult<ReplyValue> {
    Err(unsupported(
        "the convolution reverb is not implemented yet (fx-space)",
    ))
}
