//! The built-in devices, one module per device or group: the basic synth, sampler,
//! compressor, delay, EQ, reverb, limiter, utility and drum rack; [`poly_synth`],
//! [`fm_synth`] (four-operator FM), [`multisampler`]; the effect groups [`fx_color`],
//! [`fx_modulation`], [`fx_dynamics`], [`fx_analysis`], [`fx_space`] (convolution reverb),
//! [`grain_delay`], [`shifter`], [`maximizer`] and [`midi_fx`]; [`racks`] and [`modulators`]; [`external`] (external instruments and
//! effects). Shared scaffolding is in [`contract`]; factory presets come from
//! [`factory_presets`] ([`factory`] holds the older types).
//!
//! Every device implements [`ether_core::Device`]; parameter ids and ranges are defined
//! by each device's [`DeviceDescriptor`] (the UI renders a generic param UI from it).

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
pub mod fm_synth;
pub mod fx_analysis;
pub mod fx_color;
pub mod fx_dynamics;
pub mod fx_modulation;
pub mod fx_space;
pub mod grain_delay;
pub mod limiter;
pub mod maximizer;
pub mod midi_fx;
pub mod modulators;
pub mod multisampler;
pub mod poly_synth;
pub mod racks;
pub mod reverb;
pub mod sampler;
pub mod shifter;
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

/// Built-in types whose node is still a contract [`contract::Placeholder`] (silent
/// instrument, pass-through effect, MIDI-thru): a contract wave lists each type it declares
/// before its device node lands, the device node removes it. The single source of
/// [`DeviceDescriptor::implemented`]: the device browser, insert menus, command palette and
/// the agent's (and MCP's) device lists hide these types; documents holding one still load.
/// Empty: every built-in has a working node.
pub const UNIMPLEMENTED: &[BuiltinDeviceType] = &[];

/// Whether `device` has a working node (not a contract placeholder): see [`UNIMPLEMENTED`].
pub fn implemented(device: BuiltinDeviceType) -> bool {
    !UNIMPLEMENTED.contains(&device)
}

/// Descriptor of a built-in device type (param list, category, I/O, [`implemented`]). Every
/// instance of a type reports exactly this descriptor (`Device::descriptor` delegates here),
/// so callers may cache it per type.
pub fn descriptor(device: BuiltinDeviceType) -> DeviceDescriptor {
    DeviceDescriptor {
        implemented: implemented(device),
        ..type_descriptor(device)
    }
}

fn type_descriptor(device: BuiltinDeviceType) -> DeviceDescriptor {
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
        | BuiltinDeviceType::AutoFilter
        | BuiltinDeviceType::Erosion
        | BuiltinDeviceType::Pedal => fx_color::descriptor(device),
        BuiltinDeviceType::Chorus
        | BuiltinDeviceType::Phaser
        | BuiltinDeviceType::Flanger
        | BuiltinDeviceType::Tremolo
        | BuiltinDeviceType::Vibrato
        | BuiltinDeviceType::VolumeLfo => fx_modulation::descriptor(device),
        BuiltinDeviceType::Gate
        | BuiltinDeviceType::MultibandCompressor
        | BuiltinDeviceType::TransientShaper => fx_dynamics::descriptor(device),
        BuiltinDeviceType::SpectrumAnalyzer
        | BuiltinDeviceType::Tuner
        | BuiltinDeviceType::Loudness => fx_analysis::descriptor(device),
        BuiltinDeviceType::Arpeggiator
        | BuiltinDeviceType::Chord
        | BuiltinDeviceType::ScaleQuantize
        | BuiltinDeviceType::NoteLength
        | BuiltinDeviceType::Velocity
        | BuiltinDeviceType::Randomizer
        | BuiltinDeviceType::Pitch
        | BuiltinDeviceType::NoteBlocker => midi_fx::descriptor(device),
        BuiltinDeviceType::InstrumentRack
        | BuiltinDeviceType::AudioEffectRack
        | BuiltinDeviceType::MidiEffectRack => racks::descriptor(device),
        // --- v0.3 groups ---
        BuiltinDeviceType::ConvolutionReverb => fx_space::descriptor(device),
        BuiltinDeviceType::ExternalInstrument | BuiltinDeviceType::ExternalAudioEffect => {
            external::descriptor(device)
        }
        BuiltinDeviceType::FmSynth => fm_synth::descriptor(device),
        BuiltinDeviceType::GrainDelay => grain_delay::descriptor(device),
        BuiltinDeviceType::Shifter => shifter::descriptor(device),
        BuiltinDeviceType::Maximizer => maximizer::descriptor(device),
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
        | BuiltinDeviceType::AutoFilter
        | BuiltinDeviceType::Erosion
        | BuiltinDeviceType::Pedal => fx_color::factory_presets(device),
        BuiltinDeviceType::Chorus
        | BuiltinDeviceType::Phaser
        | BuiltinDeviceType::Flanger
        | BuiltinDeviceType::Tremolo
        | BuiltinDeviceType::Vibrato
        | BuiltinDeviceType::VolumeLfo => fx_modulation::factory_presets(device),
        BuiltinDeviceType::Gate
        | BuiltinDeviceType::MultibandCompressor
        | BuiltinDeviceType::TransientShaper => fx_dynamics::factory_presets(device),
        BuiltinDeviceType::SpectrumAnalyzer
        | BuiltinDeviceType::Tuner
        | BuiltinDeviceType::Loudness => fx_analysis::factory_presets(device),
        BuiltinDeviceType::Arpeggiator
        | BuiltinDeviceType::Chord
        | BuiltinDeviceType::ScaleQuantize
        | BuiltinDeviceType::NoteLength
        | BuiltinDeviceType::Velocity
        | BuiltinDeviceType::Randomizer
        | BuiltinDeviceType::Pitch
        | BuiltinDeviceType::NoteBlocker => midi_fx::factory_presets(device),
        BuiltinDeviceType::InstrumentRack
        | BuiltinDeviceType::AudioEffectRack
        | BuiltinDeviceType::MidiEffectRack => racks::factory_presets(device),
        BuiltinDeviceType::ConvolutionReverb => fx_space::factory_presets(device),
        BuiltinDeviceType::ExternalInstrument | BuiltinDeviceType::ExternalAudioEffect => {
            external::factory_presets(device)
        }
        BuiltinDeviceType::FmSynth => fm_synth::factory_presets(device),
        BuiltinDeviceType::GrainDelay => grain_delay::factory_presets(device),
        BuiltinDeviceType::Shifter => shifter::factory_presets(device),
        BuiltinDeviceType::Maximizer => maximizer::factory_presets(device),
    }
}

