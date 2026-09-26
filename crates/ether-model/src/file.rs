//! The `.ether` file format: versioned JSON with migrations from day one.
//!
//! ```json
//! { "format": "ethereal-project", "version": 1, "app_version": "0.1.0", "project": { ... } }
//! ```
//!
//! Loading: parse to `serde_json::Value`, read `version`, run every migration from that
//! version up to [`CURRENT_VERSION`] on the untyped value, then deserialize and `validate`.
//! Media paths inside are preferably project-relative (`Samples/...`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::FileError;
use crate::project::Project;

/// Magic string in the `format` field.
pub const FORMAT_TAG: &str = "ethereal-project";
/// Current `.ether` version. Bump + add a [`Migration`] for every breaking schema change.
pub const CURRENT_VERSION: u32 = 1;
/// File extension (without dot).
pub const EXTENSION: &str = "ether";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EtherFile {
    pub format: String,
    pub version: u32,
    /// Version of the app that wrote the file (informational).
    pub app_version: String,
    pub project: Project,
}

/// Upgrades an untyped document from `from_version()` to `from_version() + 1`.
pub trait Migration: Send + Sync {
    fn from_version(&self) -> u32;
    fn migrate(&self, doc: &mut serde_json::Value) -> Result<(), FileError>;
}

/// All migrations, in order. Empty at version 1.
pub fn migrations() -> Vec<Box<dyn Migration>> {
    Vec::new()
}

/// Parse, migrate and validate an `.ether` document.
pub fn load(json: &str) -> Result<Project, FileError> {
    let _ = json;
    todo!("model node")
}

/// Serialize a project to pretty JSON at [`CURRENT_VERSION`] (stable key order: tables are
/// `BTreeMap`s, so saves are diff-friendly).
pub fn save(project: &Project, app_version: &str) -> Result<String, FileError> {
    let _ = (project, app_version);
    todo!("model node")
}
