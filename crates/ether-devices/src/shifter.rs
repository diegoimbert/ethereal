//! Shifter: a pitch shifter, a frequency shifter and a ring modulator in one device (in the
//! spirit of Live's Shifter).
//!
//! - **Pitch** mode: a time-domain (two-tap delay-line) pitch shifter. Each channel writes a
//!   short delay line read by two taps half a window apart; their delay sweeps across
//!   `Window` at the speed that transposes by `Pitch` + `Detune` (a tap read faster than the
//!   line fills rises in pitch), and jumps back when it reaches the end. Each tap is
//!   weighted by `sin²` of its position in the sweep, so the jump happens at zero weight and
//!   the two weights always sum to one. Long windows suit low notes (fewer, smoother
//!   splices), short ones high notes and drums (less doubling). No latency is reported: the
//!   taps sit `Window / 2` behind the input on average (like a hardware shifter).
//! - **Frequency** mode: single-sideband frequency shifting. A pair of IIR allpass chains
//!   (four 2nd-order sections each, Olli Niemitalo's coefficients) puts the input in
//!   quadrature (an analytic signal); multiplying it by a complex carrier at `Shift` +
//!   `Fine` Hz and keeping the real part moves every partial by that many Hz (not a ratio:
//!   harmonics stop being harmonic, metallic at large shifts, a phaser-like swirl at a few
//!   Hz).
//! - **Ring** mode: the input multiplied by a cosine at `Carrier` + `Fine` Hz (both
//!   sidebands; at a few Hz with some dry signal, a tremolo). `Drive` soft-clips the
//!   modulated signal after `Amount` dB of gain.
//! - `Wide` mirrors the right channel: the opposite `Detune` (pitch), the opposite shift
//!   (frequency), a carrier 90° apart (ring: the tremolo alternates between the sides).
//! - `Mix` blends the dry signal (linear). A mode change crossfades over 10 ms, a `Window`
//!   change over 20 ms (from the old sweep to the new one); every continuous
//!   param glides.
//!
//! - **Modulation**: an LFO and an envelope follower add to the shift: `LFO Depth` /
//!   `Env Depth` Hz in the frequency and ring modes, `LFO Pitch` / `Env Pitch` semitones in
//!   the pitch mode. The LFO (Live's ten waveforms; `Duty` moves the middle of the cycle)
//!   runs free or synced (song-locked while playing, the fx-modulation LFO); the right one
//!   is `Phase` degrees ahead or, in `Spin` mode, faster by `Spin`. `Offset` shifts both
//!   phases; `Width` blends an independent right draw into the stepped random. The
//!   follower tracks the louder channel's peak (0..1) with `Attack` / `Release`.
//!
//! - **Delay**: after the shifter, 10 ms ..= 8 s or a synced division; its output goes back
//!   into the shifter's input through the `Tone` low-pass (300 Hz ..= off at 22 kHz) and a
//!   soft clip, so every repeat is shifted again (rising or falling spirals, octave
//!   cascades; `Feedback` up to 150 % saturates). With the delay on, the wet signal is the
//!   delay's output; the switch fades over 10 ms, and a time change glides (tape-like).
//!
//! - **MIDI pitch** (`Pitch Mode` = MIDI, Live's Shifter "MIDI" mode): the pitch follows
//!   the last held note of the device's note input — usually a MIDI sidechain
//!   (`ether_model::MidiSidechain`) — relative to `Root` (a note on the root = no shift),
//!   plus the sidechain's pitch bend (± `Bend Range`), gliding over `Glide`. Releasing a note
//!   goes back to the previous one still held; with none held the pitch stays where it was.
//!   `Pitch` doesn't apply (`Detune`, the LFO and the envelope still do).
//!
//! The delay lines are allocated in `prepare`; `process` allocates nothing.
//!
//! # Params (`BuiltinDeviceType::Shifter`, stable and append-only)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Mode | `Mode` | Pitch / Frequency / Ring, default Pitch |
//! | 1 | Pitch | `Pitch` | -24 ..= 24 Semitones (whole steps on the knob), default 0 |
//! | 2 | Pitch | `Detune` | -100 ..= 100 Cents, default 0 |
//! | 3 | Pitch | `Window` | 10 ..= 350 Milliseconds (log), default 80 |
//! | 4 | Frequency | `Shift` | -5000 ..= 5000 Hertz, default 0 |
//! | 5 | Frequency | `Fine` | -50 ..= 50 Hertz (adds to `Shift` and `Carrier`), default 0 |
//! | 6 | Ring | `Carrier` | 1 ..= 10000 Hertz (log), default 100 |
//! | 7 | Ring | `Drive` | toggle, default off |
//! | 8 | Ring | `Amount` | 0 ..= 24 Decibels (drive gain), default 6 |
//! | 9 | Output | `Wide` | toggle, default off |
//! | 10 | Output | `Mix` | 0 ..= 100 Percent, default 100 |
//! | 11 | LFO | `Waveform` | [`WAVEFORMS`], default Sine |
//! | 12 | LFO | `Sync` | toggle, default off |
//! | 13 | LFO | `Rate` | 0.01 ..= 50 Hertz (log), default 2 |
//! | 14 | LFO | `Sync Rate` | sync rate, default 1/4 |
//! | 15 | LFO | `LFO Depth` | 0 ..= 5000 Hertz (cubic), default 0 |
//! | 16 | LFO | `LFO Pitch` | 0 ..= 24 Semitones, default 0 |
//! | 17 | LFO | `Stereo` | Phase / Spin, default Phase |
//! | 18 | LFO | `Phase` | 0 ..= 360 degrees (right ahead), default 0 |
//! | 19 | LFO | `Spin` | 0 ..= 50 Percent (right faster), default 0 |
//! | 20 | LFO | `Offset` | 0 ..= 360 degrees, default 0 |
//! | 21 | LFO | `Duty` | -100 ..= 100 Percent, default 0 |
//! | 22 | LFO | `Width` | 0 ..= 100 Percent (random S&H), default 50 |
//! | 23 | Envelope | `Follower` | toggle, default off |
//! | 24 | Envelope | `Attack` | 0.1 ..= 30 Milliseconds (log), default 6 |
//! | 25 | Envelope | `Release` | 0.1 ..= 400 Milliseconds (log), default 200 |
//! | 26 | Envelope | `Env Depth` | -5000 ..= 5000 Hertz, default 0 |
//! | 27 | Envelope | `Env Pitch` | -24 ..= 24 Semitones, default 0 |
//! | 28 | Delay | `Delay` | toggle, default off |
//! | 29 | Delay | `Delay Sync` | toggle, default on |
//! | 30 | Delay | `Delay Time` | 10 ..= 8000 Milliseconds (log), default 250 |
//! | 31 | Delay | `Delay Rate` | sync rate, default 1/8 |
//! | 32 | Delay | `Feedback` | 0 ..= 150 Percent, default 0 |
//! | 33 | Delay | `Tone` | 300 ..= 22000 Hertz (log; 22000 = off), default off |
//! | 34 | MIDI | `Pitch Mode` | Internal / MIDI, default Internal |
//! | 35 | MIDI | `Root` | C-2 ..= G8 (MIDI notes 0 ..= 127), default C3 (60) |
//! | 36 | MIDI | `Glide` | 0 ..= 2000 Milliseconds (cubic), default 0 |
//! | 37 | MIDI | `Bend Range` | 0 ..= 24 Semitones, default 2 |

