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
//! are cached until a full rescan: a new or removed component id is still picked up, but an
//! updated AU keeps its old descriptor until then.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;
use serde::{Deserialize, Serialize};

use crate::scan::{Failure, TargetResult};
use crate::{ScanReport, ScanRunner, ScanTarget};

/// Bump when the cache file layout or the meaning of a cached result changes.
const SCHEMA: u32 = 1;

/// The scanner version a cache is valid for (besides the scanner binary's fingerprint).
pub const SCANNER_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+cache1");

/// File name of the cache, next to the plugin list (`plugin-db/plugins.json`).
pub const SCAN_CACHE_FILE: &str = "scan-cache.json";

/// What is on disk for a target: changes when the bundle is replaced or edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// Newest modification time in the tree, ns since the Unix epoch.
    pub mtime_ns: u64,
    /// Total size of the files in the tree, bytes.
    pub size: u64,
    /// Files and directories in the tree.
    pub entries: u64,
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
    Ok { plugins: Vec<PluginDescriptor> },
    Err { message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    format: PluginFormat,
    /// Canonical path of the target (or the target as given if it isn't on disk).
    target: String,
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
    /// cached too (retried when the target changes); infrastructure failures (scanner not
    /// starting) are not. Targets no longer listed are dropped from the cache. `full`
    /// ignores the cache and rescans everything (refreshing it).
    ///
    /// Progress counts cached targets as already done.
    pub fn scan_targets_cached(
        &self,
        targets: &[ScanTarget],
        cache: &mut ScanCache,
        full: bool,
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
                let fp = fingerprint(Path::new(&k.1));
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
                Some(match &e.result {
                    CachedResult::Ok { plugins } => Ok(plugins.clone()),
                    CachedResult::Err { message } => Err(Failure {
                        message: message.clone(),
                        plugin_fault: true,
                    }),
                })
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
            let cached = match &r {
                Ok(plugins) => Some(CachedResult::Ok {
                    plugins: plugins.clone(),
                }),
                Err(f) if f.plugin_fault => Some(CachedResult::Err {
                    message: f.message.clone(),
                }),
                Err(_) => None,
            };
            if let Some(result) = cached {
                entries.insert(
                    k.clone(),
                    Entry {
                        format: k.0,
                        target: k.1,
                        fingerprint: fp,
                        result,
                    },
                );
            }
            report.push(&target.path, r);
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
