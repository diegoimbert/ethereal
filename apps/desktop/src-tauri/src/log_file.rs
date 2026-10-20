//! The app's log as size-capped, rotating files, and a terminal window that follows it
//! (Settings > Advanced > "Open engine log"). Release builds have no console, so this is
//! where the log lines (including the watchdog's "engine unresponsive" ones) can be read.
//!
//! The current file is `<logs dir>/ethereal.log`; each launch, and each time it reaches
//! [`MAX_BYTES`], it moves to `ethereal.1.log` (`.1` to `.2` …), and the oldest beyond
//! [`KEEP`] files is deleted: at most 25 MB, the last few runs. Lines logged before the logs
//! dir is known (very early startup) are kept in memory and written when it opens. Every line
//! goes through an extension's filter first ([`set_line_filter`], e.g. to redact secrets).

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use tracing_subscriber::fmt::MakeWriter;

const NAME: &str = "ethereal.log";
/// The pre-rotation name of the previous run's file (removed on open).
const LEGACY_PREVIOUS: &str = "ethereal.prev.log";
/// Size at which the current file is rotated.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Files kept, the current one included.
pub const KEEP: usize = 5;
/// Early lines kept in memory until [`open`].
const EARLY_MAX: usize = 256 * 1024;

/// A log file that rotates by size: `ethereal.log`, `ethereal.1.log`, …
/// `ethereal.{keep-1}.log`.
#[derive(Debug)]
pub struct Rotating {
    dir: PathBuf,
    file: File,
    written: u64,
    max_bytes: u64,
    keep: usize,
}

impl Rotating {
    /// Open a fresh current file in `dir`, rotating what an earlier run left.
    pub fn open(dir: &Path, max_bytes: u64, keep: usize) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        // Best effort: an old run's leftover.
        let _ = std::fs::remove_file(dir.join(LEGACY_PREVIOUS));
        if dir.join(NAME).exists() {
            shift(dir, keep);
        }
        let file = File::create(dir.join(NAME))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            file,
            written: 0,
            max_bytes,
            keep: keep.max(1),
        })
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(NAME)
    }

    /// Append `buf`; rotates first when it would pass the size cap.
    pub fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if self.written > 0 && self.written + buf.len() as u64 > self.max_bytes {
            self.file.flush()?;
            shift(&self.dir, self.keep);
            self.file = File::create(self.dir.join(NAME))?;
            self.written = 0;
        }
        self.file.write_all(buf)?;
        self.written += buf.len() as u64;
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// `ethereal.{i}.log` (`i` 0: the current file).
fn numbered(dir: &Path, i: usize) -> PathBuf {
    if i == 0 {
        dir.join(NAME)
    } else {
        dir.join(format!("ethereal.{i}.log"))
    }
}

/// Move every file one number up, deleting the one that would pass `keep`. Best effort: a
/// file that can't move is overwritten later, the log goes on.
fn shift(dir: &Path, keep: usize) {
    let keep = keep.max(1);
    let _ = std::fs::remove_file(numbered(dir, keep - 1));
    for i in (0..keep - 1).rev() {
        let from = numbered(dir, i);
        if from.exists() {
            let _ = std::fs::rename(&from, numbered(dir, i + 1));
        }
    }
}

/// The log files in `dir`, the current one first (for "export logs").
pub fn files_in(dir: &Path) -> Vec<PathBuf> {
    (0..KEEP)
        .map(|i| numbered(dir, i))
        .filter(|p| p.is_file())
        .collect()
}

