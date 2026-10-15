//! Mixer: volume, pan, mute, solo, routing, sends.
//!
//! Continuous controls (volume, pan, send level) are undoable document edits AND are pushed
//! to the engine's lock-free param queue immediately. Send them with a gesture id while
//! dragging and close with `Edit::EndGesture`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Decibels, Pan, SendId, TrackId, TrackOutput};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MixerCommand {
    SetVolume {
        track: TrackId,
        volume: Decibels,
    },
    SetPan {
        track: TrackId,
        pan: Pan,
    },
    SetMute {
        track: TrackId,
        mute: bool,
    },
    /// `exclusive`: unsolo all other tracks (plain click in Ableton; ctrl-click = additive).
    SetSolo {
        track: TrackId,
        solo: bool,
        exclusive: bool,
    },
    SetOutput {
        track: TrackId,
        output: TrackOutput,
    },
    /// `to` must be a Return track.
    CreateSend {
        id: SendId,
        from: TrackId,
        to: TrackId,
        level: Decibels,
        pre_fader: bool,
    },
    SetSendLevel {
        send: SendId,
        level: Decibels,
    },
    SetSendPreFader {
        send: SendId,
        pre_fader: bool,
    },
    DeleteSend {
        send: SendId,
    },
}
