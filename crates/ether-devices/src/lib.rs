//! Built-in devices: basic-shape synth, sampler, compressor, delay; roadmap v2 adds EQ,
//! reverb, limiter, utility (`devices-2`) and the drum rack (`drum-rack`), one module each.
//! v0.2 (contracts-3) adds one module per device group, each owned by its node:
//! [`poly_synth`] (`synth-2`), [`multisampler`], [`fx_color`], [`fx_modulation`],
//! [`fx_dynamics`], [`fx_analysis`], [`midi_fx`], [`racks`] + [`modulators`]
//! (`racks-modulation`); shared scaffolding in [`contract`]; factory presets through
//! [`factory_presets`] (v0.1 types: [`factory`], `presets`). v0.3 (contracts-4) adds
//! [`fx_space`] (`fx-space`: convolution reverb) and [`external`] (`external-instrument`).
//!
//! Every device implements [`ether_core::Device`]; parameter ids and ranges are defined
//! by each device's [`DeviceDescriptor`] (the UI renders a generic param UI from it).
//! Owned by the `devices` node.

use std::sync::Arc;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
use ether_core::{AudioSource, Device};

pub mod compressor;
pub mod contract;
pub mod delay;
pub mod drum_rack;
mod dsp;
pub mod eq;
pub mod external;
pub mod factory;
pub mod fx_analysis;
pub mod fx_color;
pub mod fx_dynamics;
pub mod fx_modulation;
pub mod fx_space;
pub mod limiter;
pub mod midi_fx;
pub mod modulators;
pub mod multisampler;
pub mod poly_synth;
pub mod racks;
pub mod reverb;
pub mod sampler;
pub mod synth;
mod util;
pub mod utility;

pub use compressor::Compressor;
pub use delay::Delay;
pub use eq::Eq;
pub use limiter::Limiter;
pub use reverb::Reverb;
pub use sampler::Sampler;
pub use synth::Synth;
pub use utility::Utility;

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
        // --- v0.2 groups ---
        BuiltinDeviceType::PolySynth => poly_synth::descriptor(device),
        BuiltinDeviceType::MultiSampler => multisampler::descriptor(device),
        BuiltinDeviceType::Saturator
        | BuiltinDeviceType::Bitcrusher
        | BuiltinDeviceType::AutoFilter => fx_color::descriptor(device),
        BuiltinDeviceType::Chorus
        | BuiltinDeviceType::Phaser
        | BuiltinDeviceType::Flanger
        | BuiltinDeviceType::Tremolo => fx_modulation::descriptor(device),
        BuiltinDeviceType::Gate
        | BuiltinDeviceType::MultibandCompressor
        | BuiltinDeviceType::TransientShaper => fx_dynamics::descriptor(device),
        BuiltinDeviceType::SpectrumAnalyzer | BuiltinDeviceType::Tuner => {
            fx_analysis::descriptor(device)
        }
        BuiltinDeviceType::Arpeggiator
        | BuiltinDeviceType::Chord
        | BuiltinDeviceType::ScaleQuantize
        | BuiltinDeviceType::NoteLength
        | BuiltinDeviceType::Velocity
        | BuiltinDeviceType::Randomizer => midi_fx::descriptor(device),
        BuiltinDeviceType::InstrumentRack
        | BuiltinDeviceType::AudioEffectRack
        | BuiltinDeviceType::MidiEffectRack => racks::descriptor(device),
        // --- v0.3 groups ---
        BuiltinDeviceType::ConvolutionReverb => fx_space::descriptor(device),
        BuiltinDeviceType::ExternalInstrument | BuiltinDeviceType::ExternalAudioEffect => {
            external::descriptor(device)
        }
    }
}

