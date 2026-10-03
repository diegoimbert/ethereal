//! v0.2 multisampler (node `multisampler`; contracts-3 froze the param table, see
//! docs/ROADMAP.md "v0.2" and the device agent guide there). **Param ids are stable and
//! append-only** (documents, automation and presets store them).
//!
//! # Multisampler (`BuiltinDeviceType::MultiSampler`)
//!
//! Zones (`BuiltinDevice::MultiSampler::zones`, `ether_model::multisampler`) with key and
//! velocity ranges, root key, tune, gain, pan, loop points with crossfade and round-robin
//! groups; a global amp ADSR and a per-voice filter with its own envelope.
//!
//! - [`zones`]: the resolved [`ZoneSet`] (built off the audio thread from the zones and the
//!   host's [`SampleResolver`]) and the per-note selection (candidates, round robin, velocity
//!   crossfades).
//! - [`device`]: the [`MultiSampler`] node. Live zone edits arrive through `Node::set_data`
//!   (a boxed [`ZoneSet`]) without cutting sounding notes ([`updatable_in_place`]).
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Output | `Volume` | -70 ..= 6 dB (fader), default -6 |
//! | 1 | Pitch | `Transpose` | -48 ..= 48 (stepped), default 0 |
//! | 2 | Pitch | `Fine` | -1 ..= 1 Semitones, default 0 |
//! | 3 | Voice | `Voices` | 1 ..= 64 (stepped), default 32 |
//! | 4 | Voice | `Glide` | 0 ..= 2000 Milliseconds, default 0 |
//! | 5 | Voice | `Velocity` | 0 ..= 100 Percent, default 100 |
//! | 6 | Amp Envelope | `Attack` | 0 ..= 10000 Milliseconds, default 1 |
//! | 7 | Amp Envelope | `Decay` | 1 ..= 10000 Milliseconds, default 200 |
//! | 8 | Amp Envelope | `Sustain` | 0 ..= 100 Percent, default 100 |
//! | 9 | Amp Envelope | `Release` | 1 ..= 20000 Milliseconds, default 100 |
//! | 10 | Filter | `Type` | Off / Low-pass 24 / Low-pass 12 / High-pass 12 / Band-pass 12 (default Off) |
//! | 11 | Filter | `Cutoff` | 20 ..= 20000 Hertz (log), default 20000 |
//! | 12 | Filter | `Resonance` | 0 ..= 100 Percent, default 0 |
//! | 13 | Filter | `Key Tracking` | 0 ..= 100 Percent, default 0 |
//! | 14 | Filter | `Env Amount` | -100 ..= 100 Percent, default 0 |
//! | 15 | Filter Envelope | `Attack` | 0 ..= 10000 Milliseconds, default 1 |
//! | 16 | Filter Envelope | `Decay` | 1 ..= 10000 Milliseconds, default 400 |
//! | 17 | Filter Envelope | `Sustain` | 0 ..= 100 Percent, default 0 |
//! | 18 | Filter Envelope | `Release` | 1 ..= 20000 Milliseconds, default 300 |
//! | 19 | Voice | `Round Robin` | Cycle / Random (default Cycle) |
//! | 20 | Output | `Pan` | -1 ..= 1 Pan, default 0 |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::DeviceLayout;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

use crate::SampleResolver;
use crate::contract::{
    FactoryPreset, choice, descriptor as build, item, knob, param, section, stepped,
};

pub mod device;
pub mod zones;

pub use device::{MAX_VOICES, MultiSampler};
pub use zones::{Hit, MAX_LAYERS, RoundRobin, ZoneSet};

