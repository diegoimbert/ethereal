//! Multisampler zones (v0.2, owned by the `multisampler` node; CONTRACTS.md §12.4).
//!
//! `BuiltinDevice::MultiSampler { zones }` keeps its zones in the device kind (like the
//! sampler's slices): they reach the engine with the node (`ether_devices::create`) and live
//! edits go through `EngineBridge::update_builtin` → `Node::set_data` (falling back to
//! re-creating the node), so they never cut sounding notes unnecessarily.
//!
//! # Zone selection (per note-on)
//! 1. Candidate zones: `keys` contains the key and `velocities` contains the velocity
//!    (`round(velocity · 127)`, clamped to 1..=127). Overlapping zones all sound (layers).
//! 2. Round robin: candidates with the same non-zero `round_robin` group alternate: the
//!    device keeps one counter per group and plays only the candidate whose position in the
//!    group (zones sorted by index) equals `counter % group size`; counters advance once per
//!    note-on. Group 0 = always plays.
//! 3. Each playing zone is transposed by `key - root_key + tune_cents/100` semitones
//!    (repitch), starts at `start`, loops `[loop_start, loop_end)` while held when `looping`
//!    is on, stops at `end`, and runs through the device's ADSR and filter.
//!
//! Zones reference media by `MediaId`: project media or external references
//! (`MediaLocation::External`, `media-references`), resolved like the sampler's sample.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::MediaId;
use crate::rack::Zone;
use crate::value::{Decibels, Pan, Seconds};

/// Maximum zones per multisampler.
pub const MAX_ZONES: usize = 512;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SampleZone {
    /// `None` = empty zone (silent).
    pub media: Option<MediaId>,
    /// Key at which the sample plays at its original pitch (0..=127).
    pub root_key: u8,
    /// Fine tune in cents, -100..=100.
    pub tune_cents: f32,
    pub keys: Zone,
    /// Velocity zone (1..=127 scale; 0 is treated as 1).
    pub velocities: Zone,
    /// Round-robin group (0 = none).
    pub round_robin: u8,
    /// Playback start / end in source seconds (`end: None` = end of the sample).
    pub start: Seconds,
    pub end: Option<Seconds>,
    /// Sustain loop in source seconds, used when `looping`.
    pub looping: bool,
    pub loop_start: Seconds,
    pub loop_end: Seconds,
    /// Loop crossfade in seconds (0 = none).
    pub loop_crossfade: Seconds,
    pub gain: Decibels,
    pub pan: Pan,
}

impl Default for SampleZone {
    /// Empty full-range zone rooted at C3 (60).
    fn default() -> Self {
        Self {
            media: None,
            root_key: 60,
            tune_cents: 0.0,
            keys: Zone::FULL,
            velocities: Zone { lo: 1, hi: 127 },
            round_robin: 0,
            start: Seconds(0.0),
            end: None,
            looping: false,
            loop_start: Seconds(0.0),
            loop_end: Seconds(0.0),
            loop_crossfade: Seconds(0.0),
            gain: Decibels(0.0),
            pan: Pan(0.0),
        }
    }
}
