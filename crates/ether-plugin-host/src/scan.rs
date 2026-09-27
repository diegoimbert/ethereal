//! Host side of the out-of-process scanner (`ether-plugin-scanner`): one scan target per
//! child process, with a timeout, so a plugin that crashes or hangs while loading only loses
//! itself.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
}

/// Name of the scanner binary.
pub const SCANNER_BIN: &str = if cfg!(windows) {
    "ether-plugin-scanner.exe"
} else {
    "ether-plugin-scanner"
};

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

impl ScanRunner {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
    pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

    pub fn new(scanner: impl Into<PathBuf>) -> Self {
        Self {
            scanner: scanner.into(),
            timeout: Self::DEFAULT_TIMEOUT,
            drain_timeout: Self::DEFAULT_DRAIN_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_drain_timeout(mut self, drain_timeout: Duration) -> Self {
        self.drain_timeout = drain_timeout;
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
        self.run(&ScanRequest {
            bundle_path: bundle.to_string_lossy().into_owned(),
            format: None,
        })
    }

    /// Scan one target of a known format in a fresh scanner process.
    pub fn scan_target(&self, target: &ScanTarget) -> Result<Vec<PluginDescriptor>, String> {
        self.run(&ScanRequest {
            bundle_path: target.path.to_string_lossy().into_owned(),
            format: Some(target.format),
        })
    }

    fn run(&self, request: &ScanRequest) -> Result<Vec<PluginDescriptor>, String> {
        let request = serde_json::to_string(request).map_err(|e| e.to_string())?;

        let mut child = Command::new(&self.scanner)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start scanner {}: {e}", self.scanner.display()))?;

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
                Err(e) => return Err(format!("scanner wait failed: {e}")),
            }
        };
        let Some(status) = status else {
            // Killed: the pipes may be held open by grandchildren; don't wait for them.
            return Err(format!("scan timed out after {:?}", self.timeout));
        };
        // Bounded: a daemon spawned by the plugin may hold the pipes open forever.
        let drain_deadline = Instant::now() + self.drain_timeout;
        let out = drain(&out_rx, drain_deadline);
        let err = drain(&err_rx, drain_deadline);

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

    /// Scan every bundle sequentially (format inferred per bundle). `progress(done, total,
    /// current)` is called before each bundle and once at the end with `current = None`.
    pub fn scan_all(
        &self,
        bundles: &[PathBuf],
        mut progress: impl FnMut(u32, u32, Option<&Path>),
    ) -> ScanReport {
        self.scan_each(
            bundles,
            |b| self.scan_bundle(b),
            |b| b.as_path(),
            &mut progress,
        )
    }

    /// Scan every target sequentially, like [`ScanRunner::scan_all`].
    pub fn scan_targets(
        &self,
        targets: &[ScanTarget],
        mut progress: impl FnMut(u32, u32, Option<&Path>),
    ) -> ScanReport {
        self.scan_each(
            targets,
            |t| self.scan_target(t),
            |t| t.path.as_path(),
            &mut progress,
        )
    }

    fn scan_each<T>(
        &self,
        items: &[T],
        scan: impl Fn(&T) -> Result<Vec<PluginDescriptor>, String>,
        path: impl Fn(&T) -> &Path,
        progress: &mut dyn FnMut(u32, u32, Option<&Path>),
    ) -> ScanReport {
        let total = items.len() as u32;
        let mut report = ScanReport::default();
        for (i, item) in items.iter().enumerate() {
            progress(i as u32, total, Some(path(item)));
            match scan(item) {
                Ok(plugins) => report.plugins.extend(plugins),
                Err(message) => report.failed.push(ScanFailure {
                    path: path(item).to_string_lossy().into_owned(),
                    message,
                }),
            }
        }
        progress(total, total, None);
        report
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A fake "scanner" shell script.
    fn script(purpose: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(ether_core::plugin::ipc_name(
            "test",
            std::process::id(),
            purpose,
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scanner.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_daemon_holding_the_pipes_does_not_hang_the_scan() {
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
        assert!(start.elapsed() < Duration::from_secs(4), "{:?}", start.elapsed());
    }

    #[test]
    fn request_carries_the_format_of_a_target() {
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
        assert_eq!(req.format, Some(ether_core::protocol::model::PluginFormat::Au));
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
}
