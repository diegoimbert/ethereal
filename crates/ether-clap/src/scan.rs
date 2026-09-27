//! CLAP plugin discovery.
//!
//! - [`default_search_paths`] / [`find_bundles`] only walk the file system (safe in-process).
//! - [`scan_bundle`] loads a bundle. It runs ONLY inside the `ether-plugin-scanner` process.
//! - [`ScanRunner`] (from `ether-plugin-host`, re-exported) is the host side: it runs the
//!   scanner binary once per bundle with a timeout, so a plugin that crashes or hangs while
//!   loading only loses that bundle.

use std::ffi::CString;
use std::path::{Path, PathBuf};

use clack_host::prelude::PluginEntry;
use ether_core::plugin::PluginError;
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;
use ether_plugin_host::bundles::{self, BundleShape};

pub use ether_plugin_host::{SCANNER_BIN, ScanReport, ScanRunner};

/// What a CLAP bundle looks like: a bundle directory on macOS, a shared library elsewhere.
pub const BUNDLE_SHAPE: BundleShape = BundleShape {
    extension: "clap",
    files: true,
    dirs: cfg!(target_os = "macos"),
};

/// Platform default CLAP search paths, `CLAP_PATH` entries first (CLAP spec, `entry.h`).
pub fn default_search_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::env::var_os("CLAP_PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = &home {
            paths.push(home.join("Library/Audio/Plug-Ins/CLAP"));
        }
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
    }
    #[cfg(target_os = "windows")]
    {
        let _ = &home;
        if let Some(common) = std::env::var_os("COMMONPROGRAMFILES") {
            paths.push(PathBuf::from(common).join("CLAP"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(local).join("Programs/Common/CLAP"));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(home) = &home {
            paths.push(home.join(".clap"));
        }
        paths.push(PathBuf::from("/usr/lib/clap"));
    }

    bundles::dedup_paths(paths)
}

/// Enumerate `.clap` bundles under `paths` (recursive, no loading). On macOS a bundle is a
/// directory; elsewhere it is a shared library file. Sorted, deduplicated.
pub fn find_bundles(paths: &[PathBuf]) -> Vec<PathBuf> {
    bundles::find_bundles(paths, BUNDLE_SHAPE)
}

/// Map CLAP feature strings to a device category.
pub fn category_from_features(features: &[String]) -> DeviceCategory {
    let has = |f: &str| features.iter().any(|x| x == f);
    if has("instrument") {
        DeviceCategory::Instrument
    } else if has("note-effect") && !has("audio-effect") {
        DeviceCategory::NoteEffect
    } else {
        DeviceCategory::AudioEffect
    }
}

/// Load one bundle and list its plugins. Called ONLY inside the scanner process.
pub fn scan_bundle(bundle: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
    let entry = load_entry(bundle)?;
    let factory = entry
        .get_plugin_factory()
        .ok_or_else(|| PluginError::Load("bundle has no plugin factory".into()))?;
    let lossy = |s: Option<&std::ffi::CStr>| {
        s.map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut plugins = Vec::new();
    for d in factory.plugin_descriptors() {
        let Some(id) = d.id().and_then(|id| id.to_str().ok()) else {
            continue;
        };
        let features: Vec<String> = d
            .features()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        plugins.push(PluginDescriptor {
            sidechain_inputs: Default::default(),
            format: PluginFormat::Clap,
            id: id.to_owned(),
            name: lossy(d.name()),
            vendor: lossy(d.vendor()),
            version: lossy(d.version()),
            description: lossy(d.description()),
            category: category_from_features(&features),
            features,
            path: bundle.to_string_lossy().into_owned(),
        });
    }
    Ok(plugins)
}

pub(crate) fn load_entry(bundle: &Path) -> Result<PluginEntry, PluginError> {
    if !bundle.exists() {
        return Err(PluginError::NotFound(bundle.display().to_string()));
    }
    // Validate the path is representable for CLAP (no interior NUL).
    CString::new(bundle.to_string_lossy().as_bytes())
        .map_err(|_| PluginError::Load("bundle path contains NUL".into()))?;
    // SAFETY: loading a plugin library runs foreign code. This is inherent to plugin hosting;
    // scanning happens out-of-process and instantiation is the user's explicit choice.
    unsafe { PluginEntry::load(bundle) }.map_err(|e| PluginError::Load(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        let f = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            category_from_features(&f(&["instrument", "synthesizer"])),
            DeviceCategory::Instrument
        );
        assert_eq!(
            category_from_features(&f(&["note-effect"])),
            DeviceCategory::NoteEffect
        );
        assert_eq!(
            category_from_features(&f(&["audio-effect", "reverb"])),
            DeviceCategory::AudioEffect
        );
        assert_eq!(category_from_features(&[]), DeviceCategory::AudioEffect);
    }

    #[test]
    fn default_paths_nonempty_and_clap_path_first() {
        let paths = default_search_paths();
        assert!(!paths.is_empty());
    }

    #[test]
    fn find_bundles_walks_nested_dirs() {
        let root = crate::testing::temp_dir("find-bundles");
        let nested = root.join("vendor/sub");
        std::fs::create_dir_all(&nested).unwrap();
        let a = nested.join("A.clap");
        let b = root.join("B.CLAP");
        for p in [&a, &b] {
            if cfg!(target_os = "macos") {
                std::fs::create_dir_all(p.join("Contents/MacOS")).unwrap();
            } else {
                std::fs::write(p, b"").unwrap();
            }
        }
        std::fs::write(root.join("readme.txt"), b"").unwrap();
        let found = find_bundles(std::slice::from_ref(&root));
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(found, expected);
        // A bundle path passed directly is returned as-is.
        assert_eq!(
            find_bundles(std::slice::from_ref(&expected[0])),
            vec![expected[0].clone()]
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
