//! Device chains, parameters and device descriptors.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{
    BuiltinDevice, BuiltinDeviceType, DeviceId, MediaId, ParamId, PluginFormat, TrackId,
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
    Duplicate {
        id: DeviceId,
        new_id: DeviceId,
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
    /// Roadmap v2 (`sidechain`): feed `source`'s post-fader output into the device's
    /// sidechain input (`None` = off). `InvalidArgument` if it would create a routing cycle
    /// or the device has no sidechain input (`sidechain_inputs == 0`). Undoable.
    SetSidechain {
        device: DeviceId,
        source: Option<TrackId>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceSpec {
    Builtin {
        device: BuiltinDevice,
    },
    /// A plugin from the plugin list (`PluginDescriptor { format, id }`). `sandboxed: None`
    /// = user default. `format` omitted/`null` = CLAP (the only format before VST3/AU).
    Plugin {
        plugin_id: String,
        sandboxed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        format: Option<PluginFormat>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum DeviceCategory {
    Instrument,
    AudioEffect,
    NoteEffect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceTypeRef {
    Builtin { device: BuiltinDeviceType },
    Plugin { plugin_id: String },
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
    };
    n.clamp(0.0, 1.0)
}

impl ParamInfo {
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
        for scale in [
            ParamScale::Linear,
            ParamScale::Log,
            ParamScale::Power { exponent: 3.0 },
            ParamScale::Fader,
        ] {
            let (min, max) = if scale == ParamScale::Fader {
                (-70.0, 6.0)
            } else {
                (20.0, 20000.0)
            };
            for i in 1..10 {
                let n = i as f64 / 10.0;
                let back = scale_to_normalized(scale, min, max, scale_to_plain(scale, min, max, n));
                assert!((back - n).abs() < 1e-9, "{scale:?} {n} -> {back}");
            }
        }
    }
}