#[derive(Default)]
struct State {
    early: Vec<u8>,
    file: Option<Rotating>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

/// Rewrites a log line before it is written ([`set_line_filter`]).
pub type LineFilter = fn(&str) -> String;

static LINE_FILTER: std::sync::OnceLock<LineFilter> = std::sync::OnceLock::new();

/// Extension point: every line goes through `filter` before it reaches the file (e.g. to
/// redact secrets). Set once, at start.
pub fn set_line_filter(filter: LineFilter) {
    let _ = LINE_FILTER.set(filter);
}

/// Start writing the log to `logs_dir` (once; later calls are ignored).
pub fn open(logs_dir: &Path) -> io::Result<()> {
    let mut state = STATE.lock().map_err(|_| io::Error::other("log lock"))?;
    if state.file.is_some() {
        return Ok(());
    }
    let mut file = Rotating::open(logs_dir, MAX_BYTES, KEEP)?;
    let early = std::mem::take(&mut state.early);
    if !early.is_empty() {
        file.write_all(&early)?;
    }
    state.file = Some(file);
    Ok(())
}

/// The current log file, once [`open`]ed.
pub fn path() -> Option<PathBuf> {
    STATE.lock().ok()?.file.as_ref().map(Rotating::path)
}

/// The logs folder, once [`open`]ed.
pub fn dir() -> Option<PathBuf> {
    STATE.lock().ok()?.file.as_ref().map(|f| f.dir.clone())
}

/// Flush the current file (before reading it back for an export).
pub fn flush() {
    if let Ok(mut state) = STATE.lock()
        && let Some(f) = state.file.as_mut()
        && let Err(e) = f.flush()
    {
        eprintln!("could not flush the log file: {e}");
    }
}

/// `tracing_subscriber` writer for the log file (keeps early lines in memory until [`open`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct LogFile;

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = String::from_utf8_lossy(buf);
        let line = match LINE_FILTER.get() {
            Some(filter) => std::borrow::Cow::Owned(filter(&line)),
            None => line,
        };
        if let Ok(mut state) = STATE.lock() {
            let state = &mut *state;
            match state.file.as_mut() {
                Some(f) => f.write_all(line.as_bytes())?,
                None if state.early.len() + line.len() <= EARLY_MAX => {
                    state.early.extend_from_slice(line.as_bytes());
                }
                None => {}
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Ok(mut state) = STATE.lock()
            && let Some(f) = state.file.as_mut()
        {
            f.flush()?;
        }
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogFile {
    type Writer = LogFile;

    fn make_writer(&'a self) -> Self::Writer {
        LogFile
    }
}

/// Open a terminal window that shows the log file and follows it.
#[tauri::command]
pub fn open_engine_log() -> Result<(), String> {
    let path = path().ok_or("the engine log is not available")?;
    follow_in_terminal(&path).map_err(|e| format!("could not open a terminal: {e}"))
}

#[cfg(windows)]
fn follow_in_terminal(path: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let quoted = path.display().to_string().replace('\'', "''");
    std::process::Command::new("powershell.exe")
        .args([
            "-NoExit",
            "-NoProfile",
            "-Command",
            &format!(
                "$Host.UI.RawUI.WindowTitle = 'Ethereal engine log'; \
                 Get-Content -LiteralPath '{quoted}' -Tail 1000 -Wait"
            ),
        ])
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(drop)
}

#[cfg(target_os = "macos")]
fn follow_in_terminal(path: &Path) -> io::Result<()> {
    // Console.app follows log files as they grow.
    std::process::Command::new("open")
        .args(["-a", "Console"])
        .arg(path)
        .spawn()
        .map(drop)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn follow_in_terminal(path: &Path) -> io::Result<()> {
    std::process::Command::new("x-terminal-emulator")
        .args(["-e", "tail", "-n", "1000", "-f"])
        .arg(path)
        .spawn()
        .or_else(|_| std::process::Command::new("xdg-open").arg(path).spawn())
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ether-log-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn rotates_by_size_and_keeps_at_most_keep_files() {
        let dir = tmp("size");
        let mut log = Rotating::open(&dir, 100, 3).unwrap();
        for i in 0..20 {
            log.write_all(format!("line {i:02} ........................\n").as_bytes())
                .unwrap();
        }
        log.flush().unwrap();
        let files = files_in(&dir);
        assert_eq!(files.len(), 3, "{files:?}");
        for f in &files {
            assert!(std::fs::metadata(f).unwrap().len() <= 100);
        }
        // The current file has the latest line; the oldest lines are gone.
        let current = std::fs::read_to_string(dir.join(NAME)).unwrap();
        assert!(current.contains("line 19"));
        let all: String = files
            .iter()
            .map(|f| std::fs::read_to_string(f).unwrap())
            .collect();
        assert!(!all.contains("line 00"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_run_starts_a_new_file_and_keeps_the_previous_ones() {
        let dir = tmp("runs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LEGACY_PREVIOUS), "old").unwrap();
        for run in 0..7 {
            let mut log = Rotating::open(&dir, MAX_BYTES, KEEP).unwrap();
            log.write_all(format!("run {run}\n").as_bytes()).unwrap();
            log.flush().unwrap();
        }
        assert!(!dir.join(LEGACY_PREVIOUS).exists());
        let files = files_in(&dir);
        assert_eq!(files.len(), KEEP);
        assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "run 6\n");
        assert_eq!(std::fs::read_to_string(&files[1]).unwrap(), "run 5\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_cap_is_five_files_of_five_megabytes() {
        assert_eq!(MAX_BYTES, 5 * 1024 * 1024);
        assert_eq!(KEEP, 5);
    }
}
