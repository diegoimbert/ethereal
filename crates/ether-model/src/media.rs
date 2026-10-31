//! References to media used by the project.
//!
//! v0.1 projects were self-contained: importing audio copied it into the project's `media/`
//! folder. **v0.2 references library files in place**
//! ([`MediaLocation::External`]); recordings, bounces, freezes, uploads from a remote UI and
//! media received from collaborators still live in the project's `media/`
//! ([`MediaLocation::Project`]). The UI never opens or supplies file-system paths (it may
//! display an external path). See CONTRACTS.md §12.9.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::MediaId;

/// An imported audio file. Decoded data and peak mipmaps live in the store's per-project
/// cache (keyed by `MediaId` + `hash`), never in the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MediaRef {
    pub id: MediaId,
    /// Display name (original file name).
    pub name: String,
    /// Path relative to the project folder, always under `media/` (e.g.
    /// `media/01J...-kick.wav`). Resolved only by the engine-side `ProjectStore`.
    pub file: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Length in sample frames at `sample_rate`.
    #[ts(type = "number")]
    pub frames: u64,
    /// Content hash (hex) of the file, used to key caches and dedupe imports. Required for
    /// external references (relink and collab transfer match on it).
    pub hash: Option<String>,
    /// Where the audio is read from (v0.2). Missing in older files =
    /// `Project`.
    #[serde(default)]
    pub location: MediaLocation,
}

/// Where a [`MediaRef`]'s audio lives (v0.2).
///
/// Resolution on every site (engine-side): an `External` path that exists and whose content
/// hash matches `hash`; else the project copy at `MediaRef::file` (collected media, or media
/// a collaborator/remote client transferred by hash); else the media is **missing**
/// (`MediaEvent::Missing`, silence, relink offered).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaLocation {
    /// The project copy at `MediaRef::file` (v0.1 behaviour).
    #[default]
    Project,
    /// Referenced in place: an absolute engine-side path (inside a library root or a user
    /// folder). `MediaRef::file` is still a valid `media/...` path: where "collect all"
    /// copies it and where collaborators store their copy.
    External { path: String },
}

/// A path that names another machine: it starts with two separators, `/` and `\` in any mix
/// (`\\host\share`, `//host/share`, `/\host\share`, `\\?\UNC\...`, `\\.\pipe\...`). Windows
/// reads all of these as UNC or device paths, and merely looking at one authenticates to the
/// host, so a path that came with a document (an imported project or bundle) is never opened
/// when this holds.
pub fn is_network_path(path: &str) -> bool {
    let mut chars = path.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('/' | '\\'), Some('/' | '\\'))
    )
}

#[cfg(test)]
mod tests {
    use super::is_network_path;

    #[test]
    fn network_paths() {
        for p in [
            r"\\host\share\a.wav",
            "//host/share/a.wav",
            r"/\host\share\a.wav",
            r"\/host/share/a.wav",
            r"\\?\UNC\host\share\a.wav",
            r"\\.\pipe\a.wav",
        ] {
            assert!(is_network_path(p), "{p}");
        }
        for p in [
            "/home/a.wav",
            r"C:\a.wav",
            "C:/a.wav",
            r"\a.wav",
            "a.wav",
            "",
        ] {
            assert!(!is_network_path(p), "{p}");
        }
    }
}
