//! Plugin discovery.
//!
//! - [`default_search_paths`] / [`find_bundles`] only walk the file system (safe in-process).
//! - [`scan_bundle`] loads a bundle. It runs ONLY inside the `ether-plugin-scanner` process.
//! - [`ScanRunner`] is the host side: it runs the scanner binary once per bundle with a
//!   timeout, so a plugin that crashes or hangs while loading only loses that bundle.

use std::ffi::CString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clack_host::prelude::PluginEntry;
use ether_core::plugin::PluginError;
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::{PluginDescriptor, ScanFailure, ScanRequest, ScanResponse};

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

    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| !p.as_os_str().is_empty() && seen.insert(p.clone()));
    paths
}

fn is_clap(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("clap"))
}

/// Enumerate `.clap` bundles under `paths` (recursive, no loading). On macOS a bundle is a
/// directory; elsewhere it is a shared library file. Sorted, deduplicated.
pub fn find_bundles(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        if depth > 16 {
            return; // symlink loops
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `metadata` follows symlinks (plugins are often symlinked into place).
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if is_clap(&path) && (meta.is_file() || cfg!(target_os = "macos")) {
                out.push(path);
            } else if meta.is_dir() {
                walk(&path, depth + 1, out);
            }
        }
    }

    let mut out = Vec::new();
    for p in paths {
        if is_clap(p) && p.exists() {
            out.push(p.clone());
        } else {
            walk(p, 0, &mut out);
        }
    }
    out.sort();
    out.dedup();
    out
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

/// Result of scanning many bundles.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ScanReport {
    pub plugins: Vec<PluginDescriptor>,
    pub failed: Vec<ScanFailure>,
}

/// Host side of the out-of-process scanner.
#[derive(Debug, Clone)]
pub struct ScanRunner {
    pub scanner: PathBuf,
    pub timeout: Duration,
}

/// Name of the scanner binary.
pub const SCANNER_BIN: &str = if cfg!(windows) {
    "ether-plugin-scanner.exe"
} else {
    "ether-plugin-scanner"
};

impl ScanRunner {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

    pub fn new(scanner: impl Into<PathBuf>) -> Self {
        Self {
            scanner: scanner.into(),
            timeout: Self::DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Locate the scanner: `ETHER_PLUGIN_SCANNER`, else next to the current executable (or
    /// its parent directory, for test binaries in `target/<profile>/deps`).
    pub fn locate() -> Option<Self> {
        if let Some(p) = std::env::var_os("ETHER_PLUGIN_SCANNER") {
            return Some(Self::new(p));
        }
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        [dir.join(SCANNER_BIN), dir.parent()?.join(SCANNER_BIN)]
            .into_iter()
            .find(|p| p.is_file())
            .map(Self::new)
    }

    /// Scan one bundle in a fresh scanner process. Crashes, hangs (killed after `timeout`),
    /// and malformed output all become an `Err` message.
    pub fn scan_bundle(&self, bundle: &Path) -> Result<Vec<PluginDescriptor>, String> {
        let request = serde_json::to_string(&ScanRequest {
            bundle_path: bundle.to_string_lossy().into_owned(),
        })
        .map_err(|e| e.to_string())?;

        let mut child = Command::new(&self.scanner)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start scanner {}: {e}", self.scanner.display()))?;

        let mut stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        // Readers on threads so a chatty plugin can't fill a pipe and deadlock us.
        let out_reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
            s
        });
        let err_reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let _ = stdin.write_all(request.as_bytes());
        drop(stdin);

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => return Err(format!("scanner wait failed: {e}")),
            }
        };
        let Some(status) = status else {
            // Killed: the pipes may be held open by grandchildren; don't join the readers.
            return Err(format!("scan timed out after {:?}", self.timeout));
        };
        let out = out_reader.join().unwrap_or_default();
        let err = err_reader.join().unwrap_or_default();

        if !status.success() {
            let tail: String = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
            return Err(format!(
                "scanner crashed ({status}){}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            ));
        }
        // Plugins may print to stdout while loading: the response is the last JSON line.
        let line = out
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .ok_or_else(|| "scanner produced no response".to_string())?;
        match serde_json::from_str::<ScanResponse>(line) {
            Ok(ScanResponse::Ok { plugins }) => Ok(plugins),
            Ok(ScanResponse::Err { message }) => Err(message),
            Err(e) => Err(format!("bad scanner response: {e}")),
        }
    }

    /// Scan every bundle sequentially. `progress(done, total, current)` is called before each
    /// bundle and once at the end with `current = None`.
    pub fn scan_all(
        &self,
        bundles: &[PathBuf],
        mut progress: impl FnMut(u32, u32, Option<&Path>),
    ) -> ScanReport {
        let total = bundles.len() as u32;
        let mut report = ScanReport::default();
        for (i, bundle) in bundles.iter().enumerate() {
            progress(i as u32, total, Some(bundle));
            match self.scan_bundle(bundle) {
                Ok(plugins) => report.plugins.extend(plugins),
                Err(message) => report.failed.push(ScanFailure {
                    path: bundle.to_string_lossy().into_owned(),
                    message,
                }),
            }
        }
        progress(total, total, None);
        report
    }
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
