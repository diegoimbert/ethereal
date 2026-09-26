//! Built-in devices: basic-shape synth, sampler, compressor, delay. Minimal by design.
//!
//! Every device implements [`ether_core::Device`]; parameter ids and ranges are defined
//! by each device's [`DeviceDescriptor`] (the UI renders a generic param UI from it).
//! Owned by the `devices` node.

use std::sync::Arc;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
use ether_core::{AudioSource, Device};

pub mod compressor;
pub mod delay;
pub mod sampler;
pub mod synth;

/// Descriptor of a built-in device type (param list, category, I/O).
pub fn descriptor(device: BuiltinDeviceType) -> DeviceDescriptor {
    let _ = device;
    todo!("devices node")
}

/// All built-in descriptors (for `DeviceCommand::ListBuiltin`).
pub fn all_descriptors() -> Vec<DeviceDescriptor> {
    [
        BuiltinDeviceType::Synth,
        BuiltinDeviceType::Sampler,
        BuiltinDeviceType::Compressor,
        BuiltinDeviceType::Delay,
    ]
    .into_iter()
    .map(descriptor)
    .collect()
}

/// Resolves media for devices that need samples (sampler).
pub trait SampleResolver {
    fn resolve(&self, media: ether_core::protocol::model::MediaId) -> Option<Arc<dyn AudioSource>>;
}

/// Non-RT. Instantiate a built-in device with default params (the caller then applies the
/// document's param values with `Device::set_param` and calls `Node::prepare`).
pub fn create(device: &BuiltinDevice, samples: &dyn SampleResolver) -> Box<dyn Device> {
    let _ = (device, samples);
    todo!("devices node")
}