/// Embedded factory presets of a built-in type (v0.2, `presets` and the device nodes; see
/// [`contract::FactoryPreset`]). Ids are `"<device-key>/<slug>"`.
pub fn factory_presets(device: BuiltinDeviceType) -> &'static [contract::FactoryPreset] {
    match device {
        BuiltinDeviceType::Synth
        | BuiltinDeviceType::Sampler
        | BuiltinDeviceType::Compressor
        | BuiltinDeviceType::Delay
        | BuiltinDeviceType::Eq
        | BuiltinDeviceType::Reverb
        | BuiltinDeviceType::Limiter
        | BuiltinDeviceType::Utility
        | BuiltinDeviceType::DrumRack => factory::factory_presets(device),
        BuiltinDeviceType::PolySynth => poly_synth::factory_presets(device),
        BuiltinDeviceType::MultiSampler => multisampler::factory_presets(device),
        BuiltinDeviceType::Saturator
        | BuiltinDeviceType::Bitcrusher
        | BuiltinDeviceType::AutoFilter => fx_color::factory_presets(device),
        BuiltinDeviceType::Chorus
        | BuiltinDeviceType::Phaser
        | BuiltinDeviceType::Flanger
        | BuiltinDeviceType::Tremolo => fx_modulation::factory_presets(device),
        BuiltinDeviceType::Gate
        | BuiltinDeviceType::MultibandCompressor
        | BuiltinDeviceType::TransientShaper => fx_dynamics::factory_presets(device),
        BuiltinDeviceType::SpectrumAnalyzer | BuiltinDeviceType::Tuner => {
            fx_analysis::factory_presets(device)
        }
        BuiltinDeviceType::Arpeggiator
        | BuiltinDeviceType::Chord
        | BuiltinDeviceType::ScaleQuantize
        | BuiltinDeviceType::NoteLength
        | BuiltinDeviceType::Velocity
        | BuiltinDeviceType::Randomizer => midi_fx::factory_presets(device),
        BuiltinDeviceType::InstrumentRack
        | BuiltinDeviceType::AudioEffectRack
        | BuiltinDeviceType::MidiEffectRack => racks::factory_presets(device),
        BuiltinDeviceType::ConvolutionReverb => fx_space::factory_presets(device),
        BuiltinDeviceType::ExternalInstrument | BuiltinDeviceType::ExternalAudioEffect => {
            external::factory_presets(device)
        }
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
        BuiltinDevice::Sampler { sample, slices } => Box::new(Sampler::with_slices(
            sample.and_then(|m| samples.resolve(m)),
            slices.clone(),
        )),
        BuiltinDevice::Compressor => Box::new(Compressor::new()),
        BuiltinDevice::Delay => Box::new(Delay::new()),
        BuiltinDevice::Eq => eq::create(),
        BuiltinDevice::Reverb => reverb::create(),
        BuiltinDevice::Limiter => limiter::create(),
        BuiltinDevice::Utility => utility::create(),
        BuiltinDevice::DrumRack => drum_rack::create(),
        // --- v0.2 groups (placeholders until their node lands) ---
        BuiltinDevice::PolySynth => poly_synth::create(device),
        BuiltinDevice::MultiSampler { .. } => multisampler::create(device, samples),
        BuiltinDevice::Saturator | BuiltinDevice::Bitcrusher | BuiltinDevice::AutoFilter => {
            fx_color::create(device)
        }
        BuiltinDevice::Chorus
        | BuiltinDevice::Phaser
        | BuiltinDevice::Flanger
        | BuiltinDevice::Tremolo => fx_modulation::create(device),
        BuiltinDevice::Gate
        | BuiltinDevice::MultibandCompressor
        | BuiltinDevice::TransientShaper => fx_dynamics::create(device),
        BuiltinDevice::SpectrumAnalyzer | BuiltinDevice::Tuner => fx_analysis::create(device),
        BuiltinDevice::Arpeggiator
        | BuiltinDevice::Chord
        | BuiltinDevice::ScaleQuantize
        | BuiltinDevice::NoteLength
        | BuiltinDevice::Velocity
        | BuiltinDevice::Randomizer => midi_fx::create(device),
        BuiltinDevice::InstrumentRack
        | BuiltinDevice::AudioEffectRack
        | BuiltinDevice::MidiEffectRack => racks::create(device),
        // --- v0.3 groups (placeholders until their node lands) ---
        BuiltinDevice::ConvolutionReverb { .. } => fx_space::create(device, samples),
        BuiltinDevice::ExternalInstrument { .. } | BuiltinDevice::ExternalAudioEffect { .. } => {
            external::create(device)
        }
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
