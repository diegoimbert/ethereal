//! Device instances on a track's device chain (built-in devices and plugins).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::drum_rack::SliceSettings;
use crate::external::{ExternalRouting, IrSource};
use crate::ids::{DeviceId, DrumPadId, MediaId, RackChainId, TrackId};
use crate::multisampler::SampleZone;
use crate::value::{Base64Bytes, OrderKey, ParamId};

/// A device on a track. Chain order = `order` among devices with the same `track`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Device {
    pub id: DeviceId,
    pub track: TrackId,
    pub order: OrderKey,
    pub name: String,
    /// Bypass switch (Ableton's device on/off).
    pub enabled: bool,
    pub kind: DeviceKind,
    /// Parameter values in *plain* units (Hz, dB, ms, ...). Missing = the param's default.
    /// For plugins this is a mirror for UI/automation; the plugin state blob is authoritative
    /// on load.
    pub params: BTreeMap<ParamId, f64>,
    /// Sidechain source (roadmap v2, `sidechain`; `.ether` v3): the post-fader output of this
    /// track feeds the device's sidechain input. Ignored by devices without one
    /// (`DeviceDescriptor::sidechain_inputs == 0`). Not allowed on pad devices (`pad` set):
    /// the model rejects it. Sidechain edges count as routing edges (no cycles); see
    /// CONTRACTS.md §11.10 for processing order and PDC.
    pub sidechain: Option<TrackId>,
    /// Drum pad whose chain this device is on (roadmap v2, `drum-rack`; `.ether` v3). `None` =
    /// the track's own chain. Pad devices keep `track` = the rack's track; chain order is
    /// `order` among devices with the same `(track, pad)`.
    pub pad: Option<DrumPadId>,
    /// Rack chain this device is on (v0.2, `racks-modulation`; see [`crate::rack`]). `None` =
    /// not in a rack. Mutually exclusive with `pad`; `track` = the rack's track. Omitted from
    /// JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub chain: Option<RackChainId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceKind {
    Builtin { device: BuiltinDevice },
    Plugin { plugin: PluginInstance },
}

/// Built-in devices and their non-parameter data.
///
/// Parameters are *not* here (they are `Device::params`, described by the device type's
/// `DeviceDescriptor` from `ether-devices`). Variants after `Delay` are roadmap v2: their
/// parameter lists are defined by the owning node (`devices-2`, `drum-rack`); ids are
/// append-only, never renumbered once released. Variants after `DrumRack` are v0.2
/// (contracts-3): their param tables are frozen in their `ether-devices` module (append-only)
/// and implemented by the owning node (docs/ROADMAP.md "v0.2"). Variants after
/// `MidiEffectRack` are v0.3 (contracts-4), same rules (docs/ROADMAP.md "v0.3").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BuiltinDevice {
    Synth,
    /// `slices` (roadmap v2, `drum-rack`; `.ether` v3): slice markers + slice mode, see
    /// [`crate::drum_rack`].
    Sampler {
        sample: Option<MediaId>,
        slices: SliceSettings,
    },
    Compressor,
    Delay,
    /// Parametric EQ with a fixed number of bands (`devices-2`).
    Eq,
    /// Algorithmic reverb (`devices-2`).
    Reverb,
    /// Brickwall/lookahead limiter (`devices-2`).
    Limiter,
    /// Gain, pan, stereo width, phase invert, mono (`devices-2`).
    Utility,
    /// Instrument hosting [`crate::drum_rack::DrumPad`]s, each with its own chain
    /// (`drum-rack`).
    DrumRack,
    // --- v0.2 (contracts-3); owning node in parentheses ---
    /// Polyphonic subtractive/wavetable synth (`synth-2`).
    PolySynth,
    /// Key/velocity-zoned sampler with round robin (`multisampler`).
    MultiSampler {
        zones: Vec<SampleZone>,
    },
    /// Distortion/saturation with several curves (`fx-color`).
    Saturator,
    /// Bit depth and sample-rate reduction (`fx-color`).
    Bitcrusher,
    /// Multimode filter with LFO and envelope follower (`fx-color`).
    AutoFilter,
    /// Chorus / ensemble (`fx-modulation`).
    Chorus,
    /// Phaser (`fx-modulation`).
    Phaser,
    /// Flanger (`fx-modulation`).
    Flanger,
    /// Tremolo and auto-pan (`fx-modulation`).
    Tremolo,
    /// Gate / expander (`fx-dynamics`).
    Gate,
    /// Three-band compressor (`fx-dynamics`).
    MultibandCompressor,
    /// Attack/sustain transient shaper (`fx-dynamics`).
    TransientShaper,
    /// Spectrum analyzer, audio pass-through (`fx-analysis`).
    SpectrumAnalyzer,
    /// Chromatic tuner, audio pass-through (`fx-analysis`).
    Tuner,
    /// Arpeggiator, MIDI effect (`midi-fx`).
    Arpeggiator,
    /// Chord generator, MIDI effect (`midi-fx`).
    Chord,
    /// Scale quantizer (track/project `MusicalScale`), MIDI effect (`midi-fx`).
    ScaleQuantize,
    /// Note length / gate, MIDI effect (`midi-fx`).
    NoteLength,
    /// Velocity curve/range, MIDI effect (`midi-fx`).
    Velocity,
    /// Random pitch/velocity/timing (humanize), MIDI effect (`midi-fx`).
    Randomizer,
    /// Rack of parallel instrument chains (`racks-modulation`, [`crate::rack`]).
    InstrumentRack,
    /// Rack of parallel audio effect chains (`racks-modulation`).
    AudioEffectRack,
    /// Rack of parallel MIDI effect chains (`racks-modulation`).
    MidiEffectRack,
    // --- v0.3 (contracts-4); owning node in parentheses ---
    /// Partitioned convolution reverb (`fx-space`). `ir: None` = no IR loaded (dry only).
    ConvolutionReverb {
        ir: Option<IrSource>,
    },
    /// Hardware synth: MIDI out + audio return (`external-instrument`).
    ExternalInstrument {
        routing: ExternalRouting,
    },
    /// Hardware effect: audio send + return (`external-instrument`).
    ExternalAudioEffect {
        routing: ExternalRouting,
    },
}

