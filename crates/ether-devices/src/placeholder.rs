//! Placeholder device for roadmap v2 built-ins whose DSP is not implemented yet
//! (contracts-2). Audio passes through unchanged (instruments output silence); no params.
//! Each owning node replaces its type's use of this with the real device and deletes the
//! call; the module itself goes away once all are implemented.

use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{AudioBuffers, Device, Node, PrepareConfig, ProcessContext, ProcessStatus};

/// Descriptor of a not-yet-implemented built-in: no params, stereo in/out.
pub fn descriptor(ty: BuiltinDeviceType, name: &str, category: DeviceCategory) -> DeviceDescriptor {
    let instrument = category == DeviceCategory::Instrument;
    DeviceDescriptor {
        device_type: DeviceTypeRef::Builtin { device: ty },
        name: name.to_owned(),
        category,
        params: Vec::new(),
        audio_inputs: if instrument { 0 } else { 2 },
        audio_outputs: 2,
        midi_input: instrument,
        sidechain_inputs: 0,
    }
}

/// Pass-through (effects) or silent (instruments) node.
#[derive(Debug)]
pub struct Placeholder {
    descriptor: DeviceDescriptor,
    /// Pass audio through even as an instrument (the drum rack: the engine mixes its pads
    /// into its input).
    pass_through: bool,
}

impl Placeholder {
    /// A placeholder that always passes its input through.
    pub fn pass_through(descriptor: DeviceDescriptor) -> Self {
        Self {
            descriptor,
            pass_through: true,
        }
    }
}

impl Node for Placeholder {
    fn prepare(&mut self, _config: &PrepareConfig) {}

    fn reset(&mut self) {}

    fn process(
        &mut self,
        _ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        if self.descriptor.category == DeviceCategory::Instrument && !self.pass_through {
            audio.clear_outputs();
            ProcessStatus::Silent
        } else {
            audio.pass_through();
            ProcessStatus::Continue
        }
    }
}

impl Device for Placeholder {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, _id: ParamId) -> Option<f64> {
        None
    }

    fn set_param(&mut self, _id: ParamId, _value: f64) {}
}
