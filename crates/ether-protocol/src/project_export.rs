//! Exporting a project to other DAWs (`Project::ExportTo`, `project-export`; docs/EXPORT-DAW.md).
//!
//! `Project::ExportTo` converts a stored project (the open one: its current state) into a
//! project file of another application, with its audio files, and replies
//! [`ProjectExportResult`] carrying a [`ProjectExportReport`]: what was written, what was
//! approximated and what the target format could not hold. The work is synchronous (like
//! `ExportBundle`): bounded by the project's size, no job.
//!
//! Where the result goes (never a UI path, CONTRACTS.md §2b): the desktop passes the
//! absolute path chosen in its OS save dialog and the host writes the project file there,
//! its audio files in a `Samples/` folder next to it (a `.dawproject` is one ZIP that holds
//! its audio); hosts without OS files (web, remote) get a download to pull with
//! `Export::ReadChunk` and drop with `Export::Release` (Ableton and FL Studio exports with
//! audio are delivered as a `.zip` of the project file and its `Samples/` folder).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::export::ExportDownload;
use crate::import::{ImportReportItem, ImportSummary};

/// Target formats of `Project::ExportTo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum ProjectExportFormat {
    /// The open DAWproject 1.0 exchange format (`.dawproject`).
    DawProject,
    /// Ableton Live set (`.als`).
    Ableton,
    /// FL Studio project (`.flp`).
    FlStudio,
}

impl ProjectExportFormat {
    /// Every format, in menu order.
    pub const ALL: [ProjectExportFormat; 3] = [
        ProjectExportFormat::DawProject,
        ProjectExportFormat::Ableton,
        ProjectExportFormat::FlStudio,
    ];

    /// File extension of the project file (lowercase, no dot).
    pub fn extension(self) -> &'static str {
        match self {
            ProjectExportFormat::DawProject => "dawproject",
            ProjectExportFormat::Ableton => "als",
            ProjectExportFormat::FlStudio => "flp",
        }
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            ProjectExportFormat::DawProject => "DAWproject",
            ProjectExportFormat::Ableton => "Ableton Live Set",
            ProjectExportFormat::FlStudio => "FL Studio Project",
        }
    }
}

/// What `Project::ExportTo` wrote: counts, plus every approximation and everything the
/// target could not hold (`Dropped` = not exported). Same line types as the import report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ProjectExportReport {
    pub format: ProjectExportFormat,
    /// Counts of what was written (`media_files`: audio files copied next to / into it).
    pub summary: ImportSummary,
    /// Approximations, then drops, grouped by area (stable order).
    pub items: Vec<ImportReportItem>,
}

/// Reply payload of `Project::ExportTo`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ProjectExportResult {
    pub report: ProjectExportReport,
    /// Desktop (`path` given): the project file written (absolute engine-machine path).
    pub path: Option<String>,
    /// Web / remote (no `path`): the file to pull with `Export::ReadChunk`.
    pub download: Option<ExportDownload>,
}
