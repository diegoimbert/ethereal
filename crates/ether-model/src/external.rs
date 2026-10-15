//! Hardware I/O devices (v0.3, owned by `external-instrument`; CONTRACTS.md §13.7) and the
//! convolution reverb's impulse response source (v0.3, `fx-space`; CONTRACTS.md §13.6).
//!
//! # External Instrument / External Audio Effect
//! `BuiltinDevice::ExternalInstrument { routing }` sends the track's MIDI to a hardware MIDI
//! output (`routing.midi_out`, a `MidiPort::id`, on `routing.midi_channel`) and plays the
//! hardware audio coming back on `routing.audio_return` (hardware input channels) as its
//! output. `BuiltinDevice::ExternalAudioEffect { routing }` sends its audio input to
//! `routing.audio_send` (hardware output channels) and returns `routing.audio_return`,
//! mixed with the dry signal by its `MIX` param.
//!
//! Port and channel choices are stored in the document (like Ableton): a project opened on
//! another machine keeps them and the device is silent until they resolve (a site without the
//! port shows it as missing; collab peers each use their own hardware). The round-trip
//! hardware latency is the device's `LATENCY` param (ms, measured by `External::
//! MeasureLatency` or set by hand) and is reported to PDC as the node's latency, so the
//! returned audio lines up with the rest of the mix.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::MediaId;

/// Hardware audio channels `[first, first + count)`, `count` 1 (mono) or 2 (stereo).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct HwChannels {
    pub first: u16,
    pub count: u16,
}

/// Hardware routing of an external device. Fields a device type doesn't use stay `None`
/// (`midi_out` is the instrument's; `audio_send` is the effect's).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ExternalRouting {
    /// Hardware MIDI output (`MidiPort::id`), External Instrument only. `None` = no MIDI out.
    pub midi_out: Option<String>,
    /// MIDI channel 1..=16 the instrument's notes go out on.
    pub midi_channel: u8,
    /// Hardware outputs the effect's input is sent to (External Audio Effect only).
    pub audio_send: Option<HwChannels>,
    /// Hardware inputs played as the device's output. `None` = silent return.
    pub audio_return: Option<HwChannels>,
}

impl Default for ExternalRouting {
    fn default() -> Self {
        Self {
            midi_out: None,
            midi_channel: 1,
            audio_send: None,
            audio_return: None,
        }
    }
}

/// Where a convolution reverb's impulse response comes from (`fx-space`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum IrSource {
    /// A factory IR shipped with the device (`ether_devices::fx_space::FACTORY_IRS` ids,
    /// append-only: `"room"`, `"hall"`, `"plate"`, `"chamber"`, ...).
    Factory { id: String },
    /// Any audio media of the project (imported or referenced in place, like samples).
    Media { media: MediaId },
}

/// Check a routing (channel ranges).
pub fn check_routing(r: &ExternalRouting) -> Result<(), String> {
    if !(1..=16).contains(&r.midi_channel) {
        return Err("external MIDI channel must be 1..=16".into());
    }
    for ch in [r.audio_send, r.audio_return].into_iter().flatten() {
        if !(1..=2).contains(&ch.count) {
            return Err("hardware channels are mono (1) or stereo (2)".into());
        }
        if ch.first.checked_add(ch.count).is_none() {
            return Err("hardware channel range overflows".into());
        }
    }
    if r.midi_out
        .as_deref()
        .is_some_and(|p| p.is_empty() || p.len() > 256)
    {
        return Err("MIDI output port id must be 1..=256 bytes".into());
    }
    Ok(())
}
