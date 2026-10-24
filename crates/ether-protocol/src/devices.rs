//! Device chains, parameters and device descriptors.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{
    BuiltinDevice, BuiltinDeviceType, DeviceId, IrSource, LfoCurve, MediaId, MidiSidechain,
    ParamId, PluginFormat, SampleZone, Seconds, ShaperLane, SidechainTap, TrackId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceCommand {
    /// Insert on `track` before `before` (None = end of chain).
    Insert {
        id: DeviceId,
        track: TrackId,
        device: DeviceSpec,
        before: Option<DeviceId>,
    },
    Remove {
        id: DeviceId,
    },
    /// Move within / across chains.
    Move {
        id: DeviceId,
        track: TrackId,
        before: Option<DeviceId>,
    },
    /// Copy a device (params, plugin state, a rack's chains, a drum rack's pads) right after
    /// it, or, with `track`, at the end of that track's chain (with its chains or pads on
    /// that track: a rack can be pasted onto another track, which `Move` can't do).
    Duplicate {
        id: DeviceId,
        new_id: DeviceId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        track: Option<TrackId>,
    },
    Rename {
        id: DeviceId,
        name: String,
    },
    SetEnabled {
        id: DeviceId,
        enabled: bool,
    },
    /// Set a parameter to a *plain* value (clamped to the param range). Undoable; send with
    /// a gesture id while dragging. The engine receives it through the lock-free param queue
    /// immediately (no snapshot rebuild).
    SetParam {
        device: DeviceId,
        param: ParamId,
        value: f64,
    },
    ResetParam {
        device: DeviceId,
        param: ParamId,
    },
    /// Sampler only.
    SetSample {
        device: DeviceId,
        media: Option<MediaId>,
    },
    /// Replies `DeviceTypes` with every built-in device descriptor.
    ListBuiltin,
    /// Replies `Descriptor` for an instantiated device (plugins: params discovered at load).
    GetDescriptor {
        device: DeviceId,
    },
    /// Roadmap v2 (`sidechain`): feed `source`'s signal into the device's sidechain input
    /// (`None` = off), at `tap` (v0.4: a point of the track or of one of its rack chains /
    /// drum pads, `ether_model::SidechainTap`; omitted = post-fader). Any device, at any
    /// depth (drum pads and rack chains included). `InvalidArgument` if it would create a
    /// routing cycle (same-track rules: `SidechainTap`) or the device has no sidechain input
    /// (`sidechain_inputs == 0`). Undoable (one step).
    SetSidechain {
        device: DeviceId,
        source: Option<TrackId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        tap: Option<SidechainTap>,
    },
    /// MIDI sidechain (`ether_model::MidiSidechain`): feed a MIDI track's notes (pre or post
    /// its MIDI effects) into an audio effect's note input, instead of its chain's (`None` =
    /// off). `InvalidArgument` if the device isn't an audio effect with a note input
    /// (`DeviceDescriptor::midi_input`), the source isn't a MIDI track, or it would create a
    /// routing cycle. Undoable (one step).
    SetMidiSidechain {
        device: DeviceId,
        source: Option<MidiSidechain>,
    },
    /// v0.2 (`multisampler`): replace a multisampler's zones (`ether_model::multisampler`;
    /// referenced media must exist). Undoable; live nodes update in place
    /// (`EngineBridge::update_builtin`).
    SetZones {
        device: DeviceId,
        zones: Vec<SampleZone>,
    },
    /// v0.3 (`fx-space`): a convolution reverb's impulse response (`None` = unload).
    /// `Media` IRs must exist in the project (import or reference the file first, like a
    /// sample). Undoable; the live node swaps its IR without a click (crossfade).
    SetIr {
        device: DeviceId,
        ir: Option<IrSource>,
    },
    /// v0.3 (`fx-space`): replies `FactoryIrs` (the IRs shipped with the convolution reverb).
    ListFactoryIrs,
    /// `fx-modulation`: replace a Volume LFO's drawn cycle (`ether_model::lfo_curve`;
    /// sanitized: sorted, clamped, at most 64 points). One undo step (send with a gesture id
    /// while dragging); the live node swaps it in place (`EngineBridge::update_builtin`).
    SetLfoCurve {
        device: DeviceId,
        curve: LfoCurve,
    },
    /// `shaper`: replace one lane's drawn cycle of a shaper (`ether_model::ShaperCurves`;
    /// sanitized like [`DeviceCommand::SetLfoCurve`]). One undo step (send with a gesture id
    /// while dragging); the live node swaps it in place.
    SetShaperCurve {
        device: DeviceId,
        lane: ShaperLane,
        curve: LfoCurve,
    },
    /// Rename macro `index` (`< RACK_MACROS`) of a rack (any rack type, drum racks
    /// included; `ether_model::RackMacros`): trimmed, at most 64 chars; empty = the default
    /// name ("Macro N"). One undo step.
    RenameMacro {
        device: DeviceId,
        index: u8,
        name: String,
    },
    /// Show the first `count` macros of a rack (`1..=RACK_MACROS`, default 8; clamped).
    /// Hidden macros keep their values and mappings. One undo step.
    SetMacroCount {
        device: DeviceId,
        count: u8,
    },
}

