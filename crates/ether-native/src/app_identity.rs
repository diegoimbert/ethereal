//! The desktop app's bundle identifier, and the one-time move of the data of an install made
//! under an earlier identifier.
//!
//! The identifier names the app's per-user directories: the app data dir (settings, plugin
//! database and caches, autosaves, telemetry install id), the logs, and the WebView's own
//! storage (localStorage, IndexedDB, cookies). When it changes, a new build would start with
//! empty directories while the old ones sit next to them. [`migrate_legacy_dirs`] runs once
//! at startup, before any of them is opened, and moves the old directories to the new names.
//!
//! Nothing is deleted. A directory whose new name does not exist yet is renamed. When both
//! exist (a build with the new identifier already ran, or an older build ran again after the
//! move), the old one is merged in: entries missing from the new directory are moved, and
//! entries present in both stay where they are (the new one wins, the old copy is left in the
//! old directory, and the report lists them). The old directory is removed only once it is
//! empty. Running it again is harmless: without an old directory there is nothing to do.
//!
//! Dev builds keep their own per-instance directories (`ethereal-dev/<instance>`) and are not
//! migrated.

use std::io;
use std::path::{Path, PathBuf};

/// The release bundle identifier (`identifier` in `apps/desktop/src-tauri/tauri.conf.json`):
/// reverse-DNS of ethereal.ws, plus a third segment.
pub const DESKTOP_IDENTIFIER: &str = "ws.ethereal.daw";

/// Identifiers earlier releases used, most recent first. Their directories are moved to
/// [`DESKTOP_IDENTIFIER`]'s by [`migrate_legacy_dirs`].
pub const LEGACY_DESKTOP_IDENTIFIERS: &[&str] = &["dev.ethereal.app"];

/// What [`migrate_legacy_dirs`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// `(from, to)`: directories and files moved.
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// `(from, to)`: files copied (macOS preferences, see [`IdentityDirs::copied_files`]).
    pub copied: Vec<(PathBuf, PathBuf)>,
    /// Old entries kept in place because the new location already has an entry of that name.
    pub kept: Vec<PathBuf>,
    /// Entries that could not be moved (left in place; retried at the next launch).
    pub errors: Vec<(PathBuf, String)>,
}

impl MigrationReport {
    pub fn is_empty(&self) -> bool {
        self.moved.is_empty()
            && self.copied.is_empty()
            && self.kept.is_empty()
            && self.errors.is_empty()
    }

    /// One log line per entry (nothing when there was nothing to do).
    pub fn log(&self) {
        for (from, to) in &self.moved {
            tracing::info!(from = %from.display(), to = %to.display(), "app data moved to the new identifier");
        }
        for (from, to) in &self.copied {
            tracing::info!(from = %from.display(), to = %to.display(), "app file copied to the new identifier");
        }
        for p in &self.kept {
            tracing::warn!(path = %p.display(), "old app data kept: the new location already has it");
        }
        for (p, e) in &self.errors {
            tracing::warn!(path = %p.display(), error = %e, "old app data not moved (retried at the next launch)");
        }
    }
}

/// [`migrate_legacy_dirs`] on this OS's directories, logged. The desktop app calls it first
/// thing in release builds, before Tauri or the WebView open any directory.
pub fn migrate_on_startup() -> MigrationReport {
    let report = migrate_legacy_dirs(&IdentityDirs::current());
    report.log();
    report
}

/// Where the identifier names directories on this OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityDirs {
    /// Parent directories of `<identifier>/` directories.
    pub bases: Vec<PathBuf>,
    /// `(dir, suffix)`: files named `<identifier><suffix>` in `dir`, copied (never moved)
    /// when the new one is missing.
    pub copied_files: Vec<(PathBuf, &'static str)>,
}

