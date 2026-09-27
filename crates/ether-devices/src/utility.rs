//! Built-in `Utility` device (roadmap v2, owned by the `devices-2` node; see `docs/ROADMAP.md`).
//!
//! Gain, pan, stereo width, phase invert (L/R), mono.
//!
//! Until implemented it is a [`crate::placeholder::Placeholder`] with no params.
//! Implement `descriptor()` and `create()` here; `lib.rs` already dispatches to them.

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor};
use ether_core::protocol::model::BuiltinDeviceType;

/// Descriptor of the `Utility` type.
pub fn descriptor() -> DeviceDescriptor {
    crate::placeholder::descriptor(BuiltinDeviceType::Utility, "Utility", DeviceCategory::AudioEffect)
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(crate::placeholder::Placeholder::new(descriptor()))
}
