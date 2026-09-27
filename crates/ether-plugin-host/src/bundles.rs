//! File-system discovery of plugin bundles (no loading; safe in-process).

use std::path::{Path, PathBuf};

/// What a bundle of a format looks like on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BundleShape {
    /// Lowercase extension without the dot (`clap`, `vst3`, `component`). Matched
    /// case-insensitively.
    pub extension: &'static str,
    /// A matching regular file is a bundle (a single shared library).
    pub files: bool,
    /// A matching directory is a bundle (macOS bundles, VST3 bundle folders). It is not
    /// descended into. Otherwise matching directories are walked like any other folder.
    pub dirs: bool,
}

/// Whether `path` ends in `.<ext>` (case-insensitive).
pub fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Enumerate bundles of `shape` under `paths` (recursive, following symlinks, bounded depth).
/// A path that is itself a bundle is returned as-is. Sorted, deduplicated.
pub fn find_bundles(paths: &[PathBuf], shape: BundleShape) -> Vec<PathBuf> {
    fn is_bundle(path: &Path, meta: &std::fs::Metadata, shape: BundleShape) -> bool {
        has_extension(path, shape.extension)
            && ((meta.is_file() && shape.files) || (meta.is_dir() && shape.dirs))
    }
    fn walk(dir: &Path, depth: u32, shape: BundleShape, out: &mut Vec<PathBuf>) {
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
            if is_bundle(&path, &meta, shape) {
                out.push(path);
            } else if meta.is_dir() {
                walk(&path, depth + 1, shape, out);
            }
        }
    }

    let mut out = Vec::new();
    for p in paths {
        match std::fs::metadata(p) {
            Ok(meta) if is_bundle(p, &meta, shape) => out.push(p.clone()),
            Ok(meta) if meta.is_dir() => walk(p, 0, shape, &mut out),
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Deduplicate `paths` (keeping the first occurrence) and drop empty entries.
pub fn dedup_paths(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| !p.as_os_str().is_empty() && seen.insert(p.clone()));
    paths
}

/// The user's home directory (`HOME`, or `USERPROFILE` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| {
            if cfg!(windows) {
                std::env::var_os("USERPROFILE")
            } else {
                None
            }
        })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(purpose: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(ether_core::plugin::ipc_name(
            "test",
            std::process::id(),
            purpose,
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dir_bundles_are_not_descended_into() {
        let root = temp("bundles-dirs");
        // A VST3 bundle folder with the Windows-style inner binary also named `.vst3`.
        let a = root.join("vendor/A.vst3");
        std::fs::create_dir_all(a.join("Contents/x86_64-win")).unwrap();
        std::fs::write(a.join("Contents/x86_64-win/A.vst3"), b"").unwrap();
        let b = root.join("B.VST3");
        std::fs::write(&b, b"").unwrap();
        std::fs::write(root.join("readme.txt"), b"").unwrap();
        let shape = BundleShape {
            extension: "vst3",
            files: true,
            dirs: true,
        };
        let found = find_bundles(std::slice::from_ref(&root), shape);
        let mut expected = vec![a.clone(), b];
        expected.sort();
        assert_eq!(found, expected);
        assert_eq!(find_bundles(std::slice::from_ref(&a), shape), vec![a]);
        // Files only: matching directories are walked instead.
        let files_only = BundleShape {
            dirs: false,
            ..shape
        };
        let found = find_bundles(std::slice::from_ref(&root), files_only);
        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .any(|p| p.ends_with("Contents/x86_64-win/A.vst3"))
        );
        assert!(find_bundles(&[root.join("missing")], shape).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dedup_keeps_order() {
        let v = dedup_paths(vec!["/b".into(), "".into(), "/a".into(), "/b".into()]);
        assert_eq!(v, vec![PathBuf::from("/b"), PathBuf::from("/a")]);
    }
}