/// A factory impulse response (v0.3, `fx-space`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct FactoryIr {
    /// `IrSource::Factory { id }`.
    pub id: String,
    pub name: String,
    /// Category for the browser (`"Room"`, `"Hall"`, `"Plate"`, ...).
    pub category: String,
    pub length: Seconds,
    pub channels: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceSpec {
    Builtin {
        device: BuiltinDevice,
    },
    /// A plugin from the plugin list (`PluginDescriptor { format, id }`). `sandboxed: None`
    /// = user default. `format` omitted/`null` = CLAP (the only format before VST3/AU).
    /// `name` = the listed plugin's name (`PluginDescriptor.name`): it picks one of several
    /// plugins sharing a `(format, id)` (two VST2 plugins with the same uniqueID); omitted =
    /// the first one listed. It is also the device's plugin name until the host names it.
    Plugin {
        plugin_id: String,
        sandboxed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        format: Option<PluginFormat>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        name: Option<String>,
    },
}

/// Device category (append-only).
///
/// `NoteEffect` = **MIDI effect** (v0.2 `midi-fx`, CONTRACTS.md §12.4.4): sits before the
/// instrument, transforms note/MIDI events (`Node` MIDI-in → MIDI-out), audio passes through
/// untouched (`audio_inputs == audio_outputs == 0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum DeviceCategory {
    Instrument,
    AudioEffect,
    NoteEffect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceTypeRef {
    Builtin {
        device: BuiltinDeviceType,
    },
    Plugin {
        plugin_id: String,
    },
    /// A missing device (placeholder, `DeviceKind::Missing`): its descriptor lists the
    /// parameters its source gave (linear over their ranges).
    Missing {
        source_id: String,
    },
}

/// Static description of a device type (or of a loaded plugin instance).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeviceDescriptor {
    pub device_type: DeviceTypeRef,
    pub name: String,
    pub category: DeviceCategory,
    pub params: Vec<ParamInfo>,
    /// Audio I/O channel counts of the main ports.
    pub audio_inputs: u16,
    pub audio_outputs: u16,
    pub midi_input: bool,
    /// Roadmap v2 (`sidechain`): channels of the sidechain input (0 = none; the UI shows a
    /// sidechain source selector when > 0).
    pub sidechain_inputs: u16,
    /// v0.2: declarative panel layout rendered by the shared renderer
    /// ([`crate::layout`]). `None` = the generic layout. Omitted from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub layout: Option<crate::layout::DeviceLayout>,
    /// Built-ins only: `false` = the type's node is still a contract placeholder (silent
    /// instrument, pass-through effect, MIDI-thru), not a working device
    /// (`ether_devices::implemented`). Every place that offers devices (device browser, insert
    /// menus, command palette, the agent's and MCP's device lists) hides these; a document
    /// that already holds one still loads and shows it. Omitted from JSON when `true` (plugins,
    /// missing devices and every working built-in).
    #[serde(
        default = "implemented_default",
        skip_serializing_if = "is_implemented"
    )]
    #[ts(optional, as = "Option<bool>")]
    pub implemented: bool,
}

