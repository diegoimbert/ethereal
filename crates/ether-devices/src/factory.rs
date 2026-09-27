//! Factory presets of the v0.1/v2 built-in devices (synth, sampler, compressor, delay, EQ,
//! reverb, limiter, utility, drum rack). v0.2, owned by the `presets` node: add
//! `FactoryPreset { id: "<device-key>/<slug>", json: include_str!(...) }` entries for files
//! under `crates/ether-devices/presets/<device-key>/`. The v0.2 device groups register their
//! own presets in their modules.

use ether_core::protocol::model::BuiltinDeviceType;

use crate::contract::FactoryPreset;

/// Factory presets of a v0.1/v2 built-in type.
pub fn factory_presets(device: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = device;
    &[]
}
