//! Device instances on a track's device chain (built-in devices and plugins).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::drum_rack::SliceSettings;
use crate::external::{ExternalRouting, IrSource};
use crate::ids::{DeviceId, DrumPadId, MediaId, RackChainId, TrackId};
use crate::lfo_curve::{LfoCurve, ShaperCurves};
use crate::multisampler::SampleZone;
use crate::value::{Base64Bytes, OrderKey, ParamId};

/// The factory impulse response a new convolution reverb starts on (with none it
/// passed audio through untouched). The UI's `builtinDevice` mirrors it.
pub const DEFAULT_CONVOLUTION_IR: &str = "hall";

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
    /// Sidechain source (roadmap v2, `sidechain`; pre-release `.ether` v3): this track's signal (at
    /// [`Device::sidechain_tap`], post-fader by default) feeds the device's sidechain input.
    /// Ignored by devices without one (`DeviceDescriptor::sidechain_inputs == 0`). Any device
    /// may take one, at any depth (track chain, drum pads, rack chains). Sidechain edges count
    /// as routing edges (no cycles); see CONTRACTS.md §11.10 for processing order and PDC and
    /// [`SidechainTap`] for the same-track rules.
    pub sidechain: Option<TrackId>,
    /// Where the sidechain taps its source track (v0.4). `None` = the track's post-fader
    /// output (the original meaning). Meaningless without `sidechain`. Omitted from JSON when
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sidechain_tap: Option<SidechainTap>,
    /// Drum pad whose chain this device is on (roadmap v2, `drum-rack`; pre-release `.ether` v3). `None` =
    /// the track's own chain. Pad devices keep `track` = the rack's track; chain order is
    /// `order` among devices with the same `(track, pad)`.
    pub pad: Option<DrumPadId>,
    /// Rack chain this device is on (v0.2; see [`crate::rack`]). `None` =
    /// not in a rack. Mutually exclusive with `pad`; `track` = the rack's track. Omitted from
    /// JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub chain: Option<RackChainId>,
    /// MIDI sidechain: the device's note input hears this MIDI track's notes
    /// ([`MidiSidechain`]) instead of the chain's. `None` = off. Omitted from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub midi_sidechain: Option<MidiSidechain>,
}

/// A device's MIDI sidechain (Live's "MIDI From" on an effect: Shifter's MIDI pitch mode,
/// vocoders, plugin effects with a note input): the notes of a MIDI track feed the device's
/// note input, and replace the notes reaching it through its own chain.
///
/// Rules (model invariants): `track` is a MIDI track (any one, the device's own included:
/// both points are produced before any device that can hear them runs); a MIDI sidechain
/// is a routing edge `track → the device's track` (no cycles, with audio routing and
/// sidechains). Devices declare a note input with `DeviceDescriptor::midi_input` (the
/// controller refuses the others).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MidiSidechain {
    pub track: TrackId,
    /// Missing = `PostFx`.
    #[serde(default)]
    pub tap: MidiTap,
}

/// Where a MIDI sidechain taps its source track's notes (Live's "Pre FX" / "Post FX").
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS,
)]
pub enum MidiTap {
    /// The notes entering the track's chain: its clips and its monitored input.
    PreFx,
    /// The notes after its MIDI effects: what its instrument plays (for a chain of MIDI
    /// effects only, what leaves it).
    #[default]
    PostFx,
}

/// Where a device's sidechain taps its source track (v0.4): a point of the track or of one
/// of its rack chains / drum pads (Live's "Audio From" second chooser: "Pre FX", "Post FX",
/// "Post Mixer", and "<Rack> | <Chain> | <point>" for chains).
///
/// Points (the engine's taps, `ether-core/src/sidechain.rs`):
/// - `PreFx`: after the first instrument of the track (or chain) — a rack or drum rack counts
///   — and before its audio effects; where there is no instrument, the input of the track
///   (its clips, input bus and monitored input) or of the chain (silence for instrument
///   rack chains and pads, the rack's input for audio effect racks).
/// - `PostFx`: after the device chain (for a chain: aligned with the rack's other chains,
///   before the chain's volume/pan/mute).
/// - `PostFader`: after the fader, pan and mute (for a chain: after its volume, pan, mute
///   and, on a pad, choke).
///
/// Rules (model invariants): the chain must be on the source track. When the source is the
/// device's own track, only signals finished before the device runs are allowed (anything
/// else would be a cycle): the track's `PreFx`, or a chain of a rack that comes before the
/// device (or before the rack/pad holding it) in the same chain, or another chain of a rack
/// the device is in (any depth): the keyed chain runs first (`Project::chain_order`; chains
/// never key each other in a loop), and a chain of the rack holding the device is tapped
/// before the rack's alignment, so only its `PreFx` / `PostFx` (not `PostFader`). A device
/// never keys its own chain of a rack holding it; pads of a drum rack never key each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SidechainTap {
    pub point: crate::track::InputTap,
    /// A rack chain or drum pad of the source track; `None` = the track itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub chain: Option<SidechainChain>,
}