/// Data-less discriminant of [`BuiltinDevice`] (used in descriptors and factories).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum BuiltinDeviceType {
    Synth,
    Sampler,
    Compressor,
    Delay,
    Eq,
    Reverb,
    Limiter,
    Utility,
    DrumRack,
    PolySynth,
    MultiSampler,
    Saturator,
    Bitcrusher,
    AutoFilter,
    Chorus,
    Phaser,
    Flanger,
    Tremolo,
    Gate,
    MultibandCompressor,
    TransientShaper,
    SpectrumAnalyzer,
    Tuner,
    Arpeggiator,
    Chord,
    ScaleQuantize,
    NoteLength,
    Velocity,
    Randomizer,
    InstrumentRack,
    AudioEffectRack,
    MidiEffectRack,
    // --- v0.3 (contracts-4) ---
    ConvolutionReverb,
    ExternalInstrument,
    ExternalAudioEffect,
}

impl BuiltinDeviceType {
    /// Every built-in type, in `DeviceCommand::ListBuiltin` order.
    pub const ALL: [BuiltinDeviceType; 35] = [
        Self::Synth,
        Self::Sampler,
        Self::Compressor,
        Self::Delay,
        Self::Eq,
        Self::Reverb,
        Self::Limiter,
        Self::Utility,
        Self::DrumRack,
        Self::PolySynth,
        Self::MultiSampler,
        Self::Saturator,
        Self::Bitcrusher,
        Self::AutoFilter,
        Self::Chorus,
        Self::Phaser,
        Self::Flanger,
        Self::Tremolo,
        Self::Gate,
        Self::MultibandCompressor,
        Self::TransientShaper,
        Self::SpectrumAnalyzer,
        Self::Tuner,
        Self::Arpeggiator,
        Self::Chord,
        Self::ScaleQuantize,
        Self::NoteLength,
        Self::Velocity,
        Self::Randomizer,
        Self::InstrumentRack,
        Self::AudioEffectRack,
        Self::MidiEffectRack,
        Self::ConvolutionReverb,
        Self::ExternalInstrument,
        Self::ExternalAudioEffect,
    ];

    /// Rack types ([`crate::rack`]).
    pub fn is_rack(self) -> bool {
        matches!(
            self,
            Self::InstrumentRack | Self::AudioEffectRack | Self::MidiEffectRack
        )
    }

    /// Stable lowercase key (preset folders, factory preset ids): the variant name in kebab
    /// case (`poly-synth`, `multiband-compressor`).
    pub fn key(self) -> String {
        let name = format!("{self:?}");
        let mut out = String::with_capacity(name.len() + 4);
        for (i, c) in name.chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    out.push('-');
                }
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }
}

