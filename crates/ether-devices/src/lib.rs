//! Built-in devices: basic-shape synth, sampler, compressor, delay; roadmap v2 adds EQ,
//! reverb, limiter, utility (`devices-2`) and the drum rack (`drum-rack`), one module each.
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
pub mod drum_rack;
pub mod eq;
pub mod limiter;
mod placeholder;
pub mod reverb;
pub mod sampler;
pub mod synth;
pub mod utility;
mod util;

pub use compressor::Compressor;
pub use delay::Delay;
pub use sampler::Sampler;
pub use synth::Synth;

/// Descriptor of a built-in device type (param list, category, I/O). Every instance of a
/// type reports exactly this descriptor (`Device::descriptor` delegates here), so callers
/// may cache it per type.
pub fn descriptor(device: BuiltinDeviceType) -> DeviceDescriptor {
    match device {
        BuiltinDeviceType::Synth => synth::descriptor(),
        BuiltinDeviceType::Sampler => sampler::descriptor(),
        BuiltinDeviceType::Compressor => compressor::descriptor(),
        BuiltinDeviceType::Delay => delay::descriptor(),
        BuiltinDeviceType::Eq => eq::descriptor(),
        BuiltinDeviceType::Reverb => reverb::descriptor(),
        BuiltinDeviceType::Limiter => limiter::descriptor(),
        BuiltinDeviceType::Utility => utility::descriptor(),
        BuiltinDeviceType::DrumRack => drum_rack::descriptor(),
    }
}

/// All built-in descriptors (for `DeviceCommand::ListBuiltin`).
pub fn all_descriptors() -> Vec<DeviceDescriptor> {
    BuiltinDeviceType::ALL.into_iter().map(descriptor).collect()
}

/// Resolves media for devices that need samples (sampler).
pub trait SampleResolver {
    fn resolve(&self, media: ether_core::protocol::model::MediaId) -> Option<Arc<dyn AudioSource>>;
}

/// Non-RT. Instantiate a built-in device with default params (the caller then applies the
/// document's param values with `Device::set_param` and calls `Node::prepare`).
///
/// A sampler whose media is missing or unresolved is created silent.
pub fn create(device: &BuiltinDevice, samples: &dyn SampleResolver) -> Box<dyn Device> {
    match device {
        BuiltinDevice::Synth => Box::new(Synth::new()),
        // `slices` (roadmap v2) are applied by the `drum-rack` node.
        BuiltinDevice::Sampler { sample, slices: _ } => {
            Box::new(Sampler::new(sample.and_then(|m| samples.resolve(m))))
        }
        BuiltinDevice::Compressor => Box::new(Compressor::new()),
        BuiltinDevice::Delay => Box::new(Delay::new()),
        BuiltinDevice::Eq => eq::create(),
        BuiltinDevice::Reverb => reverb::create(),
        BuiltinDevice::Limiter => limiter::create(),
        BuiltinDevice::Utility => utility::create(),
        BuiltinDevice::DrumRack => drum_rack::create(),
    }
}

/// A [`SampleResolver`] that resolves nothing (for devices without samples, tests).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSamples;

impl SampleResolver for NoSamples {
    fn resolve(
        &self,
        _media: ether_core::protocol::model::MediaId,
    ) -> Option<Arc<dyn AudioSource>> {
        None
    }
}