impl SidechainTap {
    /// The original sidechain point: the source track's post-fader output.
    pub const POST_FADER: SidechainTap = SidechainTap {
        point: crate::track::InputTap::PostFader,
        chain: None,
    };
}

/// A chain of the sidechain source track ([`SidechainTap::chain`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SidechainChain {
    /// A chain of an instrument / audio effect / MIDI effect rack.
    Rack { chain: RackChainId },
    /// A drum pad's chain.
    Pad { pad: DrumPadId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceKind {
    Builtin {
        device: BuiltinDevice,
    },
    Plugin {
        plugin: PluginInstance,
    },
    /// A device that could not be created, kept as a placeholder ([`MissingDevice`]).
    /// Additive: `.ether` readers before it reject such a document.
    Missing {
        missing: MissingDevice,
    },
}

impl DeviceKind {
    /// An instrument / audio effect / MIDI effect rack (not a drum rack).
    pub fn is_chain_rack(&self) -> bool {
        matches!(self, DeviceKind::Builtin { device } if device.device_type().is_rack())
    }

    /// A drum rack ([`crate::drum_rack`]).
    pub fn is_drum_rack(&self) -> bool {
        matches!(
            self,
            DeviceKind::Builtin {
                device: BuiltinDevice::DrumRack { .. }
            }
        )
    }

    /// Any device holding device chains: a chain rack or a drum rack (a nesting level).
    pub fn is_any_rack(&self) -> bool {
        self.is_chain_rack() || self.is_drum_rack()
    }
}

/// A placeholder for a device Ethereal could not create (owner request "missing devices"):
/// a device of another DAW without an Ethereal equivalent (Live's Hybrid Reverb), kept so
/// that its place in the chain, its parameter values (`Device::params`, plain units of
/// [`MissingDevice::params`]) and its automation lanes (`AutomationTarget::DeviceParam`
/// with those ids) survive until it is resolved (`Resolve::Apply`: replaced by a plugin or
/// a built-in device, values and lanes carried over by parameter name). The engine runs no
/// node for it: audio passes through; an instrument placeholder is silent.
///
/// Plugins that are not installed stay `DeviceKind::Plugin` (the host resolves them by id).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MissingDevice {
    /// What it was, for the user ("Live's Hybrid Reverb").
    pub description: String,
    /// Identity in its source, the same for every device of that kind (Live's device tag
    /// `"Hybrid"`): the resolver groups placeholders by it.
    pub source_id: String,
    /// The application it comes from ("Ableton Live 12.1"), when known.
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub role: MissingRole,
    /// The parameters the source gave (the placeholder's own ids).
    #[serde(default)]
    pub params: Vec<MissingParam>,
}

/// What a [`MissingDevice`] was (where a replacement fits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum MissingRole {
    Instrument,
    #[default]
    AudioEffect,
    NoteEffect,
}

/// One parameter of a [`MissingDevice`]: its source name and plain range. Automation lanes
/// hold normalized values over `min..=max` (linear).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MissingParam {
    pub id: ParamId,
    pub name: String,
    pub min: f64,
    pub max: f64,
}

impl MissingParam {
    /// Plain → normalized 0..=1 (linear; a flat or invalid range gives 0).
    pub fn normalize(&self, plain: f64) -> f64 {
        let span = self.max - self.min;
        if !span.is_finite() || span.abs() < f64::EPSILON || !plain.is_finite() {
            return 0.0;
        }
        ((plain - self.min) / span).clamp(0.0, 1.0)
    }

    /// Normalized 0..=1 → plain.
    pub fn plain(&self, normalized: f64) -> f64 {
        self.min + normalized.clamp(0.0, 1.0) * (self.max - self.min)
    }
}