impl BuiltinDevice {
    pub fn device_type(&self) -> BuiltinDeviceType {
        match self {
            Self::Synth => BuiltinDeviceType::Synth,
            Self::Sampler { .. } => BuiltinDeviceType::Sampler,
            Self::Compressor => BuiltinDeviceType::Compressor,
            Self::Delay => BuiltinDeviceType::Delay,
            Self::Eq => BuiltinDeviceType::Eq,
            Self::Reverb => BuiltinDeviceType::Reverb,
            Self::Limiter => BuiltinDeviceType::Limiter,
            Self::Utility => BuiltinDeviceType::Utility,
            Self::DrumRack => BuiltinDeviceType::DrumRack,
            Self::MultiSampler { .. } => BuiltinDeviceType::MultiSampler,
            Self::PolySynth => BuiltinDeviceType::PolySynth,
            Self::Saturator => BuiltinDeviceType::Saturator,
            Self::Bitcrusher => BuiltinDeviceType::Bitcrusher,
            Self::AutoFilter => BuiltinDeviceType::AutoFilter,
            Self::Chorus => BuiltinDeviceType::Chorus,
            Self::Phaser => BuiltinDeviceType::Phaser,
            Self::Flanger => BuiltinDeviceType::Flanger,
            Self::Tremolo => BuiltinDeviceType::Tremolo,
            Self::Gate => BuiltinDeviceType::Gate,
            Self::MultibandCompressor => BuiltinDeviceType::MultibandCompressor,
            Self::TransientShaper => BuiltinDeviceType::TransientShaper,
            Self::SpectrumAnalyzer => BuiltinDeviceType::SpectrumAnalyzer,
            Self::Tuner => BuiltinDeviceType::Tuner,
            Self::Arpeggiator => BuiltinDeviceType::Arpeggiator,
            Self::Chord => BuiltinDeviceType::Chord,
            Self::ScaleQuantize => BuiltinDeviceType::ScaleQuantize,
            Self::NoteLength => BuiltinDeviceType::NoteLength,
            Self::Velocity => BuiltinDeviceType::Velocity,
            Self::Randomizer => BuiltinDeviceType::Randomizer,
            Self::InstrumentRack => BuiltinDeviceType::InstrumentRack,
            Self::AudioEffectRack => BuiltinDeviceType::AudioEffectRack,
            Self::MidiEffectRack => BuiltinDeviceType::MidiEffectRack,
            Self::ConvolutionReverb { .. } => BuiltinDeviceType::ConvolutionReverb,
            Self::ExternalInstrument { .. } => BuiltinDeviceType::ExternalInstrument,
            Self::ExternalAudioEffect { .. } => BuiltinDeviceType::ExternalAudioEffect,
        }
    }

    /// A fresh instance of `ty` with default data (empty sampler, no slices).
    pub fn new(ty: BuiltinDeviceType) -> Self {
        match ty {
            BuiltinDeviceType::Synth => Self::Synth,
            BuiltinDeviceType::Sampler => Self::Sampler {
                sample: None,
                slices: SliceSettings::default(),
            },
            BuiltinDeviceType::Compressor => Self::Compressor,
            BuiltinDeviceType::Delay => Self::Delay,
            BuiltinDeviceType::Eq => Self::Eq,
            BuiltinDeviceType::Reverb => Self::Reverb,
            BuiltinDeviceType::Limiter => Self::Limiter,
            BuiltinDeviceType::Utility => Self::Utility,
            BuiltinDeviceType::DrumRack => Self::DrumRack,
            BuiltinDeviceType::MultiSampler => Self::MultiSampler { zones: Vec::new() },
            BuiltinDeviceType::PolySynth => Self::PolySynth,
            BuiltinDeviceType::Saturator => Self::Saturator,
            BuiltinDeviceType::Bitcrusher => Self::Bitcrusher,
            BuiltinDeviceType::AutoFilter => Self::AutoFilter,
            BuiltinDeviceType::Chorus => Self::Chorus,
            BuiltinDeviceType::Phaser => Self::Phaser,
            BuiltinDeviceType::Flanger => Self::Flanger,
            BuiltinDeviceType::Tremolo => Self::Tremolo,
            BuiltinDeviceType::Gate => Self::Gate,
            BuiltinDeviceType::MultibandCompressor => Self::MultibandCompressor,
            BuiltinDeviceType::TransientShaper => Self::TransientShaper,
            BuiltinDeviceType::SpectrumAnalyzer => Self::SpectrumAnalyzer,
            BuiltinDeviceType::Tuner => Self::Tuner,
            BuiltinDeviceType::Arpeggiator => Self::Arpeggiator,
            BuiltinDeviceType::Chord => Self::Chord,
            BuiltinDeviceType::ScaleQuantize => Self::ScaleQuantize,
            BuiltinDeviceType::NoteLength => Self::NoteLength,
            BuiltinDeviceType::Velocity => Self::Velocity,
            BuiltinDeviceType::Randomizer => Self::Randomizer,
            BuiltinDeviceType::InstrumentRack => Self::InstrumentRack,
            BuiltinDeviceType::AudioEffectRack => Self::AudioEffectRack,
            BuiltinDeviceType::MidiEffectRack => Self::MidiEffectRack,
            BuiltinDeviceType::ConvolutionReverb => Self::ConvolutionReverb { ir: None },
            BuiltinDeviceType::ExternalInstrument => Self::ExternalInstrument {
                routing: ExternalRouting::default(),
            },
            BuiltinDeviceType::ExternalAudioEffect => Self::ExternalAudioEffect {
                routing: ExternalRouting::default(),
            },
        }
    }