fn implemented_default() -> bool {
    true
}

fn is_implemented(v: &bool) -> bool {
    *v
}

/// Parameter metadata. Plain values are what the document stores; normalized 0..=1 values
/// are what automation and generic UI controls use. `scale` defines the mapping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ParamInfo {
    pub id: ParamId,
    pub name: String,
    /// Group/section name for the generic device UI (e.g. "Filter", "Envelope").
    pub group: Option<String>,
    pub unit: ParamUnit,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub scale: ParamScale,
    /// `Some(labels)` = discrete/enum param with `labels.len()` steps from `min` to `max`.
    pub labels: Option<Vec<String>>,
    pub automatable: bool,
    pub hidden: bool,
    /// v0.2: plain-value step (`None` = continuous). Integer params (transpose, root key,
    /// voices, counts) and enum/toggle params use 1. Knobs, automation lanes, MIDI learn and
    /// the layout renderer snap to it (`min + round((v - min) / step) · step`); the scale
    /// helpers (`to_plain`/`to_normalized`) don't. Omitted from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step: Option<f64>,
    /// Position of this param among the plugin's own quick controls (CLAP
    /// remote-controls pages: `page index · 8 + slot`, first occurrence). The device card
    /// shows these right after the params the user pinned. Omitted from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remote: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ParamUnit {
    None,
    Decibels,
    Hertz,
    Milliseconds,
    Seconds,
    Percent,
    Semitones,
    Ratio,
    Pan,
    Toggle,
    /// Hundredths of a semitone (unison detune, fine tuning). Appended.
    Cents,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ParamScale {
    Linear,
    /// Logarithmic (min must be > 0), e.g. frequencies.
    Log,
    /// `normalized^exponent` (e.g. 2.0 for time/attack knobs).
    Power {
        exponent: f64,
    },
    /// Decibel fader law for params in dB: linear amplitude = `normalized³ · amp(max)`, so
    /// 0 → -inf (clamped to `min`) and 1 → `max` dB (e.g. min -70, max +6).
    Fader,
    /// Gain knob law for params in dB with unity at the centre (max > 0, min = silence,
    /// e.g. -144 ..= +36): the upper half is linear in dB (`(2n - 1) · max`), the lower half
    /// `max · ln(2n)`, which meets it at 0 dB with the same slope and falls to -inf (clamped
    /// to `min`) at 0. Appended.
    Gain,
}

/// Normalized ↔ plain mapping, shared by the engine (automation), the controller and the
/// UI (generic knobs). `ui/src` must mirror this exactly (keep the test vectors in sync).
pub fn scale_to_plain(scale: ParamScale, min: f64, max: f64, normalized: f64) -> f64 {
    let n = normalized.clamp(0.0, 1.0);
    let v = match scale {
        ParamScale::Linear => min + n * (max - min),
        ParamScale::Log => min * (max / min).powf(n),
        ParamScale::Power { exponent } => min + n.powf(exponent) * (max - min),
        ParamScale::Fader => {
            let amp = n * n * n * 10f64.powf(max / 20.0);
            if amp <= 0.0 {
                min
            } else {
                (20.0 * amp.log10()).max(min)
            }
        }
        ParamScale::Gain => {
            if n >= 0.5 {
                (2.0 * n - 1.0) * max
            } else if n <= 0.0 {
                min
            } else {
                (max * (2.0 * n).ln()).max(min)
            }
        }
    };
    v.clamp(min.min(max), max.max(min))
}

