//! VST3 plugin hosting (native only). Owned by the `vst3` node; see `docs/PLUGIN-FORMATS.md`.
//!
//! [`Vst3Format`] is the [`PluginFormatHost`] for `.vst3` bundles. Discovery (search paths,
//! bundle walk) and the id convention ([`class_id_to_string`] / [`parse_class_id`]) are
//! real; scanning and instantiation return [`PluginError::Unsupported`] until the `vst3`
//! node implements them (COM hosting via the `vst3` crate).
//!
//! # Ids
//! A VST3 plugin id is the audio-processor class id (`PClassInfo::cid`) in the SDK's
//! canonical `FUID::toString` form (the `CID` of `moduleinfo.json`): 32 uppercase hex chars,
//! the four 32-bit words of `INLINE_UID(l1, l2, l3, l4)`. It is the same on every OS, so a
//! project moves between machines; [`class_id_to_string`] / [`parse_class_id`] convert from
//! and to the in-memory TUID (which uses the COM GUID byte layout on Windows). The id does
//! not contain the bundle path: `PluginDescriptor.path` (the `.vst3` bundle) comes from the
//! host's plugin catalog.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use ether_core::plugin::{PluginController, PluginError};
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;
use ether_plugin_host::bundles::{self, BundleShape};
use ether_plugin_host::{PluginFormatHost, unsupported};

/// What a VST3 bundle looks like: a `.vst3` bundle folder on every OS (VST 3.6.10+), or a
/// legacy single-file `.vst3` library on Windows/Linux. Bundle folders are not descended into
/// (on Windows they contain an inner `.vst3` binary).
pub const BUNDLE_SHAPE: BundleShape = BundleShape {
    extension: "vst3",
    files: true,
    dirs: true,
};

/// Platform default VST3 search paths (VST3 SDK "Plug-in Locations"), user paths first.
/// - macOS: `~/Library/Audio/Plug-Ins/VST3`, `/Library/Audio/Plug-Ins/VST3`
/// - Windows: `%LOCALAPPDATA%\Programs\Common\VST3`, `%COMMONPROGRAMFILES%\VST3`
/// - Linux/other: `~/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3`
pub fn default_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let home = bundles::home_dir();
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = &home {
            paths.push(home.join("Library/Audio/Plug-Ins/VST3"));
        }
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
    }
    #[cfg(target_os = "windows")]
    {
        let _ = &home;
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(local).join("Programs\\Common\\VST3"));
        }
        if let Some(common) = std::env::var_os("COMMONPROGRAMFILES") {
            paths.push(PathBuf::from(common).join("VST3"));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(home) = &home {
            paths.push(home.join(".vst3"));
        }
        paths.push(PathBuf::from("/usr/lib/vst3"));
        paths.push(PathBuf::from("/usr/local/lib/vst3"));
    }
    bundles::dedup_paths(paths)
}

/// Enumerate `.vst3` bundles under `paths` (no loading). Sorted, deduplicated.
pub fn find_bundles(paths: &[PathBuf]) -> Vec<PathBuf> {
    bundles::find_bundles(paths, BUNDLE_SHAPE)
}

/// Convert between the COM (Windows) TUID byte layout and the canonical big-endian
/// `l1 l2 l3 l4` order: `l1` little-endian, `l2` as two little-endian 16-bit halves, `l3`/`l4`
/// big-endian (`INLINE_UID` with `COM_COMPATIBLE`). It is its own inverse.
pub fn com_tuid_swizzle(t: &[u8; 16]) -> [u8; 16] {
    let mut o = *t;
    o[0..4].copy_from_slice(&[t[3], t[2], t[1], t[0]]);
    o[4..8].copy_from_slice(&[t[5], t[4], t[7], t[6]]);
    o
}