    /// Media referenced by the device kind (sampler sample, multisampler zones, convolution
    /// IR).
    pub fn media(&self) -> Vec<MediaId> {
        match self {
            Self::Sampler {
                sample: Some(m), ..
            } => vec![*m],
            Self::MultiSampler { zones } => zones.iter().filter_map(|z| z.media).collect(),
            Self::ConvolutionReverb {
                ir: Some(IrSource::Media { media }),
            } => vec![*media],
            _ => Vec::new(),
        }
    }
}

/// A plugin instance. `state` is the opaque blob from the plugin's state extension.
///
/// The document never stores where a plugin lives on disk: the host resolves
/// `(format, plugin_id)` against its scanned plugin catalog, so a project opens on any machine
/// that has the plugin installed (see `docs/PLUGIN-FORMATS.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginInstance {
    pub format: PluginFormat,
    /// Format-specific plugin id (see [`PluginFormat`] for the per-format convention), e.g.
    /// `com.u-he.diva` (CLAP), `565354416D627261736F6E6963000000` (VST3),
    /// `aufx:dely:appl` (AU).
    pub plugin_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// Run out-of-process (crash isolation, +1 block latency).
    pub sandboxed: bool,
    /// Last saved plugin state. `None` = fresh instance.
    pub state: Option<Base64Bytes>,
}

/// Plugin format. Serialized as the plain variant name (`"Clap"`, `"Vst3"`, `"Au"`); these
/// tags are stable (`.ether` files and the wire depend on them) and new formats are additive.
///
/// `plugin_id` convention per format (what [`PluginInstance::plugin_id`] and the scanner's
/// `PluginDescriptor::id` hold):
/// - [`PluginFormat::Clap`]: the CLAP plugin id, reverse-DNS (`com.u-he.diva`).
/// - [`PluginFormat::Vst3`]: the audio-processor class id (`PClassInfo::cid`) in the SDK's
///   canonical `FUID::toString` form (the `CID` in `moduleinfo.json`): 32 **uppercase** hex
///   characters, the words `l1 l2 l3 l4` of `INLINE_UID`, no separators
///   (`565354416D627261736F6E6963000000`). Identical on every OS (on Windows the in-memory
///   TUID uses the COM GUID layout; `ether_vst3::class_id_to_string` converts). The `.vst3`
///   bundle path is not part of the id: the host finds it in its plugin catalog.
/// - [`PluginFormat::Au`]: the `AudioComponentDescription` four-char codes
///   `type:subtype:manufacturer` (`aufx:dely:appl` = Apple AUDelay). Each code is exactly
///   four characters (Mac OS Roman, printable), kept verbatim (case-sensitive, spaces kept).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
pub enum PluginFormat {
    Clap,
    Vst3,
    Au,
}

impl PluginFormat {
    /// Every format, in scan/display order.
    pub const ALL: [PluginFormat; 3] = [PluginFormat::Clap, PluginFormat::Vst3, PluginFormat::Au];

    /// Lowercase CLI/log name: `clap`, `vst3`, `au` (e.g. `ether-sandbox-helper --format`).
    pub fn as_str(self) -> &'static str {
        match self {
            PluginFormat::Clap => "clap",
            PluginFormat::Vst3 => "vst3",
            PluginFormat::Au => "au",
        }
    }

    /// Inverse of [`PluginFormat::as_str`] (case-insensitive).
    pub fn parse(s: &str) -> Option<PluginFormat> {
        Self::ALL
            .into_iter()
            .find(|f| f.as_str().eq_ignore_ascii_case(s))
    }
}

impl std::fmt::Display for PluginFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
