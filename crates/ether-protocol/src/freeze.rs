//! Freeze, flatten, bounce in place and consolidate (v0.2, `freeze-bounce` node; CONTRACTS.md
//! §12.3).
//!
//! Rendering commands (`Freeze`, `Bounce`, `Consolidate` of audio) reply `RenderStarted {
//! job }` and run in the background on an `ether_core::offline::OfflineRenderer` (fresh nodes,
//! stepped from the controller tick, like `export`). Progress arrives as `Event::Freeze`, then
//! exactly one of `Done`, `Failed`, `Cancelled`. The document changes only at `Done`, as one
//! undo step (media insert + the edit). Renders land in the project's `media/` (never an
//! external reference). One render job at a time (`InvalidState` otherwise; independent of
//! export jobs).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, MediaId, TrackId};

/// Client-chosen job id (ULID string, like export jobs).
pub type RenderJobId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum FreezeCommand {
    /// Render the track's post-chain, pre-fader signal from beat 0 to the end of its last
    /// clip plus the chain tail (`tail_seconds`, default 2 s, until silence below -90 dB) into
    /// `media`, then set `Track::freeze` (audio/MIDI tracks only).
    Freeze {
        job: RenderJobId,
        track: TrackId,
        media: MediaId,
    },
    /// Clear `Track::freeze` (instantaneous; the media stays until unused and purged).
    Unfreeze { track: TrackId },
    /// Frozen tracks only (instantaneous, one undo step): make the freeze permanent.
    /// - Audio track: its clips and devices are deleted and replaced by one audio clip `clip`
    ///   of the freeze media at beat 0; the freeze is cleared.
    /// - MIDI track (it cannot hold audio): a new audio track `new_track` takes its place in
    ///   the track order (same name, color, parent, mixer, output; its sends are copied with
    ///   ids `derive_id(new_track, i)` in send-id order) holding the clip, and the MIDI track is
    ///   deleted. `new_track` is ignored for audio tracks.
    ///
    /// The clip is warped to the current tempo map (markers derived from it, playback rate 1),
    /// so it plays exactly the frozen audio at song time.
    Flatten {
        track: TrackId,
        clip: ClipId,
        new_track: TrackId,
    },
    /// Bounce `[start, end)` of `track` (post-chain when `include_chain`, else the clips
    /// only) to a new audio clip. Target: see [`BounceTarget`]. Replies `RenderStarted`.
    Bounce {
        job: RenderJobId,
        track: TrackId,
        start: Beats,
        end: Beats,
        include_chain: bool,
        media: MediaId,
        target: BounceTarget,
    },
    /// Join the clips of each listed track overlapping `[start, end)` into one clip per track
    /// spanning the range. MIDI: instantaneous (notes copied, new ids from `derive_id(seed, i)`
    /// by (track order, note start, pitch); the clip gets `derive_id(seed_clips, t)`). Audio:
    /// renders the clips only (pre-chain, gain/fades/warp applied) to project media
    /// (`derive_id(seed_media, t)`), replies `RenderStarted` if any audio track is listed.
    Consolidate {
        job: RenderJobId,
        tracks: Vec<TrackId>,
        start: Beats,
        end: Beats,
        seed_clips: ClipId,
        seed_notes: ClipId,
        seed_media: MediaId,
    },
    /// Cancel a running render (`Cancelled` follows). No-op if finished.
    Cancel { job: RenderJobId },
}

/// Where a bounce goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BounceTarget {
    /// A new audio track `track` right below the source (same output routing), with the clip
    /// at `start`; the source track is muted.
    NewTrack { track: TrackId, clip: ClipId },
    /// Audio tracks, `include_chain: false` only: the source clips in the range are trimmed
    /// out and replaced by `clip`.
    InPlace { clip: ClipId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum FreezeEvent {
    Progress {
        job: RenderJobId,
        /// 0..=1.
        progress: f32,
    },
    /// The document edit was applied (its patch precedes this event).
    Done {
        job: RenderJobId,
    },
    Failed {
        job: RenderJobId,
        message: String,
    },
    Cancelled {
        job: RenderJobId,
    },
}
