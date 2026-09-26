//! Sends. (Per-track volume/pan/mute/solo live in [`crate::track::TrackMixer`].)

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{SendId, TrackId};
use crate::value::Decibels;

/// A send from `from` to the return track `to`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TrackSend {
    pub id: SendId,
    pub from: TrackId,
    pub to: TrackId,
    pub level: Decibels,
    /// Tap before the track fader (default: post-fader).
    pub pre_fader: bool,
}
