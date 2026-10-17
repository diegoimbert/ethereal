//! v0.2 devices owned by the `fx-modulation` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! Modulation effects: chorus, phaser, flanger, tremolo/auto-pan, vibrato.
//!
//! DSP in [`chorus`](self::chorus_device), [`phaser`](self::phaser_device),
//! [`flanger`](self::flanger_device), [`tremolo`](self::tremolo_device) and
//! [`vibrato`](self::vibrato_device); shared building
//! blocks (tempo-lockable LFO, Hermite-interpolated modulated delay line, param ramps) in
//! `shared`. Rules: no allocation after `prepare`, params applied at their sample offset
//! (`util::split_at_events`), gains ramped over one automation grid interval, delay times /
//! depths / frequencies glided, states flushed (denormal-safe). Synced LFOs lock their phase
//! to the song position while the transport plays.
//!
//! **Param ids are stable and append-only** (documents, automation and presets store them):
//! never renumber, only append.
//!
//! # Chorus (`BuiltinDeviceType::Chorus`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Chorus | `Mode` | Chorus / Ensemble / Vibrato (default Chorus) |
//! | 1 | Chorus | `Rate` | 0.01 ..= 10 Hertz (log), default 0.8 |
//! | 2 | Chorus | `Depth` | 0 ..= 100 Percent, default 50 |
//! | 3 | Chorus | `Delay` | 1 ..= 40 Milliseconds, default 7 |
//! | 4 | Chorus | `Voices` | 1 ..= 4 (stepped), default 2 |
//! | 5 | Chorus | `Spread` | 0 ..= 100 Percent, default 100 |
//! | 6 | Chorus | `Feedback` | 0 ..= 95 Percent, default 0 |
//! | 7 | Chorus | `High Cut` | 1000 ..= 20000 Hertz (log), default 20000 |
//! | 8 | Output | `Mix` | 0 ..= 100 Percent, default 50 |
//! | 9 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//! | 10 | Chorus | `Sync` | toggle, default off (appended by `devices-pass-effects`) |
//! | 11 | Chorus | `Sync Rate` | sync rate (default index 10, 1 bar) |
//!
//! # Phaser (`BuiltinDeviceType::Phaser`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Phaser | `Stages` | 2 / 4 / 6 / 8 / 12 (default 4) |
//! | 1 | Phaser | `Rate` | 0.01 ..= 20 Hertz (log), default 0.3 |
//! | 2 | Phaser | `Sync` | toggle, default off |
//! | 3 | Phaser | `Sync Rate` | sync rate (default index 6) |
//! | 4 | Phaser | `Depth` | 0 ..= 100 Percent, default 50 |
//! | 5 | Phaser | `Center` | 100 ..= 10000 Hertz (log), default 800 |
//! | 6 | Phaser | `Feedback` | -95 ..= 95 Percent, default 30 |
//! | 7 | Phaser | `Stereo Phase` | 0 ..= 180 None, default 90 |
//! | 8 | Output | `Mix` | 0 ..= 100 Percent, default 50 |
//! | 9 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//! | 10 | Phaser | `LFO Shape` | Sine / Triangle / Square / Saw Up / Saw Down (default Sine; `devices-pass-effects`) |
//!
//! # Flanger (`BuiltinDeviceType::Flanger`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Flanger | `Rate` | 0.01 ..= 20 Hertz (log), default 0.2 |
//! | 1 | Flanger | `Sync` | toggle, default off |
//! | 2 | Flanger | `Sync Rate` | sync rate (default index 6) |
//! | 3 | Flanger | `Depth` | 0 ..= 100 Percent, default 50 |
//! | 4 | Flanger | `Delay` | 0.1 ..= 20 Milliseconds (log), default 2 |
//! | 5 | Flanger | `Feedback` | -95 ..= 95 Percent, default 50 |
//! | 6 | Flanger | `Stereo Phase` | 0 ..= 180 None, default 90 |
//! | 7 | Output | `Mix` | 0 ..= 100 Percent, default 50 |
//! | 8 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//! | 9 | Flanger | `Through Zero` | toggle, default off (appended by `fx-modulation`) |
//! | 10 | Flanger | `LFO Shape` | Sine / Triangle / Square / Saw Up / Saw Down (default Sine; `devices-pass-effects`) |
//!
//! # Tremolo (`BuiltinDeviceType::Tremolo`)
//!
//! Auto-Pan mode modulates the balance (Stereo Phase is ignored).
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Tremolo | `Mode` | Tremolo / Auto-Pan (default Tremolo) |
//! | 1 | Tremolo | `Shape` | Sine / Triangle / Square / Saw Up / Saw Down (default Sine) |
//! | 2 | Tremolo | `Rate` | 0.1 ..= 40 Hertz (log), default 4 |
//! | 3 | Tremolo | `Sync` | toggle, default off |
//! | 4 | Tremolo | `Sync Rate` | sync rate (default index 6) |
//! | 5 | Tremolo | `Depth` | 0 ..= 100 Percent, default 50 |
//! | 6 | Tremolo | `Stereo Phase` | 0 ..= 180 None, default 0 |
//! | 7 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//!
//! # Vibrato (`BuiltinDeviceType::Vibrato`)
//!
//! The pitch wobbles with the LFO (modulated delay line; the shape is the pitch contour).
//! `Depth` is the peak deviation whatever the rate (capped at very slow rates by the 20 ms
//! excursion); `Phase` offsets the LFO in its cycle; latency = the centre delay (~20 ms).
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Vibrato | `Shape` | Sine / Triangle / Square / Saw Up / Saw Down / Random / Drift (default Sine) |
//! | 1 | Vibrato | `Rate` | 0.05 ..= 20 Hertz (log), default 3 |
//! | 2 | Vibrato | `Sync` | toggle, default off |
//! | 3 | Vibrato | `Sync Rate` | sync rate (default index 6, 1/8) |
//! | 4 | Vibrato | `Depth` | 0 ..= 200 Cents (power 2), default 30 |
//! | 5 | Vibrato | `Phase` | 0 ..= 360 None (degrees), default 0 |
//! | 6 | Vibrato | `Stereo Phase` | 0 ..= 180 None, default 0 |
//! | 7 | Vibrato | `Retrigger` | Free / Transport (default Transport) |
//! | 8 | Output | `Mix` | 0 ..= 100 Percent, default 100 |
//! | 9 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//!
//! # Volume LFO (`BuiltinDeviceType::VolumeLfo`)
//!
//! The gain follows a drawn LFO cycle (`BuiltinDevice::VolumeLfo { curve }`,
//! `ether_model::lfo_curve`): sidechain pumping, gates, chops. Synced, the phase is the song
//! position (phase 0 on every multiple of the cycle from beat 0). No `Mix`: on a gain-only
//! effect it would be the same blend as `Depth`. Zero latency.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | LFO | `Rate` | 0.05 ..= 20 Hertz (log), default 1 |
//! | 1 | LFO | `Sync` | toggle, default on |
//! | 2 | LFO | `Sync Rate` | sync rate (default index 8, 1/4) |
//! | 3 | LFO | `Depth` | 0 ..= 100 Percent, default 100 |
//! | 4 | LFO | `Phase` | 0 ..= 360 None (degrees), default 0 |
//! | 5 | LFO | `Stereo Phase` | 0 ..= 180 None, default 0 |
//! | 6 | Output | `Smoothing` | 0 ..= 20 Milliseconds (power 2), default 1 |
//! | 7 | Output | `Output` | -24 ..= 24 Decibels, default 0 |

