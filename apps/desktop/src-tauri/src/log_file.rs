//! The engine log as a file, and a terminal window that follows it (Settings > Advanced >
//! "Open engine log"): release builds have no console, so this is where the log lines
//! (including the watchdog's "engine unresponsive" ones) can be read.
//!
//! The log goes to `<data_dir>/logs/ethereal.log`; the previous run's is kept as
//! `ethereal.prev.log`. Lines logged before the data dir is known (very early startup) go
//! to stdout only.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::fmt::MakeWriter;

const NAME: &str = "ethereal.log";
const PREVIOUS: &str = "ethereal.prev.log";

static FILE: OnceLock<(PathBuf, Mutex<File>)> = OnceLock::new();

/// Start writing the log to `logs_dir` (once; later calls are ignored).
pub fn open(logs_dir: &Path) -> io::Result<()> {
    if FILE.get().is_some() {
        return Ok(());
    }
    std::fs::create_dir_all(logs_dir)?;
    let path = logs_dir.join(NAME);
    if path.exists() {
        let _ = std::fs::rename(&path, logs_dir.join(PREVIOUS));
    }
    let file = File::create(&path)?;
    let _ = FILE.set((path, Mutex::new(file)));
    Ok(())
}

/// The log file, once [`open`]ed.
pub fn path() -> Option<&'static Path> {
    FILE.get().map(|(p, _)| p.as_path())
}

/// `tracing_subscriber` writer for the log file (drops lines until [`open`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct LogFile;

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some((_, f)) = FILE.get()
            && let Ok(mut f) = f.lock()
        {
            f.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some((_, f)) = FILE.get()
            && let Ok(mut f) = f.lock()
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
    follow_in_terminal(path).map_err(|e| format!("could not open a terminal: {e}"))
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
