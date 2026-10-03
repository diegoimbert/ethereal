//! Shared v0.2 device scaffolding (contracts-3; changes via BCR): param-table builders,
//! the placeholder node every v0.2 device starts as, factory-preset registration and layout
//! builders. Device nodes use these from their own modules.
//!
//! See the "Device agent guide" in `docs/ROADMAP.md` (v0.2) for the rules: RT safety
//! (`assert_no_alloc` tests), append-only param ids, descriptor ↔ mock parity
//! (`tests/v02_descriptors.rs` + `ui/src/transport/mock/devices/*.json`), factory presets
//! under `crates/ether-devices/presets/<device-key>/`, declarative layouts only.

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::layout::{DeviceLayout, LayoutItem, LayoutSection, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

/// Tempo-synced rate labels shared by every synced param (LFOs, delays, arpeggiators).
pub const SYNC_RATES: [&str; 14] = [
    "1/64", "1/32T", "1/32", "1/16T", "1/16", "1/8T", "1/8", "1/4T", "1/4", "1/2", "1 bar",
    "2 bars", "4 bars", "8 bars",
];
/// Index of "1/8" in [`SYNC_RATES`].
pub const SYNC_EIGHTH: usize = 6;
/// Index of "1/16" in [`SYNC_RATES`].
pub const SYNC_SIXTEENTH: usize = 4;
/// Length in beats of each [`SYNC_RATES`] entry (triplets = 2/3).
pub const SYNC_RATE_BEATS: [f64; 14] = [
    1.0 / 16.0,
    1.0 / 12.0,
    1.0 / 8.0,
    1.0 / 6.0,
    1.0 / 4.0,
    1.0 / 3.0,
    1.0 / 2.0,
    2.0 / 3.0,
    1.0,
    2.0,
    4.0,
    8.0,
    16.0,
    32.0,
];

/// A continuous param.
pub fn param(
    id: u32,
    name: &str,
    group: &str,
    unit: ParamUnit,
    (min, max, default): (f64, f64, f64),
    scale: ParamScale,
) -> ParamInfo {
    ParamInfo {
        step: None,
        remote: None,
        id: ParamId(id),
        name: name.to_owned(),
        group: Some(group.to_owned()),
        unit,
        min,
        max,
        default,
        scale,
        labels: None,
        automatable: true,
        hidden: false,
    }
}

/// An enum param with plain values `0..labels.len()`.
pub fn choice(id: u32, name: &str, group: &str, labels: &[&str], default: usize) -> ParamInfo {
    ParamInfo {
        id: ParamId(id),
        name: name.to_owned(),
        group: Some(group.to_owned()),
        unit: ParamUnit::None,
        min: 0.0,
        max: (labels.len() - 1) as f64,
        default: default as f64,
        scale: ParamScale::Linear,
        labels: Some(labels.iter().map(|s| (*s).to_owned()).collect()),
        automatable: true,
        hidden: false,
        step: Some(1.0),
        remote: None,
    }
}

/// An off/on param (plain 0/1).
pub fn toggle(id: u32, name: &str, group: &str, default: bool) -> ParamInfo {
    ParamInfo {
        unit: ParamUnit::Toggle,
        ..choice(id, name, group, &["Off", "On"], default as usize)
    }
}

/// An integer param `min..=max` (stepped: one label per value).
pub fn stepped(
    id: u32,
    name: &str,
    group: &str,
    unit: ParamUnit,
    min: i32,
    max: i32,
    default: i32,
) -> ParamInfo {
    ParamInfo {
        id: ParamId(id),
        name: name.to_owned(),
        group: Some(group.to_owned()),
        unit,
        min: f64::from(min),
        max: f64::from(max),
        default: f64::from(default),
        scale: ParamScale::Linear,
        labels: Some((min..=max).map(|v| v.to_string()).collect()),
        automatable: true,
        hidden: false,
        step: Some(1.0),
        remote: None,
    }
}

/// A built-in descriptor.
#[allow(clippy::too_many_arguments)]
pub fn descriptor(
    device: BuiltinDeviceType,
    name: &str,
    category: DeviceCategory,
    params: Vec<ParamInfo>,
    audio_inputs: u16,
    audio_outputs: u16,
    midi_input: bool,
    sidechain_inputs: u16,
) -> DeviceDescriptor {
    DeviceDescriptor {
        device_type: DeviceTypeRef::Builtin { device },
        name: name.to_owned(),
        category,
        params,
        audio_inputs,
        audio_outputs,
        midi_input,
        sidechain_inputs,
        layout: None,
    }
}

/// An embedded factory preset: `id` = `"<device-key>/<slug>"`, `json` = the preset file
/// (`ether_model::preset`, usually `include_str!("../../presets/<device-key>/<slug>.etherpreset")`).
#[derive(Clone, Copy, Debug)]
pub struct FactoryPreset {
    pub id: &'static str,
    pub json: &'static str,
}

/// What a [`Placeholder`] does until its device is implemented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceholderMode {
    /// Audio effects, analyzers, racks: input copied to output.
    PassThrough,
    /// Instruments: silence.
    Silent,
    /// MIDI effects: note/MIDI events forwarded unchanged to `out_events`; audio untouched
    /// (`channels() == (0, 0)`).
    MidiThru,
}

