//! v0.2 devices owned by the `midi-fx` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! MIDI effects (`DeviceCategory::NoteEffect`, CONTRACTS.md §12.4.4): consume `ctx.events`, write the transformed note/MIDI stream to `ctx.out_events` (forward what they don't transform and every `AllNotesOff`, never `Param`); generated notes use note ids from `0x8000_0000 | n`; audio untouched (`channels() == (0, 0)`). Delays only (never earlier than input); a device that holds notes back reports no latency (musical delay).
//!
//! Every device here starts as a [`Placeholder`] (pass-through / silent / MIDI-thru) with its
//! final descriptor. **Param ids are stable and append-only** (documents, automation and
//! presets store them): never renumber, only append. Split this module into files as you like.
//!
//! # Arpeggiator (`BuiltinDeviceType::Arpeggiator`)
//!
//! Steps are tempo-synced to the transport (`TransportInfo`), restarting on the retrigger rule; with the transport stopped it runs from the host's free-running beat clock.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Arpeggiator | `Style` | Up / Down / Up-Down / Down-Up / Converge / Diverge / As Played / Random / Chord (default Up) |
//! | 1 | Arpeggiator | `Rate` | sync rate (default index 4) |
//! | 2 | Arpeggiator | `Gate` | 1 ..= 200 Percent, default 75 |
//! | 3 | Arpeggiator | `Octaves` | 1 ..= 4 (stepped), default 1 |
//! | 4 | Arpeggiator | `Swing` | 0 ..= 100 Percent, default 0 |
//! | 5 | Arpeggiator | `Hold` | toggle, default off |
//! | 6 | Velocity | `Velocity` | As Played / Fixed (default As Played) |
//! | 7 | Velocity | `Fixed Velocity` | 1 ..= 127 (stepped), default 100 |
//! | 8 | Arpeggiator | `Retrigger` | Off / Note / Beat (default Off) |
//!
//! # Chord (`BuiltinDeviceType::Chord`)
//!
//! Shift 0 = off.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Notes | `Shift 1` | -24 ..= 24 (stepped), default 0 |
//! | 1 | Notes | `Shift 2` | -24 ..= 24 (stepped), default 0 |
//! | 2 | Notes | `Shift 3` | -24 ..= 24 (stepped), default 0 |
//! | 3 | Notes | `Shift 4` | -24 ..= 24 (stepped), default 0 |
//! | 4 | Notes | `Shift 5` | -24 ..= 24 (stepped), default 0 |
//! | 5 | Notes | `Shift 6` | -24 ..= 24 (stepped), default 0 |
//! | 6 | Velocities | `Velocity 1` | 0 ..= 200 Percent, default 100 |
//! | 7 | Velocities | `Velocity 2` | 0 ..= 200 Percent, default 100 |
//! | 8 | Velocities | `Velocity 3` | 0 ..= 200 Percent, default 100 |
//! | 9 | Velocities | `Velocity 4` | 0 ..= 200 Percent, default 100 |
//! | 10 | Velocities | `Velocity 5` | 0 ..= 200 Percent, default 100 |
//! | 11 | Velocities | `Velocity 6` | 0 ..= 200 Percent, default 100 |
//! | 12 | Strum | `Strum` | 0 ..= 400 Milliseconds, default 0 |
//! | 13 | Strum | `Tension` | -100 ..= 100 Percent, default 0 |
//!
//! # Scale (`BuiltinDeviceType::ScaleQuantize`)
//!
//! `Scale = Track/Project`: the controller sends the resolved `ether_model::MusicalScale` with `Node::set_data` (on creation and whenever the track/project scale changes).
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Scale | `Scale` | Track / Project / Custom (default Track) |
//! | 1 | Scale | `Root` | C / C# / D / D# / E / F / F# / G / G# / A / A# / B (default C) |
//! | 2 | Scale | `Kind` | Chromatic / Major / Minor / Harmonic Minor / Melodic Minor / Major Pentatonic / Minor Pentatonic / Blues / Dorian / Phrygian / Lydian / Mixolydian / Locrian / Whole Tone (default Major) |
//! | 3 | Quantize | `Direction` | Nearest / Up / Down (default Nearest) |
//! | 4 | Quantize | `Transpose` | -24 ..= 24 (stepped), default 0 |
//!
//! # Note Length (`BuiltinDeviceType::NoteLength`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Length | `Mode` | Time / Sync (default Time) |
//! | 1 | Length | `Length` | 1 ..= 10000 Milliseconds (log), default 250 |
//! | 2 | Length | `Sync Length` | sync rate (default index 6) |
//! | 3 | Length | `Gate` | 1 ..= 200 Percent, default 100 |
//! | 4 | Length | `Trigger` | Note On / Note Off (default Note On) |
//!
//! # Velocity (`BuiltinDeviceType::Velocity`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Velocity | `Mode` | Clip / Gate / Fixed (default Clip) |
//! | 1 | Curve | `Drive` | -100 ..= 100 Percent, default 0 |
//! | 2 | Curve | `Compand` | -100 ..= 100 Percent, default 0 |
//! | 3 | Range | `Out Low` | 1 ..= 127 (stepped), default 1 |
//! | 4 | Range | `Out High` | 1 ..= 127 (stepped), default 127 |
//! | 5 | Range | `In Low` | 1 ..= 127 (stepped), default 1 |
//! | 6 | Range | `In High` | 1 ..= 127 (stepped), default 127 |
//! | 7 | Curve | `Random` | 0 ..= 127 (stepped), default 0 |
//! | 8 | Velocity | `Target` | Note On / Note Off / Both (default Note On) |
//!
//! # Random (`BuiltinDeviceType::Randomizer`)
//!
//! Deterministic from `Seed` + the note's position, so renders repeat.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Pitch | `Chance` | 0 ..= 100 Percent, default 50 |
//! | 1 | Pitch | `Range` | 0 ..= 24 (stepped), default 0 |
//! | 2 | Pitch | `Mode` | Random / Alternate (default Random) |
//! | 3 | Pitch | `Use Scale` | toggle, default off |
//! | 4 | Humanize | `Velocity` | 0 ..= 100 Percent, default 0 |
//! | 5 | Humanize | `Timing` | 0 ..= 100 Milliseconds, default 0 |
//! | 6 | Humanize | `Length` | 0 ..= 100 Percent, default 0 |
//! | 7 | Humanize | `Seed` | 0 ..= 999 (stepped), default 0 |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, SYNC_RATES, choice, descriptor as build, param,
    stepped, toggle,
};