impl IdentityDirs {
    /// The directories of the current OS, from the environment (`HOME`, `APPDATA`,
    /// `LOCALAPPDATA`, `XDG_*`), as Tauri and the WebView use them:
    /// - macOS: `~/Library/{Application Support,Caches,Logs,WebKit,HTTPStorages}/<id>` and the
    ///   `~/Library/Preferences/<id>.plist` preferences file;
    /// - Windows: `%APPDATA%\<id>` and `%LOCALAPPDATA%\<id>` (the WebView2 data, `EBWebView`);
    /// - Linux: `$XDG_DATA_HOME/<id>` (also the WebView's data), `$XDG_CONFIG_HOME/<id>`,
    ///   `$XDG_CACHE_HOME/<id>`.
    pub fn current() -> Self {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = env("HOME").or_else(|| env("USERPROFILE"));
        let mut bases = Vec::new();
        let mut copied_files = Vec::new();
        if cfg!(target_os = "macos") {
            if let Some(home) = home {
                let lib = home.join("Library");
                for sub in [
                    "Application Support",
                    "Caches",
                    "Logs",
                    "WebKit",
                    "HTTPStorages",
                ] {
                    bases.push(lib.join(sub));
                }
                copied_files.push((lib.join("Preferences"), ".plist"));
            }
        } else if cfg!(windows) {
            bases.extend(env("APPDATA"));
            bases.extend(env("LOCALAPPDATA"));
        } else {
            let xdg = |var: &str, rel: &[&str]| {
                env(var).or_else(|| {
                    home.as_ref()
                        .map(|h| rel.iter().fold(h.clone(), |p, s| p.join(s)))
                })
            };
            bases.extend(xdg("XDG_DATA_HOME", &[".local", "share"]));
            bases.extend(xdg("XDG_CONFIG_HOME", &[".config"]));
            bases.extend(xdg("XDG_CACHE_HOME", &[".cache"]));
        }
        bases.dedup();
        Self {
            bases,
            copied_files,
        }
    }
}

/// Moves the directories of every [`LEGACY_DESKTOP_IDENTIFIERS`] entry to
/// [`DESKTOP_IDENTIFIER`]'s, in each of `dirs` (see the module docs). Never fails: what
/// can't be moved is left in place and reported.
pub fn migrate_legacy_dirs(dirs: &IdentityDirs) -> MigrationReport {
    let mut report = MigrationReport::default();
    for legacy in LEGACY_DESKTOP_IDENTIFIERS {
        migrate(dirs, legacy, DESKTOP_IDENTIFIER, &mut report);
    }
    report
}

/// [`migrate_legacy_dirs`] for one `from` → `to` identifier.
pub fn migrate(dirs: &IdentityDirs, from: &str, to: &str, report: &mut MigrationReport) {
    for base in &dirs.bases {
        let src = base.join(from);
        if !is_real_dir(&src) {
            continue;
        }
        merge_into(&src, &base.join(to), report);
    }
    for (dir, suffix) in &dirs.copied_files {
        let src = dir.join(format!("{from}{suffix}"));
        let dst = dir.join(format!("{to}{suffix}"));
        if src.is_file() && !exists(&dst) {
            match std::fs::copy(&src, &dst) {
                Ok(_) => report.copied.push((src, dst)),
                Err(e) => report.errors.push((src, e.to_string())),
            }
        }
    }
}

/// Moves `src` to `dst`: a rename when `dst` does not exist, else a merge (entries missing
/// from `dst` are moved, directories present in both are merged, anything else stays). `src`
/// is removed once empty.
fn merge_into(src: &Path, dst: &Path, report: &mut MigrationReport) {
    if !exists(dst) {
        match rename(src, dst) {
            Ok(()) => report.moved.push((src.to_path_buf(), dst.to_path_buf())),
            Err(e) => report.errors.push((src.to_path_buf(), e.to_string())),
        }
        return;
    }
    if !is_real_dir(src) || !is_real_dir(dst) {
        report.kept.push(src.to_path_buf());
        return;
    }
    let entries = match std::fs::read_dir(src) {
        Ok(it) => it,
        Err(e) => {
            report.errors.push((src.to_path_buf(), e.to_string()));
            return;
        }
    };
    let mut names: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    names.sort();
    for name in names {
        merge_into(&src.join(&name), &dst.join(&name), report);
    }
    // Only removes an empty directory: whatever was kept stays.
    let _ = std::fs::remove_dir(src);
}

