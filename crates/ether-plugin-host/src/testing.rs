//! Test support shared by the plugin crates' tests (`ether-clap`, `ether-vst3`,
//! the scanner, the sandbox, `ether-native`): temp dirs that don't outlive the
//! test process. Not part of the hosting API.
//!
//! Test fixtures often live in a `static` (a plugin bundle built once per test binary), and
//! statics are never dropped, so a drop guard can't clean them up. Instead every dir made
//! here is removed when the process exits (`atexit`, which runs both when the test harness
//! returns and when it calls `process::exit` after a failure). Only a killed process leaks.

use std::path::PathBuf;
use std::sync::{Mutex, Once};

use ether_core::plugin::ipc_name;

/// Sanitized dev instance id (`ETHER_INSTANCE`, see README "Running multiple dev instances").
pub fn instance_id() -> String {
    let raw = std::env::var("ETHER_INSTANCE").unwrap_or_else(|_| "default".into());
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() { "default".into() } else { s }
}

/// A fresh, empty temp directory named with the instance + pid rule (`ipc_name`), unique
/// within the process, removed at process exit.
pub fn temp_dir(purpose: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    temp_dir_named(&ipc_name(
        &instance_id(),
        std::process::id(),
        &format!("{purpose}-{n}"),
    ))
}

/// `<tmp>/<name>`, emptied and created, removed at process exit. `name` should contain the
/// pid so concurrent test processes don't share it.
pub fn temp_dir_named(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    remove_at_exit(dir.clone());
    dir
}

/// Dirs [`remove_registered`] deletes at process exit.
static AT_EXIT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Removes `dir` (recursively) when the process exits.
pub fn remove_at_exit(dir: PathBuf) {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        unsafe extern "C" {
            fn atexit(cb: extern "C" fn()) -> std::ffi::c_int;
        }
        // SAFETY: `remove_registered` is a plain `extern "C" fn()` that never unwinds (it
        // only takes a lock with `try_lock` and ignores I/O errors).
        unsafe { atexit(remove_registered) };
    });
    AT_EXIT.lock().unwrap_or_else(|e| e.into_inner()).push(dir);
}

extern "C" fn remove_registered() {
    // `try_lock`: a test thread still running at exit could hold the lock; leaking then
    // beats deadlocking the exit.
    let dirs = match AT_EXIT.try_lock() {
        Ok(mut d) => std::mem::take(&mut *d),
        Err(std::sync::TryLockError::Poisoned(e)) => std::mem::take(&mut *e.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    for dir in dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A child test process makes a dir and exits: the dir is gone afterwards.
    #[test]
    fn dirs_are_removed_at_process_exit() {
        const CHILD: &str = "ETHER_TEST_AT_EXIT_CHILD";
        if let Some(out) = std::env::var_os(CHILD) {
            let dir = temp_dir("at-exit");
            std::fs::write(dir.join("f"), b"x").unwrap();
            std::fs::write(out, dir.to_str().unwrap()).unwrap();
            return;
        }
        let out = temp_dir("at-exit-parent").join("child-dir.txt");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "testing::tests::dirs_are_removed_at_process_exit",
                "--test-threads=1",
            ])
            .env(CHILD, &out)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let child_dir = PathBuf::from(std::fs::read_to_string(&out).unwrap());
        assert!(child_dir.is_absolute(), "{child_dir:?}");
        assert!(!child_dir.exists(), "{child_dir:?} survived its process");
    }
}
