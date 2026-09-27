//! Built-in `DrumRack` device (roadmap v2, owned by the `drum-rack` node; see `docs/ROADMAP.md`).
//!
//! The drum rack's own node in the track chain. Pad chains are separate nodes listed in `TrackDesc::racks`; the engine runs them where this node sits in the chain (CONTRACTS.md §11.12). This node applies the rack's own params (e.g. rack volume) to the summed pads.
//!
//! Until implemented it is a [`crate::placeholder::Placeholder`] with no params.
//! Implement `descriptor()` and `create()` here; `lib.rs` already dispatches to them.

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor};
use ether_core::protocol::model::BuiltinDeviceType;

/// Descriptor of the `DrumRack` type.
pub fn descriptor() -> DeviceDescriptor {
    crate::placeholder::descriptor(BuiltinDeviceType::DrumRack, "Drum Rack", DeviceCategory::Instrument)
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(crate::placeholder::Placeholder::new(descriptor()))
}