/// Inverse of [`scale_to_plain`].
pub fn scale_to_normalized(scale: ParamScale, min: f64, max: f64, plain: f64) -> f64 {
    if max == min {
        return 0.0;
    }
    let p = plain.clamp(min.min(max), max.max(min));
    let n = match scale {
        ParamScale::Linear => (p - min) / (max - min),
        ParamScale::Log => (p / min).ln() / (max / min).ln(),
        ParamScale::Power { exponent } => ((p - min) / (max - min)).powf(1.0 / exponent),
        ParamScale::Fader => {
            if p <= min {
                0.0
            } else {
                (10f64.powf(p / 20.0) / 10f64.powf(max / 20.0)).cbrt()
            }
        }
        ParamScale::Gain => {
            if p >= 0.0 {
                0.5 + 0.5 * p / max
            } else if p <= min {
                0.0
            } else {
                0.5 * (p / max).exp()
            }
        }
    };
    n.clamp(0.0, 1.0)
}

impl ParamInfo {
    /// This param with a plain-value step (builder).
    pub fn with_step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Snap a plain value to [`ParamInfo::step`] (unchanged when continuous), clamped.
    pub fn snap(&self, plain: f64) -> f64 {
        let (lo, hi) = (self.min.min(self.max), self.max.max(self.min));
        let v = plain.clamp(lo, hi);
        match self.step {
            Some(s) if s > 0.0 => (self.min + ((v - self.min) / s).round() * s).clamp(lo, hi),
            _ => v,
        }
    }

    /// Map a normalized value 0..=1 to a plain value (snapped to steps for enum params).
    pub fn to_plain(&self, normalized: f64) -> f64 {
        let v = scale_to_plain(self.scale, self.min, self.max, normalized);
        match &self.labels {
            Some(labels) if labels.len() > 1 => {
                let step = (self.max - self.min) / (labels.len() - 1) as f64;
                self.min + ((v - self.min) / step).round() * step
            }
            _ => v,
        }
    }

    /// Map a plain value to normalized 0..=1.
    pub fn to_normalized(&self, plain: f64) -> f64 {
        scale_to_normalized(self.scale, self.min, self.max, plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn scale_vectors_roundtrip() {
        // Test vectors mirrored by the UI implementation.
        assert!(close(
            scale_to_plain(ParamScale::Linear, -1.0, 1.0, 0.5),
            0.0
        ));
        assert!(close(
            scale_to_plain(ParamScale::Log, 20.0, 20000.0, 0.5),
            632.455_532_033_675_9
        ));
        assert!(close(
            scale_to_plain(ParamScale::Power { exponent: 2.0 }, 0.0, 100.0, 0.5),
            25.0
        ));
        assert!(close(
            scale_to_plain(ParamScale::Fader, -70.0, 6.0, 1.0),
            6.0
        ));
        assert!(close(
            scale_to_plain(ParamScale::Fader, -70.0, 6.0, 0.0),
            -70.0
        ));
        assert!(close(
            scale_to_plain(ParamScale::Gain, -144.0, 36.0, 0.5),
            0.0
        ));
        assert!(close(
            scale_to_plain(ParamScale::Gain, -144.0, 36.0, 0.75),
            18.0
        ));
        assert!(close(
            scale_to_plain(ParamScale::Gain, -144.0, 36.0, 0.25),
            -24.953_298_500_158_03
        ));
        assert!(close(
            scale_to_plain(ParamScale::Gain, -144.0, 36.0, 0.0),
            -144.0
        ));
        for scale in [
            ParamScale::Linear,
            ParamScale::Log,
            ParamScale::Power { exponent: 3.0 },
            ParamScale::Fader,
            ParamScale::Gain,
        ] {
            let (min, max) = match scale {
                ParamScale::Fader => (-70.0, 6.0),
                ParamScale::Gain => (-144.0, 36.0),
                _ => (20.0, 20000.0),
            };
            for i in 1..10 {
                let n = i as f64 / 10.0;
                let back = scale_to_normalized(scale, min, max, scale_to_plain(scale, min, max, n));
                assert!((back - n).abs() < 1e-9, "{scale:?} {n} -> {back}");
            }
        }
    }
}