/// Built-in devices and their non-parameter data.
///
/// Parameters are *not* here (they are `Device::params`, described by the device type's
/// `DeviceDescriptor` from `ether-devices`). Variants after `Delay` are roadmap v2: their
/// parameter lists are defined in `ether-devices` (docs/DEVICES.md); ids are
/// append-only, never renumbered once released. Variants after `DrumRack` are v0.2:
/// their param tables are frozen in their `ether-devices` module (append-only)
/// and implemented there (docs/DEVICES.md). Variants after `MidiEffectRack` are v0.3,
/// same rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BuiltinDevice {
    Synth,
    /// `slices` (roadmap v2, `drum-rack`; pre-release `.ether` v3): slice markers + slice mode, see
    /// [`crate::drum_rack`].
    Sampler {
        sample: Option<MediaId>,
        slices: SliceSettings,
    },
    Compressor,
    Delay,
    /// Parametric EQ with a fixed number of bands.
    Eq,
    /// Algorithmic reverb.
    Reverb,
    /// Brickwall/lookahead limiter.
    Limiter,
    /// Gain, pan, stereo width, phase invert, mono.
    Utility,
    /// Instrument hosting [`crate::drum_rack::DrumPad`]s, each with its own chain
    /// (`drum-rack`).
    DrumRack {
        /// Macro bank settings (visible count, names; `None` = the defaults,
        /// [`crate::rack::RackMacros`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        macros: Option<crate::rack::RackMacros>,
    },
    // --- v0.2 ---
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
    /// Transpose by semitones within a note range, MIDI effect (`midi-fx`).
    Pitch,
    /// Rack of parallel instrument chains ([`crate::rack`]).
    InstrumentRack {
        /// Macro bank settings (visible count, names; `None` = the defaults,
        /// [`crate::rack::RackMacros`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        macros: Option<crate::rack::RackMacros>,
    },
    /// Rack of parallel audio effect chains.
    AudioEffectRack {
        /// Macro bank settings (visible count, names; `None` = the defaults,
        /// [`crate::rack::RackMacros`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        macros: Option<crate::rack::RackMacros>,
    },
    /// Rack of parallel MIDI effect chains.
    MidiEffectRack {
        /// Macro bank settings (visible count, names; `None` = the defaults,
        /// [`crate::rack::RackMacros`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        macros: Option<crate::rack::RackMacros>,
    },
    // --- v0.3 ---
    /// Partitioned convolution reverb (`fx-space`). `ir: None` = no IR loaded (dry only); a
    /// new one ([`BuiltinDevice::new`]) starts on the factory [`DEFAULT_CONVOLUTION_IR`].
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
    // --- after v0.3 ---
    /// Four-operator FM synth with 11 algorithms (`fm-synth`).
    FmSynth,
    /// Pitch wobble: LFO-modulated delay line (vibrato, wow and flutter; `fx-modulation`).
    Vibrato,
    /// Volume shaped by a drawn LFO cycle, tempo-synced (sidechain pumping, gates,
    /// tremolo; `fx-modulation`, [`crate::lfo_curve`]).
    VolumeLfo {
        /// Omitted (an agent's `{"type": "VolumeLfo"}`) = the default pump.
        #[serde(default)]
        curve: LfoCurve,
    },
    /// Drops every note (and, with `Block Everything`, all other MIDI), MIDI effect
    /// (`midi-fx`).
    NoteBlocker,
    /// Digital degradation: a very short delay modulated by a sine or band-limited noise
    /// (`fx-color`).
    Erosion,
    /// Guitar-pedal distortion: overdrive / distortion / fuzz with a tone stack
    /// (`fx-color`).
    Pedal,
    /// Granular delay: transposed, sprayed, randomly detuned grains with feedback
    /// (`grain-delay`).
    GrainDelay,
    /// Pitch shifter, frequency shifter and ring modulator (`shifter`).
    Shifter,
    /// Loudness meter: momentary / short-term / integrated LUFS, loudness range and true
    /// peak with a history (`fx-analysis`).
    Loudness,
    /// Mastering maximizer: a threshold-driven lookahead true-peak brickwall limiter
    /// (`maximizer`).
    Maximizer,
    /// Volume, filter cutoff and pan each shaped by a drawn, tempo-synced cycle
    /// (`shaper`, [`crate::lfo_curve::ShaperCurves`]).
    Shaper {
        /// Omitted (an agent's `{"type": "Shaper"}`) = the defaults.
        #[serde(default)]
        curves: ShaperCurves,
    },
    /// Monophonic pitch correction: detected pitch snapped to a scale or the note input,
    /// formant-preserving shift (`pitch-correction`).
    PitchCorrection,
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
    Pitch,
    InstrumentRack,
    AudioEffectRack,
    MidiEffectRack,
    // --- v0.3 ---
    ConvolutionReverb,
    ExternalInstrument,
    ExternalAudioEffect,
    // --- after v0.3 ---
    FmSynth,
    Vibrato,
    VolumeLfo,
    NoteBlocker,
    Erosion,
    Pedal,
    GrainDelay,
    Shifter,
    Loudness,
    Maximizer,
    Shaper,
    PitchCorrection,
}

