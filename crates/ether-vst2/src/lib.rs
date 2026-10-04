//! VST2 plugin hosting (native only). Owned by the `vst2` node; see `docs/PLUGIN-FORMATS.md`.
//!
//! [`Vst2Format`] is the [`PluginFormatHost`] for VST 2.4 plugins, built on this crate's own
//! ABI bindings ([`abi`]), written from the GPL clean-room headers of FST and VeSTige: the
//! Steinberg VST2 SDK is not used (see the `abi` module docs and `THIRD_PARTY_NOTICES.txt`).
//!
//! - discovery: `.dll` (Windows), `.so` (Linux) libraries and `.vst` bundles (macOS) under the
//!   search paths (no loading);
//! - [`scan_library`]: load the library (`VSTPluginMain` / `main_macho` / `main`), open the
//!   plugin and describe it; a **shell** (`kPlugCategShell`) lists each sub-plugin
//!   (`effShellGetNextPlugin`), opened one by one through `audioMasterCurrentId`. Runs only
//!   in `ether-plugin-scanner`;
//! - [`instantiate`]: a [`Vst2Plugin`] (`PluginController`) whose `activate` resumes it
//!   (`effSetSampleRate`, `effSetBlockSize`, `effMainsChanged`, `effStartProcess`) and returns
//!   a [`Vst2Node`] (`PluginNode`: `processReplacing`, or `processDoubleReplacing` for
//!   64-bit-only plugins; MIDI through `effProcessEvents` with sample offsets; blocks split at
//!   param events; `VstTimeInfo` from `TransportInfo`; no allocation on the audio thread).
//!
//! # Threading
//! Like the other formats: [`Vst2Plugin`] is `!Send` and lives on the plugin main thread
//! (dispatcher calls, editor); call `poll` regularly (GUI edits, `audioMasterIOChanged`,
//! `audioMasterUpdateDisplay`, editor idle and resize). The node calls `processReplacing`,
//! `effProcessEvents` and `setParameter` on the audio thread. Both share one `AEffect`
//! (VST2 has a single object); `effClose` runs when both are gone.
//!
//! # Parameters
//! `ParamId` = the VST2 parameter index. Plain values are the plugin's normalized `0..=1`
//! floats (`ParamInfo { min: 0, max: 1, scale: Linear }`; `default` = the value right after
//! opening, as VST2 declares no defaults). Names come from `effGetParamName`; the plugin's own
//! text is [`Vst2Plugin::param_text`] (`effGetParamDisplay` + `effGetParamLabel`).
//! `audioMasterAutomate` / `BeginEdit` / `EndEdit` from the GUI become `ParamEdited` and
//! gesture notifications; host-initiated changes (`set_param_value`, `load_state`) and
//! changes the plugin makes while processing are not reported back.
//!
//! # State
//! `save_state`: `b"EthVST2\0"`, u32 LE version (1), u8 kind, i32 LE current program, then
//! - kind 1: u32 LE length + the bank chunk (`effGetChunk` index 0) for plugins with
//!   `effFlagsProgramChunks`;
//! - kind 0: u32 LE count + every parameter as f32 LE (plugins without chunks, or a chunk
//!   that came back empty).
//!
//! # Latency
//! `AEffect::initialDelay`, read when resumed; `audioMasterIOChanged` re-reads it (and the
//! channel counts) and reports `LatencyChanged` (plus `RestartRequested` when the channel
//! counts changed while active).
//!
//! # Ids
//! A VST2 plugin id is the plugin's `uniqueID` (a four-char code by convention) as **8
//! uppercase hex digits** of its 32-bit value (`'EtG2'` → `"45744732"`), see [`plugin_id`] /
//! [`parse_plugin_id`]. Shell sub-plugins use their own `uniqueID`. The library path is not
//! part of the id: `PluginDescriptor.path` (the `.dll`/`.so`/`.vst`) comes from the host's
//! plugin catalog.
#![cfg(not(target_arch = "wasm32"))]
// The ABI keeps the original C names (`effOpen`, `audioMasterAutomate`, ...).
#![allow(non_upper_case_globals)]

