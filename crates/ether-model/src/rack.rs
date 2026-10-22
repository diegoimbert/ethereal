//! Racks: devices containing parallel chains (v0.2; see
//! CONTRACTS.md §12.6).
//!
//! A rack is a built-in device (`BuiltinDevice::{InstrumentRack, AudioEffectRack,
//! MidiEffectRack}`) on a track chain, a drum pad's chain or another rack's chain (nested).
//! Its chains are [`RackChain`] entities (`RackChain::rack == the rack device`), each with its own device chain: devices whose
//! `Device::chain` is the chain (their `Device::track` is still the rack's track; chain order =
//! `order` among devices of the same chain). This mirrors drum pads (`Device::pad`).
//!
//! # Signal flow
//! - **Instrument rack** (category `Instrument`): every chain receives the rack's incoming
//!   notes/MIDI filtered by the chain's key zone, velocity zone and chain-selector zone; the
//!   chains' audio outputs are mixed (chain volume/pan/mute/solo) into the rack's output.
//! - **Audio effect rack** (`AudioEffect`): every chain receives the rack's input audio (and
//!   its events filtered as above); the mixed chain outputs replace it. A rack with no chain
//!   passes audio through.
//! - **MIDI effect rack** (`NoteEffect`): every chain receives the incoming events filtered
//!   as above; the chains' output events are merged (sorted by offset) into the rack's
//!   output events. Audio passes through untouched.
//! - Solo: when any chain of a rack is soloed, only soloed chains sound.
//! - PDC inside a rack: every chain is delayed to the longest chain's latency (like drum pads).
//!
//! # Macros and the chain selector
//! Every rack has the same leading params: [`RACK_MACROS`] macro knobs (ids `0..8`, plain
//! `0..=1`, named "Macro 1".."Macro 8") and the chain selector (id [`RACK_SELECTOR_PARAM`],
//! `0..=127`). Macros do nothing by themselves: they are **modulation sources**
//! (`ModSource::Macro`, see [`crate::modulation`]) mapped to params of devices inside the rack.
//! Rack-type-specific params follow from id [`RACK_FIRST_TYPE_PARAM`] (append-only).
//!
//! A drum rack ([`crate::drum_rack`]) has the same 8 macros, appended after its own volume
//! and pan (ids [`DRUM_RACK_FIRST_MACRO`]`..+8`; no chain selector). [`macro_param`] and
//! [`macro_index`] map between a rack's macro index and its param id for every rack type.
//!
//! # Structure rules (model invariants)
//! - A chain's devices are on the rack's track; a device is on at most one of: the track
//!   chain, a drum pad (`pad`), a rack chain (`chain`).
//! - Nesting: a rack chain may hold racks (instrument / audio effect / MIDI effect), and an
//!   instrument rack's chain may hold a drum rack (an instrument). A drum pad's chain may
//!   hold chain racks, never a drum rack: no drum rack sits under a drum pad at any depth.
//!   At most [`MAX_RACK_DEPTH`] racks deep counting every rack on the way down from the
//!   track chain, drum racks included (a rack on a track-chain drum rack's pad is level 2),
//!   never inside itself (`Project::device_path`).
//! - A rack's macros (and modulators on the track-chain rack) reach devices at any depth
//!   inside it: on its chains or pads, and through the chains and pads of the racks nested
//!   in it (a drum rack on an instrument rack's chain included).
//! - Deleting a rack cascades (controller): mappings and modulators referencing its chain
//!   devices, chain devices (nested racks with their own chains), chains, then the rack.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::device::BuiltinDeviceType;
use crate::ids::{DeviceId, RackChainId};
use crate::value::{Color, Decibels, OrderKey, Pan, ParamId};

/// Number of macro knobs of every rack.
pub const RACK_MACROS: u8 = 8;
/// Param id of macro `i` (`0..RACK_MACROS`).
pub const fn rack_macro_param(i: u8) -> ParamId {
    ParamId(i as u32)
}
/// Param id of a drum rack's macro 1 (after its volume and pan; append-only).
pub const DRUM_RACK_FIRST_MACRO: u32 = 2;

/// Param id of macro `index` of a rack of type `ty` (`None`: not a rack, or `index` out of
/// range).
pub fn macro_param(ty: BuiltinDeviceType, index: u8) -> Option<ParamId> {
    if index >= RACK_MACROS {
        return None;
    }
    match ty {
        BuiltinDeviceType::DrumRack => Some(ParamId(DRUM_RACK_FIRST_MACRO + u32::from(index))),
        t if t.is_rack() => Some(rack_macro_param(index)),
        _ => None,
    }
}

/// Macro index of param `param` of a rack of type `ty` (`None`: not a macro param).
pub fn macro_index(ty: BuiltinDeviceType, param: ParamId) -> Option<u8> {
    let first = macro_param(ty, 0)?.0;
    let i = param.0.checked_sub(first)?;
    (i < u32::from(RACK_MACROS)).then_some(i as u8)
}

/// [`macro_index`] of a device kind (`None` for non-rack devices).
pub fn kind_macro_index(kind: &crate::device::DeviceKind, param: ParamId) -> Option<u8> {
    match kind {
        crate::device::DeviceKind::Builtin { device } => macro_index(device.device_type(), param),
        _ => None,
    }
}

/// Chain selector param (plain `0..=127`, default 0).
pub const RACK_SELECTOR_PARAM: ParamId = ParamId(8);
/// First rack-type-specific param id.
pub const RACK_FIRST_TYPE_PARAM: u32 = 9;
/// Deepest rack nesting: a track-chain rack is level 1, a rack on its chain (or on its pad,
/// for a drum rack) level 2, ...
pub const MAX_RACK_DEPTH: usize = 6;

/// An inclusive `0..=127` range (keys, velocities, selector values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Zone {
    pub lo: u8,
    pub hi: u8,
}

impl Zone {
    /// `0..=127`.
    pub const FULL: Zone = Zone { lo: 0, hi: 127 };

    pub fn contains(self, v: u8) -> bool {
        (self.lo..=self.hi).contains(&v)
    }
}

impl Default for Zone {
    fn default() -> Self {
        Self::FULL
    }
}

/// A parallel chain of a rack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct RackChain {
    pub id: RackChainId,
    /// The rack device (a `BuiltinDevice::*Rack` on a track chain).
    pub rack: DeviceId,
    /// Order among the rack's chains.
    pub order: OrderKey,
    pub name: String,
    /// `None` = the rack's color.
    pub color: Option<Color>,
    pub volume: Decibels,
    pub pan: Pan,
    pub mute: bool,
    pub solo: bool,
    /// Incoming note keys this chain receives (instrument and MIDI effect racks; ignored by
    /// audio effect racks for audio, applied to their events).
    pub keys: Zone,
    /// Incoming note-on velocities (1..=127 scale) this chain receives.
    pub velocities: Zone,
    /// Chain-selector values for which this chain is active (`RACK_SELECTOR_PARAM`).
    pub select: Zone,
}
