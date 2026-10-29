//! Freeze, flatten, bounce in place and consolidate (v0.2; CONTRACTS.md
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

use crate::model::{Beats, ClipId, DeviceId, MediaId, Seconds, TrackId};

/// Client-chosen job id (ULID string, like export jobs).
pub type RenderJobId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum FreezeCommand {
    /// Render the track's post-chain, pre-fader signal from beat 0 to the end of its last
    /// clip plus the chain tail (`tail_seconds`, default 2 s, until silence below -90 dB) into
    /// `media`, then set `Track::freeze` (audio, MIDI and group tracks).
    ///
    /// **Group**: renders the group's post-chain, pre-fader signal: all its descendants
    /// (nested groups included) through their faders, then the group's devices, from beat 0
    /// to the end of the last descendant clip plus the tail. While frozen the descendants are
    /// inert (not processed, locked), their sends and the sidechains they feed go silent (a
    /// warning notification lists them), and nothing can be created or moved inside the
    /// group. Rejected (`InvalidState`) when a descendant is routed to a track outside the
    /// group, or the group is empty (`InvalidArgument`).
    ///
    /// **Freeze up to a device** (`up_to`): render only the source and the chain up to and
    /// including that device (`TrackFreeze::up_to`); the devices after it stay live and
    /// editable, processing the render. It must be a device of the track's own chain and
    /// not a MIDI effect (nor before the instrument of a MIDI track): `InvalidArgument`
    /// otherwise. The last device of the chain (or `None`) freezes the whole chain, exactly
    /// like a plain track freeze (stored as `up_to: None`).
    ///
    /// Flatten of a partial freeze keeps the live tail: it stays on the audio track, or moves
    /// to the new audio track (MIDI track, group).
    Freeze {
        job: RenderJobId,
        track: TrackId,
        media: MediaId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        up_to: Option<DeviceId>,
    },
    /// Clear `Track::freeze` (instantaneous; the media stays until unused and purged). A group
    /// inside a frozen group cannot be unfrozen on its own (`InvalidState`).
    Unfreeze { track: TrackId },
    /// Frozen tracks only (instantaneous, one undo step): make the freeze permanent.
    /// - Audio track: its clips and devices are deleted and replaced by one audio clip `clip`
    ///   of the freeze media at beat 0; the freeze is cleared.
    /// - MIDI track (it cannot hold audio): a new audio track `new_track` takes its place in
    ///   the track order (same name, color, parent, mixer, output; its sends are copied with
    ///   ids `derive_id(new_track, i)` in send-id order) holding the clip, and the MIDI track is
    ///   deleted. `new_track` is ignored for audio tracks.
    /// - Group: the group and everything inside it (descendants, their clips, devices and
    ///   sends) is replaced by the audio track `new_track` (same name, color, parent, mixer,
    ///   output, VCA and sends as the group, in its place) holding the clip. The group's own
    ///   fader/pan automation lanes are not carried over.
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
    ///
    /// The track's content is cut at `end` for the render (clips crossing it end there, later
    /// ones are silent). `NewTrack` with `include_chain` then keeps rendering past `end` until
    /// the output decays below -90 dBFS (instrument release, reverb and delay tails; checked
    /// every 0.5 s, at most 30 s, trailing silence trimmed): the clip is longer than the
    /// range by its tail. `InPlace` and clips-only renders are exactly the range.
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
    /// Convert an instrument device on a track's chain (an instrument rack, a synth, ...)
    /// into a multisampler of rendered notes: for each sampled key and velocity, render the
    /// chain up to and including `device` with that one note held for `note_seconds`, plus
    /// its release tail (`tail_seconds`, trailing silence trimmed), and replace the device
    /// with a `MultiSampler` whose zones play those samples.
    ///
    /// `keys` are the sampled keys and `velocities` the velocities each is rendered at (any
    /// order, duplicates ignored). Every key plays the nearest sampled key, pitched
    /// (`root_key` = the sampled key; a tie goes to the lower one), and every velocity the
    /// nearest rendered velocity (a tie goes to the lower one), so the zones cover 0..=127
    /// keys and 1..=127 velocities. One velocity keeps the multisampler's velocity
    /// sensitivity on (playing soft stays soft); several switch it off (each layer already
    /// has its level). Pitching is plain resampling (an octave up plays twice as fast),
    /// never pitch-preserving.
    ///
    /// Each sample's note starts `pre_roll_seconds` in: the device's latency is dropped, the
    /// start is never trimmed, the multisampler's `Offset` skips the pre-roll and its amp
    /// attack is 0 (no added fade-in), so a sampled key plays back sample for sample like the
    /// instrument. Lowering the `Offset` a little keeps what the instrument plays just
    /// before its note.
    ///
    /// Each sample is `device` alone (with what is nested in it): the track's other devices
    /// (the MIDI effects before it included, which stay in front of the sampler) and its
    /// racks' other chains don't play, its chain's mix isn't applied, and the macros of its
    /// racks that move its params are baked in at their current values.
    ///
    /// `device` must be a device of category `Instrument` on a MIDI track, on its chain or
    /// nested in racks there (not in a drum pad, not on a frozen track); the sampler takes
    /// its place (on the track chain or its rack chain). `new_device` and the
    /// media (`derive_id(seed_media, i)`) must not exist. At most `MAX_ZONES` samples are
    /// allowed (`InvalidArgument` otherwise). Replies `RenderStarted`; at `Done` the media
    /// and the replaced device are one undo step.
    ConvertToSampler {
        job: RenderJobId,
        device: DeviceId,
        new_device: DeviceId,
        seed_media: MediaId,
        /// Sampled keys (0..=127, at least one).
        keys: Vec<u8>,
        /// Rendered velocities (1..=127, 1 to 16 of them).
        velocities: Vec<u8>,
        /// Silence before each note: a render starts this long before its note (the
        /// instrument runs, transport playing, before the note-on), and the multisampler's
        /// `Offset` is set to it so playback starts on the note. A pass with plugins also
        /// holds before the note until this much real time has passed since they were
        /// loaded (each pass loads fresh instances). 0..=2 s.
        pre_roll_seconds: Seconds,
        /// With a pre-roll: also play the note very briefly
        /// ([`CONVERT_WARM_UP_NOTE_SECONDS`]) at the render's start, in the pre-roll the
        /// sampler skips (some plugins only finish loading a sound once it has played).
        warm_up_note: bool,
        /// How long each note is held before note-off.
        note_seconds: Seconds,
        /// Longest release tail kept after note-off (silence beyond is trimmed).
        tail_seconds: Seconds,
    },
    /// Cancel a running render (`Cancelled` follows). No-op if finished.
    Cancel { job: RenderJobId },
    /// Bounce a time selection: `[start, end)` of each listed audio/MIDI track (clips cut
    /// by the range are rendered trimmed to it) to a new audio track, exactly like
    /// `Bounce { include_chain: true, target: NewTrack }` per track (post-chain, tail, the
    /// source muted), as one render job and one undo step. Track `i` (in list order,
    /// duplicates ignored) bounces to track `derive_id(seed_tracks, i)` holding clip
    /// `derive_id(seed_clips, i)` of media `derive_id(seed_media, i)`. Listed tracks with no
    /// unmuted main-lane clip in the range are skipped; if none has one: `InvalidArgument`.
    BounceRange {
        job: RenderJobId,
        tracks: Vec<TrackId>,
        start: Beats,
        end: Beats,
        seed_tracks: TrackId,
        seed_clips: ClipId,
        seed_media: MediaId,
    },
}

/// Length of `ConvertToSampler`'s warm-up note (seconds).
pub const CONVERT_WARM_UP_NOTE_SECONDS: f64 = 0.005;

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
        /// The name of the computer that renders it, when it isn't this one (an extension's
        /// remote render). `None`: this one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        host: Option<String>,
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
