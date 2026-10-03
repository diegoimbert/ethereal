//! Host side of the out-of-process scanner (`ether-plugin-scanner`): one scan target per
//! child process, with a timeout, so a plugin that crashes or hangs while loading only loses
//! itself. A bounded pool runs several children at once ([`ScanRunner::jobs`]).

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ether_core::protocol::plugins::{PluginDescriptor, ScanFailure, ScanRequest, ScanResponse};

use crate::ScanTarget;

/// Result of scanning many targets.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ScanReport {
    pub plugins: Vec<PluginDescriptor>,
    pub failed: Vec<ScanFailure>,
}

/// Why one target failed to scan.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Failure {
    pub message: String,
    /// The plugin itself failed (crash, hang, bad answer, load error), as opposed to the
    /// scan infrastructure (the scanner binary could not start): only the former is worth
    /// remembering in the [`crate::ScanCache`].
    pub plugin_fault: bool,
}

impl Failure {
    fn plugin(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            plugin_fault: true,
        }
    }
}

pub(crate) type TargetResult = Result<Vec<PluginDescriptor>, Failure>;

/// Host side of the out-of-process scanner.
#[derive(Debug, Clone)]
pub struct ScanRunner {
    pub scanner: PathBuf,
    /// Max time a scan may take before the child is killed.
    pub timeout: Duration,
    /// After the child exits, how long to wait for its stdout/stderr to reach EOF. A plugin
    /// may leave a daemon behind that inherited the pipes; we then use what was read so far
    /// instead of blocking forever.
    pub drain_timeout: Duration,
    /// How many scanner children run at once (>= 1). Each child still scans exactly one
    /// target. Default: [`ScanRunner::default_jobs`].
    pub jobs: usize,
}

/// Name of the scanner binary.
pub const SCANNER_BIN: &str = if cfg!(windows) {
    "ether-plugin-scanner.exe"
} else {
    "ether-plugin-scanner"
};

/// Environment override of [`ScanRunner::default_jobs`] (a positive integer).
pub const SCAN_JOBS_ENV: &str = "ETHER_SCAN_JOBS";

/// Reads a pipe line by line on a thread; lines arrive through the channel, which closes at
/// EOF.
fn spawn_reader(pipe: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut r = BufReader::new(pipe);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(String::from_utf8_lossy(&buf).into_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    rx
}

/// Everything read from `rx` until EOF or `deadline`, whichever comes first.
fn drain(rx: &mpsc::Receiver<String>, deadline: Instant) -> String {
    let mut out = String::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) => out.push_str(&line),
            Err(_) => return out,
        }
    }
}

/// What a pool worker tells the thread that reports progress.
enum Msg {
    Started(usize),
    Done(usize, TargetResult),
}

impl ScanRunner {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
    pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
    /// Upper bound of [`ScanRunner::default_jobs`].
    pub const MAX_DEFAULT_JOBS: usize = 8;

    pub fn new(scanner: impl Into<PathBuf>) -> Self {
        Self {
            scanner: scanner.into(),
            timeout: Self::DEFAULT_TIMEOUT,
            drain_timeout: Self::DEFAULT_DRAIN_TIMEOUT,
            jobs: Self::default_jobs(),
        }
    }

    /// Scanner children run at once by default: `ETHER_SCAN_JOBS` if set to a positive
    /// integer, else half the available cores (loading plugins is CPU and disk heavy, and
    /// the audio engine may be running), capped at [`ScanRunner::MAX_DEFAULT_JOBS`], at
    /// least 1.
    pub fn default_jobs() -> usize {
        if let Some(n) = std::env::var(SCAN_JOBS_ENV)
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&n| n > 0)
        {
            return n;
        }
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        (cores / 2).clamp(1, Self::MAX_DEFAULT_JOBS)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_drain_timeout(mut self, drain_timeout: Duration) -> Self {
        self.drain_timeout = drain_timeout;
        self
    }

