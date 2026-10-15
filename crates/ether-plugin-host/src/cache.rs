//! Incremental scan cache: per-target results (descriptors or failure) remembered across
//! scans, so a rescan only runs the scanner on targets that are new or changed.
//!
//! A target is keyed by format + canonical path and validated by a [`Fingerprint`] of what
//! is on disk (newest mtime, total size and file count of the bundle tree; a bundle is a
//! directory on macOS, and replacing the binary inside it doesn't touch the directory's own
//! mtime). The whole cache is dropped when the scanner changes (crate version, cache schema,
//! or the scanner binary's own fingerprint).
//!
//! Targets with nothing on disk to fingerprint (AU component ids from the system registry)
//! are fingerprinted by the version the registry reports
//! ([`crate::PluginFormatHost::registry_version`]), so an updated AU is rescanned; without
//! one they are cached until a full rescan (a new or removed component id is still picked
//! up).
//!
//! Failures: a plugin that crashed or failed to load is remembered until its target
//! changes (or a full rescan). A timeout is remembered too (it is still listed as a
//! failure, e.g. after a restart) but not trusted: the next rescan tries the target again,
//! since a slow plugin may load fine on a less busy machine. [`ScanCache::record`] stores
//! the result of a one-off retry ([`ScanRunner::scan_one`], typically with a longer
//! timeout).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::{PluginDescriptor, ScanFailure};
use serde::{Deserialize, Serialize};

use crate::scan::{Failure, TargetResult, TargetScan};
use crate::{ScanReport, ScanRunner, ScanTarget};

/// Bump when the cache file layout or the meaning of a cached result changes.
/// 2: failures remember whether they were a timeout (retried by the next rescan).
const SCHEMA: u32 = 2;

/// The scanner version a cache is valid for (besides the scanner binary's fingerprint).
pub const SCANNER_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+cache1");

/// File name of the cache, next to the plugin list (`plugin-db/plugins.json`).
pub const SCAN_CACHE_FILE: &str = "scan-cache.json";

/// What is on disk for a target: changes when the bundle is replaced or edited. For a
/// registry target, its registered version instead ([`Fingerprint::of_version`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// Newest modification time in the tree, ns since the Unix epoch.
    pub mtime_ns: u64,
    /// Total size of the files in the tree, bytes.
    pub size: u64,
    /// Files and directories in the tree.
    pub entries: u64,
    /// A registry target's version ([`crate::PluginFormatHost::registry_version`]; the
    /// other fields are 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
}

impl Fingerprint {
    /// The fingerprint of a registry target at `version`.
    pub fn of_version(version: u64) -> Self {
        Self {
            mtime_ns: 0,
            size: 0,
            entries: 0,
            version: Some(version),
        }
    }
}

/// Fingerprint of `path` (a file, or a bundle directory walked without following symlinks
/// below the root). `None` if nothing is there.
pub fn fingerprint(path: &Path) -> Option<Fingerprint> {
    /// A bundle has a handful of files; don't walk a whole disk if handed a root.
    const MAX_ENTRIES: u64 = 20_000;
    fn mtime_ns(m: &std::fs::Metadata) -> u64 {
        m.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as u64)
    }
    let meta = std::fs::metadata(path).ok()?;
    let mut fp = Fingerprint {
        mtime_ns: mtime_ns(&meta),
        size: if meta.is_file() { meta.len() } else { 0 },
        entries: 1,
        version: None,
    };
    if meta.is_dir() {
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                let Ok(m) = e.metadata() else { continue };
                fp.entries += 1;
                fp.mtime_ns = fp.mtime_ns.max(mtime_ns(&m));
                if m.is_dir() {
                    stack.push(e.path());
                } else {
                    fp.size += m.len();
                }
                if fp.entries >= MAX_ENTRIES {
                    return Some(fp);
                }
            }
        }
    }
    Some(fp)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
enum CachedResult {
    Ok {
        plugins: Vec<PluginDescriptor>,
    },
    Err {
        message: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        timed_out: bool,
    },
}

