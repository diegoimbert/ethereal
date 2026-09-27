//! Drum racks and sampler slicing. Roadmap v2, owned by the `drum-rack` node (see
//! `docs/ROADMAP.md`).
//!
//! # Drum rack
//! A drum rack is a built-in instrument device (`BuiltinDevice::DrumRack`) on a MIDI track's
//! chain. Its pads are [`DrumPad`] entities (`DrumPad::rack == the rack device`), each with
//! its own device chain: devices whose `Device::pad` is the pad (their `Device::track` is
//! still the rack's track; chain order = `order` among devices of the same pad).
//!
//! - A pad receives the rack's notes whose key equals `DrumPad::note` (unique per rack) and
//!   passes them to its chain transposed to [`PAD_PLAY_NOTE`] (C3), so a sampler on the pad
//!   plays its sample at the root key.
//! - Pads in the same `choke_group` choke each other (a new hit silences the others).
//! - Each pad chain's output goes through the pad's volume/pan/mute and is summed into the
//!   rack's output. Pad chains contain no drum racks (no nesting).
//! - Removing a pad requires removing its devices first (the controller cascades), like
//!   tracks.
//!
//! # Sampler slicing
//! `BuiltinDevice::Sampler` carries its slice markers ([`SliceSettings`]). In slice mode,
//! slice `i` spans `[markers[i], markers[i + 1])` (the last one runs to the end of the
//! sample) and plays on note `base_note + i`. The data lives in the device kind (not as
//! entities) so it reaches the engine with the node (the controller re-creates the sampler
//! node when it changes, and the web bridge serializes it as-is).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{DeviceId, DrumPadId};
use crate::value::{Color, Decibels, Pan, Seconds};

/// The note a pad chain receives for every hit (MIDI C3).
pub const PAD_PLAY_NOTE: u8 = 60;
/// Choke groups are `1..=MAX_CHOKE_GROUP`.
pub const MAX_CHOKE_GROUP: u8 = 16;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DrumPad {
    pub id: DrumPadId,
    /// The `BuiltinDevice::DrumRack` device this pad belongs to.
    pub rack: DeviceId,
    /// Incoming MIDI key 0..=127 that triggers this pad (unique within the rack).
    pub note: u8,
    pub name: String,
    /// `None` = the rack's color.
    pub color: Option<Color>,
    /// `1..=MAX_CHOKE_GROUP`; `None` = no choke group.
    pub choke_group: Option<u8>,
    pub volume: Decibels,
    pub pan: Pan,
    pub mute: bool,
}

/// Slice markers of a sampler (see the module docs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SliceSettings {
    /// Slice mode on/off. Markers are kept while off.
    pub enabled: bool,
    /// Note of slice 0 (0..=127). Slices past key 127 are not playable.
    pub base_note: u8,
    /// Slice start positions in source seconds: sorted, distinct, finite, `>= 0`.
    pub markers: Vec<Seconds>,
}

impl Default for SliceSettings {
    /// Slice mode off, no markers, slice 0 on C1 (36, the usual first drum pad).
    fn default() -> Self {
        Self {
            enabled: false,
            base_note: 36,
            markers: Vec::new(),
        }
    }
}