impl BuiltinDeviceType {
    /// Every built-in type, in `DeviceCommand::ListBuiltin` order.
    pub const ALL: [BuiltinDeviceType; 48] = [
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
        Self::Pitch,
        Self::InstrumentRack,
        Self::AudioEffectRack,
        Self::MidiEffectRack,
        Self::ConvolutionReverb,
        Self::ExternalInstrument,
        Self::ExternalAudioEffect,
        Self::FmSynth,
        Self::Vibrato,
        Self::VolumeLfo,
        Self::NoteBlocker,
        Self::Erosion,
        Self::Pedal,
        Self::GrainDelay,
        Self::Shifter,
        Self::Loudness,
        Self::Maximizer,
        Self::Shaper,
        Self::PitchCorrection,
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
            Self::DrumRack { .. } => BuiltinDeviceType::DrumRack,
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
            Self::Pitch => BuiltinDeviceType::Pitch,
            Self::InstrumentRack { .. } => BuiltinDeviceType::InstrumentRack,
            Self::AudioEffectRack { .. } => BuiltinDeviceType::AudioEffectRack,
            Self::MidiEffectRack { .. } => BuiltinDeviceType::MidiEffectRack,
            Self::ConvolutionReverb { .. } => BuiltinDeviceType::ConvolutionReverb,
            Self::ExternalInstrument { .. } => BuiltinDeviceType::ExternalInstrument,
            Self::ExternalAudioEffect { .. } => BuiltinDeviceType::ExternalAudioEffect,
            Self::FmSynth => BuiltinDeviceType::FmSynth,
            Self::Vibrato => BuiltinDeviceType::Vibrato,
            Self::VolumeLfo { .. } => BuiltinDeviceType::VolumeLfo,
            Self::NoteBlocker => BuiltinDeviceType::NoteBlocker,
            Self::Erosion => BuiltinDeviceType::Erosion,
            Self::Pedal => BuiltinDeviceType::Pedal,
            Self::GrainDelay => BuiltinDeviceType::GrainDelay,
            Self::Shifter => BuiltinDeviceType::Shifter,
            Self::Loudness => BuiltinDeviceType::Loudness,
            Self::Maximizer => BuiltinDeviceType::Maximizer,
            Self::Shaper { .. } => BuiltinDeviceType::Shaper,
            Self::PitchCorrection => BuiltinDeviceType::PitchCorrection,
        }
    }

    /// A rack's macro bank settings (the defaults when unset); `None` for non-rack devices.
    pub fn rack_macros(&self) -> Option<crate::rack::RackMacros> {
        match self {
            Self::DrumRack { macros }
            | Self::InstrumentRack { macros }
            | Self::AudioEffectRack { macros }
            | Self::MidiEffectRack { macros } => Some(macros.clone().unwrap_or_default()),
            _ => None,
        }
    }

    /// `self` with its macro bank settings replaced (normalized; the defaults are stored as
    /// `None`). Non-rack devices are returned unchanged.
    pub fn with_rack_macros(mut self, new: crate::rack::RackMacros) -> Self {
        let new = new.normalized();
        let new = (!new.is_default()).then_some(new);
        match &mut self {
            Self::DrumRack { macros }
            | Self::InstrumentRack { macros }
            | Self::AudioEffectRack { macros }
            | Self::MidiEffectRack { macros } => *macros = new,
            _ => {}
        }
        self
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
            BuiltinDeviceType::DrumRack => Self::DrumRack { macros: None },
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
            BuiltinDeviceType::Pitch => Self::Pitch,
            BuiltinDeviceType::InstrumentRack => Self::InstrumentRack { macros: None },
            BuiltinDeviceType::AudioEffectRack => Self::AudioEffectRack { macros: None },
            BuiltinDeviceType::MidiEffectRack => Self::MidiEffectRack { macros: None },
            BuiltinDeviceType::ConvolutionReverb => Self::ConvolutionReverb {
                ir: Some(IrSource::Factory {
                    id: DEFAULT_CONVOLUTION_IR.to_owned(),
                }),
            },
            BuiltinDeviceType::ExternalInstrument => Self::ExternalInstrument {
                routing: ExternalRouting::default(),
            },
            BuiltinDeviceType::ExternalAudioEffect => Self::ExternalAudioEffect {
                routing: ExternalRouting::default(),
            },
            BuiltinDeviceType::FmSynth => Self::FmSynth,
            BuiltinDeviceType::Vibrato => Self::Vibrato,
            BuiltinDeviceType::VolumeLfo => Self::VolumeLfo {
                curve: LfoCurve::default(),
            },
            BuiltinDeviceType::NoteBlocker => Self::NoteBlocker,
            BuiltinDeviceType::Erosion => Self::Erosion,
            BuiltinDeviceType::Pedal => Self::Pedal,
            BuiltinDeviceType::GrainDelay => Self::GrainDelay,
            BuiltinDeviceType::Shifter => Self::Shifter,
            BuiltinDeviceType::Loudness => Self::Loudness,
            BuiltinDeviceType::Maximizer => Self::Maximizer,
            BuiltinDeviceType::Shaper => Self::Shaper {
                curves: ShaperCurves::default(),
            },
            BuiltinDeviceType::PitchCorrection => Self::PitchCorrection,
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
    /// Never sleep on silence (CONTRACTS.md §13.19): for plugins that make sound out of
    /// nothing without reporting a tail. Off by default; omitted from JSON when off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub always_process: bool,
    /// Last saved plugin state. `None` = fresh instance.
    pub state: Option<Base64Bytes>,
}