    /// Run at most `jobs` scanner children at once (0 is treated as 1).
    pub fn with_jobs(mut self, jobs: usize) -> Self {
        self.jobs = jobs.max(1);
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

    /// Scan one bundle in a fresh scanner process; the scanner infers the format from the
    /// path. Crashes, hangs (killed after `timeout`) and malformed output all become an
    /// `Err` message.
    pub fn scan_bundle(&self, bundle: &Path) -> Result<Vec<PluginDescriptor>, String> {
        self.scan_bundle_inner(bundle).map_err(|f| f.message)
    }

    /// Scan one target of a known format in a fresh scanner process.
    pub fn scan_target(&self, target: &ScanTarget) -> Result<Vec<PluginDescriptor>, String> {
        self.scan_target_inner(target).map_err(|f| f.message)
    }

    fn scan_bundle_inner(&self, bundle: &Path) -> TargetResult {
        self.run(&ScanRequest {
            bundle_path: bundle.to_string_lossy().into_owned(),
            format: None,
        })
    }

    fn scan_target_inner(&self, target: &ScanTarget) -> TargetResult {
        self.run(&ScanRequest {
            bundle_path: target.path.to_string_lossy().into_owned(),
            format: Some(target.format),
        })
    }

    fn run(&self, request: &ScanRequest) -> TargetResult {
        let request =
            serde_json::to_string(request).map_err(|e| Failure::plugin(e.to_string()))?;

        let mut child = Command::new(&self.scanner)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Failure {
                message: format!("failed to start scanner {}: {e}", self.scanner.display()),
                plugin_fault: false,
            })?;

        let mut stdin = child.stdin.take().expect("piped stdin");
        // Readers on threads so a chatty plugin can't fill a pipe and deadlock us.
        let out_rx = spawn_reader(child.stdout.take().expect("piped stdout"));
        let err_rx = spawn_reader(child.stderr.take().expect("piped stderr"));
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
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Failure {
                        message: format!("scanner wait failed: {e}"),
                        plugin_fault: false,
                    });
                }
            }
        };
        let Some(status) = status else {
            // Killed: the pipes may be held open by grandchildren; don't wait for them.
            return Err(Failure::plugin(format!(
                "scan timed out after {:?}",
                self.timeout
            )));
        };
        // Bounded: a daemon spawned by the plugin may hold the pipes open forever.
        let drain_deadline = Instant::now() + self.drain_timeout;
        let out = drain(&out_rx, drain_deadline);
        let err = drain(&err_rx, drain_deadline);

        if !status.success() {
            let tail: String = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
            return Err(Failure::plugin(format!(
                "scanner crashed ({status}){}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            )));
        }
        // Plugins may print to stdout while loading: the response is the last JSON line.
        let line = out
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .ok_or_else(|| Failure::plugin("scanner produced no response"))?;
        match serde_json::from_str::<ScanResponse>(line) {
            Ok(ScanResponse::Ok { plugins }) => Ok(plugins),
            Ok(ScanResponse::Err { message }) => Err(Failure::plugin(message)),
            Err(e) => Err(Failure::plugin(format!("bad scanner response: {e}"))),
        }
    }

    /// Scan every bundle (format inferred per bundle), [`ScanRunner::jobs`] at a time. Same
    /// progress and ordering contract as [`ScanRunner::scan_targets`].
    pub fn scan_all(
        &self,
        bundles: &[PathBuf],
        mut progress: impl FnMut(u32, u32, Option<&Path>),
    ) -> ScanReport {
        let results = self.scan_pool(
            bundles,
            0,
            bundles.len(),
            |b| self.scan_bundle_inner(b),
            |b| b.as_path(),
            &mut progress,
        );
        let mut report = ScanReport::default();
        for (b, r) in bundles.iter().zip(results) {
            report.push(b, r);
        }
        report
    }

    /// Scan every target, [`ScanRunner::jobs`] scanner children at a time (one target per
    /// child, each with its own timeout).
    ///
    /// The report lists plugins and failures in input order, whatever order the children
    /// finish in. `progress(done, total, current)` runs on the calling thread: when a
    /// target starts (`current` = that target), when one finishes while others are still
    /// running (`current` = the earliest of those), and once at the end with
    /// `done == total` and `current = None`. With `jobs == 1` that is the sequential
    /// contract: one call before each target, then the final one.
    pub fn scan_targets(
        &self,
        targets: &[ScanTarget],
        mut progress: impl FnMut(u32, u32, Option<&Path>),
    ) -> ScanReport {
        let results = self.scan_target_results(targets, 0, targets.len(), &mut progress);
        let mut report = ScanReport::default();
        for (t, r) in targets.iter().zip(results) {
            report.push(&t.path, r);
        }
        report
    }

    /// Per-target results of [`ScanRunner::scan_targets`], in input order. Progress counts
    /// `already_done` targets as done out of `total` (for callers that skip cached ones).
    pub(crate) fn scan_target_results(
        &self,
        targets: &[ScanTarget],
        already_done: usize,
        total: usize,
        progress: &mut dyn FnMut(u32, u32, Option<&Path>),
    ) -> Vec<TargetResult> {
        self.scan_pool(
            targets,
            already_done,
            total,
            |t| self.scan_target_inner(t),
            |t| t.path.as_path(),
            progress,
        )
    }

    /// The bounded pool: `jobs` worker threads take the next index from a shared counter,
    /// each running one scanner child at a time; results land at their input index.
    fn scan_pool<T: Sync>(
        &self,
        items: &[T],
        already_done: usize,
        total: usize,
        scan: impl Fn(&T) -> TargetResult + Sync,
        path: impl Fn(&T) -> &Path,
        progress: &mut dyn FnMut(u32, u32, Option<&Path>),
    ) -> Vec<TargetResult> {
        let total = total as u32;
        let mut done = already_done as u32;
        let mut results: Vec<Option<TargetResult>> = (0..items.len()).map(|_| None).collect();
        let workers = self.jobs.max(1).min(items.len());
        if workers > 0 {
            let next = AtomicUsize::new(0);
            let (tx, rx) = mpsc::channel::<Msg>();
            std::thread::scope(|s| {
                for _ in 0..workers {
                    let tx = tx.clone();
                    let (next, scan) = (&next, &scan);
                    s.spawn(move || {
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(item) = items.get(i) else { break };
                            if tx.send(Msg::Started(i)).is_err() {
                                break;
                            }
                            let r = scan(item);
                            if tx.send(Msg::Done(i, r)).is_err() {
                                break;
                            }
                        }
                    });
                }
                drop(tx);
                // In flight, in input order (`current` is the earliest one).
                let mut running = BTreeSet::new();
                for msg in rx {
                    match msg {
                        Msg::Started(i) => {
                            running.insert(i);
                            progress(done, total, Some(path(&items[i])));
                        }
                        Msg::Done(i, r) => {
                            running.remove(&i);
                            results[i] = Some(r);
                            done += 1;
                            if let Some(&j) = running.first() {
                                progress(done, total, Some(path(&items[j])));
                            }
                        }
                    }
                }
            });
        }
        progress(total, total, None);
        results
            .into_iter()
            .map(|r| r.unwrap_or_else(|| Err(Failure::plugin("scan worker died"))))
            .collect()
    }
}

