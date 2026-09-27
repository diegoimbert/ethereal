//! Built-in `Reverb` device (roadmap v2, owned by the `devices-2` node; see `docs/ROADMAP.md`).
//!
//! Algorithmic stereo reverb (size, decay, damping, pre-delay, width, dry/wet).
//!
//! Until implemented it is a [`crate::placeholder::Placeholder`] with no params.
//! Implement `descriptor()` and `create()` here; `lib.rs` already dispatches to them.

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor};
use ether_core::protocol::model::BuiltinDeviceType;

/// Descriptor of the `Reverb` type.
pub fn descriptor() -> DeviceDescriptor {
    crate::placeholder::descriptor(BuiltinDeviceType::Reverb, "Reverb", DeviceCategory::AudioEffect)
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(crate::placeholder::Placeholder::new(descriptor()))
}