pub mod abi;
#[allow(dead_code)] // the shared host-window API: not every method is used on every OS
mod gui;
mod host;
mod module;
mod node;
mod plugin;
mod scan;
#[doc(hidden)]
pub mod testing;

use std::path::{Path, PathBuf};

use ether_core::plugin::{PluginController, PluginError};
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;
use ether_plugin_host::PluginFormatHost;
use ether_plugin_host::bundles;

pub use node::{Precision, Vst2Node};
pub use plugin::Vst2Plugin;
pub use scan::{category_from_features, features_for, scan_library};

/// The extension of a VST2 plugin on this OS: `dll` (Windows), `vst` (macOS bundles), `so`.
pub const EXTENSION: &str = if cfg!(windows) {
    "dll"
} else if cfg!(target_os = "macos") {
    "vst"
} else {
    "so"
};

/// Bundle-like folders of other formats (and app bundles) that are never descended into when
/// looking for VST2 libraries: a VST3 bundle on Linux holds a `.so`, a CLAP bundle a `.dll`...
const FOREIGN_BUNDLES: &[&str] = &[
    "vst3",
    "clap",
    "component",
    "lv2",
    "app",
    "bundle",
    "framework",
    "vst",
];

/// Instantiate a plugin in-process (plugin main thread).
pub fn instantiate(path: &Path, plugin_id: &str) -> Result<Box<dyn PluginController>, PluginError> {
    Ok(Box::new(Vst2Plugin::load(path, plugin_id)?))
}

/// The plugin id of a `uniqueID` (see the crate docs).
pub fn plugin_id(unique_id: i32) -> String {
    format!("{:08X}", unique_id as u32)
}

/// Inverse of [`plugin_id`]. Accepts lowercase hex; anything but 8 hex chars is `None`.
pub fn parse_plugin_id(id: &str) -> Option<i32> {
    (id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(id, 16).ok())
        .flatten()
        .map(|v| v as i32)
}

/// Platform default VST2 search paths, `VST_PATH` entries first.
/// - Windows: `%ProgramFiles%\VSTPlugins`, `%ProgramFiles%\Steinberg\VSTPlugins`,
///   `%CommonProgramFiles%\VST2`, `%CommonProgramFiles%\Steinberg\VST2`
/// - macOS: `/Library/Audio/Plug-Ins/VST`, `~/Library/Audio/Plug-Ins/VST`
/// - Linux/other: `~/.vst`, `/usr/lib/vst`, `/usr/local/lib/vst`
pub fn default_search_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::env::var_os("VST_PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    let home = bundles::home_dir();
    #[cfg(target_os = "windows")]
    {
        let _ = &home;
        let env = |k: &str, fallback: &str| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(fallback))
        };
        let pf = env("ProgramFiles", "C:\\Program Files");
        let common = env("CommonProgramFiles", "C:\\Program Files\\Common Files");
        paths.push(pf.join("VSTPlugins"));
        paths.push(pf.join("Steinberg\\VSTPlugins"));
        paths.push(common.join("VST2"));
        paths.push(common.join("Steinberg\\VST2"));
    }
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/VST"));
        if let Some(home) = &home {
            paths.push(home.join("Library/Audio/Plug-Ins/VST"));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(home) = &home {
            paths.push(home.join(".vst"));
        }
        paths.push(PathBuf::from("/usr/lib/vst"));
        paths.push(PathBuf::from("/usr/local/lib/vst"));
    }
    bundles::dedup_paths(paths)
}

