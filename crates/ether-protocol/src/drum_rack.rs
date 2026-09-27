//! Drum racks and sampler slicing (roadmap v2, `drum-rack` node). All undoable. See
//! `ether_model::drum_rack` for the data model and playback rules.
//!
//! Pad chain devices are ordinary devices: rename, bypass, params, remove and automation go
//! through `DeviceCommand` by device id; only inserting into / moving between pad chains
//! needs these commands.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::devices::DeviceSpec;
use crate::model::{Color, Decibels, DeviceId, DrumPadId, Pan, Seconds};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DrumRackCommand {
    /// Add a pad on `rack` for `note` (client-chosen id; `InvalidArgument` if the note is
    /// taken). `name: None` = the note name ("C1").
    AddPad {
        id: DrumPadId,
        rack: DeviceId,
        note: u8,
        name: Option<String>,
    },
    /// Removes the pad with its chain.
    RemovePad {
        id: DrumPadId,
    },
    /// Move a pad to another note; if `note` is taken the two pads swap notes.
    SetPadNote {
        id: DrumPadId,
        note: u8,
    },
    RenamePad {
        id: DrumPadId,
        name: String,
    },
    SetPadColor {
        id: DrumPadId,
        color: Option<Color>,
    },
    /// `group: None` = no choke group; otherwise 1..=16.
    SetChokeGroup {
        id: DrumPadId,
        group: Option<u8>,
    },
    /// Continuous (send with a gesture).
    SetPadVolume {
        id: DrumPadId,
        volume: Decibels,
    },
    SetPadPan {
        id: DrumPadId,
        pan: Pan,
    },
    SetPadMute {
        id: DrumPadId,
        mute: bool,
    },
    /// Insert a device into a pad chain before `before` (None = end).
    InsertDevice {
        id: DeviceId,
        pad: DrumPadId,
        device: DeviceSpec,
        before: Option<DeviceId>,
    },
    /// Move a device into a pad chain (`pad: Some`) or back to the rack track's chain
    /// (`pad: None`), before `before`.
    MoveDevice {
        id: DeviceId,
        pad: Option<DrumPadId>,
        before: Option<DeviceId>,
    },
    /// Drop-a-sample shortcut: create pad `pad` on `note` with a sampler `device` playing
    /// `media` (one undo step).
    AddSamplePad {
        pad: DrumPadId,
        device: DeviceId,
        rack: DeviceId,
        note: u8,
        media: crate::model::MediaId,
    },
}

/// Slice markers of a sampler (`BuiltinDevice::Sampler { slices }`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SliceCommand {
    SetEnabled {
        device: DeviceId,
        enabled: bool,
    },
    SetBaseNote {
        device: DeviceId,
        note: u8,
    },
    /// Add markers (source seconds; duplicates within 1 ms are ignored).
    Add {
        device: DeviceId,
        positions: Vec<Seconds>,
    },
    /// Move marker `index` (in sorted order) to `position` (re-sorted after).
    Move {
        device: DeviceId,
        index: u32,
        position: Seconds,
    },
    Remove {
        device: DeviceId,
        indices: Vec<u32>,
    },
    /// Replace all markers by an automatic slicing of the sample.
    Auto {
        device: DeviceId,
        mode: AutoSlice,
    },
    /// Create a drum rack on the sampler's track holding one sampler pad per slice,
    /// replacing the sampler. One undo step. All ids are client-chosen (idempotent retries,
    /// collab-safe: two sites converting the same sampler must not mint different ids):
    /// `pads[i]` is used for slice `i`; fewer entries than slices is `InvalidArgument`, extra
    /// entries are ignored. Slice `i` gets pad note `slices.base_note + i` (slices past key
    /// 127 are dropped), name "Slice i+1", no choke group, and a sampler on the same sample
    /// whose own slice mode is off and whose sample range is that slice. The rack takes the
    /// sampler's place in the chain (same order key); the sampler's automation lanes and
    /// MIDI mappings are removed with it.
    ToDrumRack {
        device: DeviceId,
        rack: DeviceId,
        pads: Vec<SlicePadIds>,
    },
}

/// Ids for one pad created by `SliceCommand::ToDrumRack`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SlicePadIds {
    pub pad: DrumPadId,
    /// The sampler device on that pad.
    pub device: DeviceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AutoSlice {
    /// Onset detection; `sensitivity` 0..=1 (higher = more slices).
    Transients { sensitivity: f32 },
    /// Every `beats` at the project tempo at beat 0 (or the sample's warp tempo).
    Grid { beats: f64 },
    /// `count` equal slices.
    Equal { count: u32 },
}
