//! The default sample pack (#519; docs/SAMPLES.md): a signed, versioned manifest on our R2
//! storage, downloaded on first use, its files fetched lazily and cached engine-side (the
//! app data folder natively, OPFS on the web), plus a tiny built-in kit synthesized in code
//! for offline use.
//!
//! Sample, kit and instrument ids are qualified `<pack>/<id>` (a pack id has no `/`; the
//! built-in kit is pack [`BUILTIN_PACK`]). They are stable across pack versions: projects
//! reference pack samples by id and content hash, never by URL.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where the app looks for the default pack's signed manifest (overridable for testing:
/// `Samples::Open { url }`, `ETHER_SAMPLES_URL` natively). Nothing is published there until
/// the owner publishes a pack (scripts/samples/README.md): until then the app shows the
/// built-in kit and "more samples coming soon".
pub const DEFAULT_SAMPLES_URL: &str = "https://samples.ethereal.ws/packs/default/manifest.json";

/// Pack id of the built-in kit (synthesized in code, always available, never downloaded).
pub const BUILTIN_PACK: &str = "builtin";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SamplesCommand {
    /// First use (the browser's samples section, the drum rack kit picker or a sampler's
    /// picker opened): load the cached manifest, then refresh it from `url` (`None` = the
    /// default, [`DEFAULT_SAMPLES_URL`] or the host's override; `""` = no remote pack).
    /// Later calls only refresh when the source changed or the last refresh failed.
    /// Replies `Samples` with the current view; changes come as `SamplesEvent::Changed`.
    Open {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        url: Option<String>,
    },
    /// The current view, without network. Replies `Samples`.
    Get,
    /// Download these samples (qualified ids) if they aren't cached yet: each is checked
    /// against the manifest's size and sha256 before it's stored. Replies `Unit` at once;
    /// progress comes as `SamplesEvent::Progress`, then one `Fetched` or `Failed` per
    /// sample (cached and built-in samples report `Fetched` right away).
    Fetch { samples: Vec<String> },
    /// Drop the queued downloads (the ones in flight finish). Replies `Unit`.
    Cancel,
}

/// Where the pack stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackStatus {
    /// Not opened yet, or the first manifest download is running (no cached one).
    Loading,
    /// A verified manifest is in use; `offline`: the last refresh failed, so it's the
    /// cached one.
    Ready { offline: bool },
    /// No pack is published at the source (or none is configured): the built-in kit only.
    ComingSoon,
    /// No cached manifest and the source can't be reached.
    Offline,
    /// The source served something that failed verification (signature, schema, an older
    /// version than the cached one): the built-in kit only (or the cached pack, if any,
    /// keeps `Ready`).
    Invalid { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SampleCategory {
    /// A drum hit (used by kits; also listed as one-shots).
    Drum,
    OneShot,
    Loop,
    /// A sampler / multisampler source (see the instruments' zones).
    Instrument,
}

/// The license of a pack file. Only CC0 and our own recordings are accepted (#519).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SampleLicense {
    Cc0,
    Own,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SampleInfo {
    /// Qualified id `<pack>/<id>`.
    pub id: String,
    pub name: String,
    pub category: SampleCategory,
    pub tags: Vec<String>,
    pub bpm: Option<f64>,
    /// Musical key, lowercase (`"a minor"`).
    pub key: Option<String>,
    pub license: SampleLicense,
    /// Download size in bytes (0 for built-in samples).
    #[ts(type = "number")]
    pub size: u64,
    pub sample_rate: u32,
    pub channels: u16,
    #[ts(type = "number")]
    pub frames: u64,
    /// Already on this machine (or built in): plays without a download.
    pub cached: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct KitPadInfo {
    pub note: u8,
    /// Qualified sample id.
    pub sample: String,
    pub name: String,
    /// Choke group (pads of a group cut each other).
    pub choke: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct KitInfo {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub pads: Vec<KitPadInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ZoneInfo {
    pub sample: String,
    pub root: u8,
    pub lo_key: u8,
    pub hi_key: u8,
    pub lo_vel: u8,
    pub hi_vel: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InstrumentInfo {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub zones: Vec<ZoneInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    #[ts(type = "number")]
    pub version: u64,
}

/// Everything the browser and the pickers show: the built-in kit first, then the pack's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SamplesView {
    pub status: PackStatus,
    /// The downloaded pack in use, if any.
    pub pack: Option<PackInfo>,
    pub samples: Vec<SampleInfo>,
    pub kits: Vec<KitInfo>,
    pub instruments: Vec<InstrumentInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SamplesEvent {
    /// The status, the pack (a newer version was installed) or the cached flags changed.
    Changed {
        view: SamplesView,
    },
    /// Downloads: bytes of the current batch so far, its total, samples still queued.
    Progress {
        #[ts(type = "number")]
        done: u64,
        #[ts(type = "number")]
        total: u64,
        pending: u32,
    },
    /// The sample is cached (or built in): previews and imports work now.
    Fetched {
        sample: String,
    },
    Failed {
        sample: String,
        reason: String,
    },
}
