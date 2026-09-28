//! Factory presets of the v0.1/v2 built-in devices (synth, sampler, compressor, delay, EQ,
//! reverb, limiter, utility), owned by the `presets` node. Each entry is a file under
//! `crates/ether-devices/presets/<device-key>/<slug>.etherpreset` embedded with
//! `include_str!`; ids are `"<device-key>/<slug>"`. The v0.2 device groups register their
//! own presets in their modules. The drum rack ships none: its sound lives in its pads
//! (separate devices), not in its two params.
//!
//! `tests/v02_descriptors.rs` parses every file and checks its type and param ids.

use ether_core::protocol::model::BuiltinDeviceType;

use crate::contract::FactoryPreset;

/// `preset!("synth", "soft-pad")` → the embedded `presets/synth/soft-pad.etherpreset`.
macro_rules! preset {
    ($key:literal, $slug:literal) => {
        FactoryPreset {
            id: concat!($key, "/", $slug),
            json: include_str!(concat!("../presets/", $key, "/", $slug, ".etherpreset")),
        }
    };
}

const SYNTH: &[FactoryPreset] = &[
    preset!("synth", "soft-pad"),
    preset!("synth", "pluck"),
    preset!("synth", "sub-bass"),
    preset!("synth", "square-lead"),
    preset!("synth", "glass-keys"),
];

const SAMPLER: &[FactoryPreset] = &[
    preset!("sampler", "one-shot"),
    preset!("sampler", "pitched-keys"),
    preset!("sampler", "soft-swell"),
];

const COMPRESSOR: &[FactoryPreset] = &[
    preset!("compressor", "gentle-glue"),
    preset!("compressor", "vocal-leveler"),
    preset!("compressor", "drum-punch"),
    preset!("compressor", "bass-tamer"),
];

const DELAY: &[FactoryPreset] = &[
    preset!("delay", "quarter-echo"),
    preset!("delay", "dotted-eighth"),
    preset!("delay", "ping-pong-eighths"),
    preset!("delay", "slapback"),
];

const EQ: &[FactoryPreset] = &[
    preset!("eq", "low-cut-80-hz"),
    preset!("eq", "vocal-presence"),
    preset!("eq", "warm-master"),
    preset!("eq", "telephone"),
];

const REVERB: &[FactoryPreset] = &[
    preset!("reverb", "small-room"),
    preset!("reverb", "plate"),
    preset!("reverb", "large-hall"),
    preset!("reverb", "ambient-wash"),
];

const LIMITER: &[FactoryPreset] = &[
    preset!("limiter", "transparent"),
    preset!("limiter", "loud-master"),
    preset!("limiter", "safety"),
];

const UTILITY: &[FactoryPreset] = &[
    preset!("utility", "mono"),
    preset!("utility", "wide"),
    preset!("utility", "phase-flip"),
    preset!("utility", "minus-6-db"),
];

/// Factory presets of a v0.1/v2 built-in type.
pub fn factory_presets(device: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match device {
        BuiltinDeviceType::Synth => SYNTH,
        BuiltinDeviceType::Sampler => SAMPLER,
        BuiltinDeviceType::Compressor => COMPRESSOR,
        BuiltinDeviceType::Delay => DELAY,
        BuiltinDeviceType::Eq => EQ,
        BuiltinDeviceType::Reverb => REVERB,
        BuiltinDeviceType::Limiter => LIMITER,
        BuiltinDeviceType::Utility => UTILITY,
        _ => &[],
    }
}
