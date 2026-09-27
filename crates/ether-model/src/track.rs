//! Tracks and their mixer / routing settings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{MediaId, TrackId};
use crate::value::{Color, Decibels, OrderKey, Pan, Seconds};

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
    /// v0.2 (`groups-buses`): a VCA fader. No audio, clips, devices or sends: its fader
    /// (`mixer.volume`, dB), mute and solo apply to the tracks assigned to it
    /// (`Track::vca`). Top-level only. See [`Track::vca`].
    Vca,
}

/// A track. Tracks form an ordered tree: siblings sorted by `order`, nested via `parent`
/// (only `Group` tracks can be parents; groups may nest). Return tracks and the master track
/// are top-level.
///
/// Group routing: a child whose `output` is `Master` is routed into its parent group's bus
/// (the controller resolves this when compiling the render graph); the group bus then goes
/// to its own `output`. Muting/soloing a group applies to its children.
///
/// Record-arm is **not** part of the document (momentary performance state, not undoable):
/// it lives in the controller and is reported via `RecordingEvent::ArmChanged`.
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
    pub monitor: MonitorMode,
    /// Piano-roll scale (MIDI tracks only; others stay `FollowProject`). Older files load
    /// as `FollowProject`.
    #[serde(default)]
    pub scale: crate::scale::TrackScale,
    /// Freeze state (v0.2, `freeze-bounce`; CONTRACTS.md §12.3). `Some` = the track plays
    /// `freeze.media` instead of its clips and device chain. Omitted from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub freeze: Option<TrackFreeze>,
    /// VCA assignment (v0.2, `groups-buses`; CONTRACTS.md §12.10): a `TrackKind::Vca` track.
    /// Effective fader gain = own volume (dB) + the VCA's volume (dB) + its own VCA's, and so
    /// on up the VCA chain (VCAs can be assigned to VCAs; no cycles). A muted VCA mutes its
    /// tracks; a soloed VCA solos them. Master cannot be assigned. Omitted when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub vca: Option<TrackId>,
}

/// A frozen track's render (v0.2, `freeze-bounce`).
///
/// While frozen, an audio or MIDI track plays `media` (the track's **post-chain, pre-fader**
/// signal rendered offline with `OfflineRenderer`, latency removed) at song time: frame `f`
/// of the media sounds at song second `start + f / media.sample_rate`, whatever the tempo
/// map. Its clips and devices are neither compiled nor instantiated (their CPU is saved), but
/// they stay in the document. Fader, pan, mute/solo, sends and routing stay live. The
/// controller rejects edits to a frozen track's clips, devices and device automation
/// (`InvalidState`); unfreeze first. Group, return and master tracks cannot be frozen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TrackFreeze {
    /// The render (project media, under `media/`; never an external reference).
    pub media: MediaId,
    /// Song time of the media's first frame (seconds, >= 0; 0 in v0.2).
    pub start: Seconds,
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
    /// Output of another track or bus (resampling; v0.2 `groups-buses` adds the tap point).
    /// A routing edge `track → this` (no cycles). The tapped signal is aligned to this track's
    /// input latency like a bus input (CONTRACTS.md §12.10).
    Track {
        track: TrackId,
        /// Missing in older files = `PostFader` (the v0.1 meaning).
        #[serde(default)]
        tap: InputTap,
    },
}

/// Where a track input taps its source track (v0.2, `groups-buses`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum InputTap {
    /// The source's input bus + clips, before its device chain.
    PreFx,
    /// After the source's device chain, before its fader.
    PostFx,
    /// After the fader, pan and mute/solo gate (sends excluded).
    #[default]
    PostFader,
}

/// Where a track's post-fader signal goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TrackOutput {
    /// Default: the parent group bus if the track is in a group, else the master track.
    /// For the master track itself: the hardware output.
    Default,
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