impl ScanReport {
    pub(crate) fn push(&mut self, path: &Path, result: TargetResult) {
        match result {
            Ok(plugins) => self.plugins.extend(plugins),
            Err(f) => self.failed.push(ScanFailure {
                path: path.to_string_lossy().into_owned(),
                message: f.message,
            }),
        }
    }
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;

    /// Serializes the tests that write and then exec a script. Linux refuses to exec a file
    /// that any process has open for writing (ETXTBSY): if one test spawns a scanner while
    /// another is writing its script, the forked child briefly inherits that write fd (until
    /// its own exec closes it), and the second test's spawn fails with "Text file busy".
    pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A fresh per-test temp dir.
    pub(crate) fn temp_dir(purpose: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(ether_core::plugin::ipc_name(
            "test",
            std::process::id(),
            purpose,
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A fake "scanner" shell script (hold [`serial`] while writing and running it).
    pub(crate) fn script(purpose: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_dir(purpose).join("scanner.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A fake scanner that behaves by the target's file name: `*hang*` sleeps forever,
    /// `*crash*` exits 3, `*bad*` answers `Err`, anything else sleeps `$SLEEP` (default 0)
    /// then reports one plugin whose id is the target path. Every run appends the target
    /// to `<dir>/runs.log`.
    pub(crate) fn behaving_scanner(purpose: &str, sleep_s: &str) -> PathBuf {
        let s = script(purpose, "");
        let log = s.with_file_name("runs.log");
        let body = format!(
            r#"req=$(cat)
p=$(printf '%s' "$req" | sed 's/.*"bundle_path":"\([^"]*\)".*/\1/')
echo "$p" >> '{log}'
case "$p" in
  *hang*) exec sleep 600 ;;
  *crash*) echo 'boom' >&2; exit 3 ;;
  *bad*) echo '{{"type":"Err","message":"not a plugin"}}'; exit 0 ;;
esac
sleep {sleep_s}
printf '{{"type":"Ok","plugins":[{{"format":"Clap","id":"%s","name":"N","vendor":"","version":"","description":"","features":[],"category":"AudioEffect","path":"%s"}}]}}\n' "$p" "$p""#,
            log = log.display()
        );
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&s, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o755)).unwrap();
        s
    }