mod chorus_device;
mod flanger_device;
mod phaser_device;
mod shared;
mod tremolo_device;
mod vibrato_device;
mod volume_lfo_device;

pub use flanger_device::TZ_MS as FLANGER_THROUGH_ZERO_MS;
pub use vibrato_device::{
    MAX_EXCURSION_MS as VIBRATO_MAX_EXCURSION_MS, SHAPES as VIBRATO_SHAPES,
    contour as vibrato_contour,
};
pub use volume_lfo_device::{CurveData as VolumeLfoCurve, curve_data as volume_lfo_curve};

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::{DeviceLayout, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};

use crate::contract::{
    FactoryPreset, SYNC_RATES, choice, descriptor as build, item, knob, layout, param, section,
    stepped, toggle,
};

/// Param ids of `Chorus` (stable, append-only).
pub mod chorus {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const RATE: ParamId = ParamId(1);
    pub const DEPTH: ParamId = ParamId(2);
    pub const DELAY: ParamId = ParamId(3);
    pub const VOICES: ParamId = ParamId(4);
    pub const SPREAD: ParamId = ParamId(5);
    pub const FEEDBACK: ParamId = ParamId(6);
    pub const HIGH_CUT: ParamId = ParamId(7);
    pub const MIX: ParamId = ParamId(8);
    pub const OUTPUT: ParamId = ParamId(9);
    /// Appended (devices-pass-effects): tempo-synced LFO.
    pub const SYNC: ParamId = ParamId(10);
    pub const SYNC_RATE: ParamId = ParamId(11);
    /// Number of params.
    pub const COUNT: usize = 12;
}