/// Param ids of `MultiSampler` (stable, append-only).
pub mod multi_sampler {
    use ether_core::protocol::model::ParamId;
    pub const VOLUME: ParamId = ParamId(0);
    pub const TRANSPOSE: ParamId = ParamId(1);
    pub const FINE: ParamId = ParamId(2);
    pub const VOICES: ParamId = ParamId(3);
    pub const GLIDE: ParamId = ParamId(4);
    pub const VELOCITY: ParamId = ParamId(5);
    pub const AMP_ATTACK: ParamId = ParamId(6);
    pub const AMP_DECAY: ParamId = ParamId(7);
    pub const AMP_SUSTAIN: ParamId = ParamId(8);
    pub const AMP_RELEASE: ParamId = ParamId(9);
    pub const FILTER_TYPE: ParamId = ParamId(10);
    pub const CUTOFF: ParamId = ParamId(11);
    pub const RESONANCE: ParamId = ParamId(12);
    pub const KEY_TRACKING: ParamId = ParamId(13);
    pub const FILTER_ENV_AMOUNT: ParamId = ParamId(14);
    pub const FILTER_ATTACK: ParamId = ParamId(15);
    pub const FILTER_DECAY: ParamId = ParamId(16);
    pub const FILTER_SUSTAIN: ParamId = ParamId(17);
    pub const FILTER_RELEASE: ParamId = ParamId(18);
    pub const ROUND_ROBIN: ParamId = ParamId(19);
    pub const PAN: ParamId = ParamId(20);
    /// Number of params.
    pub const COUNT: usize = 21;
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::MultiSampler => DeviceDescriptor {
            layout: Some(layout()),
            ..build(
                BuiltinDeviceType::MultiSampler,
                "Multisampler",
                DeviceCategory::Instrument,
                vec![
                    param(
                        0,
                        "Volume",
                        "Output",
                        ParamUnit::Decibels,
                        (-70.0, 6.0, -6.0),
                        ParamScale::Fader,
                    ),
                    stepped(1, "Transpose", "Pitch", ParamUnit::Semitones, -48, 48, 0),
                    param(
                        2,
                        "Fine",
                        "Pitch",
                        ParamUnit::Semitones,
                        (-1.0, 1.0, 0.0),
                        ParamScale::Linear,
                    ),
                    stepped(3, "Voices", "Voice", ParamUnit::None, 1, 64, 32),
                    param(
                        4,
                        "Glide",
                        "Voice",
                        ParamUnit::Milliseconds,
                        (0.0, 2000.0, 0.0),
                        ParamScale::Power { exponent: 3.0 },
                    ),
                    param(
                        5,
                        "Velocity",
                        "Voice",
                        ParamUnit::Percent,
                        (0.0, 100.0, 100.0),
                        ParamScale::Linear,
                    ),
                    param(
                        6,
                        "Attack",
                        "Amp Envelope",
                        ParamUnit::Milliseconds,
                        (0.0, 10000.0, 1.0),
                        ParamScale::Power { exponent: 3.0 },
                    ),
                    param(
                        7,
                        "Decay",
                        "Amp Envelope",
                        ParamUnit::Milliseconds,
                        (1.0, 10000.0, 200.0),
                        ParamScale::Power { exponent: 3.0 },
                    ),
                    param(
                        8,
                        "Sustain",
                        "Amp Envelope",
                        ParamUnit::Percent,
                        (0.0, 100.0, 100.0),
                        ParamScale::Linear,
                    ),
                    param(
                        9,
                        "Release",
                        "Amp Envelope",
                        ParamUnit::Milliseconds,
                        (1.0, 20000.0, 100.0),
                        ParamScale::Power { exponent: 3.0 },
                    ),
                    choice(
                        10,
                        "Type",
                        "Filter",
                        &[
                            "Off",
                            "Low-pass 24",
                            "Low-pass 12",
                            "High-pass 12",
                            "Band-pass 12",
                        ],
                        0,
                    ),
                    param(
                        11,
                        "Cutoff",
                        "Filter",
                        ParamUnit::Hertz,
                        (20.0, 20000.0, 20000.0),
                        ParamScale::Log,
                    ),
                    param(
                        12,
                        "Resonance",
                        "Filter",
                        ParamUnit::Percent,
                        (0.0, 100.0, 0.0),
                        ParamScale::Linear,
                    ),
                    param(
                        13,
                        "Key Tracking",
                        "Filter",
                        ParamUnit::Percent,
                        (0.0, 100.0, 0.0),
                        ParamScale::Linear,
                    ),
                    param(
                        14,
                        "Env Amount",
                        "Filter",
                        ParamUnit::Percent,
                        (-100.0, 100.0, 0.0),
                        ParamScale::Linear,
                    ),
                    param(
                        15,
                        "Attack",
                        "Filter Envelope",
                        ParamUnit::Milliseconds,
                        (0.0, 10000.0, 1.0),
                        ParamScale::Power { exponent: 3.0 },
                    ),
                    param(
                        16,
                        "Decay",
                        "Filter Envelope",
                        ParamUnit::Milliseconds,
                        (1.0, 10000.0, 400.0),
                        ParamScale::Power { exponent: 3.0 },
                    ),
                    param(
                        17,
                        "Sustain",
                        "Filter Envelope",
                        ParamUnit::Percent,
                        (0.0, 100.0, 0.0),
                        ParamScale::Linear,
                    ),
                    param(
                        18,
                        "Release",
                        "Filter Envelope",
                        ParamUnit::Milliseconds,
                        (1.0, 20000.0, 300.0),
                        ParamScale::Power { exponent: 3.0 },
                    ),
                    choice(19, "Round Robin", "Voice", &["Cycle", "Random"], 0),
                    param(
                        20,
                        "Pan",
                        "Output",
                        ParamUnit::Pan,
                        (-1.0, 1.0, 0.0),
                        ParamScale::Linear,
                    ),
                ],
                0,
                2,
                true,
                0,
            )
        },
        other => unreachable!("{other:?} is not a `multisampler` device"),
    }
}

