//! Convolution reverb IRs (v0.3, owned by the `fx-space` node; device
//! `ether_devices::fx_space`, model `ether_model::IrSource`; CONTRACTS.md §13.6).
//!
//! - [`set_ir`]: `Device::SetIr` (document command from `doc/devices.rs`): validates the
//!   source (a known factory id, or project media) and replaces the device kind
//!   (`DeviceChange::Kind`, one undo step); the live node swaps its IR through
//!   `EngineBridge::update_builtin` → `Node::set_data` with a crossfade
//!   ([`updatable_in_place`]; hosts without it re-create the node).
//! - [`list_factory_irs`]: `Device::ListFactoryIrs` (from `handlers.rs`).
//! - IR media are ordinary media: `BuiltinDevice::media()` lists them, so they load, resolve
//!   (references in place), go missing and relink like samples. A reverb is rebuilt when its
//!   IR media finishes loading ([`uses_media`], `handlers.rs`, next to samplers); while the
//!   media is missing the reverb passes the dry signal through.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::{
    BuiltinDevice, DeviceChange, DeviceId, DeviceKind, IrSource, MediaId,
};
use ether_devices::fx_space::FACTORY_IRS;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid, not_found};

/// `Device::SetIr`.
pub(crate) fn set_ir(ctx: &mut DocCtx, device: DeviceId, ir: Option<&IrSource>) -> CmdResult<()> {
    let d = ctx.device(device)?;
    if !matches!(
        d.kind,
        DeviceKind::Builtin {
            device: BuiltinDevice::ConvolutionReverb { .. }
        }
    ) {
        return Err(invalid(format!(
            "device {} is not a convolution reverb",
            d.id
        )));
    }
    match ir {
        Some(IrSource::Factory { id }) if !FACTORY_IRS.iter().any(|s| s.id == id) => {
            return Err(not_found(format!("factory impulse response {id:?}")));
        }
        Some(IrSource::Media { media }) if !ctx.p().media.contains_key(media) => {
            return Err(not_found(format!("media {media}")));
        }
        _ => {}
    }
    ctx.set_device(
        d.id,
        DeviceChange::Kind(DeviceKind::Builtin {
            device: BuiltinDevice::ConvolutionReverb { ir: ir.cloned() },
        }),
    )
}

/// `Device::ListFactoryIrs`.
pub(crate) fn list_factory_irs() -> CmdResult<ReplyValue> {
    Ok(ReplyValue::FactoryIrs {
        irs: ether_devices::fx_space::factory_irs(),
    })
}

/// Whether a live node built from `old` takes `new` in place (`engine.rs`, next to the
/// sampler slices): only the IR of a convolution reverb changed.
pub(crate) fn updatable_in_place(old: &BuiltinDevice, new: &BuiltinDevice) -> bool {
    ether_devices::fx_space::updatable_in_place(old, new)
}

/// Whether `kind` is a convolution reverb whose IR is one of `media` (rebuilt on load).
pub(crate) fn uses_media(kind: &DeviceKind, media: &[MediaId]) -> bool {
    matches!(kind, DeviceKind::Builtin {
        device: BuiltinDevice::ConvolutionReverb { ir: Some(IrSource::Media { media: m }) }
    } if media.contains(m))
}