/// Param ids of `Arpeggiator` (stable, append-only).
pub mod arpeggiator {
    use ether_core::protocol::model::ParamId;
    pub const STYLE: ParamId = ParamId(0);
    pub const RATE: ParamId = ParamId(1);
    pub const GATE: ParamId = ParamId(2);
    pub const OCTAVES: ParamId = ParamId(3);
    pub const SWING: ParamId = ParamId(4);
    pub const HOLD: ParamId = ParamId(5);
    pub const VELOCITY_MODE: ParamId = ParamId(6);
    pub const FIXED_VELOCITY: ParamId = ParamId(7);
    pub const RETRIGGER: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}

/// Param ids of `Chord` (stable, append-only).
pub mod chord {
    use ether_core::protocol::model::ParamId;
    pub const SHIFT_1: ParamId = ParamId(0);
    pub const SHIFT_2: ParamId = ParamId(1);
    pub const SHIFT_3: ParamId = ParamId(2);
    pub const SHIFT_4: ParamId = ParamId(3);
    pub const SHIFT_5: ParamId = ParamId(4);
    pub const SHIFT_6: ParamId = ParamId(5);
    pub const VELOCITY_1: ParamId = ParamId(6);
    pub const VELOCITY_2: ParamId = ParamId(7);
    pub const VELOCITY_3: ParamId = ParamId(8);
    pub const VELOCITY_4: ParamId = ParamId(9);
    pub const VELOCITY_5: ParamId = ParamId(10);
    pub const VELOCITY_6: ParamId = ParamId(11);
    pub const STRUM: ParamId = ParamId(12);
    pub const STRUM_TENSION: ParamId = ParamId(13);
    /// Number of params.
    pub const COUNT: usize = 14;
}

/// Param ids of `ScaleQuantize` (stable, append-only).
pub mod scale_quantize {
    use ether_core::protocol::model::ParamId;
    pub const SOURCE: ParamId = ParamId(0);
    pub const ROOT: ParamId = ParamId(1);
    pub const KIND: ParamId = ParamId(2);
    pub const DIRECTION: ParamId = ParamId(3);
    pub const TRANSPOSE: ParamId = ParamId(4);
    /// Number of params.
    pub const COUNT: usize = 5;
}