/// RT-safe stand-in for an unimplemented v0.2 device: keeps its param values (so documents,
/// automation and presets round-trip) and behaves per [`PlaceholderMode`].
pub struct Placeholder {
    descriptor: DeviceDescriptor,
    values: Vec<f64>,
    mode: PlaceholderMode,
}

impl Placeholder {
    /// Non-RT.
    pub fn new(descriptor: DeviceDescriptor, mode: PlaceholderMode) -> Self {
        let values = descriptor.params.iter().map(|p| p.default).collect();
        Self {
            descriptor,
            values,
            mode,
        }
    }

    fn slot(&self, id: ParamId) -> Option<usize> {
        self.descriptor.params.iter().position(|p| p.id == id)
    }
}

impl Node for Placeholder {
    fn prepare(&mut self, _config: &PrepareConfig) {}

    fn reset(&mut self) {}

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            match e.kind {
                EventKind::Param { param, value } => self.set_param(param, value),
                _ if self.mode == PlaceholderMode::MidiThru => {
                    ctx.out_events.push(*e);
                }
                _ => {}
            }
        }
        let frames = ctx.frames;
        match self.mode {
            PlaceholderMode::PassThrough => {
                for (ch, out) in audio.outputs.iter_mut().enumerate() {
                    match audio.inputs.get(ch).or(audio.inputs.first()) {
                        Some(input) => out[..frames].copy_from_slice(&input[..frames]),
                        None => out[..frames].fill(0.0),
                    }
                }
                ProcessStatus::Continue
            }
            PlaceholderMode::Silent | PlaceholderMode::MidiThru => {
                for out in audio.outputs.iter_mut() {
                    out[..frames].fill(0.0);
                }
                ProcessStatus::Silent
            }
        }
    }

    fn channels(&self) -> (u16, u16) {
        match self.mode {
            PlaceholderMode::MidiThru => (0, 0),
            PlaceholderMode::Silent => (0, 2),
            PlaceholderMode::PassThrough => (2, 2),
        }
    }
}

impl Device for Placeholder {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.slot(id).map(|i| self.values[i])
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        if let Some(i) = self.slot(id) {
            let p = &self.descriptor.params[i];
            self.values[i] = if value.is_nan() {
                p.default
            } else {
                value.clamp(p.min.min(p.max), p.max.max(p.min))
            };
        }
    }
}

// --- layout builders (see `ether_protocol::layout`) ---

/// A section.
pub fn section(
    id: &str,
    title: Option<&str>,
    span: u8,
    columns: u8,
    items: Vec<LayoutItem>,
) -> LayoutSection {
    LayoutSection {
        id: id.to_owned(),
        title: title.map(str::to_owned),
        span,
        columns,
        items,
    }
}

/// An item of `size` spanning one cell.
pub fn item(widget: Widget, size: WidgetSize) -> LayoutItem {
    LayoutItem {
        widget,
        size,
        colspan: 1,
        label: None,
    }
}

/// A knob.
pub fn knob(param: ParamId, size: WidgetSize) -> LayoutItem {
    item(Widget::Knob { param }, size)
}

/// A layout from sections.
pub fn layout(sections: Vec<LayoutSection>) -> DeviceLayout {
    DeviceLayout { sections }
}
