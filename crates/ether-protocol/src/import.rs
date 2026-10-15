//! Importing projects from other DAWs (`Project::Import`, `import-dawproject`; CONTRACTS.md
//! §14.1).
//!
//! `Project::Import` reads a project file of another application (DAWproject, Ableton Live
//! `.als`, FL Studio `.flp`, REAPER `.rpp`), converts it to a new stored project and opens
//! it. It replies `Unit` at once; the conversion runs from the controller tick as a job with
//! progress (bounded work per tick: parsing, then one media file per tick) and ends with
//! [`ImportEvent::Done`] (after `Event::ProjectLoaded` of the new project) carrying the
//! [`ImportReport`], or [`ImportEvent::Failed`]. One import at a time per controller.
//!
//! The format is detected from the content (ZIP with `project.xml` = DAWproject, gzip =
//! Ableton, `FLhd` = FL Studio, `<REAPER_PROJECT` = REAPER), never from the file name.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::ProjectId;

/// Source formats of `Project::Import`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum ImportFormat {
    /// The open DAWproject exchange format (`.dawproject`): Bitwig Studio, Studio One,
    /// Cubase, Nuendo, Cubasis, VST Live, n-Track Studio, Waveform, ... (docs/IMPORT.md).
    DawProject,
    /// Ableton Live set (`.als`).
    Ableton,
    /// FL Studio project (`.flp`).
    FlStudio,
    /// REAPER project (`.rpp`).
    Reaper,
}

impl ImportFormat {
    /// Every format, in display order.
    pub const ALL: [ImportFormat; 4] = [
        ImportFormat::DawProject,
        ImportFormat::Ableton,
        ImportFormat::FlStudio,
        ImportFormat::Reaper,
    ];

    /// File extensions (lowercase, without the dot) the UI offers in pickers and accepts on
    /// drop for this format.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            ImportFormat::DawProject => &["dawproject"],
            ImportFormat::Ableton => &["als"],
            ImportFormat::FlStudio => &["flp"],
            ImportFormat::Reaper => &["rpp", "rpp-bak"],
        }
    }

    /// Display name ("DAWproject", "Ableton Live", ...).
    pub fn label(self) -> &'static str {
        match self {
            ImportFormat::DawProject => "DAWproject",
            ImportFormat::Ableton => "Ableton Live",
            ImportFormat::FlStudio => "FL Studio",
            ImportFormat::Reaper => "REAPER",
        }
    }
}

/// What a report line says about an element of the source project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub enum ImportItemStatus {
    /// Imported, but not exactly: mapped to the closest Ethereal equivalent.
    Approximated,
    /// Not imported (no Ethereal equivalent, unreadable or missing data).
    Dropped,
    /// Imported as a missing item to resolve (a device kept as a placeholder, an audio file
    /// not found): its data is kept until it is resolved (`Resolve`, CONTRACTS.md §14.2).
    NeedsResolving,
}

/// Area of the project a report line is about (grouping in the report dialog).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub enum ImportArea {
    Project,
    Tracks,
    Routing,
    Clips,
    Notes,
    Audio,
    Automation,
    Tempo,
    Markers,
    Devices,
    Plugins,
    Media,
}

/// One line of the report. Identical lines are merged (`count`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ImportReportItem {
    pub status: ImportItemStatus,
    pub area: ImportArea,
    /// What it is about (a track, device or file name), when one thing.
    pub subject: Option<String>,
    /// Plain-language explanation ("Clip launcher scenes are not supported").
    pub message: String,
    /// How many times this line occurred (>= 1).
    pub count: u32,
}

/// Counts of what was imported.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ImportSummary {
    /// Tracks created (groups, returns included; the master is not counted).
    pub tracks: u32,
    pub midi_clips: u32,
    pub audio_clips: u32,
    pub notes: u32,
    pub automation_lanes: u32,
    pub automation_points: u32,
    /// Built-in devices created.
    pub devices: u32,
    /// Plugin instances created (with their state when the source carries it).
    pub plugins: u32,
    pub sends: u32,
    pub markers: u32,
    /// Tempo points (beat 0 included).
    pub tempo_points: u32,
    /// Time-signature points (beat 0 included).
    pub time_signatures: u32,
    /// Audio files copied into (or referenced by) the project.
    pub media_files: u32,
}

/// What `Project::Import` did: counts, plus every approximation and everything dropped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ImportReport {
    pub format: ImportFormat,
    /// The application that wrote the source ("Bitwig Studio 5.0"), when known.
    pub application: Option<String>,
    pub summary: ImportSummary,
    /// Approximations, then drops, grouped by area (stable order).
    pub items: Vec<ImportReportItem>,
}

/// `Event::Import` (one job at a time; `id` = the new project's id).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ImportEvent {
    Progress {
        id: ProjectId,
        /// 0..=1.
        progress: f32,
        /// What is being done ("reading project", "copying audio 3/12").
        stage: String,
    },
    /// The new project is stored and open (its `Event::ProjectLoaded` precedes this).
    Done {
        id: ProjectId,
        project: crate::project::ProjectSummary,
        report: ImportReport,
    },
    /// Nothing was created (any half-written project folder is removed).
    Failed { id: ProjectId, message: String },
    /// `Project::CancelImport`: nothing was created.
    Cancelled { id: ProjectId },
}
