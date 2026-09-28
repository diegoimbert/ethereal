//! v0.2 devices owned by the `racks-modulation` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! Racks (`ether_model::rack`): the rack node's params are the 8 macros and the chain
//! selector (shared by the three types, `ether_model::rack_macro_param`,
//! `RACK_SELECTOR_PARAM`). The node itself ([`RackNode`]) passes audio/events through: chains
//! are run by the engine before it (`ether_core::rack_chains`), which also replaces a MIDI
//! effect rack's output with its chains' merged output; macros are modulation sources
//! (`ether_core::modulation`).
//!
//! **Param ids are stable and append-only** (documents, automation and presets store them):
//! never renumber, only append.
//!
//! # Instrument Rack (`BuiltinDeviceType::InstrumentRack`)
//!
//! Its node takes the chains' mix as input (`channels() == (2, 2)`), like the drum rack.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Macros | `Macro 1` | 0 ..= 1 None, default 0 |
//! | 1 | Macros | `Macro 2` | 0 ..= 1 None, default 0 |
//! | 2 | Macros | `Macro 3` | 0 ..= 1 None, default 0 |
//! | 3 | Macros | `Macro 4` | 0 ..= 1 None, default 0 |
//! | 4 | Macros | `Macro 5` | 0 ..= 1 None, default 0 |
//! | 5 | Macros | `Macro 6` | 0 ..= 1 None, default 0 |
//! | 6 | Macros | `Macro 7` | 0 ..= 1 None, default 0 |
//! | 7 | Macros | `Macro 8` | 0 ..= 1 None, default 0 |
//! | 8 | Chains | `Chain Selector` | 0 ..= 127 (stepped), default 0 |
//!
//! # Audio Effect Rack (`BuiltinDeviceType::AudioEffectRack`)
//!
//! Same table.
//!
//! # MIDI Effect Rack (`BuiltinDeviceType::MidiEffectRack`)
//!
//! Same table; `channels() == (0, 0)` (MIDI effect category).

