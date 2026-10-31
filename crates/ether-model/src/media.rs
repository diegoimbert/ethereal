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

/// `name` (a file or folder name) opens a device on Windows whatever its extension: `CON`,
/// `PRN`, `AUX`, `NUL`, `COM0`..`COM9`, `LPT0`..`LPT9` (also with superscript digits),
/// `CONIN$`, `CONOUT$`.
pub fn is_windows_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            let mut c = rest.chars();
            matches!((c.next(), c.next()), (Some(d), None) if d.is_ascii_digit() || "¹²³".contains(d))
        })
    };
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
}

/// Most bytes of a file stem [`safe_file_stem`] makes: file systems allow 255 per name,
/// which leaves room for an extension, a ` (2)` and a temporary-file suffix.
pub const MAX_FILE_STEM_BYTES: usize = 200;

/// `name` made a file stem that is valid and visible on every OS: characters unsafe in file
/// names become `replacement`, leading and trailing dots and spaces go, at most `max_chars`
/// characters and [`MAX_FILE_STEM_BYTES`] bytes, and a Windows device name gets a `_`
/// prefix. `None` when nothing is left.
pub fn safe_file_stem(name: &str, replacement: char, max_chars: usize) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                replacement
            } else {
                c
            }
        })
        .collect();
    let edge = |c: char| c == '.' || c.is_whitespace();
    let mut out = String::new();
    for c in cleaned.trim_start_matches(edge).chars().take(max_chars) {
        if out.len() + c.len_utf8() > MAX_FILE_STEM_BYTES {
            break;
        }
        out.push(c);
    }
    out.truncate(out.trim_end_matches(edge).len());
    if out.is_empty() {
        return None;
    }
    if is_windows_device_name(&out) {
        out.insert(0, '_');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stems() {
        let stem = |s: &str| safe_file_stem(s, '-', 80);
        assert_eq!(stem("Warm Pad").as_deref(), Some("Warm Pad"));
        assert_eq!(stem("a/b:c?").as_deref(), Some("a-b-c-"));
        assert_eq!(stem("  ..hidden. ").as_deref(), Some("hidden"));
        assert_eq!(stem(". . .x . .").as_deref(), Some("x"));
        assert_eq!(stem(" . ").as_deref(), None);
        assert_eq!(stem("con").as_deref(), Some("_con"));
        assert_eq!(stem("Com1.backup").as_deref(), Some("_Com1.backup"));
        assert_eq!(stem("console").as_deref(), Some("console"));
        assert_eq!(stem(&"x".repeat(200)).unwrap().len(), 80);
        // Four-byte characters: the byte cap, at a character boundary.
        let wide = stem(&"\u{1F3B9}".repeat(80)).unwrap();
        assert_eq!(wide.len(), MAX_FILE_STEM_BYTES);
        assert_eq!(wide.chars().count(), MAX_FILE_STEM_BYTES / 4);
    }

    #[test]
    fn device_names() {
        for n in [
            "CON",
            "nul.txt",
            "Aux",
            "com9",
            "LPT\u{b9}",
            "conout$",
            "prn.a.b",
        ] {
            assert!(is_windows_device_name(n), "{n}");
        }
        for n in ["CONS", "COM10", "LPT", "nulx", "a.con", ""] {
            assert!(!is_windows_device_name(n), "{n}");
        }
    }

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