/// Param ids of `Phaser` (stable, append-only).
pub mod phaser {
    use ether_core::protocol::model::ParamId;
    pub const STAGES: ParamId = ParamId(0);
    pub const RATE: ParamId = ParamId(1);
    pub const SYNC: ParamId = ParamId(2);
    pub const SYNC_RATE: ParamId = ParamId(3);
    pub const DEPTH: ParamId = ParamId(4);
    pub const CENTER: ParamId = ParamId(5);
    pub const FEEDBACK: ParamId = ParamId(6);
    pub const STEREO_PHASE: ParamId = ParamId(7);
    pub const MIX: ParamId = ParamId(8);
    pub const OUTPUT: ParamId = ParamId(9);
    /// Appended (devices-pass-effects): LFO waveform.
    pub const LFO_SHAPE: ParamId = ParamId(10);
    /// Number of params.
    pub const COUNT: usize = 11;
}

/// Param ids of `Flanger` (stable, append-only).
pub mod flanger {
    use ether_core::protocol::model::ParamId;
    pub const RATE: ParamId = ParamId(0);
    pub const SYNC: ParamId = ParamId(1);
    pub const SYNC_RATE: ParamId = ParamId(2);
    pub const DEPTH: ParamId = ParamId(3);
    pub const DELAY: ParamId = ParamId(4);
    pub const FEEDBACK: ParamId = ParamId(5);
    pub const STEREO_PHASE: ParamId = ParamId(6);
    pub const MIX: ParamId = ParamId(7);
    pub const OUTPUT: ParamId = ParamId(8);
    /// Appended (fx-modulation): through-zero flanging (fixed dry delay, reported latency).
    pub const THROUGH_ZERO: ParamId = ParamId(9);
    /// Appended (devices-pass-effects): LFO waveform.
    pub const LFO_SHAPE: ParamId = ParamId(10);
    /// Number of params.
    pub const COUNT: usize = 11;
}

/// Param ids of `Tremolo` (stable, append-only).
pub mod tremolo {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const SHAPE: ParamId = ParamId(1);
    pub const RATE: ParamId = ParamId(2);
    pub const SYNC: ParamId = ParamId(3);
    pub const SYNC_RATE: ParamId = ParamId(4);
    pub const DEPTH: ParamId = ParamId(5);
    pub const STEREO_PHASE: ParamId = ParamId(6);
    pub const OUTPUT: ParamId = ParamId(7);
    /// Number of params.
    pub const COUNT: usize = 8;
}

/// Param ids of `Vibrato` (stable, append-only).
pub mod vibrato {
    use ether_core::protocol::model::ParamId;
    pub const SHAPE: ParamId = ParamId(0);
    pub const RATE: ParamId = ParamId(1);
    pub const SYNC: ParamId = ParamId(2);
    pub const SYNC_RATE: ParamId = ParamId(3);
    pub const DEPTH: ParamId = ParamId(4);
    pub const PHASE: ParamId = ParamId(5);
    pub const STEREO_PHASE: ParamId = ParamId(6);
    pub const RETRIGGER: ParamId = ParamId(7);
    pub const MIX: ParamId = ParamId(8);
    pub const OUTPUT: ParamId = ParamId(9);
    /// Number of params.
    pub const COUNT: usize = 10;
}

/// Param ids of `VolumeLfo` (stable, append-only).
pub mod volume_lfo {
    use ether_core::protocol::model::ParamId;
    pub const RATE: ParamId = ParamId(0);
    pub const SYNC: ParamId = ParamId(1);
    pub const SYNC_RATE: ParamId = ParamId(2);
    pub const DEPTH: ParamId = ParamId(3);
    pub const PHASE: ParamId = ParamId(4);
    pub const STEREO_PHASE: ParamId = ParamId(5);
    pub const SMOOTHING: ParamId = ParamId(6);
    pub const OUTPUT: ParamId = ParamId(7);
    /// Number of params.
    pub const COUNT: usize = 8;
}