use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::{DeviceLayout, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use crate::contract::{
    FactoryPreset, descriptor as build, item, knob, layout, param, section, stepped,
};

/// Param ids of `InstrumentRack` (stable, append-only).
pub mod instrument_rack {
    use ether_core::protocol::model::ParamId;
    pub const MACRO_1: ParamId = ParamId(0);
    pub const MACRO_2: ParamId = ParamId(1);
    pub const MACRO_3: ParamId = ParamId(2);
    pub const MACRO_4: ParamId = ParamId(3);
    pub const MACRO_5: ParamId = ParamId(4);
    pub const MACRO_6: ParamId = ParamId(5);
    pub const MACRO_7: ParamId = ParamId(6);
    pub const MACRO_8: ParamId = ParamId(7);
    pub const CHAIN_SELECTOR: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}

/// Param ids of `AudioEffectRack` (stable, append-only).
pub mod audio_effect_rack {
    pub use super::instrument_rack::*;
}

/// Param ids of `MidiEffectRack` (stable, append-only).
pub mod midi_effect_rack {
    pub use super::instrument_rack::*;
}

/// Declarative panel: the macro bank (hero), the chain list and the chain selector.
fn rack_layout() -> DeviceLayout {
    let mut macros = item(Widget::Macros, WidgetSize::Medium);
    macros.colspan = 4;
    let mut chains = item(Widget::RackChains, WidgetSize::Medium);
    chains.colspan = 2;
    let mut selector = knob(instrument_rack::CHAIN_SELECTOR, WidgetSize::Small);
    selector.label = Some("Selector".into());
    layout(vec![
        section("macros", Some("Macros"), 2, 4, vec![macros]),
        section("chains", Some("Chains"), 2, 3, vec![chains, selector]),
    ])
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    let (name, category, inputs, outputs, midi) = match ty {
        BuiltinDeviceType::InstrumentRack => {
            ("Instrument Rack", DeviceCategory::Instrument, 0, 2, true)
        }
        BuiltinDeviceType::AudioEffectRack => (
            "Audio Effect Rack",
            DeviceCategory::AudioEffect,
            2,
            2,
            false,
        ),
        BuiltinDeviceType::MidiEffectRack => {
            ("MIDI Effect Rack", DeviceCategory::NoteEffect, 0, 0, true)
        }
        other => unreachable!("{other:?} is not a `racks-modulation` device"),
    };
    let mut params: Vec<_> = (0..8u32)
        .map(|i| {
            param(
                i,
                &format!("Macro {}", i + 1),
                "Macros",
                ParamUnit::None,
                (0.0, 1.0, 0.0),
                ParamScale::Linear,
            )
        })
        .collect();
    params.push(stepped(
        8,
        "Chain Selector",
        "Chains",
        ParamUnit::None,
        0,
        127,
        0,
    ));
    let mut d = build(ty, name, category, params, inputs, outputs, midi, 0);
    d.layout = Some(rack_layout());
    d
}

/// The rack device node: keeps its param values (macros, selector) for readback, passes
/// audio through (instrument/audio effect racks: the chains' mix arrives as its input) and
/// forwards note/MIDI events (MIDI effect racks: the engine replaces its output with the
/// chains' merged output when it has chains). Never forwards `Param` events.
pub struct RackNode {
    ty: BuiltinDeviceType,
    values: [f64; instrument_rack::COUNT],
}

impl RackNode {
    /// Non-RT.
    pub fn new(ty: BuiltinDeviceType) -> Self {
        let mut values = [0.0; instrument_rack::COUNT];
        for p in &descriptor(ty).params {
            if let Some(v) = values.get_mut(p.id.0 as usize) {
                *v = p.default;
            }
        }
        Self { ty, values }
    }

    fn midi(&self) -> bool {
        self.ty == BuiltinDeviceType::MidiEffectRack
    }

    fn set(&mut self, id: ParamId, value: f64) {
        if let Some(v) = self.values.get_mut(id.0 as usize)
            && value.is_finite()
        {
            *v = if id == instrument_rack::CHAIN_SELECTOR {
                value.round().clamp(0.0, 127.0)
            } else {
                value.clamp(0.0, 1.0)
            };
        }
    }
}

impl Node for RackNode {
    fn prepare(&mut self, _config: &PrepareConfig) {}

    fn reset(&mut self) {}

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let midi = self.midi();
        for e in ctx.events {
            match e.kind {
                EventKind::Param { param, value } => self.set(param, value),
                _ if midi => {
                    ctx.out_events.push(*e);
                }
                _ => {}
            }
        }
        if !midi {
            audio.pass_through();
        }
        ProcessStatus::Continue
    }

    fn channels(&self) -> (u16, u16) {
        if self.midi() { (0, 0) } else { (2, 2) }
    }
}

impl Device for RackNode {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(self.ty)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value);
    }
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    Box::new(RackNode::new(device.device_type()))
}

const INSTRUMENT_PRESETS: &[FactoryPreset] = &[FactoryPreset {
    id: "instrument-rack/init",
    json: include_str!("../../presets/instrument-rack/init.etherpreset"),
}];
const AUDIO_PRESETS: &[FactoryPreset] = &[
    FactoryPreset {
        id: "audio-effect-rack/init",
        json: include_str!("../../presets/audio-effect-rack/init.etherpreset"),
    },
    FactoryPreset {
        id: "audio-effect-rack/macros-centered",
        json: include_str!("../../presets/audio-effect-rack/macros-centered.etherpreset"),
    },
];
const MIDI_PRESETS: &[FactoryPreset] = &[FactoryPreset {
    id: "midi-effect-rack/init",
    json: include_str!("../../presets/midi-effect-rack/init.etherpreset"),
}];

/// Factory presets of a type of this group. Rack presets hold the macro positions and the
/// chain selector (the preset format has no place for chains yet).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::InstrumentRack => INSTRUMENT_PRESETS,
        BuiltinDeviceType::AudioEffectRack => AUDIO_PRESETS,
        BuiltinDeviceType::MidiEffectRack => MIDI_PRESETS,
        _ => &[],
    }
}