/// All built-in descriptors (for `DeviceCommand::ListBuiltin`), unimplemented ones included
/// (flagged; the UI names a document's devices from this list).
pub fn all_descriptors() -> Vec<DeviceDescriptor> {
    BuiltinDeviceType::ALL.into_iter().map(descriptor).collect()
}

/// The built-in descriptors to offer (device lists for the agent and MCP): the
/// [`implemented`] ones.
pub fn offered_descriptors() -> Vec<DeviceDescriptor> {
    BuiltinDeviceType::ALL
        .into_iter()
        .filter(|t| implemented(*t))
        .map(descriptor)
        .collect()
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
        BuiltinDevice::DrumRack { .. } => drum_rack::create(),
        // --- v0.2 groups ---
        BuiltinDevice::PolySynth => poly_synth::create(device),
        BuiltinDevice::MultiSampler { .. } => multisampler::create(device, samples),
        BuiltinDevice::Saturator
        | BuiltinDevice::Bitcrusher
        | BuiltinDevice::AutoFilter
        | BuiltinDevice::Erosion
        | BuiltinDevice::Pedal => fx_color::create(device),
        BuiltinDevice::Chorus
        | BuiltinDevice::Phaser
        | BuiltinDevice::Flanger
        | BuiltinDevice::Tremolo
        | BuiltinDevice::Vibrato
        | BuiltinDevice::VolumeLfo { .. } => fx_modulation::create(device),
        BuiltinDevice::Gate
        | BuiltinDevice::MultibandCompressor
        | BuiltinDevice::TransientShaper => fx_dynamics::create(device),
        BuiltinDevice::SpectrumAnalyzer | BuiltinDevice::Tuner | BuiltinDevice::Loudness => {
            fx_analysis::create(device)
        }
        BuiltinDevice::Arpeggiator
        | BuiltinDevice::Chord
        | BuiltinDevice::ScaleQuantize
        | BuiltinDevice::NoteLength
        | BuiltinDevice::Velocity
        | BuiltinDevice::Randomizer
        | BuiltinDevice::Pitch
        | BuiltinDevice::NoteBlocker => midi_fx::create(device),
        BuiltinDevice::InstrumentRack { .. }
        | BuiltinDevice::AudioEffectRack { .. }
        | BuiltinDevice::MidiEffectRack { .. } => racks::create(device),
        // --- v0.3 groups ---
        BuiltinDevice::ConvolutionReverb { .. } => fx_space::create(device, samples),
        BuiltinDevice::ExternalInstrument { .. } | BuiltinDevice::ExternalAudioEffect { .. } => {
            external::create(device)
        }
        BuiltinDevice::FmSynth => fm_synth::create(device),
        BuiltinDevice::GrainDelay => grain_delay::create(device),
        BuiltinDevice::Shifter => shifter::create(device),
        BuiltinDevice::Maximizer => maximizer::create(device),
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