/// Plugin format. Serialized as the plain variant name (`"Clap"`, `"Vst3"`, `"Au"`, `"Vst2"`,
/// `"Webclap"`); these
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
/// - [`PluginFormat::Vst2`]: the plugin's `AEffect::uniqueID` as 8 **uppercase** hex digits
///   of its 32-bit value (`'EtG2'` → `45744732`); shell sub-plugins use their own uniqueID.
///   The library path (`.dll`/`.so`/`.vst`) is not part of the id: the host finds it in its
///   plugin catalog. Two unrelated VST2 plugins may share a uniqueID (the format has no
///   registry anymore): both are listed, and the catalog picks the one whose name is
///   [`PluginInstance::name`], else the first one scanned.
/// - [`PluginFormat::Webclap`]: the CLAP plugin id inside a WebCLAP module (a CLAP plugin
///   compiled to wasm32, hosted by the web version only). The module is not part of the id:
///   the browser's installed WebCLAP catalog maps the id to the module that provides it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
pub enum PluginFormat {
    Clap,
    Vst3,
    Au,
    /// VST 2.4. The tag is stable; the host is not in this (open-source) build.
    Vst2,
    /// WebCLAP: a CLAP plugin compiled to WebAssembly (`docs/PLUGIN-FORMATS.md` "WebCLAP").
    /// Hosted by the web version only; native hosts report it as unavailable on this
    /// platform.
    Webclap,
}

impl PluginFormat {
    /// Every format, in scan/display order.
    pub const ALL: [PluginFormat; 5] = [
        PluginFormat::Clap,
        PluginFormat::Vst3,
        PluginFormat::Vst2,
        PluginFormat::Au,
        PluginFormat::Webclap,
    ];

    /// Formats a native (desktop/server) host can load. WebCLAP is web-only.
    pub fn is_native(self) -> bool {
        !matches!(self, PluginFormat::Webclap)
    }

    /// Lowercase CLI/log name: `clap`, `vst3`, `vst2`, `au`, `webclap` (e.g.
    /// `ether-sandbox-helper --format`).
    pub fn as_str(self) -> &'static str {
        match self {
            PluginFormat::Clap => "clap",
            PluginFormat::Vst3 => "vst3",
            PluginFormat::Au => "au",
            PluginFormat::Vst2 => "vst2",
            PluginFormat::Webclap => "webclap",
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