use std::f64::consts::{PI, TAU};

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::layout::{DeviceLayout, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use ether_core::TransportInfo;

use crate::contract::{
    FactoryPreset, SYNC_EIGHTH, SYNC_RATE_BEATS, SYNC_RATES, choice, descriptor as build, item,
    knob, layout, param, section, toggle,
};
use crate::dsp::flush;
use crate::dsp::pitch_taps::PitchTaps;
use crate::fx_modulation::shared::{Lfo, LfoStep, Rate};
use crate::util::{self, split_at_events};

/// Index of "1/4" in [`SYNC_RATES`].
const SYNC_QUARTER: usize = 8;

/// Param ids (stable, append-only).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const PITCH: ParamId = ParamId(1);
    pub const DETUNE: ParamId = ParamId(2);
    pub const WINDOW: ParamId = ParamId(3);
    pub const SHIFT: ParamId = ParamId(4);
    pub const FINE: ParamId = ParamId(5);
    pub const CARRIER: ParamId = ParamId(6);
    pub const DRIVE: ParamId = ParamId(7);
    pub const DRIVE_AMOUNT: ParamId = ParamId(8);
    pub const WIDE: ParamId = ParamId(9);
    pub const MIX: ParamId = ParamId(10);
    pub const LFO_WAVEFORM: ParamId = ParamId(11);
    pub const LFO_SYNC: ParamId = ParamId(12);
    pub const LFO_RATE: ParamId = ParamId(13);
    pub const LFO_SYNC_RATE: ParamId = ParamId(14);
    pub const LFO_DEPTH: ParamId = ParamId(15);
    pub const LFO_PITCH: ParamId = ParamId(16);
    pub const LFO_STEREO: ParamId = ParamId(17);
    pub const LFO_PHASE: ParamId = ParamId(18);
    pub const LFO_SPIN: ParamId = ParamId(19);
    pub const LFO_OFFSET: ParamId = ParamId(20);
    pub const LFO_DUTY: ParamId = ParamId(21);
    pub const LFO_WIDTH: ParamId = ParamId(22);
    pub const ENV_ON: ParamId = ParamId(23);
    pub const ENV_ATTACK: ParamId = ParamId(24);
    pub const ENV_RELEASE: ParamId = ParamId(25);
    pub const ENV_DEPTH: ParamId = ParamId(26);
    pub const ENV_PITCH: ParamId = ParamId(27);
    pub const DELAY_ON: ParamId = ParamId(28);
    pub const DELAY_SYNC: ParamId = ParamId(29);
    pub const DELAY_TIME: ParamId = ParamId(30);
    pub const DELAY_RATE: ParamId = ParamId(31);
    pub const FEEDBACK: ParamId = ParamId(32);
    pub const TONE: ParamId = ParamId(33);
    pub const PITCH_MODE: ParamId = ParamId(34);
    pub const ROOT: ParamId = ParamId(35);
    pub const GLIDE: ParamId = ParamId(36);
    pub const BEND_RANGE: ParamId = ParamId(37);
    /// Number of params.
    pub const COUNT: usize = 38;
}

/// `Pitch Mode` values.
pub mod pitch_mode {
    pub const INTERNAL: usize = 0;
    pub const MIDI: usize = 1;
}

/// `Pitch Mode` labels.
pub const PITCH_MODES: [&str; 2] = ["Internal", "MIDI"];
/// Default `Root`: C3 (Live's name for MIDI note 60).
pub const DEFAULT_ROOT: u8 = 60;
/// Longest `Glide` (ms).
pub const MAX_GLIDE_MS: f64 = 2000.0;
/// Notes the MIDI pitch remembers (the last held one wins; older ones come back on release).
const HELD_NOTES: usize = 16;

/// `Root` labels: the 128 MIDI notes, C3 = 60 (Live's octave numbering).
fn note_labels() -> Vec<String> {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    (0..128)
        .map(|n: i32| format!("{}{}", NAMES[(n % 12) as usize], n / 12 - 2))
        .collect()
}

/// `LFO Waveform` values (Live's Shifter order).
pub mod waveform {
    pub const SINE: usize = 0;
    pub const TRIANGLE: usize = 1;
    pub const TRIANGLE_ANALOG: usize = 2;
    pub const TRIANGLE_8: usize = 3;
    pub const TRIANGLE_16: usize = 4;
    pub const SAW_UP: usize = 5;
    pub const SAW_DOWN: usize = 6;
    pub const RECTANGLE: usize = 7;
    pub const RANDOM: usize = 8;
    pub const RANDOM_SH: usize = 9;
}

/// `LFO Waveform` labels.
pub const WAVEFORMS: [&str; 10] = [
    "Sine",
    "Triangle",
    "Triangle Analog",
    "Triangle 8",
    "Triangle 16",
    "Saw Up",
    "Saw Down",
    "Rectangle",
    "Random",
    "Random S&H",
];

/// `Stereo` labels: the right LFO `Phase` degrees apart, or `Spin` faster.
pub const STEREO_MODES: [&str; 2] = ["Phase", "Spin"];
use params as ids;

/// `Mode` values.
pub mod mode {
    pub const PITCH: usize = 0;
    pub const FREQUENCY: usize = 1;
    pub const RING: usize = 2;
}

/// `Mode` labels.
pub const MODES: [&str; 3] = ["Pitch", "Frequency", "Ring"];
/// Longest `Window` (ms).
pub const MAX_WINDOW_MS: f64 = 350.0;
/// Mode-change crossfade (ms).
const XFADE_MS: f64 = 10.0;
/// Glide time constant of the continuous params (ms).
const GLIDE_MS: f64 = 15.0;
/// Longest `Delay Time` (ms; Live's Shifter delay goes to 8 s).
pub const MAX_DELAY_MS: f64 = 8000.0;
/// `Tone` at its top: no filtering of the feedback.
pub const TONE_OFF_HZ: f64 = 22000.0;
/// Glide of the delay time (ms): a time change bends the echoes like tape.
const DELAY_GLIDE_MS: f64 = 80.0;
/// Fade of the delay switch (ms).
const DELAY_FADE_MS: f32 = 10.0;