/// Enumerate VST2 plugins under `paths` (no loading). Sorted, deduplicated.
pub fn find_plugins(paths: &[PathBuf]) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        return bundles::find_bundles(
            paths,
            bundles::BundleShape {
                extension: EXTENSION,
                files: false,
                dirs: true,
            },
        );
    }
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        if depth > 16 {
            return; // symlink loops
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                if !FOREIGN_BUNDLES
                    .iter()
                    .any(|ext| bundles::has_extension(&path, ext))
                {
                    walk(&path, depth + 1, out);
                }
            } else if bundles::has_extension(&path, EXTENSION) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        match std::fs::metadata(p) {
            Ok(m) if m.is_file() && bundles::has_extension(p, EXTENSION) => out.push(p.clone()),
            Ok(m) if m.is_dir() => walk(p, 0, &mut out),
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

/// VST2 as a [`PluginFormatHost`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Vst2Format;

impl PluginFormatHost for Vst2Format {
    fn format(&self) -> PluginFormat {
        PluginFormat::Vst2
    }
    fn default_search_paths(&self) -> Vec<PathBuf> {
        default_search_paths()
    }
    fn discover(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        find_plugins(paths)
    }
    fn claims(&self, target: &Path) -> bool {
        bundles::has_extension(target, EXTENSION)
    }
    fn scan(&self, target: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
        scan_library(target)
    }
    fn instantiate(
        &self,
        path: &Path,
        plugin_id: &str,
    ) -> Result<Box<dyn PluginController>, PluginError> {
        instantiate(path, plugin_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_round_trip() {
        let uid = abi::fourcc(*b"EtG2");
        assert_eq!(plugin_id(uid), "45744732");
        assert_eq!(parse_plugin_id("45744732"), Some(uid));
        assert_eq!(parse_plugin_id("4574473f"), Some(0x4574_473F));
        assert_eq!(plugin_id(-1), "FFFFFFFF");
        assert_eq!(parse_plugin_id("FFFFFFFF"), Some(-1));
        assert_eq!(plugin_id(0), "00000000");
        assert_eq!(parse_plugin_id("4574473"), None);
        assert_eq!(parse_plugin_id("+4574473"), None);
        assert_eq!(parse_plugin_id("G5744732"), None);
        assert_eq!(parse_plugin_id("ééééé"), None);
    }

    #[test]
    fn format_shape_and_paths() {
        let f = Vst2Format;
        assert_eq!(f.format(), PluginFormat::Vst2);
        assert!(f.claims(Path::new(&format!("/p/A.{EXTENSION}"))));
        assert!(f.claims(Path::new(&format!("/p/A.{}", EXTENSION.to_uppercase()))));
        assert!(!f.claims(Path::new("/p/A.vst3")));
        assert!(!f.claims(Path::new("/p/A.clap")));
        assert!(!default_search_paths().is_empty());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn discovery_skips_other_bundles() {
        let dir = testing::temp_dir("vst2-discover");
        let lib = |p: &Path| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"").unwrap();
        };
        let a = dir.join(format!("A.{EXTENSION}"));
        let b = dir.join(format!("vendor/B.{}", EXTENSION.to_uppercase()));
        lib(&a);
        lib(&b);
        // A VST3 bundle's inner library and a CLAP bundle are not VST2 plugins.
        lib(&dir.join(format!("C.vst3/Contents/x86_64-linux/C.{EXTENSION}")));
        lib(&dir.join(format!("D.clap/D.{EXTENSION}")));
        lib(&dir.join("readme.txt"));
        let mut expected = vec![a.clone(), b];
        expected.sort();
        assert_eq!(find_plugins(std::slice::from_ref(&dir)), expected);
        assert_eq!(find_plugins(std::slice::from_ref(&a)), vec![a]);
        assert!(find_plugins(&[dir.join("missing")]).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_and_bad_libraries() {
        let dir = testing::temp_dir("vst2-bad");
        let missing = dir.join(format!("Missing.{EXTENSION}"));
        let e = Vst2Format.scan(&missing).unwrap_err();
        assert!(matches!(e, PluginError::NotFound(_)), "{e:?}");
        let e = Vst2Format.instantiate(&missing, "nope").err().unwrap();
        assert!(matches!(e, PluginError::NotFound(_)), "{e:?}");
        let e = Vst2Format.instantiate(&missing, "45744732").err().unwrap();
        assert!(matches!(e, PluginError::NotFound(_)), "{e:?}");
        if !cfg!(target_os = "macos") {
            // Not a loadable library at all.
            let junk = dir.join(format!("Junk.{EXTENSION}"));
            std::fs::write(&junk, b"not a library").unwrap();
            let e = Vst2Format.scan(&junk).unwrap_err();
            assert!(matches!(e, PluginError::Load(_)), "{e:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