impl CachedResult {
    /// What to remember of a scan result; `None` for infrastructure failures.
    fn of(r: &TargetResult) -> Option<Self> {
        match r {
            Ok(plugins) => Some(Self::Ok {
                plugins: plugins.clone(),
            }),
            Err(f) if f.plugin_fault => Some(Self::Err {
                message: f.message.clone(),
                timed_out: f.timed_out,
            }),
            Err(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    format: PluginFormat,
    /// Canonical path of the target (or the target as given if it isn't on disk).
    target: String,
    /// The target as given, when it differs from `target` (failures are listed with it: a
    /// canonical Windows path is a `\\?\` path).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    path: String,
    fingerprint: Option<Fingerprint>,
    result: CachedResult,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    schema: u32,
    version: String,
    scanner: Option<Fingerprint>,
    entries: Vec<Entry>,
}

/// Per-target scan results, optionally persisted to a JSON file.
#[derive(Debug, Default)]
pub struct ScanCache {
    file: Option<PathBuf>,
    version: String,
    scanner: Option<Fingerprint>,
    entries: HashMap<(PluginFormat, String), Entry>,
}

/// A cached scan: the report plus how much work it took.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CachedScan {
    pub report: ScanReport,
    /// Targets the scanner ran on.
    pub scanned: usize,
    /// Targets answered from the cache.
    pub reused: usize,
}

fn key(target: &ScanTarget) -> (PluginFormat, String) {
    let path = std::fs::canonicalize(&target.path).unwrap_or_else(|_| target.path.clone());
    (target.format, path.to_string_lossy().into_owned())
}

fn entry(
    (format, canonical): (PluginFormat, String),
    target: &ScanTarget,
    fingerprint: Option<Fingerprint>,
    result: CachedResult,
) -> Entry {
    let given = target.path.to_string_lossy();
    Entry {
        format,
        path: if given == canonical {
            String::new()
        } else {
            given.into_owned()
        },
        target: canonical,
        fingerprint,
        result,
    }
}

impl ScanCache {
    /// An empty cache that is never written to disk.
    pub fn in_memory() -> Self {
        Self {
            version: SCANNER_VERSION.into(),
            ..Self::default()
        }
    }

    /// Load `file` if it is a cache of this scanner version (else start empty); [`save`]
    /// writes it back.
    ///
    /// [`save`]: ScanCache::save
    pub fn open(file: impl Into<PathBuf>) -> Self {
        let file = file.into();
        let loaded = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str::<CacheFile>(&s).ok())
            .filter(|c| c.schema == SCHEMA && c.version == SCANNER_VERSION);
        let mut cache = Self::in_memory();
        cache.file = Some(file);
        if let Some(c) = loaded {
            cache.scanner = c.scanner;
            cache.entries = c
                .entries
                .into_iter()
                .map(|e| ((e.format, e.target.clone()), e))
                .collect();
        }
        cache
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The targets whose remembered result is a failure (from the last scan, or a retry
    /// since), sorted by path.
    pub fn failures(&self) -> Vec<ScanFailure> {
        let mut v: Vec<ScanFailure> = self
            .entries
            .values()
            .filter_map(|e| match &e.result {
                CachedResult::Err { message, .. } => Some(ScanFailure {
                    path: if e.path.is_empty() {
                        e.target.clone()
                    } else {
                        e.path.clone()
                    },
                    message: message.clone(),
                    format: Some(e.format),
                }),
                CachedResult::Ok { .. } => None,
            })
            .collect();
        v.sort_by(|a, b| (&a.path, a.format).cmp(&(&b.path, b.format)));
        v
    }

    /// Remember the result of scanning one target on its own, replacing what was cached for
    /// it (an infrastructure failure forgets it). The rest of the cache is untouched.
    pub fn record(&mut self, scan: &TargetScan) {
        self.record_with(scan, None);
    }

    /// [`ScanCache::record`] of a target with this registry version
    /// ([`crate::Formats::registry_version`]; `None` for targets on disk).
    pub fn record_with(&mut self, scan: &TargetScan, registry_version: Option<u64>) {
        let k = key(&scan.target);
        match CachedResult::of(&scan.result) {
            Some(result) => {
                let fp = fingerprint(Path::new(&k.1))
                    .or_else(|| registry_version.map(Fingerprint::of_version));
                self.entries
                    .insert(k.clone(), entry(k, &scan.target, fp, result));
            }
            None => {
                self.entries.remove(&k);
            }
        }
    }

    /// Write the cache file atomically (no-op for [`ScanCache::in_memory`]).
    pub fn save(&self) -> std::io::Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let mut entries: Vec<Entry> = self.entries.values().cloned().collect();
        entries.sort_by(|a, b| (a.format, &a.target).cmp(&(b.format, &b.target)));
        let json = serde_json::to_vec_pretty(&CacheFile {
            schema: SCHEMA,
            version: self.version.clone(),
            scanner: self.scanner,
            entries,
        })
        .map_err(std::io::Error::other)?;
        let dir = file.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(
            ".{}.{}.tmp",
            file.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id()
        ));
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, file).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

impl ScanRunner {
    /// [`ScanRunner::scan_targets`], but only for targets that are new or changed since
    /// `cache` last saw them; the rest are answered from the cache. Plugin failures are
    /// cached too (retried when the target changes), except that a timed-out target is
    /// retried by every rescan; infrastructure failures (scanner not starting) are not
    /// cached. Targets no longer listed are dropped from the cache. `full` ignores the
    /// cache and rescans everything (refreshing it).
    ///
    /// Progress counts cached targets as already done.
    pub fn scan_targets_cached(
        &self,
        targets: &[ScanTarget],
        cache: &mut ScanCache,
        full: bool,
        progress: impl FnMut(u32, u32, Option<&Path>),
    ) -> CachedScan {
        self.scan_targets_cached_with(targets, cache, full, |_| None, progress)
    }

    /// [`ScanRunner::scan_targets_cached`], with `registry_version` telling the version of
    /// targets that aren't on disk ([`crate::Formats::registry_version`]): a cached one is
    /// rescanned when its version changes.
    pub fn scan_targets_cached_with(
        &self,
        targets: &[ScanTarget],
        cache: &mut ScanCache,
        full: bool,
        registry_version: impl Fn(&ScanTarget) -> Option<u64>,
        mut progress: impl FnMut(u32, u32, Option<&Path>),
    ) -> CachedScan {
        let scanner = fingerprint(&self.scanner);
        if cache.version != SCANNER_VERSION || cache.scanner != scanner {
            cache.entries.clear();
            cache.version = SCANNER_VERSION.into();
            cache.scanner = scanner;
        }

        let keyed: Vec<_> = targets
            .iter()
            .map(|t| {
                let k = key(t);
                let fp = fingerprint(Path::new(&k.1))
                    .or_else(|| registry_version(t).map(Fingerprint::of_version));
                (k, fp)
            })
            .collect();
        let mut results: Vec<Option<TargetResult>> = keyed
            .iter()
            .map(|(k, fp)| {
                let e = cache
                    .entries
                    .get(k)
                    .filter(|e| !full && e.fingerprint == *fp)?;
                match &e.result {
                    CachedResult::Ok { plugins } => Some(Ok(plugins.clone())),
                    CachedResult::Err {
                        timed_out: true, ..
                    } => None,
                    CachedResult::Err { message, .. } => Some(Err(Failure::plugin(message))),
                }
            })
            .collect();
        let misses: Vec<usize> = (0..targets.len())
            .filter(|&i| results[i].is_none())
            .collect();
        let to_scan: Vec<ScanTarget> = misses.iter().map(|&i| targets[i].clone()).collect();
        let reused = targets.len() - misses.len();
        let fresh = self.scan_target_results(&to_scan, reused, targets.len(), &mut progress);
        for (&i, r) in misses.iter().zip(fresh) {
            results[i] = Some(r);
        }

        let mut entries = HashMap::with_capacity(targets.len());
        let mut report = ScanReport::default();
        for ((target, (k, fp)), r) in targets.iter().zip(keyed).zip(results) {
            let r = r.expect("every target has a result");
            if let Some(result) = CachedResult::of(&r) {
                entries.insert(k.clone(), entry(k, target, fp, result));
            }
            report.push(&target.path, Some(target.format), r);
        }
        cache.entries = entries;
        CachedScan {
            report,
            scanned: misses.len(),
            reused,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::scan::tests::{behaving_scanner, clap, runs, serial, temp_dir};
    use std::time::{Duration, Instant};

    fn bundle(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(p.join("Contents")).unwrap();
        std::fs::write(p.join("Contents/bin"), body).unwrap();
        p
    }

    fn ids(r: &ScanReport) -> Vec<String> {
        r.plugins
            .iter()
            .map(|p| {
                Path::new(&p.id)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn hits_misses_invalidation_failures_and_full_rescan() {
        let _serial = serial();
        let s = behaving_scanner("cache-basic", "0");
        let dir = temp_dir("cache-basic-plugins");
        let a = bundle(&dir, "A.clap", "a");
        let b = bundle(&dir, "B.clap", "b");
        let crash = bundle(&dir, "crash.clap", "c");
        let file = dir.join("db").join(SCAN_CACHE_FILE);
        let runner = ScanRunner::new(&s).with_jobs(2);
        let targets = vec![clap(&a), clap(&b), clap(&crash)];
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap().display().to_string();

        // Cold: everything scanned; the crash is a failure.
        let mut cache = ScanCache::open(&file);
        let r = runner.scan_targets_cached(&targets, &mut cache, false, |_, _, _| {});
        assert_eq!((r.scanned, r.reused), (3, 0));
        assert_eq!(ids(&r.report), ["A.clap", "B.clap"]);
        assert_eq!(r.report.failed.len(), 1);
        cache.save().unwrap();
        assert_eq!(runs(&s).len(), 3);

        // Warm (reloaded from disk): nothing scanned, same report (failure included).
        let mut cache = ScanCache::open(&file);
        assert_eq!(cache.len(), 3);
        let mut calls = Vec::new();
        let warm = runner.scan_targets_cached(&targets, &mut cache, false, |d, t, c| {
            calls.push((d, t, c.is_some()))
        });
        assert_eq!((warm.scanned, warm.reused), (0, 3));
        assert_eq!(warm.report, r.report);
        assert_eq!(calls, [(3, 3, false)]);
        assert_eq!(runs(&s).len(), 3);

        // Size change of A, mtime change of B: both rescanned; the failure isn't retried.
        std::fs::write(a.join("Contents/bin"), "a, but longer").unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(b.join("Contents/bin"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        let r = runner.scan_targets_cached(&targets, &mut cache, false, |_, _, _| {});
        assert_eq!((r.scanned, r.reused), (2, 1));
        assert_eq!(ids(&r.report), ["A.clap", "B.clap"]);
        let ran = runs(&s);
        assert_eq!(
            ran.iter()
                .filter(|p| **p == canon(&a) || **p == a.display().to_string())
                .count(),
            2
        );
        assert_eq!(ran.len(), 5);

        // A new target is scanned; a removed one is dropped from the cache.
        let c = bundle(&dir, "C.clap", "c");
        let targets2 = vec![clap(&a), clap(&c)];
        let r = runner.scan_targets_cached(&targets2, &mut cache, false, |_, _, _| {});
        assert_eq!((r.scanned, r.reused), (1, 1));
        assert_eq!(ids(&r.report), ["A.clap", "C.clap"]);
        assert_eq!(cache.len(), 2);

        // Changing the failed bundle retries it.
        let targets3 = vec![clap(&a), clap(&crash)];
        let _ = runner.scan_targets_cached(&targets3, &mut cache, false, |_, _, _| {});
        std::fs::write(crash.join("Contents/bin"), "fixed?").unwrap();
        let before = runs(&s).len();
        let r = runner.scan_targets_cached(&targets3, &mut cache, false, |_, _, _| {});
        assert_eq!((r.scanned, r.reused), (1, 1));
        assert_eq!(runs(&s).len(), before + 1);

        // Full rescan bypasses the cache.
        let r = runner.scan_targets_cached(&targets3, &mut cache, true, |_, _, _| {});
        assert_eq!((r.scanned, r.reused), (2, 0));
    }

    #[test]
    fn scanner_change_and_start_failures_are_not_cached() {
        let _serial = serial();
        let dir = temp_dir("cache-infra");
        let a = bundle(&dir, "A.clap", "a");
        let mut cache = ScanCache::in_memory();
        // A scanner that can't start: failure reported, but not remembered.
        let r = ScanRunner::new(dir.join("missing-scanner")).scan_targets_cached(
            &[clap(&a)],
            &mut cache,
            false,
            |_, _, _| {},
        );
        assert_eq!(r.report.failed.len(), 1);
        assert!(cache.is_empty());

        let s = behaving_scanner("cache-infra-scanner", "0");
        let runner = ScanRunner::new(&s);
        let r = runner.scan_targets_cached(&[clap(&a)], &mut cache, false, |_, _, _| {});
        assert_eq!(r.scanned, 1);
        let r = runner.scan_targets_cached(&[clap(&a)], &mut cache, false, |_, _, _| {});
        assert_eq!(r.scanned, 0);
        // The scanner binary changed (an app update): the whole cache is stale.
        std::fs::write(&s, std::fs::read_to_string(&s).unwrap() + "\n# v2\n").unwrap();
        let r = runner.scan_targets_cached(&[clap(&a)], &mut cache, false, |_, _, _| {});
        assert_eq!(r.scanned, 1);
    }

    #[test]
    fn non_file_targets_are_cached_until_a_full_rescan() {
        let _serial = serial();
        let s = behaving_scanner("cache-au", "0");
        let runner = ScanRunner::new(&s);
        let au = ScanTarget {
            format: PluginFormat::Au,
            path: "aufx:dely:appl".into(),
        };
        let mut cache = ScanCache::in_memory();
        assert_eq!(
            runner
                .scan_targets_cached(std::slice::from_ref(&au), &mut cache, false, |_, _, _| {})
                .scanned,
            1
        );
        assert_eq!(
            runner
                .scan_targets_cached(std::slice::from_ref(&au), &mut cache, false, |_, _, _| {})
                .scanned,
            0
        );
        assert_eq!(
            runner
                .scan_targets_cached(std::slice::from_ref(&au), &mut cache, true, |_, _, _| {})
                .scanned,
            1
        );
    }

    /// An updated AU (its registered version changed) is rescanned by an incremental scan.
    #[test]
    fn registry_targets_are_rescanned_when_their_version_changes() {
        let _serial = serial();
        let s = behaving_scanner("cache-au-version", "0");
        let runner = ScanRunner::new(&s);
        let au = ScanTarget {
            format: PluginFormat::Au,
            path: "aufx:dely:appl".into(),
        };
        let version = std::cell::Cell::new(0x0001_0000u64);
        let mut cache = ScanCache::in_memory();
        let mut scan = || {
            runner
                .scan_targets_cached_with(
                    std::slice::from_ref(&au),
                    &mut cache,
                    false,
                    |t| (t.path == au.path).then(|| version.get()),
                    |_, _, _| {},
                )
                .scanned
        };
        assert_eq!(scan(), 1);
        assert_eq!(scan(), 0);
        version.set(0x0001_0100);
        assert_eq!(scan(), 1);
        assert_eq!(scan(), 0);
    }

    /// The numbers in the PR: `cargo test -p ether-plugin-host --release -- --ignored
    /// --nocapture bench_synthetic`.
    #[test]
    #[ignore = "benchmark (~15 s)"]
    fn bench_synthetic() {
        let _serial = serial();
        let s = behaving_scanner("cache-bench", "0.3");
        let dir = temp_dir("cache-bench-plugins");
        let targets: Vec<_> = (0..40)
            .map(|i| clap(bundle(&dir, &format!("P{i:02}.clap"), "x")))
            .collect();
        let time = |jobs: usize| {
            let t = Instant::now();
            let r = ScanRunner::new(&s)
                .with_jobs(jobs)
                .scan_targets(&targets, |_, _, _| {});
            assert_eq!(r.plugins.len(), 40);
            t.elapsed()
        };
        let seq = time(1);
        let jobs = ScanRunner::default_jobs();
        let pool = time(jobs);
        let mut cache = ScanCache::open(dir.join(SCAN_CACHE_FILE));
        let runner = ScanRunner::new(&s).with_jobs(jobs);
        let _ = runner.scan_targets_cached(&targets, &mut cache, false, |_, _, _| {});
        cache.save().unwrap();
        let t = Instant::now();
        let mut cache = ScanCache::open(dir.join(SCAN_CACHE_FILE));
        let r = runner.scan_targets_cached(&targets, &mut cache, false, |_, _, _| {});
        let cached = t.elapsed();
        assert_eq!(r.reused, 40);
        println!(
            "40 targets x 300 ms: sequential {seq:?}, pool of {jobs} {pool:?} ({:.1}x), cached rescan {cached:?}",
            seq.as_secs_f64() / pool.as_secs_f64()
        );
    }
}

/// The failure list, timeouts and one-target retries, on every platform: a fake scanner
/// script (`sh` on Unix, a batch file on Windows).
#[cfg(test)]
mod retry_tests {
    use super::*;
    use std::time::Duration;

    /// Unix: the scan tests' lock (writing and exec-ing scripts concurrently fails there).
    #[cfg(unix)]
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        crate::scan::tests::serial()
    }
    #[cfg(not(unix))]
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn temp_dir(purpose: &str) -> PathBuf {
        crate::testing::temp_dir_named(&format!("ether-test-{}-{purpose}", std::process::id()))
    }

    const PLUGIN: &str = r#"{"type":"Ok","plugins":[{"format":"Clap","id":"slow","name":"Slow","vendor":"","version":"","description":"","features":[],"category":"AudioEffect","path":"slow"}]}"#;

    /// A scanner in `dir`: a target whose path contains `crashme` crashes (exit 3); any
    /// other hangs, unless `dir/ok.flag` exists: then it reports the plugin `slow`.
    fn fake_scanner(dir: &Path) -> PathBuf {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("scanner.sh");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\nreq=$(cat)\ncase \"$req\" in *crashme*) echo boom >&2; exit 3 ;; esac\n[ -f \"$(dirname \"$0\")/ok.flag\" ] || exec sleep 600\necho '{PLUGIN}'\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
        #[cfg(not(unix))]
        {
            let path = dir.join("scanner.cmd");
            std::fs::write(
                &path,
                format!(
                    "@echo off\r\nset /p req=\r\necho %req% | findstr /c:\"crashme\" >nul && (echo boom 1>&2 & exit /b 3)\r\nif not exist \"%~dp0ok.flag\" ping -n 30 127.0.0.1 >nul\r\necho {PLUGIN}\r\n"
                ),
            )
            .unwrap();
            path
        }
    }

    fn clap(path: &Path) -> ScanTarget {
        ScanTarget {
            format: PluginFormat::Clap,
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn timeouts_are_listed_retried_by_the_next_rescan_and_by_a_retry() {
        let _serial = serial();
        let dir = temp_dir("cache-retry");
        let s = fake_scanner(&dir);
        let slow = dir.join("slowme.clap");
        let crash = dir.join("crashme.clap");
        std::fs::write(&slow, "s").unwrap();
        std::fs::write(&crash, "c").unwrap();
        let targets = [clap(&slow), clap(&crash)];
        let runner = ScanRunner::new(&s)
            .with_jobs(2)
            .with_timeout(Duration::from_secs(2));
        let file = dir.join("db").join(SCAN_CACHE_FILE);

        let mut cache = ScanCache::open(&file);
        let r = runner.scan_targets_cached(&targets, &mut cache, false, |_, _, _| {});
        assert_eq!(r.report.failed.len(), 2, "{:?}", r.report);
        cache.save().unwrap();

        // Listed (also after a reload) by the path as given, with format and reason.
        let mut cache = ScanCache::open(&file);
        let failed = cache.failures();
        let paths: Vec<_> = failed.iter().map(|f| f.path.clone()).collect();
        assert_eq!(
            paths,
            [crash.display().to_string(), slow.display().to_string()]
        );
        assert!(failed.iter().all(|f| f.format == Some(PluginFormat::Clap)));
        assert!(failed[0].message.contains("crashed"), "{failed:?}");
        assert!(failed[1].message.contains("timed out"), "{failed:?}");

        // The next rescan trusts the crash but tries the timeout again.
        let r = runner.scan_targets_cached(&targets, &mut cache, false, |_, _, _| {});
        assert_eq!((r.scanned, r.reused), (1, 1));
        assert_eq!(cache.failures().len(), 2);

        // A retry with a longer timeout, and the plugin loads: cached, no longer failed.
        std::fs::write(dir.join("ok.flag"), "").unwrap();
        let scan = runner
            .clone()
            .with_timeout(Duration::from_secs(60))
            .scan_one(&clap(&slow));
        assert_eq!(scan.outcome().unwrap()[0].id, "slow");
        cache.record(&scan);
        assert_eq!(cache.failures().len(), 1);
        assert_eq!(cache.failures()[0].path, crash.display().to_string());
        let r = runner.scan_targets_cached(&targets, &mut cache, false, |_, _, _| {});
        assert_eq!((r.scanned, r.reused), (0, 2));
        assert_eq!(r.report.plugins[0].id, "slow");

        // A retry that fails again keeps the target listed, with the new reason.
        let scan = runner.scan_one(&clap(&crash));
        let f = scan.outcome().unwrap_err();
        assert_eq!(f.path, crash.display().to_string());
        assert_eq!(f.format, Some(PluginFormat::Clap));
        assert!(f.message.contains("crashed"), "{f:?}");
        cache.record(&scan);
        assert_eq!(cache.failures().len(), 1);

        // A scanner that can't start says nothing about the plugin: forgotten.
        cache.record(&ScanRunner::new(dir.join("missing")).scan_one(&clap(&crash)));
        assert!(cache.failures().is_empty());
        assert_eq!(cache.len(), 1);
    }
}