/// Param ids of `NoteLength` (stable, append-only).
pub mod note_length {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const LENGTH: ParamId = ParamId(1);
    pub const SYNC_LENGTH: ParamId = ParamId(2);
    pub const GATE: ParamId = ParamId(3);
    pub const TRIGGER: ParamId = ParamId(4);
    /// Number of params.
    pub const COUNT: usize = 5;
}

/// Param ids of `Velocity` (stable, append-only).
pub mod velocity {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const DRIVE: ParamId = ParamId(1);
    pub const COMPAND: ParamId = ParamId(2);
    pub const OUT_LOW: ParamId = ParamId(3);
    pub const OUT_HIGH: ParamId = ParamId(4);
    pub const IN_LOW: ParamId = ParamId(5);
    pub const IN_HIGH: ParamId = ParamId(6);
    pub const RANDOM: ParamId = ParamId(7);
    pub const TARGET: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}

/// Param ids of `Randomizer` (stable, append-only).
pub mod randomizer {
    use ether_core::protocol::model::ParamId;
    pub const CHANCE: ParamId = ParamId(0);
    pub const PITCH_RANGE: ParamId = ParamId(1);
    pub const PITCH_MODE: ParamId = ParamId(2);
    pub const SCALE_AWARE: ParamId = ParamId(3);
    pub const VELOCITY_RANDOM: ParamId = ParamId(4);
    pub const TIMING_RANDOM: ParamId = ParamId(5);
    pub const LENGTH_RANDOM: ParamId = ParamId(6);
    pub const SEED: ParamId = ParamId(7);
    /// Number of params.
    pub const COUNT: usize = 8;
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::Arpeggiator => build(
            BuiltinDeviceType::Arpeggiator,
            "Arpeggiator",
            DeviceCategory::NoteEffect,
            vec![
                choice(
                    0,
                    "Style",
                    "Arpeggiator",
                    &[
                        "Up",
                        "Down",
                        "Up-Down",
                        "Down-Up",
                        "Converge",
                        "Diverge",
                        "As Played",
                        "Random",
                        "Chord",
                    ],
                    0,
                ),
                choice(1, "Rate", "Arpeggiator", &SYNC_RATES, 4),
                param(
                    2,
                    "Gate",
                    "Arpeggiator",
                    ParamUnit::Percent,
                    (1.0, 200.0, 75.0),
                    ParamScale::Linear,
                ),
                stepped(3, "Octaves", "Arpeggiator", ParamUnit::None, 1, 4, 1),
                param(
                    4,
                    "Swing",
                    "Arpeggiator",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                toggle(5, "Hold", "Arpeggiator", false),
                choice(6, "Velocity", "Velocity", &["As Played", "Fixed"], 0),
                stepped(
                    7,
                    "Fixed Velocity",
                    "Velocity",
                    ParamUnit::None,
                    1,
                    127,
                    100,
                ),
                choice(8, "Retrigger", "Arpeggiator", &["Off", "Note", "Beat"], 0),
            ],
            0,
            0,
            true,
            0,
        ),
        BuiltinDeviceType::Chord => build(
            BuiltinDeviceType::Chord,
            "Chord",
            DeviceCategory::NoteEffect,
            vec![
                stepped(0, "Shift 1", "Notes", ParamUnit::Semitones, -24, 24, 0),
                stepped(1, "Shift 2", "Notes", ParamUnit::Semitones, -24, 24, 0),
                stepped(2, "Shift 3", "Notes", ParamUnit::Semitones, -24, 24, 0),
                stepped(3, "Shift 4", "Notes", ParamUnit::Semitones, -24, 24, 0),
                stepped(4, "Shift 5", "Notes", ParamUnit::Semitones, -24, 24, 0),
                stepped(5, "Shift 6", "Notes", ParamUnit::Semitones, -24, 24, 0),
                param(
                    6,
                    "Velocity 1",
                    "Velocities",
                    ParamUnit::Percent,
                    (0.0, 200.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Velocity 2",
                    "Velocities",
                    ParamUnit::Percent,
                    (0.0, 200.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    8,
                    "Velocity 3",
                    "Velocities",
                    ParamUnit::Percent,
                    (0.0, 200.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    9,
                    "Velocity 4",
                    "Velocities",
                    ParamUnit::Percent,
                    (0.0, 200.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    10,
                    "Velocity 5",
                    "Velocities",
                    ParamUnit::Percent,
                    (0.0, 200.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    11,
                    "Velocity 6",
                    "Velocities",
                    ParamUnit::Percent,
                    (0.0, 200.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    12,
                    "Strum",
                    "Strum",
                    ParamUnit::Milliseconds,
                    (0.0, 400.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    13,
                    "Tension",
                    "Strum",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            0,
            0,
            true,
            0,
        ),
        BuiltinDeviceType::ScaleQuantize => build(
            BuiltinDeviceType::ScaleQuantize,
            "Scale",
            DeviceCategory::NoteEffect,
            vec![
                choice(0, "Scale", "Scale", &["Track", "Project", "Custom"], 0),
                choice(
                    1,
                    "Root",
                    "Scale",
                    &[
                        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
                    ],
                    0,
                ),
                choice(
                    2,
                    "Kind",
                    "Scale",
                    &[
                        "Chromatic",
                        "Major",
                        "Minor",
                        "Harmonic Minor",
                        "Melodic Minor",
                        "Major Pentatonic",
                        "Minor Pentatonic",
                        "Blues",
                        "Dorian",
                        "Phrygian",
                        "Lydian",
                        "Mixolydian",
                        "Locrian",
                        "Whole Tone",
                    ],
                    1,
                ),
                choice(3, "Direction", "Quantize", &["Nearest", "Up", "Down"], 0),
                stepped(4, "Transpose", "Quantize", ParamUnit::Semitones, -24, 24, 0),
            ],
            0,
            0,
            true,
            0,
        ),
        BuiltinDeviceType::NoteLength => build(
            BuiltinDeviceType::NoteLength,
            "Note Length",
            DeviceCategory::NoteEffect,
            vec![
                choice(0, "Mode", "Length", &["Time", "Sync"], 0),
                param(
                    1,
                    "Length",
                    "Length",
                    ParamUnit::Milliseconds,
                    (1.0, 10000.0, 250.0),
                    ParamScale::Log,
                ),
                choice(2, "Sync Length", "Length", &SYNC_RATES, 6),
                param(
                    3,
                    "Gate",
                    "Length",
                    ParamUnit::Percent,
                    (1.0, 200.0, 100.0),
                    ParamScale::Linear,
                ),
                choice(4, "Trigger", "Length", &["Note On", "Note Off"], 0),
            ],
            0,
            0,
            true,
            0,
        ),
        BuiltinDeviceType::Velocity => build(
            BuiltinDeviceType::Velocity,
            "Velocity",
            DeviceCategory::NoteEffect,
            vec![
                choice(0, "Mode", "Velocity", &["Clip", "Gate", "Fixed"], 0),
                param(
                    1,
                    "Drive",
                    "Curve",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Compand",
                    "Curve",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                stepped(3, "Out Low", "Range", ParamUnit::None, 1, 127, 1),
                stepped(4, "Out High", "Range", ParamUnit::None, 1, 127, 127),
                stepped(5, "In Low", "Range", ParamUnit::None, 1, 127, 1),
                stepped(6, "In High", "Range", ParamUnit::None, 1, 127, 127),
                stepped(7, "Random", "Curve", ParamUnit::None, 0, 127, 0),
                choice(8, "Target", "Velocity", &["Note On", "Note Off", "Both"], 0),
            ],
            0,
            0,
            true,
            0,
        ),
        BuiltinDeviceType::Randomizer => build(
            BuiltinDeviceType::Randomizer,
            "Random",
            DeviceCategory::NoteEffect,
            vec![
                param(
                    0,
                    "Chance",
                    "Pitch",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                stepped(1, "Range", "Pitch", ParamUnit::Semitones, 0, 24, 0),
                choice(2, "Mode", "Pitch", &["Random", "Alternate"], 0),
                toggle(3, "Use Scale", "Pitch", false),
                param(
                    4,
                    "Velocity",
                    "Humanize",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Timing",
                    "Humanize",
                    ParamUnit::Milliseconds,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Length",
                    "Humanize",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                stepped(7, "Seed", "Humanize", ParamUnit::None, 0, 999, 0),
            ],
            0,
            0,
            true,
            0,
        ),
        other => unreachable!("{other:?} is not a `midi-fx` device"),
    }
}

/// Non-RT. A new instance (placeholder until implemented).
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    let ty = device.device_type();
    let mode = match ty {
        _ => PlaceholderMode::MidiThru,
    };
    Box::new(Placeholder::new(descriptor(ty), mode))
}

/// Factory presets of a type of this group (embedded; add `FactoryPreset { id, json:
/// include_str!("../../presets/<device-key>/<slug>.etherpreset") }` entries).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = ty;
    &[]
}