/// The plugin id of an in-memory class id (TUID) on this platform: canonical
/// `FUID::toString` hex (see the crate docs).
pub fn class_id_to_string(tuid: &[u8; 16]) -> String {
    let canonical = if cfg!(windows) {
        com_tuid_swizzle(tuid)
    } else {
        *tuid
    };
    canonical.iter().map(|b| format!("{b:02X}")).collect()
}

/// Inverse of [`class_id_to_string`]: the in-memory TUID on this platform. Accepts lowercase
/// hex too; anything but exactly 32 hex chars is `None`.
pub fn parse_class_id(id: &str) -> Option<[u8; 16]> {
    let bytes = id.as_bytes();
    if bytes.len() != 32 {
        return None;
    }
    let mut canonical = [0u8; 16];
    for (i, pair) in bytes.chunks_exact(2).enumerate() {
        let s = std::str::from_utf8(pair).ok()?;
        canonical[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(if cfg!(windows) {
        com_tuid_swizzle(&canonical)
    } else {
        canonical
    })
}

/// VST3 as a [`PluginFormatHost`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Vst3Format;

impl PluginFormatHost for Vst3Format {
    fn format(&self) -> PluginFormat {
        PluginFormat::Vst3
    }
    fn default_search_paths(&self) -> Vec<PathBuf> {
        default_search_paths()
    }
    fn discover(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        find_bundles(paths)
    }
    fn claims(&self, target: &Path) -> bool {
        bundles::has_extension(target, BUNDLE_SHAPE.extension)
    }
    fn scan(&self, target: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
        let _ = target;
        Err(unsupported(PluginFormat::Vst3, "scanning"))
    }
    fn instantiate(
        &self,
        path: &Path,
        plugin_id: &str,
    ) -> Result<Box<dyn PluginController>, PluginError> {
        let _ = (path, plugin_id);
        Err(unsupported(PluginFormat::Vst3, "loading"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_id_round_trip() {
        // INLINE_UID(0x56535441, 0x6D627261, 0x736F6E69, 0x63000000) on this platform.
        let canonical = *b"VSTAmbrasonic\0\0\0";
        let cid = if cfg!(windows) {
            com_tuid_swizzle(&canonical)
        } else {
            canonical
        };
        let id = class_id_to_string(&cid);
        assert_eq!(id, "565354416D627261736F6E6963000000");
        assert_eq!(parse_class_id(&id), Some(cid));
        assert_eq!(parse_class_id(&id.to_lowercase()), Some(cid));
        assert_eq!(parse_class_id("565354"), None);
        assert_eq!(parse_class_id(&"G".repeat(32)), None);
        assert_eq!(parse_class_id(&"é".repeat(16)), None);
    }

    #[test]
    fn com_layout() {
        // INLINE_UID(0x01020304, 0x05060708, 0x090A0B0C, 0x0D0E0F10) with COM_COMPATIBLE.
        let com = [4, 3, 2, 1, 6, 5, 8, 7, 9, 10, 11, 12, 13, 14, 15, 16];
        let canonical: [u8; 16] = std::array::from_fn(|i| i as u8 + 1);
        assert_eq!(com_tuid_swizzle(&com), canonical);
        assert_eq!(com_tuid_swizzle(&canonical), com);
    }

    #[test]
    fn stub_reports_unsupported() {
        let f = Vst3Format;
        assert_eq!(f.format(), PluginFormat::Vst3);
        assert!(f.claims(Path::new("/p/A.vst3")));
        assert!(f.claims(Path::new("/p/A.VST3")));
        assert!(!f.claims(Path::new("/p/A.clap")));
        assert!(!default_search_paths().is_empty());
        let e = f.scan(Path::new("/p/A.vst3")).unwrap_err();
        assert!(matches!(e, PluginError::Unsupported(_)), "{e:?}");
        let e = f
            .instantiate(Path::new("/p/A.vst3"), &"0".repeat(32))
            .err()
            .unwrap();
        assert!(matches!(e, PluginError::Unsupported(_)), "{e:?}");
    }
}