/// Declarative panel: the zone map on top, then pitch/voice, envelopes, filter and output.
pub fn layout() -> DeviceLayout {
    use ether_core::protocol::layout::{LayoutItem, Widget, WidgetSize::*};
    use multi_sampler as p;
    let wide = |mut it: LayoutItem, span: u8| {
        it.colspan = span;
        it
    };
    let env = |id: &str, title: &str, a, d, s, r| {
        section(
            id,
            Some(title),
            1,
            2,
            vec![wide(
                item(
                    Widget::Envelope {
                        attack: a,
                        decay: d,
                        sustain: s,
                        release: r,
                        delay: None,
                        hold: None,
                    },
                    Medium,
                ),
                2,
            )],
        )
    };
    crate::contract::layout(vec![
        section(
            "zones",
            Some("Zones"),
            4,
            1,
            vec![item(Widget::ZoneMap, Large)],
        ),
        section(
            "pitch",
            Some("Pitch"),
            1,
            2,
            vec![knob(p::TRANSPOSE, Medium), knob(p::FINE, Medium)],
        ),
        section(
            "voice",
            Some("Voice"),
            1,
            2,
            vec![
                item(Widget::Number { param: p::VOICES }, Small),
                item(
                    Widget::Choice {
                        param: p::ROUND_ROBIN,
                    },
                    Small,
                ),
                knob(p::GLIDE, Small),
                knob(p::VELOCITY, Small),
            ],
        ),
        env(
            "amp-env",
            "Amp Envelope",
            p::AMP_ATTACK,
            p::AMP_DECAY,
            p::AMP_SUSTAIN,
            p::AMP_RELEASE,
        ),
        section(
            "filter",
            Some("Filter"),
            2,
            2,
            vec![
                wide(
                    item(
                        Widget::FilterCurve {
                            cutoff: p::CUTOFF,
                            resonance: p::RESONANCE,
                            mode: Some(p::FILTER_TYPE),
                            drive: None,
                            gain: None,
                        },
                        Medium,
                    ),
                    2,
                ),
                knob(p::KEY_TRACKING, Small),
                knob(p::FILTER_ENV_AMOUNT, Small),
            ],
        ),
        env(
            "filter-env",
            "Filter Envelope",
            p::FILTER_ATTACK,
            p::FILTER_DECAY,
            p::FILTER_SUSTAIN,
            p::FILTER_RELEASE,
        ),
        section(
            "output",
            Some("Output"),
            1,
            1,
            vec![knob(p::VOLUME, Large), knob(p::PAN, Small)],
        ),
    ])
}

/// Non-RT. A new instance playing `device`'s zones, their media resolved with `samples`
/// (unresolved media = silent zones until the host rebuilds or updates the node).
pub fn create(device: &BuiltinDevice, samples: &dyn SampleResolver) -> Box<dyn Device> {
    Box::new(MultiSampler::new(zone_set(device, samples)))
}

/// Non-RT. The [`ZoneSet`] of a multisampler kind (empty for other kinds): what a host
/// sends to a live node with `set_data` (`EngineBridge::update_builtin`).
pub fn zone_set(device: &BuiltinDevice, samples: &dyn SampleResolver) -> ZoneSet {
    match device {
        BuiltinDevice::MultiSampler { zones } => ZoneSet::new(zones, |m| samples.resolve(m)),
        _ => ZoneSet::default(),
    }
}

/// Whether a live node built from `old` can take `new` in place (`Node::set_data` with
/// [`zone_set`]): both are multisamplers and only their zones differ.
pub fn updatable_in_place(old: &BuiltinDevice, new: &BuiltinDevice) -> bool {
    matches!(
        (old, new),
        (
            BuiltinDevice::MultiSampler { zones: a },
            BuiltinDevice::MultiSampler { zones: b },
        ) if a != b
    )
}

static FACTORY_PRESETS: [FactoryPreset; 3] = [
    FactoryPreset {
        id: "multi-sampler/keys",
        json: include_str!("../../presets/multisampler/keys.etherpreset"),
    },
    FactoryPreset {
        id: "multi-sampler/sustained-pad",
        json: include_str!("../../presets/multisampler/sustained-pad.etherpreset"),
    },
    FactoryPreset {
        id: "multi-sampler/plucked",
        json: include_str!("../../presets/multisampler/plucked.etherpreset"),
    },
];

/// Factory presets of a type of this group (params only: zones reference project media).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::MultiSampler => &FACTORY_PRESETS,
        _ => &[],
    }
}