/// Whether a live node built from `old` takes `new` in place (`Node::set_data` with
/// [`volume_lfo_curve`]): only the curve of a Volume LFO changed.
pub fn updatable_in_place(old: &BuiltinDevice, new: &BuiltinDevice) -> bool {
    matches!(
        (old, new),
        (
            BuiltinDevice::VolumeLfo { curve: a },
            BuiltinDevice::VolumeLfo { curve: b },
        ) if a != b
    )
}

/// Descriptor of a type of this group (param table + declarative layout).
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    let mut d = params_descriptor(ty);
    d.layout = Some(device_layout(ty));
    d
}

fn params_descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::Chorus => build(
            BuiltinDeviceType::Chorus,
            "Chorus",
            DeviceCategory::AudioEffect,
            vec![
                choice(0, "Mode", "Chorus", &["Chorus", "Ensemble", "Vibrato"], 0),
                param(
                    1,
                    "Rate",
                    "Chorus",
                    ParamUnit::Hertz,
                    (0.01, 10.0, 0.8),
                    ParamScale::Log,
                ),
                param(
                    2,
                    "Depth",
                    "Chorus",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Delay",
                    "Chorus",
                    ParamUnit::Milliseconds,
                    (1.0, 40.0, 7.0),
                    ParamScale::Linear,
                ),
                stepped(4, "Voices", "Chorus", ParamUnit::None, 1, 4, 2),
                param(
                    5,
                    "Spread",
                    "Chorus",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Feedback",
                    "Chorus",
                    ParamUnit::Percent,
                    (0.0, 95.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "High Cut",
                    "Chorus",
                    ParamUnit::Hertz,
                    (1000.0, 20000.0, 20000.0),
                    ParamScale::Log,
                ),
                param(
                    8,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    9,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                // --- devices-pass-effects ---
                toggle(10, "Sync", "Chorus", false),
                choice(11, "Sync Rate", "Chorus", &SYNC_RATES, 10),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Phaser => build(
            BuiltinDeviceType::Phaser,
            "Phaser",
            DeviceCategory::AudioEffect,
            vec![
                choice(0, "Stages", "Phaser", &["2", "4", "6", "8", "12"], 1),
                param(
                    1,
                    "Rate",
                    "Phaser",
                    ParamUnit::Hertz,
                    (0.01, 20.0, 0.3),
                    ParamScale::Log,
                ),
                toggle(2, "Sync", "Phaser", false),
                choice(3, "Sync Rate", "Phaser", &SYNC_RATES, 6),
                param(
                    4,
                    "Depth",
                    "Phaser",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Center",
                    "Phaser",
                    ParamUnit::Hertz,
                    (100.0, 10000.0, 800.0),
                    ParamScale::Log,
                ),
                param(
                    6,
                    "Feedback",
                    "Phaser",
                    ParamUnit::Percent,
                    (-95.0, 95.0, 30.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Stereo Phase",
                    "Phaser",
                    ParamUnit::None,
                    (0.0, 180.0, 90.0),
                    ParamScale::Linear,
                ),
                param(
                    8,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    9,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                // --- devices-pass-effects ---
                choice(10, "LFO Shape", "Phaser", &LFO_SHAPES, 0),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Flanger => build(
            BuiltinDeviceType::Flanger,
            "Flanger",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Rate",
                    "Flanger",
                    ParamUnit::Hertz,
                    (0.01, 20.0, 0.2),
                    ParamScale::Log,
                ),
                toggle(1, "Sync", "Flanger", false),
                choice(2, "Sync Rate", "Flanger", &SYNC_RATES, 6),
                param(
                    3,
                    "Depth",
                    "Flanger",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Delay",
                    "Flanger",
                    ParamUnit::Milliseconds,
                    (0.1, 20.0, 2.0),
                    ParamScale::Log,
                ),
                param(
                    5,
                    "Feedback",
                    "Flanger",
                    ParamUnit::Percent,
                    (-95.0, 95.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Stereo Phase",
                    "Flanger",
                    ParamUnit::None,
                    (0.0, 180.0, 90.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    8,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                toggle(9, "Through Zero", "Flanger", false),
                // --- devices-pass-effects ---
                choice(10, "LFO Shape", "Flanger", &LFO_SHAPES, 0),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Tremolo => build(
            BuiltinDeviceType::Tremolo,
            "Tremolo",
            DeviceCategory::AudioEffect,
            vec![
                choice(0, "Mode", "Tremolo", &["Tremolo", "Auto-Pan"], 0),
                choice(1, "Shape", "Tremolo", &LFO_SHAPES, 0),
                param(
                    2,
                    "Rate",
                    "Tremolo",
                    ParamUnit::Hertz,
                    (0.1, 40.0, 4.0),
                    ParamScale::Log,
                ),
                toggle(3, "Sync", "Tremolo", false),
                choice(4, "Sync Rate", "Tremolo", &SYNC_RATES, 6),
                param(
                    5,
                    "Depth",
                    "Tremolo",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Stereo Phase",
                    "Tremolo",
                    ParamUnit::None,
                    (0.0, 180.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Vibrato => build(
            BuiltinDeviceType::Vibrato,
            "Vibrato",
            DeviceCategory::AudioEffect,
            vec![
                choice(0, "Shape", "Vibrato", &vibrato_device::SHAPES, 0),
                param(
                    1,
                    "Rate",
                    "Vibrato",
                    ParamUnit::Hertz,
                    (0.05, 20.0, 3.0),
                    ParamScale::Log,
                ),
                toggle(2, "Sync", "Vibrato", false),
                choice(3, "Sync Rate", "Vibrato", &SYNC_RATES, 6),
                param(
                    4,
                    "Depth",
                    "Vibrato",
                    ParamUnit::Cents,
                    (0.0, 200.0, 30.0),
                    ParamScale::Power { exponent: 2.0 },
                ),
                param(
                    5,
                    "Phase",
                    "Vibrato",
                    ParamUnit::None,
                    (0.0, 360.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Stereo Phase",
                    "Vibrato",
                    ParamUnit::None,
                    (0.0, 180.0, 0.0),
                    ParamScale::Linear,
                ),
                choice(7, "Retrigger", "Vibrato", &["Free", "Transport"], 1),
                param(
                    8,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    9,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::VolumeLfo => build(
            BuiltinDeviceType::VolumeLfo,
            "Volume LFO",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Rate",
                    "LFO",
                    ParamUnit::Hertz,
                    (0.05, 20.0, 1.0),
                    ParamScale::Log,
                ),
                toggle(1, "Sync", "LFO", true),
                choice(2, "Sync Rate", "LFO", &SYNC_RATES, 8),
                param(
                    3,
                    "Depth",
                    "LFO",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Phase",
                    "LFO",
                    ParamUnit::None,
                    (0.0, 360.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Stereo Phase",
                    "LFO",
                    ParamUnit::None,
                    (0.0, 180.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Smoothing",
                    "Output",
                    ParamUnit::Milliseconds,
                    (0.0, 20.0, 1.0),
                    ParamScale::Power { exponent: 2.0 },
                ),
                param(
                    7,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        other => unreachable!("{other:?} is not a `fx-modulation` device"),
    }
}

/// LFO waveform labels (the order of `shared::Shape::ALL`), shared by every modulation
/// device's shape param.
const LFO_SHAPES: [&str; 5] = ["Sine", "Triangle", "Square", "Saw Up", "Saw Down"];

// --- layouts (drawn by the shared renderer) ---

fn toggle_item(param: ParamId) -> ether_core::protocol::layout::LayoutItem {
    item(Widget::Toggle { param }, WidgetSize::Small)
}

fn choice_item(param: ParamId) -> ether_core::protocol::layout::LayoutItem {
    item(Widget::Choice { param }, WidgetSize::Small)
}

/// The LFO section (devices-pass-effects): the LFO display (its shape / rate / depth
/// controls under it) spanning the section, then Sync and Sync Rate.
fn lfo_section(
    shape: ParamId,
    rate: ParamId,
    depth: ParamId,
    sync: ParamId,
    sync_rate: ParamId,
) -> ether_core::protocol::layout::LayoutSection {
    section(
        "lfo",
        Some("LFO"),
        2,
        2,
        vec![
            ether_core::protocol::layout::LayoutItem {
                colspan: 2,
                ..item(
                    Widget::Lfo {
                        shape,
                        rate,
                        amount: Some(depth),
                    },
                    WidgetSize::Large,
                )
            },
            toggle_item(sync),
            choice_item(sync_rate),
        ],
    )
}

fn device_layout(ty: BuiltinDeviceType) -> DeviceLayout {
    use WidgetSize::{Large, Medium, Small};
    match ty {
        // The chorus has no LFO waveform (its voices are sines): the display draws the
        // mode's sine and carries the Mode choice.
        BuiltinDeviceType::Chorus => layout(vec![
            lfo_section(
                chorus::MODE,
                chorus::RATE,
                chorus::DEPTH,
                chorus::SYNC,
                chorus::SYNC_RATE,
            ),
            section(
                "voices",
                Some("Voices"),
                2,
                3,
                vec![
                    knob(chorus::DELAY, Medium),
                    choice_item(chorus::VOICES),
                    knob(chorus::SPREAD, Medium),
                    knob(chorus::FEEDBACK, Medium),
                    knob(chorus::HIGH_CUT, Medium),
                ],
            ),
            section(
                "output",
                Some("Output"),
                1,
                1,
                vec![knob(chorus::MIX, Large), knob(chorus::OUTPUT, Small)],
            ),
        ]),
        BuiltinDeviceType::Phaser => layout(vec![
            lfo_section(
                phaser::LFO_SHAPE,
                phaser::RATE,
                phaser::DEPTH,
                phaser::SYNC,
                phaser::SYNC_RATE,
            ),
            section(
                "phaser",
                Some("Phaser"),
                2,
                2,
                vec![
                    knob(phaser::CENTER, Large),
                    choice_item(phaser::STAGES),
                    knob(phaser::FEEDBACK, Medium),
                    knob(phaser::STEREO_PHASE, Medium),
                ],
            ),
            section(
                "output",
                Some("Output"),
                1,
                1,
                vec![knob(phaser::MIX, Large), knob(phaser::OUTPUT, Small)],
            ),
        ]),
        BuiltinDeviceType::Flanger => layout(vec![
            lfo_section(
                flanger::LFO_SHAPE,
                flanger::RATE,
                flanger::DEPTH,
                flanger::SYNC,
                flanger::SYNC_RATE,
            ),
            section(
                "flanger",
                Some("Flanger"),
                2,
                2,
                vec![
                    knob(flanger::DELAY, Large),
                    toggle_item(flanger::THROUGH_ZERO),
                    knob(flanger::FEEDBACK, Medium),
                    knob(flanger::STEREO_PHASE, Medium),
                ],
            ),
            section(
                "output",
                Some("Output"),
                1,
                1,
                vec![knob(flanger::MIX, Large), knob(flanger::OUTPUT, Small)],
            ),
        ]),
        BuiltinDeviceType::Tremolo => layout(vec![
            section(
                "lfo",
                Some("LFO"),
                2,
                2,
                vec![
                    ether_core::protocol::layout::LayoutItem {
                        colspan: 2,
                        ..item(
                            Widget::Lfo {
                                shape: tremolo::SHAPE,
                                rate: tremolo::RATE,
                                amount: Some(tremolo::DEPTH),
                            },
                            Large,
                        )
                    },
                    toggle_item(tremolo::SYNC),
                    choice_item(tremolo::SYNC_RATE),
                ],
            ),
            section(
                "tremolo",
                Some("Tremolo"),
                1,
                1,
                vec![
                    choice_item(tremolo::MODE),
                    knob(tremolo::STEREO_PHASE, Medium),
                ],
            ),
            section(
                "output",
                Some("Output"),
                1,
                1,
                vec![knob(tremolo::OUTPUT, Medium)],
            ),
        ]),
        BuiltinDeviceType::Vibrato => layout(vec![
            lfo_section(
                vibrato::SHAPE,
                vibrato::RATE,
                vibrato::DEPTH,
                vibrato::SYNC,
                vibrato::SYNC_RATE,
            ),
            section(
                "phase",
                Some("Phase"),
                1,
                2,
                vec![
                    knob(vibrato::PHASE, Medium),
                    knob(vibrato::STEREO_PHASE, Medium),
                    choice_item(vibrato::RETRIGGER),
                ],
            ),
            section(
                "output",
                Some("Output"),
                1,
                1,
                vec![knob(vibrato::MIX, Large), knob(vibrato::OUTPUT, Small)],
            ),
        ]),
        BuiltinDeviceType::VolumeLfo => layout(vec![
            section(
                "lfo",
                Some("LFO"),
                2,
                2,
                vec![
                    ether_core::protocol::layout::LayoutItem {
                        colspan: 2,
                        ..item(
                            Widget::LfoCurve {
                                sync: volume_lfo::SYNC,
                                sync_rate: volume_lfo::SYNC_RATE,
                                rate: volume_lfo::RATE,
                                phase: volume_lfo::PHASE,
                                depth: volume_lfo::DEPTH,
                            },
                            Large,
                        )
                    },
                    toggle_item(volume_lfo::SYNC),
                    choice_item(volume_lfo::SYNC_RATE),
                    knob(volume_lfo::RATE, Medium),
                    knob(volume_lfo::DEPTH, Medium),
                ],
            ),
            section(
                "phase",
                Some("Phase"),
                1,
                1,
                vec![
                    knob(volume_lfo::PHASE, Medium),
                    knob(volume_lfo::STEREO_PHASE, Small),
                ],
            ),
            section(
                "output",
                Some("Output"),
                1,
                1,
                vec![
                    knob(volume_lfo::OUTPUT, Medium),
                    knob(volume_lfo::SMOOTHING, Small),
                ],
            ),
        ]),
        other => unreachable!("{other:?} is not a `fx-modulation` device"),
    }
}

/// Non-RT. A new instance.
///
/// # Panics
/// For a type of another group.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    let ty = device.device_type();
    let desc = params_descriptor(ty);
    match ty {
        BuiltinDeviceType::Chorus => Box::new(chorus_device::Chorus::new(&desc)),
        BuiltinDeviceType::Phaser => Box::new(phaser_device::Phaser::new(&desc)),
        BuiltinDeviceType::Flanger => Box::new(flanger_device::Flanger::new(&desc)),
        BuiltinDeviceType::Tremolo => Box::new(tremolo_device::Tremolo::new(&desc)),
        BuiltinDeviceType::Vibrato => Box::new(vibrato_device::Vibrato::new(&desc)),
        BuiltinDeviceType::VolumeLfo => {
            let BuiltinDevice::VolumeLfo { curve } = device else {
                unreachable!("checked by the type")
            };
            Box::new(volume_lfo_device::VolumeLfo::new(&desc, curve))
        }
        other => unreachable!("{other:?} is not a `fx-modulation` device"),
    }
}

macro_rules! presets {
    ($key:literal: $($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!($key, "/", $slug),
            json: include_str!(concat!("../../presets/", $key, "/", $slug, ".etherpreset")),
        }),*]
    };
}

const CHORUS_PRESETS: &[FactoryPreset] =
    presets!("chorus": "classic-chorus", "lush-ensemble", "subtle-width", "vibrato");
const PHASER_PRESETS: &[FactoryPreset] =
    presets!("phaser": "slow-swirl", "synced-quarters", "deep-twelve", "negative-bite");
const FLANGER_PRESETS: &[FactoryPreset] =
    presets!("flanger": "jet-sweep", "through-zero", "metallic-comb", "synced-flange");
const TREMOLO_PRESETS: &[FactoryPreset] =
    presets!("tremolo": "vintage-trem", "stereo-ping-pong", "synced-chop", "slow-auto-pan");
const VIBRATO_PRESETS: &[FactoryPreset] =
    presets!("vibrato": "singer", "tape-wow", "flutter", "seasick", "warble-chorus");
const VOLUME_LFO_PRESETS: &[FactoryPreset] = presets!("volume-lfo":
    "pump-1-4", "pump-1-4-soft", "pump-1-4-hard", "pump-1-8", "pump-1-2", "offbeat-duck",
    "half-time-pump", "trance-gate-1-16", "chopper-1-8", "swell", "tremolo-1-8",
    "fade-in-1-bar");

/// Factory presets of a type of this group (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::Chorus => CHORUS_PRESETS,
        BuiltinDeviceType::Phaser => PHASER_PRESETS,
        BuiltinDeviceType::Flanger => FLANGER_PRESETS,
        BuiltinDeviceType::Tremolo => TREMOLO_PRESETS,
        BuiltinDeviceType::Vibrato => VIBRATO_PRESETS,
        BuiltinDeviceType::VolumeLfo => VOLUME_LFO_PRESETS,
        _ => &[],
    }
}