    /// Targets (as recorded in `runs.log` next to `scanner`) the fake scanner ran on.
    pub(crate) fn runs(scanner: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_to_string(scanner.with_file_name("runs.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        v.sort();
        v
    }

    pub(crate) fn clap(path: impl Into<PathBuf>) -> ScanTarget {
        ScanTarget {
            format: ether_core::protocol::model::PluginFormat::Clap,
            path: path.into(),
        }
    }

    #[test]
    fn a_daemon_holding_the_pipes_does_not_hang_the_scan() {
        let _serial = serial();
        // The "plugin" leaves a background process with our stdout/stderr open, then the
        // scanner answers and exits.
        let s = script(
            "scan-daemon",
            r#"cat > /dev/null
sleep 8 &
echo 'noise from the plugin'
echo '{"type":"Ok","plugins":[]}'"#,
        );
        let start = Instant::now();
        let r = ScanRunner::new(&s)
            .with_drain_timeout(Duration::from_millis(300))
            .scan_bundle(Path::new("/x/A.clap"));
        assert_eq!(r, Ok(vec![]));
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn sidechain_inputs_cross_the_scanner_and_default_to_none() {
        let _serial = serial();
        // A current scanner reports the aux bus; an older one (no field) means none.
        let plugin = |extra: &str| {
            format!(
                r#"{{"format":"Clap","id":"x","name":"X","vendor":"","version":"","description":"","features":[],"category":"AudioEffect","path":"/x/A.clap"{extra}}}"#
            )
        };
        let s = script(
            "scan-sidechain",
            &format!(
                "cat > /dev/null\necho '{{\"type\":\"Ok\",\"plugins\":[{},{}]}}'",
                plugin(r#","sidechain_inputs":2"#),
                plugin("")
            ),
        );
        let plugins = ScanRunner::new(&s)
            .scan_bundle(Path::new("/x/A.clap"))
            .expect("scan");
        assert_eq!(
            plugins
                .iter()
                .map(|p| p.sidechain_inputs)
                .collect::<Vec<_>>(),
            [2, 0]
        );
    }

    #[test]
    fn request_carries_the_format_of_a_target() {
        let _serial = serial();
        // Echo the request back as the error message.
        let s = script(
            "scan-echo",
            r#"req=$(cat)
printf '{"type":"Err","message":%s}\n' "$(printf '%s' "$req" | sed 's/"/\\"/g; s/^/"/; s/$/"/')""#,
        );
        let runner = ScanRunner::new(&s);
        let msg = runner
            .scan_target(&ScanTarget {
                format: ether_core::protocol::model::PluginFormat::Au,
                path: "aufx:dely:appl".into(),
            })
            .unwrap_err();
        let req: ScanRequest = serde_json::from_str(&msg).unwrap();
        assert_eq!(req.bundle_path, "aufx:dely:appl");
        assert_eq!(
            req.format,
            Some(ether_core::protocol::model::PluginFormat::Au)
        );
        let msg = runner.scan_bundle(Path::new("/x/A.clap")).unwrap_err();
        assert_eq!(msg, r#"{"bundle_path":"/x/A.clap"}"#);

        let report = runner.scan_targets(
            &[ScanTarget {
                format: ether_core::protocol::model::PluginFormat::Vst3,
                path: "/x/B.vst3".into(),
            }],
            |_, _, _| {},
        );
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].path, "/x/B.vst3");
    }

    #[test]
    fn default_jobs_is_bounded() {
        let n = ScanRunner::default_jobs();
        assert!((1..=ScanRunner::MAX_DEFAULT_JOBS).contains(&n) || std::env::var(SCAN_JOBS_ENV).is_ok());
        assert_eq!(ScanRunner::new("x").with_jobs(0).jobs, 1);
    }

    #[test]
    fn the_pool_overlaps_scans() {
        let _serial = serial();
        // 8 targets x 0.4 s with 4 children: ~2 rounds (0.8 s) instead of 3.2 s.
        let s = behaving_scanner("scan-overlap", "0.4");
        let targets: Vec<_> = (0..8).map(|i| clap(format!("/x/P{i}.clap"))).collect();
        let start = Instant::now();
        let report = ScanRunner::new(&s).with_jobs(4).scan_targets(&targets, |_, _, _| {});
        let took = start.elapsed();
        assert_eq!(report.plugins.len(), 8);
        assert!(took >= Duration::from_millis(780), "{took:?}");
        assert!(took < Duration::from_millis(2400), "no overlap: {took:?}");
    }

    #[test]
    fn a_hang_or_crash_only_loses_its_own_target_and_order_is_input_order() {
        let _serial = serial();
        let s = behaving_scanner("scan-isolation", "0.05");
        let names = [
            "/x/A.clap",
            "/x/hang.clap",
            "/x/B.clap",
            "/x/crash.clap",
            "/x/C.clap",
            "/x/bad.clap",
            "/x/D.clap",
        ];
        let targets: Vec<_> = names.iter().map(|n| clap(*n)).collect();
        let mut calls = Vec::new();
        let start = Instant::now();
        let report = ScanRunner::new(&s)
            .with_jobs(3)
            .with_timeout(Duration::from_millis(1500))
            .scan_targets(&targets, |done, total, current| {
                calls.push((done, total, current.map(Path::to_path_buf)));
            });
        // The hang costs its own timeout, in parallel with the rest.
        assert!(start.elapsed() < Duration::from_secs(4), "{:?}", start.elapsed());
        let ids: Vec<_> = report.plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["/x/A.clap", "/x/B.clap", "/x/C.clap", "/x/D.clap"]);
        let failed: Vec<_> = report.failed.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(failed, ["/x/hang.clap", "/x/crash.clap", "/x/bad.clap"]);
        assert!(report.failed[0].message.contains("timed out"), "{:?}", report.failed);
        assert!(report.failed[1].message.contains("crashed"), "{:?}", report.failed);
        assert_eq!(report.failed[2].message, "not a plugin");

        // Progress: monotonic `done`, every target reported as current once it starts,
        // ends with (total, total, None).
        assert_eq!(calls.last(), Some(&(7, 7, None)));
        assert!(calls.windows(2).all(|w| w[0].0 <= w[1].0), "{calls:?}");
        for n in names {
            assert!(
                calls.iter().any(|c| c.2.as_deref() == Some(Path::new(n))),
                "{n} never current: {calls:?}"
            );
        }
        assert!(calls[..calls.len() - 1].iter().all(|c| c.2.is_some()));
    }

    #[test]
    fn one_job_keeps_the_sequential_progress_contract() {
        let _serial = serial();
        let s = behaving_scanner("scan-seq", "0");
        let targets = [clap("/x/A.clap"), clap("/x/crash.clap")];
        let mut calls = Vec::new();
        let report = ScanRunner::new(&s)
            .with_jobs(1)
            .scan_targets(&targets, |done, total, current| {
                calls.push((done, total, current.map(Path::to_path_buf)));
            });
        assert_eq!(report.plugins.len(), 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(
            calls,
            vec![
                (0, 2, Some(PathBuf::from("/x/A.clap"))),
                (1, 2, Some(PathBuf::from("/x/crash.clap"))),
                (2, 2, None)
            ]
        );
    }

    #[test]
    fn empty_input_reports_done() {
        let mut calls = Vec::new();
        let report = ScanRunner::new("/nonexistent").scan_targets(&[], |d, t, c| {
            calls.push((d, t, c.is_some()));
        });
        assert_eq!(report, ScanReport::default());
        assert_eq!(calls, [(0, 0, false)]);
    }
}