/// Descriptor of the shifter.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(ty, BuiltinDeviceType::Shifter);
    let mut d = build(
        BuiltinDeviceType::Shifter,
        "Shifter",
        DeviceCategory::AudioEffect,
        vec![
            choice(0, "Mode", "Mode", &MODES, mode::PITCH),
            ParamInfo {
                step: Some(1.0),
                ..param(
                    1,
                    "Pitch",
                    "Pitch",
                    ParamUnit::Semitones,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                )
            },
            param(
                2,
                "Detune",
                "Pitch",
                ParamUnit::Cents,
                (-100.0, 100.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                3,
                "Window",
                "Pitch",
                ParamUnit::Milliseconds,
                (10.0, MAX_WINDOW_MS, 80.0),
                ParamScale::Log,
            ),
            param(
                4,
                "Shift",
                "Frequency",
                ParamUnit::Hertz,
                (-5000.0, 5000.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                5,
                "Fine",
                "Frequency",
                ParamUnit::Hertz,
                (-50.0, 50.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                6,
                "Carrier",
                "Ring",
                ParamUnit::Hertz,
                (1.0, 10000.0, 100.0),
                ParamScale::Log,
            ),
            toggle(7, "Drive", "Ring", false),
            param(
                8,
                "Amount",
                "Ring",
                ParamUnit::Decibels,
                (0.0, 24.0, 6.0),
                ParamScale::Linear,
            ),
            toggle(9, "Wide", "Output", false),
            param(
                10,
                "Mix",
                "Output",
                ParamUnit::Percent,
                (0.0, 100.0, 100.0),
                ParamScale::Linear,
            ),
            choice(11, "Waveform", "LFO", &WAVEFORMS, waveform::SINE),
            toggle(12, "Sync", "LFO", false),
            param(
                13,
                "Rate",
                "LFO",
                ParamUnit::Hertz,
                (0.01, 50.0, 2.0),
                ParamScale::Log,
            ),
            choice(14, "Sync Rate", "LFO", &SYNC_RATES, SYNC_QUARTER),
            param(
                15,
                "LFO Depth",
                "LFO",
                ParamUnit::Hertz,
                (0.0, 5000.0, 0.0),
                ParamScale::Power { exponent: 3.0 },
            ),
            ParamInfo {
                step: Some(1.0),
                ..param(
                    16,
                    "LFO Pitch",
                    "LFO",
                    ParamUnit::Semitones,
                    (0.0, 24.0, 0.0),
                    ParamScale::Linear,
                )
            },
            choice(17, "Stereo", "LFO", &STEREO_MODES, 0),
            param(
                18,
                "Phase",
                "LFO",
                ParamUnit::None,
                (0.0, 360.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                19,
                "Spin",
                "LFO",
                ParamUnit::Percent,
                (0.0, 50.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                20,
                "Offset",
                "LFO",
                ParamUnit::None,
                (0.0, 360.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                21,
                "Duty",
                "LFO",
                ParamUnit::Percent,
                (-100.0, 100.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                22,
                "Width",
                "LFO",
                ParamUnit::Percent,
                (0.0, 100.0, 50.0),
                ParamScale::Linear,
            ),
            toggle(23, "Follower", "Envelope", false),
            param(
                24,
                "Attack",
                "Envelope",
                ParamUnit::Milliseconds,
                (0.1, 30.0, 6.0),
                ParamScale::Log,
            ),
            param(
                25,
                "Release",
                "Envelope",
                ParamUnit::Milliseconds,
                (0.1, 400.0, 200.0),
                ParamScale::Log,
            ),
            param(
                26,
                "Env Depth",
                "Envelope",
                ParamUnit::Hertz,
                (-5000.0, 5000.0, 0.0),
                ParamScale::Linear,
            ),
            ParamInfo {
                step: Some(1.0),
                ..param(
                    27,
                    "Env Pitch",
                    "Envelope",
                    ParamUnit::Semitones,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                )
            },
            toggle(28, "Delay", "Delay", false),
            toggle(29, "Delay Sync", "Delay", true),
            param(
                30,
                "Delay Time",
                "Delay",
                ParamUnit::Milliseconds,
                (10.0, MAX_DELAY_MS, 250.0),
                ParamScale::Log,
            ),
            choice(31, "Delay Rate", "Delay", &SYNC_RATES, SYNC_EIGHTH),
            param(
                32,
                "Feedback",
                "Delay",
                ParamUnit::Percent,
                (0.0, 150.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                33,
                "Tone",
                "Delay",
                ParamUnit::Hertz,
                (300.0, TONE_OFF_HZ, TONE_OFF_HZ),
                ParamScale::Log,
            ),
            choice(34, "Pitch Mode", "MIDI", &PITCH_MODES, pitch_mode::INTERNAL),
            {
                let labels = note_labels();
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                choice(35, "Root", "MIDI", &refs, usize::from(DEFAULT_ROOT))
            },
            param(
                36,
                "Glide",
                "MIDI",
                ParamUnit::Milliseconds,
                (0.0, MAX_GLIDE_MS, 0.0),
                ParamScale::Power { exponent: 3.0 },
            ),
            ParamInfo {
                step: Some(1.0),
                ..param(
                    37,
                    "Bend Range",
                    "MIDI",
                    ParamUnit::Semitones,
                    (0.0, 24.0, 2.0),
                    ParamScale::Linear,
                )
            },
        ],
        2,
        2,
        // A note input (MIDI pitch mode, fed by a MIDI sidechain).
        true,
        0,
    );
    d.layout = Some(device_layout());
    d
}

/// The panel: the mode, one section per mode, the output.
fn device_layout() -> DeviceLayout {
    use WidgetSize::*;
    layout(vec![
        section(
            "mode",
            Some("Mode"),
            1,
            1,
            vec![
                item(Widget::Choice { param: ids::MODE }, Medium),
                item(Widget::Toggle { param: ids::WIDE }, Small),
            ],
        ),
        section(
            "pitch",
            Some("Pitch"),
            1,
            3,
            vec![
                knob(ids::PITCH, Large),
                knob(ids::DETUNE, Medium),
                knob(ids::WINDOW, Medium),
            ],
        ),
        section(
            "frequency",
            Some("Frequency"),
            1,
            2,
            vec![knob(ids::SHIFT, Large), knob(ids::FINE, Medium)],
        ),
        section(
            "ring",
            Some("Ring"),
            1,
            3,
            vec![
                knob(ids::CARRIER, Large),
                item(Widget::Toggle { param: ids::DRIVE }, Small),
                knob(ids::DRIVE_AMOUNT, Medium),
            ],
        ),
        section(
            "midi",
            Some("MIDI"),
            1,
            4,
            vec![
                item(
                    Widget::Choice {
                        param: ids::PITCH_MODE,
                    },
                    Small,
                ),
                item(Widget::Choice { param: ids::ROOT }, Small),
                knob(ids::GLIDE, Medium),
                knob(ids::BEND_RANGE, Small),
            ],
        ),
        section("output", Some("Output"), 1, 1, vec![knob(ids::MIX, Large)]),
        section(
            "lfo",
            Some("LFO"),
            2,
            6,
            vec![
                item(
                    Widget::Lfo {
                        shape: ids::LFO_WAVEFORM,
                        rate: ids::LFO_RATE,
                        amount: Some(ids::LFO_DEPTH),
                    },
                    Medium,
                ),
                knob(ids::LFO_RATE, Medium),
                item(
                    Widget::Toggle {
                        param: ids::LFO_SYNC,
                    },
                    Small,
                ),
                item(
                    Widget::Choice {
                        param: ids::LFO_SYNC_RATE,
                    },
                    Small,
                ),
                knob(ids::LFO_DEPTH, Medium),
                knob(ids::LFO_PITCH, Medium),
                item(
                    Widget::Choice {
                        param: ids::LFO_STEREO,
                    },
                    Small,
                ),
                knob(ids::LFO_PHASE, Small),
                knob(ids::LFO_SPIN, Small),
                knob(ids::LFO_OFFSET, Small),
                knob(ids::LFO_DUTY, Small),
                knob(ids::LFO_WIDTH, Small),
            ],
        ),
        section(
            "envelope",
            Some("Envelope"),
            1,
            5,
            vec![
                item(Widget::Toggle { param: ids::ENV_ON }, Small),
                knob(ids::ENV_ATTACK, Small),
                knob(ids::ENV_RELEASE, Small),
                knob(ids::ENV_DEPTH, Medium),
                knob(ids::ENV_PITCH, Medium),
            ],
        ),
        section(
            "delay",
            Some("Delay"),
            1,
            4,
            vec![
                item(
                    Widget::DelayTaps {
                        time: ids::DELAY_TIME,
                        feedback: ids::FEEDBACK,
                        sync: Some(ids::DELAY_SYNC),
                        division: Some(ids::DELAY_RATE),
                        ping_pong: None,
                        freeze: None,
                    },
                    Medium,
                ),
                item(
                    Widget::Toggle {
                        param: ids::DELAY_ON,
                    },
                    Small,
                ),
                knob(ids::DELAY_TIME, Small),
                item(
                    Widget::Toggle {
                        param: ids::DELAY_SYNC,
                    },
                    Small,
                ),
                item(
                    Widget::Choice {
                        param: ids::DELAY_RATE,
                    },
                    Small,
                ),
                knob(ids::FEEDBACK, Medium),
                knob(ids::TONE, Medium),
            ],
        ),
    ])
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    assert_eq!(device.device_type(), BuiltinDeviceType::Shifter);
    Box::new(Shifter::new())
}

/// `FactoryPreset` entries for `presets/shifter/<slug>.etherpreset` files.
macro_rules! presets {
    ($($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!("shifter/", $slug),
            json: include_str!(concat!("../presets/shifter/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::Shifter => presets!(
            "octave-up",
            "fifth-harmony",
            "wide-detune",
            "barber-pole-swirl",
            "metallic-shift",
            "ring-tremolo",
            "driven-ring",
            "wide-vibrato",
            "synced-sweep",
            "envelope-ring",
            "spiral-echo",
            "octave-cascade"
        ),
        _ => &[],
    }
}

/// Plain values of the params, clamped (NaN = default).
struct Values {
    v: [f64; ids::COUNT],
    range: [(f64, f64, f64); ids::COUNT],
}

impl Values {
    fn new() -> Self {
        let d = descriptor(BuiltinDeviceType::Shifter);
        let mut s = Self {
            v: [0.0; ids::COUNT],
            range: [(0.0, 0.0, 0.0); ids::COUNT],
        };
        for (i, p) in d.params.iter().enumerate() {
            assert_eq!(p.id, ParamId(i as u32));
            s.v[i] = p.default;
            s.range[i] = (p.min, p.max, p.default);
        }
        s
    }

    fn set(&mut self, id: ParamId, value: f64) -> bool {
        let i = id.0 as usize;
        let Some(&(lo, hi, def)) = self.range.get(i) else {
            return false;
        };
        self.v[i] = if value.is_nan() {
            def
        } else {
            value.clamp(lo, hi)
        };
        true
    }

    fn f(&self, id: ParamId) -> f64 {
        self.v[id.0 as usize]
    }

    fn on(&self, id: ParamId) -> bool {
        self.f(id) >= 0.5
    }

    fn mode(&self) -> usize {
        (self.f(ids::MODE).round().max(0.0) as usize).min(MODES.len() - 1)
    }
}

/// A one-pole glide toward a target (f64; exact once within `eps`).
#[derive(Clone, Copy)]
struct Glide {
    current: f64,
    target: f64,
    coef: f64,
    eps: f64,
}

impl Glide {
    fn new(v: f64, eps: f64) -> Self {
        Self {
            current: v,
            target: v,
            coef: 0.0,
            eps,
        }
    }

    fn set_time(&mut self, ms: f64, sr: f64) {
        self.coef = (-1.0 / (ms * 0.001 * sr).max(1.0)).exp();
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        if self.current != self.target {
            self.current = self.target + (self.current - self.target) * self.coef;
            if (self.current - self.target).abs() < self.eps {
                self.current = self.target;
            }
        }
        self.current
    }

    fn settled(&self) -> bool {
        self.current == self.target
    }
}

/// Niemitalo's 90° phase-difference allpass pair: the two chains' outputs differ by 90°
/// (± 0.7°) from ~20 Hz to ~23.9 kHz at 48 kHz. Each coefficient is `a` of a section
/// `y[n] = a² (x[n] + y[n-2]) - x[n-2]`.
const HILBERT_A: [f64; 4] = [
    0.692_387_8,
    0.936_065_432_295_9,
    0.988_229_522_686,
    0.998_748_845_273_7,
];
const HILBERT_B: [f64; 4] = [
    0.402_192_116_242_6,
    0.856_171_088_242,
    0.972_290_954_565_1,
    0.995_288_479_127_8,
];

/// One allpass chain (4 sections, each with two samples of input and output history).
#[derive(Clone, Copy, Default)]
struct AllpassChain {
    x: [[f64; 2]; 4],
    y: [[f64; 2]; 4],
}

impl AllpassChain {
    #[inline]
    fn tick(&mut self, coefs: &[f64; 4], input: f64) -> f64 {
        let mut v = input;
        for (k, a) in coefs.iter().enumerate() {
            let a2 = a * a;
            let y = flush(a2 * (v + self.y[k][1]) - self.x[k][1]);
            self.x[k][1] = self.x[k][0];
            self.x[k][0] = v;
            self.y[k][1] = self.y[k][0];
            self.y[k][0] = y;
            v = y;
        }
        v
    }
}

/// Analytic signal of one channel: `(re, im)` with `im` 90° ahead of `re` (so `re·cos + im·sin` moves it up).
#[derive(Clone, Copy, Default)]
struct Hilbert {
    a: AllpassChain,
    b: AllpassChain,
    /// Chain A's previous output (it is delayed by one sample).
    a_prev: f64,
}

impl Hilbert {
    #[inline]
    fn tick(&mut self, x: f64) -> (f64, f64) {
        let re = self.a_prev;
        self.a_prev = self.a.tick(&HILBERT_A, x);
        let im = self.b.tick(&HILBERT_B, x);
        (re, im)
    }
}

/// xorshift32: deterministic per instance, RT-safe. Uniform in `[-1, 1)`.
#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    #[inline]
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

/// Random values of one channel's LFO: the one of the current cycle and the next (the
/// smooth random glides from one to the other).
#[derive(Clone, Copy)]
struct RandomCycle {
    rng: Rng,
    now: f32,
    next: f32,
    /// LFO phase at the previous sample (a smaller phase = a new cycle).
    last: f64,
}

impl RandomCycle {
    fn new(seed: u32) -> Self {
        let mut rng = Rng(seed);
        let now = rng.next();
        let next = rng.next();
        Self {
            rng,
            now,
            next,
            last: 0.0,
        }
    }

    #[inline]
    fn advance(&mut self, phase: f64) {
        if phase < self.last {
            self.now = self.next;
            self.next = self.rng.next();
        }
        self.last = phase;
    }
}

/// The LFO's value (-1..=1) for waveform `w` at `phase` (cycles), `duty` (-1..1) moving
/// the waveform's middle (the rectangle's high part is `(1 + duty) / 2` of a cycle).
#[inline]
fn lfo_value(w: usize, phase: f64, duty: f64, random: &RandomCycle) -> f32 {
    let p = phase - phase.floor();
    let split = ((1.0 + duty) * 0.5).clamp(0.01, 0.99);
    let p = if p < split {
        0.5 * p / split
    } else {
        0.5 + 0.5 * (p - split) / (1.0 - split)
    };
    let tri = |p: f64| {
        if p < 0.25 {
            4.0 * p
        } else if p < 0.75 {
            2.0 - 4.0 * p
        } else {
            4.0 * p - 4.0
        }
    };
    let steps = |p: f64, n: f64| tri((p * n).floor() / n);
    (match w {
        waveform::TRIANGLE => tri(p),
        // A triangle with rounded tips, between the triangle and the sine.
        waveform::TRIANGLE_ANALOG => (std::f64::consts::FRAC_PI_2 * tri(p)).sin(),
        waveform::TRIANGLE_8 => steps(p, 8.0),
        waveform::TRIANGLE_16 => steps(p, 16.0),
        waveform::SAW_UP => 2.0 * p - 1.0,
        waveform::SAW_DOWN => 1.0 - 2.0 * p,
        waveform::RECTANGLE => {
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        waveform::RANDOM => {
            let t = 0.5 - 0.5 * (PI * (phase - phase.floor())).cos();
            f64::from(random.now) + (f64::from(random.next) - f64::from(random.now)) * t
        }
        waveform::RANDOM_SH => f64::from(random.now),
        _ => (TAU * p).sin(),
    }) as f32
}

/// The shift amount's modulation: a stereo LFO (the right one `Phase` apart or `Spin`
/// faster) and an envelope follower of the input.
struct Modulation {
    lfo: [Lfo; 2],
    random: [RandomCycle; 2],
    /// The right channel's own random draws (`Width` of S&H blends them in).
    random_r: RandomCycle,
    env: f32,
    attack: f32,
    release: f32,
}

impl Modulation {
    fn new() -> Self {
        // Same seed on both sides: identical values when the LFOs are in phase.
        Self {
            lfo: [Lfo::default(); 2],
            random: [RandomCycle::new(0x5EED_1F0A); 2],
            random_r: RandomCycle::new(0x0DD_BA11),
            env: 0.0,
            attack: 0.0,
            release: 0.0,
        }
    }

    fn reset(&mut self) {
        *self = Self {
            attack: self.attack,
            release: self.release,
            ..Self::new()
        };
    }
}

/// The delay after the shifter: its output feeds back into the shifter's input through the
/// `Tone` low-pass and a soft clip, so each repeat is shifted again.
struct Echo {
    lines: [Vec<f32>; 2],
    write: usize,
    /// Delay in samples.
    time: Glide,
    /// The switch (0..1, faded) and the feedback gain.
    on: Smoother,
    feedback: Smoother,
    /// One-pole low-pass coefficient of `Tone` (0 = off) and its state per channel.
    tone: f32,
    lp: [f32; 2],
}

impl Echo {
    fn new() -> Self {
        Self {
            lines: [Vec::new(), Vec::new()],
            write: 0,
            time: Glide::new(0.0, 1e-3),
            on: Smoother::new(0.0, DELAY_FADE_MS, 48_000.0),
            feedback: Smoother::new(0.0, 0.0, 48_000.0),
            tone: 0.0,
            lp: [0.0; 2],
        }
    }

    /// Non-RT. Lines for `MAX_DELAY_MS` at `sr`.
    fn allocate(&mut self, sr: f64) {
        let n = (MAX_DELAY_MS * 0.001 * sr).ceil() as usize + 8;
        for l in &mut self.lines {
            if l.len() != n {
                *l = vec![0.0; n];
            }
        }
        self.on = Smoother::new(self.on.target(), DELAY_FADE_MS, sr as f32);
        self.time.set_time(DELAY_GLIDE_MS, sr);
    }

    fn reset(&mut self) {
        for l in &mut self.lines {
            l.fill(0.0);
        }
        self.write = 0;
        self.lp = [0.0; 2];
    }

    /// Linear read `d` samples behind the newest sample of line `c`.
    #[inline]
    fn read(&self, c: usize, d: f64) -> f32 {
        let line = &self.lines[c];
        let n = line.len();
        let d = d.clamp(1.0, (n - 2) as f64);
        let i = d as usize;
        let t = (d - i as f64) as f32;
        let at = |k: usize| line[(self.write + n - k) % n];
        let (a, b) = (at(i), at(i + 1));
        a + (b - a) * t
    }

    /// The feedback into the shifter for channel `c` from the delay's output `y`.
    #[inline]
    fn feed(&mut self, c: usize, y: f32, gain: f32) -> f32 {
        let lp = &mut self.lp[c];
        *lp = if self.tone > 0.0 {
            crate::dsp::flush32(y + (*lp - y) * self.tone)
        } else {
            y
        };
        gain * lp.tanh()
    }
}

/// The MIDI pitch (`Pitch Mode` = MIDI): the held notes, newest last, and the pitch bend.
#[derive(Clone, Copy)]
struct MidiPitch {
    /// `(note id, key)` of the held notes, oldest first.
    held: [(u32, u8); HELD_NOTES],
    len: usize,
    /// The key the pitch follows (the last one held; kept when every note is released).
    key: Option<u8>,
    /// Pitch bend, -1..=1.
    bend: f64,
}

impl MidiPitch {
    fn new() -> Self {
        Self {
            held: [(0, 0); HELD_NOTES],
            len: 0,
            key: None,
            bend: 0.0,
        }
    }

    fn release(&mut self, note_id: u32) {
        if let Some(i) = self.held[..self.len].iter().position(|h| h.0 == note_id) {
            self.held.copy_within(i + 1..self.len, i);
            self.len -= 1;
            if let Some(&(_, key)) = self.held[..self.len].last() {
                self.key = Some(key);
            }
        }
    }

    /// Applies a note-input event; returns whether the pitch target may have changed.
    fn event(&mut self, kind: &EventKind) -> bool {
        match *kind {
            EventKind::NoteOn { note_id, key, .. } => {
                self.release(note_id);
                if self.len == HELD_NOTES {
                    self.held.copy_within(1.., 0);
                    self.len -= 1;
                }
                self.held[self.len] = (note_id, key);
                self.len += 1;
                self.key = Some(key);
                true
            }
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                self.release(note_id);
                true
            }
            EventKind::AllNotesOff => {
                self.len = 0;
                false
            }
            // Pitch bend, any channel.
            EventKind::Midi { data } if data[0] & 0xF0 == 0xE0 => {
                let v = (u16::from(data[2] & 0x7F) << 7) | u16::from(data[1] & 0x7F);
                self.bend = ((f64::from(v) - 8192.0) / 8192.0).clamp(-1.0, 1.0);
                true
            }
            _ => false,
        }
    }
}

/// The shifter device.
pub struct Shifter {
    values: Values,
    sample_rate: f64,
    taps: [PitchTaps; 2],
    hilbert: [Hilbert; 2],
    /// Carrier phases per channel in cycles, `[0, 1)` (frequency and ring modes).
    phase: [f64; 2],
    modulation: Modulation,
    /// Current mode and the one being faded out (`fade` samples left of `fade_len`).
    mode: usize,
    prev_mode: usize,
    fade: u32,
    fade_len: u32,
    semitones: Glide,
    detune: Glide,
    /// `Window` in samples.
    window: f64,
    shift: Glide,
    carrier: Glide,
    drive: Smoother,
    mix: Smoother,
    echo: Echo,
    tail: util::Tail,
    midi: MidiPitch,
}

impl Default for Shifter {
    fn default() -> Self {
        Self::new()
    }
}

impl Shifter {
    /// Non-RT. A shifter with default params (lines allocated for 48 kHz).
    pub fn new() -> Self {
        let mut s = Self {
            values: Values::new(),
            sample_rate: 48_000.0,
            taps: [PitchTaps::new(), PitchTaps::new()],
            hilbert: [Hilbert::default(); 2],
            phase: [0.0; 2],
            modulation: Modulation::new(),
            mode: mode::PITCH,
            prev_mode: mode::PITCH,
            fade: 0,
            fade_len: 1,
            semitones: Glide::new(0.0, 1e-6),
            detune: Glide::new(0.0, 1e-4),
            window: 80.0 * 48.0,
            shift: Glide::new(0.0, 1e-4),
            carrier: Glide::new(100.0, 1e-4),
            drive: Smoother::new(0.0, 0.0, 48_000.0),
            mix: Smoother::new(1.0, 0.0, 48_000.0),
            echo: Echo::new(),
            tail: util::Tail::default(),
            midi: MidiPitch::new(),
        };
        s.configure();
        s
    }

    /// `Pitch Mode` is MIDI.
    fn midi_pitch(&self) -> bool {
        self.values.f(ids::PITCH_MODE) >= 0.5
    }

    /// The semitones the pitch mode shifts by (before `Detune` and the modulation): `Pitch`,
    /// or in MIDI pitch mode the followed key relative to `Root` plus the bend.
    fn pitch_target(&self) -> f64 {
        if !self.midi_pitch() {
            return self.values.f(ids::PITCH);
        }
        let root = self.values.f(ids::ROOT).round();
        let note = self.midi.key.map_or(0.0, |k| f64::from(k) - root);
        note + self.midi.bend * self.values.f(ids::BEND_RANGE)
    }

    /// The pitch glide: `Glide` in MIDI pitch mode, else the usual param smoothing.
    fn pitch_glide_ms(&self) -> f64 {
        if self.midi_pitch() {
            self.values.f(ids::GLIDE).max(GLIDE_MS)
        } else {
            GLIDE_MS
        }
    }

    fn configure(&mut self) {
        let sr = self.sample_rate;
        for g in [
            &mut self.semitones,
            &mut self.detune,
            &mut self.shift,
            &mut self.carrier,
        ] {
            g.set_time(GLIDE_MS, sr);
        }
        self.fade_len = ((XFADE_MS * 0.001 * sr).round() as u32).max(1);
        for i in 0..ids::COUNT {
            self.apply(ParamId(i as u32), false);
        }
        let window = self.window.max(8.0);
        for t in &mut self.taps {
            t.allocate(sr, window, MAX_WINDOW_MS);
        }
        self.echo.allocate(sr);
        self.apply(ids::DELAY_ON, false);
    }

    fn apply(&mut self, id: ParamId, smooth: bool) {
        let v = self.values.f(id);
        let glide = |g: &mut Glide, target: f64| {
            g.target = target;
            if !smooth {
                g.current = target;
            }
        };
        let ramp = |s: &mut Smoother, v: f32| {
            if smooth {
                s.ramp_param(v, ether_core::automation_rt::PARAM_GRID as u32);
            } else {
                s.set_immediate(v);
            }
        };
        match id {
            ids::MODE => {
                let m = self.values.mode();
                if m != self.mode {
                    if smooth {
                        self.prev_mode = self.mode;
                        self.fade = self.fade_len;
                    }
                    if m == mode::FREQUENCY {
                        // Idle since it was last used: start from silence, not stale state.
                        self.hilbert = [Hilbert::default(); 2];
                    }
                    self.mode = m;
                }
            }
            ids::PITCH | ids::PITCH_MODE | ids::ROOT | ids::BEND_RANGE | ids::GLIDE => {
                let ms = self.pitch_glide_ms();
                self.semitones.set_time(ms, self.sample_rate);
                let target = self.pitch_target();
                glide(&mut self.semitones, target);
            }
            ids::DETUNE => glide(&mut self.detune, v * 0.01),
            ids::WINDOW => self.window = v * 0.001 * self.sample_rate,
            ids::SHIFT | ids::FINE | ids::CARRIER => {
                let fine = self.values.f(ids::FINE);
                glide(&mut self.shift, self.values.f(ids::SHIFT) + fine);
                // A negative carrier is the same carrier (cos is even).
                glide(
                    &mut self.carrier,
                    (self.values.f(ids::CARRIER) + fine).abs(),
                );
            }
            ids::DRIVE | ids::DRIVE_AMOUNT => {
                let g = if self.values.on(ids::DRIVE) {
                    util::db_to_amp(self.values.f(ids::DRIVE_AMOUNT) as f32)
                } else {
                    0.0
                };
                ramp(&mut self.drive, g);
            }
            ids::MIX => ramp(&mut self.mix, (v * 0.01) as f32),
            ids::DELAY_ON => {
                let on = if self.values.on(ids::DELAY_ON) {
                    1.0
                } else {
                    0.0
                };
                if smooth {
                    self.echo.on.set_target(on);
                } else {
                    self.echo.on.set_immediate(on);
                }
            }
            ids::FEEDBACK => ramp(&mut self.echo.feedback, (v * 0.01) as f32),
            ids::TONE => {
                self.echo.tone = if v >= TONE_OFF_HZ - 1.0 {
                    0.0
                } else {
                    (-std::f64::consts::TAU * v / self.sample_rate).exp() as f32
                };
            }
            ids::ENV_ATTACK => {
                self.modulation.attack = util::tau_coef(v as f32, self.sample_rate as f32);
            }
            ids::ENV_RELEASE => {
                self.modulation.release = util::tau_coef(v as f32, self.sample_rate as f32);
            }
            _ => {}
        }
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.values.set(id, value) {
            self.apply(id, smooth);
        }
    }

    /// The wet output of mode `m` for this sample. `x`: input, `taps`: the pitch taps'
    /// output, `ana`: the analytic signal, `cs`: the carrier's `(cos, sin)` (the ring's
    /// already a quarter cycle back on a wide right), `drive`: the drive gain (0 = off).
    #[inline]
    fn wet(m: usize, x: f64, taps: f32, ana: (f64, f64), cs: (f64, f64), drive: f32) -> f32 {
        match m {
            // re·cos(ωt) + im·sin(ωt): every partial moves by the carrier's (signed) Hz.
            mode::FREQUENCY => (ana.0 * cs.0 + ana.1 * cs.1) as f32,
            mode::RING => {
                let y = (x * cs.0) as f32;
                if drive > 0.0 { (drive * y).tanh() } else { y }
            }
            _ => taps,
        }
    }

    /// The LFO's rate for one channel (`spin`: the right one's speed-up, 0 on the left).
    fn lfo_rate(&self, spin: f64) -> Rate {
        let sync = self.values.on(ids::LFO_SYNC);
        let i =
            (self.values.f(ids::LFO_SYNC_RATE).round().max(0.0) as usize).min(SYNC_RATES.len() - 1);
        match Rate::of(sync, self.values.f(ids::LFO_RATE) as f32, i) {
            Rate::Hz(hz) => Rate::Hz(hz * (1.0 + spin)),
            Rate::Beats(b) => Rate::Beats(b / (1.0 + spin)),
        }
    }

    /// The right LFO's speed-up (`Spin` mode) and phase lead in cycles (`Phase` mode).
    fn stereo(&self) -> (f64, f64) {
        if self.values.f(ids::LFO_STEREO) >= 0.5 {
            (self.values.f(ids::LFO_SPIN) * 0.01, 0.0)
        } else {
            (0.0, self.values.f(ids::LFO_PHASE) / 360.0)
        }
    }

    crate::dsp::simd::avx2_dispatch! {
        fn render => render_avx2, render_generic(
            &mut self,
            audio: &mut AudioBuffers<'_, '_>,
            transport: &TransportInfo,
            start: usize,
            end: usize,
        )
    }

    #[inline(always)]
    // Per-channel state lives in several arrays indexed by the channel.
    #[allow(clippy::needless_range_loop)]
    fn render_generic(
        &mut self,
        audio: &mut AudioBuffers<'_, '_>,
        transport: &TransportInfo,
        start: usize,
        end: usize,
    ) {
        let n_ch = audio.outputs.len().min(2);
        let sr = self.sample_rate;
        let v = &self.values;
        let wide = v.on(ids::WIDE);
        let (spin, lead) = self.stereo();
        let sr32 = sr as f32;
        let steps = [
            LfoStep::new(self.lfo_rate(0.0), transport, sr32),
            LfoStep::new(self.lfo_rate(spin), transport, sr32),
        ];
        let spinning = spin > 0.0;
        let wave = (v.f(ids::LFO_WAVEFORM).round().max(0.0) as usize).min(WAVEFORMS.len() - 1);
        let duty = v.f(ids::LFO_DUTY) * 0.01;
        let offset = v.f(ids::LFO_OFFSET) / 360.0;
        let width = (v.f(ids::LFO_WIDTH) * 0.01) as f32;
        let lfo_hz = v.f(ids::LFO_DEPTH);
        let lfo_st = v.f(ids::LFO_PITCH);
        let env_on = v.on(ids::ENV_ON);
        let env_hz = if env_on { v.f(ids::ENV_DEPTH) } else { 0.0 };
        let env_st = if env_on { v.f(ids::ENV_PITCH) } else { 0.0 };
        // The delay time for this sub-block (a synced one follows the tempo).
        let delay_ms = if v.on(ids::DELAY_SYNC) {
            let k = (v.f(ids::DELAY_RATE).round().max(0.0) as usize).min(SYNC_RATE_BEATS.len() - 1);
            let bpm = if transport.bpm > 0.0 {
                transport.bpm
            } else {
                120.0
            };
            SYNC_RATE_BEATS[k] * 60_000.0 / bpm
        } else {
            v.f(ids::DELAY_TIME)
        };
        let delay_target = (delay_ms.clamp(10.0, MAX_DELAY_MS) * 0.001 * sr).max(1.0);
        if self.echo.time.target != delay_target {
            self.echo.time.target = delay_target;
            if self.echo.time.current == 0.0 {
                self.echo.time.current = delay_target;
            }
        }
        for i in start..end {
            let semis = self.semitones.tick();
            let detune = self.detune.tick();
            let window = self.window.max(8.0);
            let shift = self.shift.tick();
            let carrier = self.carrier.tick();
            let drive = self.drive.tick();
            let mix = self.mix.tick();
            let fade = if self.fade > 0 {
                self.fade -= 1;
                self.fade as f32 / self.fade_len as f32
            } else {
                0.0
            };
            // Only the modes heard run (the pitch lines are always written).
            let uses = |m: usize| self.mode == m || (fade > 0.0 && self.prev_mode == m);
            let (pitch, freq, ring) = (uses(mode::PITCH), uses(mode::FREQUENCY), uses(mode::RING));

            // Modulation: the stereo LFO and the envelope follower (on the louder channel).
            let m = &mut self.modulation;
            m.lfo[0].advance(&steps[0], i);
            m.lfo[1].advance(&steps[1], i);
            let ph = [
                m.lfo[0].phase() + offset,
                if spinning {
                    m.lfo[1].phase()
                } else {
                    m.lfo[0].phase() + lead
                } + offset,
            ];
            let mut lfo = [0.0f32; 2];
            for c in 0..2 {
                let p = ph[c] - ph[c].floor();
                m.random[c].advance(p);
                lfo[c] = lfo_value(wave, p, duty, &m.random[c]);
            }
            m.random_r.advance(ph[1] - ph[1].floor());
            if wave == waveform::RANDOM_SH {
                lfo[1] = m.random[1].now + (m.random_r.now - m.random[1].now) * width;
            }
            if env_on {
                let level = (0..n_ch)
                    .map(|c| {
                        audio
                            .inputs
                            .get(c)
                            .or(audio.inputs.first())
                            .map_or(0.0, |b| b[i].abs())
                    })
                    .fold(0.0f32, f32::max)
                    .min(1.0);
                let k = if level > m.env { m.attack } else { m.release };
                m.env = crate::dsp::flush32(level + (m.env - level) * k);
            }
            let env = f64::from(m.env);
            let echo_on = self.echo.on.tick();
            let feedback = self.echo.feedback.tick() * echo_on;
            let delay = self.echo.time.tick();
            let echo = echo_on > 0.0 || self.echo.on.is_smoothing();
            if echo {
                let n = self.echo.lines[0].len();
                self.echo.write = (self.echo.write + 1) % n;
            }

            for c in 0..n_ch {
                let dry = audio
                    .inputs
                    .get(c)
                    .or(audio.inputs.first())
                    .map_or(0.0, |b| b[i]);
                // The delay's output (written one sample ago or earlier) and what it feeds
                // back into the shifter.
                let (echoed, x) = if echo {
                    let d = self.echo.read(c, delay);
                    (d, dry + self.echo.feed(c, d, feedback))
                } else {
                    (0.0, dry)
                };
                let l = f64::from(lfo[c]);
                let right = c == 1 && wide;
                let taps = &mut self.taps[c];
                taps.push(x);
                let taps = if pitch {
                    let d = if right { -detune } else { detune };
                    let st = semis + d + l * lfo_st + env * env_st;
                    taps.pitched((st / 12.0).exp2(), window)
                } else {
                    0.0
                };
                let ana = if freq {
                    self.hilbert[c].tick(f64::from(x))
                } else {
                    (0.0, 0.0)
                };
                // One carrier phase per channel: the frequency mode's at `shift` (the
                // opposite on a wide right), the ring's at `carrier` (both run only during
                // the 10 ms of a fade between them).
                let cs = if freq || ring {
                    let modu = l * lfo_hz + env * env_hz;
                    let hz = if self.mode == mode::RING {
                        (carrier + modu).abs()
                    } else if right {
                        -(shift + modu)
                    } else {
                        shift + modu
                    };
                    let back = if right && self.mode == mode::RING {
                        0.25
                    } else {
                        0.0
                    };
                    let (s, co) = (TAU * (self.phase[c] - back)).sin_cos();
                    self.phase[c] += hz / sr;
                    self.phase[c] -= self.phase[c].floor();
                    (co, s)
                } else {
                    (1.0, 0.0)
                };
                let xd = f64::from(x);
                let mut y = Self::wet(self.mode, xd, taps, ana, cs, drive);
                if fade > 0.0 {
                    let old = Self::wet(self.prev_mode, xd, taps, ana, cs, drive);
                    y += (old - y) * fade;
                }
                if echo {
                    let w = self.echo.write;
                    self.echo.lines[c][w] = crate::dsp::flush32(y);
                    y += (echoed - y) * echo_on;
                }
                audio.outputs[c][i] = dry + (y - dry) * mix;
            }
        }
    }
}

impl Node for Shifter {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = f64::from(config.sample_rate.max(1.0));
        self.configure();
        self.reset();
    }

    fn reset(&mut self) {
        for t in &mut self.taps {
            t.reset();
        }
        self.hilbert = [Hilbert::default(); 2];
        self.phase = [0.0; 2];
        self.modulation.reset();
        self.echo.reset();
        self.fade = 0;
        self.midi.len = 0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        // Asleep for a while: the LFOs go on as if they had run.
        let skipped = self.tail.skipped(ctx);
        if skipped > 0 {
            let sr = self.sample_rate as f32;
            let (spin, _) = self.stereo();
            for (k, s) in [0.0, spin].into_iter().enumerate() {
                let step = LfoStep::new(self.lfo_rate(s), ctx.transport, sr);
                self.modulation.lfo[k].skip(&step, skipped);
            }
        }
        let transport = *ctx.transport;
        split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, &transport, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.set(param, value, !step);
                } else if s.midi.event(kind) && s.midi_pitch() {
                    // The note input moves the pitch (MIDI pitch mode), gliding.
                    s.semitones.target = s.pitch_target();
                }
            },
        );
        let settled = !self.mix.is_smoothing()
            && !self.drive.is_smoothing()
            && !self.echo.on.is_smoothing()
            && !self.echo.feedback.is_smoothing()
            && self.fade == 0
            && self.semitones.settled()
            && self.taps.iter().all(PitchTaps::settled);
        // The pitch taps can hold a window of signal; the allpass chains ring briefly.
        let echo = if self.echo.on.target() > 0.0 {
            self.echo.lines[0].len()
        } else {
            0
        };
        let hold = self.taps[0].len()
            + echo
            + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate as f32);
        self.tail.status_of(ctx, audio, settled, hold)
    }
}

impl Device for Shifter {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(BuiltinDeviceType::Shifter)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.v.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value, false);
    }
}