fn rename(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(src, dst)
}

/// A directory, not a symlink to one (a symlink is moved as itself, never followed).
fn is_real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

/// Exists, as a link or otherwise (a dangling symlink counts).
fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const OLD: &str = "dev.ethereal.app";

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("ether-identity-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }
    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }
    fn dirs(bases: &[&Path]) -> IdentityDirs {
        IdentityDirs {
            bases: bases.iter().map(|p| p.to_path_buf()).collect(),
            copied_files: vec![],
        }
    }

    #[test]
    fn identifier_matches_tauri_conf() {
        let conf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/src-tauri/tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_str(&read(&conf)).unwrap();
        assert_eq!(conf["identifier"], DESKTOP_IDENTIFIER);
        assert!(!LEGACY_DESKTOP_IDENTIFIERS.contains(&DESKTOP_IDENTIFIER));
        // Tauri warns about an identifier ending in `.app` (the macOS bundle extension).
        assert!(!DESKTOP_IDENTIFIER.ends_with(".app"));
    }

    #[test]
    fn renames_the_whole_tree_when_the_new_dir_is_missing() {
        let t = Tmp::new("rename");
        let (a, b) = (t.0.join("data"), t.0.join("cache"));
        write(&a.join(OLD).join("config/settings.json"), "{\"a\":1}");
        write(&a.join(OLD).join("plugin-db/plugins.db"), "db");
        write(&a.join(OLD).join("projects/song/project.json"), "song");
        write(
            &b.join(OLD)
                .join("EBWebView/Local Storage/leveldb/000003.log"),
            "ls",
        );
        let r = migrate_legacy_dirs(&dirs(&[&a, &b, &t.0.join("missing")]));
        let new_a = a.join(DESKTOP_IDENTIFIER);
        assert_eq!(read(&new_a.join("config/settings.json")), "{\"a\":1}");
        assert_eq!(read(&new_a.join("plugin-db/plugins.db")), "db");
        assert_eq!(read(&new_a.join("projects/song/project.json")), "song");
        assert_eq!(
            read(
                &b.join(DESKTOP_IDENTIFIER)
                    .join("EBWebView/Local Storage/leveldb/000003.log")
            ),
            "ls"
        );
        assert!(!a.join(OLD).exists() && !b.join(OLD).exists());
        assert_eq!(r.moved.len(), 2, "{r:?}");
        assert!(r.kept.is_empty() && r.errors.is_empty(), "{r:?}");
    }

    #[test]
    fn merges_into_an_existing_dir_without_losing_anything() {
        let t = Tmp::new("merge");
        let (old, new) = (t.0.join(OLD), t.0.join(DESKTOP_IDENTIFIER));
        // The new build already ran once: its skeleton and a fresh settings file exist.
        fs::create_dir_all(new.join("config")).unwrap();
        fs::create_dir_all(new.join("autosave")).unwrap();
        write(&new.join("config/settings.json"), "new");
        write(&old.join("config/settings.json"), "old");
        write(&old.join("config/keymap.json"), "keys");
        write(&old.join("autosave/a.json"), "autosave");
        write(&old.join("telemetry/install-id"), "id");
        let r = migrate_legacy_dirs(&dirs(&[&t.0]));
        assert_eq!(read(&new.join("config/settings.json")), "new");
        assert_eq!(read(&new.join("config/keymap.json")), "keys");
        assert_eq!(read(&new.join("autosave/a.json")), "autosave");
        assert_eq!(read(&new.join("telemetry/install-id")), "id");
        // The conflicting old file stays where it was; nothing else is left.
        assert_eq!(read(&old.join("config/settings.json")), "old");
        assert_eq!(r.kept, vec![old.join("config/settings.json")]);
        assert!(!old.join("autosave").exists() && !old.join("telemetry").exists());
        assert!(r.errors.is_empty(), "{r:?}");
    }

    #[test]
    fn is_idempotent() {
        let t = Tmp::new("idem");
        write(&t.0.join(OLD).join("config/settings.json"), "s");
        let d = dirs(&[&t.0]);
        assert!(!migrate_legacy_dirs(&d).is_empty());
        assert!(migrate_legacy_dirs(&d).is_empty());
        assert_eq!(
            read(&t.0.join(DESKTOP_IDENTIFIER).join("config/settings.json")),
            "s"
        );
    }

    #[test]
    fn nothing_to_do_without_a_legacy_dir() {
        let t = Tmp::new("none");
        write(
            &t.0.join(DESKTOP_IDENTIFIER).join("config/settings.json"),
            "s",
        );
        write(&t.0.join("other.app/x"), "x");
        assert!(migrate_legacy_dirs(&dirs(&[&t.0])).is_empty());
        assert_eq!(read(&t.0.join("other.app/x")), "x");
    }

    #[test]
    fn a_file_where_a_dir_is_expected_is_kept() {
        let t = Tmp::new("kind");
        write(&t.0.join(OLD).join("cache"), "a file");
        write(&t.0.join(DESKTOP_IDENTIFIER).join("cache/x"), "x");
        let r = migrate_legacy_dirs(&dirs(&[&t.0]));
        assert_eq!(read(&t.0.join(OLD).join("cache")), "a file");
        assert_eq!(r.kept, vec![t.0.join(OLD).join("cache")]);
    }

    #[test]
    fn copies_preference_files_once() {
        let t = Tmp::new("prefs");
        let prefs = t.0.join("Preferences");
        write(&prefs.join(format!("{OLD}.plist")), "plist");
        let d = IdentityDirs {
            bases: vec![],
            copied_files: vec![(prefs.clone(), ".plist")],
        };
        let r = migrate_legacy_dirs(&d);
        assert_eq!(r.copied.len(), 1);
        assert_eq!(
            read(&prefs.join(format!("{DESKTOP_IDENTIFIER}.plist"))),
            "plist"
        );
        // The old file is kept (copied, not moved), and a second run changes nothing.
        assert!(prefs.join(format!("{OLD}.plist")).exists());
        write(
            &prefs.join(format!("{DESKTOP_IDENTIFIER}.plist")),
            "changed",
        );
        assert!(migrate_legacy_dirs(&d).is_empty());
        assert_eq!(
            read(&prefs.join(format!("{DESKTOP_IDENTIFIER}.plist"))),
            "changed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_moved_not_followed() {
        let t = Tmp::new("link");
        let target = t.0.join("elsewhere");
        write(&target.join("keep"), "k");
        fs::create_dir_all(t.0.join(OLD)).unwrap();
        // Both exist, so the old dir is merged entry by entry (the link itself is moved).
        fs::create_dir_all(t.0.join(DESKTOP_IDENTIFIER)).unwrap();
        std::os::unix::fs::symlink(&target, t.0.join(OLD).join("samples")).unwrap();
        let r = migrate_legacy_dirs(&dirs(&[&t.0]));
        let moved = t.0.join(DESKTOP_IDENTIFIER).join("samples");
        assert!(
            fs::symlink_metadata(&moved)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(read(&target.join("keep")), "k");
        assert!(r.errors.is_empty());
    }

    #[test]
    fn current_dirs_are_absolute_and_named_per_os() {
        let d = IdentityDirs::current();
        assert!(d.bases.iter().all(|b| b.is_absolute()), "{d:?}");
        if cfg!(target_os = "linux") && std::env::var_os("HOME").is_some() {
            assert_eq!(d.bases.len(), 3, "{d:?}");
        }
    }
}
