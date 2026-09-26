//! Tracks and their mixer / routing settings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::TrackId;
use crate::value::{Color, Decibels, OrderKey, Pan};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum TrackKind {
    /// Plays audio clips; audio effects only.
    Audio,
    /// Plays MIDI clips into an instrument device.
    Midi,
    /// Sums its child tracks (`Track::parent == this`).
    Group,
    /// Receives sends (`TrackSend::to == this`).
    Return,
    /// The single master bus. Exactly one per project; cannot be deleted.
    Master,
}

/// A track. Tracks form an ordered tree: siblings sorted by `order`, nested via `parent`
/// (only `Group` tracks can be parents). Return tracks and the master track are top-level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    pub color: Color,
    pub order: OrderKey,
    pub parent: Option<TrackId>,
    pub mixer: TrackMixer,
    pub input: TrackInput,
    pub output: TrackOutput,
    /// Armed for recording.
    pub arm: bool,
    pub monitor: MonitorMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TrackMixer {
    pub volume: Decibels,
    pub pan: Pan,
    pub mute: bool,
    pub solo: bool,
}

impl Default for TrackMixer {
    fn default() -> Self {
        Self {
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
            solo: false,
        }
    }
}

/// Where a track takes its live input from (recording / monitoring).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TrackInput {
    None,
    /// Hardware audio input channels `[first, first + count)` (count 1 = mono, 2 = stereo).
    Audio {
        first: u16,
        count: u16,
    },
    /// MIDI input. `port: None` = all ports; `channel: None` = omni.
    Midi {
        port: Option<String>,
        channel: Option<u8>,
    },
    /// Post-fader output of another track (resampling).
    Track {
        track: TrackId,
    },
}

/// Where a track's post-fader signal goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TrackOutput {
    /// The master track (default). For the master track itself: the hardware output.
    Master,
    /// A group or return track.
    Track { track: TrackId },
    /// Not routed anywhere (still metered).
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum MonitorMode {
    /// Monitor input when armed and not playing back (Ableton "Auto").
    #[default]
    Auto,
    In,
    Off,
}
