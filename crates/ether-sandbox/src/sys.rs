//! OS primitives (macOS + Linux): POSIX named semaphores for the cross-process block wake-up,
//! shared-memory naming, and stdout redirection for the helper.
//!
//! The few libc calls are declared here directly (the workspace has no `libc` dependency).

use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::io;

use ether_core::plugin::ipc_name;

unsafe extern "C" {
    fn sem_open(name: *const c_char, oflag: c_int, ...) -> *mut c_void;
    fn sem_close(sem: *mut c_void) -> c_int;
    fn sem_unlink(name: *const c_char) -> c_int;
    fn sem_post(sem: *mut c_void) -> c_int;
    fn sem_wait(sem: *mut c_void) -> c_int;
    fn dup(fd: c_int) -> c_int;
    fn dup2(src: c_int, dst: c_int) -> c_int;
}

#[cfg(target_os = "macos")]
const O_CREAT: c_int = 0x0200;
#[cfg(target_os = "macos")]
const O_EXCL: c_int = 0x0800;
#[cfg(target_os = "linux")]
const O_CREAT: c_int = 0o100;
#[cfg(target_os = "linux")]
const O_EXCL: c_int = 0o200;
const EINTR: i32 = 4;

/// OS name for a sandbox IPC object. Built from [`ipc_name`] (instance + host pid +
/// purpose), then hashed: macOS limits POSIX shm and semaphore names to 31 bytes.
pub(crate) fn os_name(instance: &str, pid: u32, purpose: &str) -> String {
    let full = ipc_name(instance, pid, purpose);
    // FNV-1a 64.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in full.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("/eth{h:016x}")
}

/// A POSIX named semaphore (`sem_open`). `post` is a non-blocking syscall, safe to call from
/// the audio thread; `wait` blocks (helper side only).
#[derive(Debug)]
pub(crate) struct Semaphore {
    sem: *mut c_void,
    name: CString,
    linked: bool,
}

// SAFETY: a semaphore handle may be used from any thread.
unsafe impl Send for Semaphore {}
// SAFETY: sem_post/sem_wait are thread-safe.
unsafe impl Sync for Semaphore {}

fn failed(p: *mut c_void) -> bool {
    // SEM_FAILED is (sem_t*)-1 on macOS and (sem_t*)0 on Linux.
    p.is_null() || p as usize == usize::MAX
}

impl Semaphore {
    /// Create (count 0), replacing a stale object of the same name left by a crash.
    pub fn create(name: &str) -> io::Result<Self> {
        let cname = CString::new(name).map_err(io::Error::other)?;
        // SAFETY: valid C string; unlinking a missing name just fails with ENOENT.
        unsafe { sem_unlink(cname.as_ptr()) };
        // SAFETY: valid C string; variadic args are (mode_t, unsigned) promoted to c_uint.
        let sem = unsafe {
            sem_open(
                cname.as_ptr(),
                O_CREAT | O_EXCL,
                0o600 as c_uint,
                0 as c_uint,
            )
        };
        if failed(sem) {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            sem,
            name: cname,
            linked: true,
        })
    }

    /// Open an existing semaphore (does not own its name).
    pub fn open(name: &str) -> io::Result<Self> {
        let cname = CString::new(name).map_err(io::Error::other)?;
        // SAFETY: valid C string.
        let sem = unsafe { sem_open(cname.as_ptr(), 0) };
        if failed(sem) {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            sem,
            name: cname,
            linked: false,
        })
    }

    /// Remove the name (the object lives on while opened). Idempotent.
    pub fn unlink(&mut self) {
        if self.linked {
            // SAFETY: valid C string.
            unsafe { sem_unlink(self.name.as_ptr()) };
            self.linked = false;
        }
    }

    /// RT-safe: never blocks, never allocates.
    #[inline]
    pub fn post(&self) {
        // SAFETY: `sem` is a valid open semaphore.
        unsafe { sem_post(self.sem) };
    }

    /// Block until posted.
    pub fn wait(&self) {
        loop {
            // SAFETY: `sem` is a valid open semaphore.
            if unsafe { sem_wait(self.sem) } == 0 {
                return;
            }
            if io::Error::last_os_error().raw_os_error() != Some(EINTR) {
                return;
            }
        }
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        self.unlink();
        // SAFETY: `sem` is valid and closed exactly once.
        unsafe { sem_close(self.sem) };
    }
}

/// Helper side: keep a private duplicate of stdout for the control channel and point fd 1 at
/// stderr, so plugins printing to stdout can't corrupt the protocol. Returns the private fd.
pub(crate) fn take_stdout() -> io::Result<c_int> {
    // SAFETY: plain fd syscalls on the process's own standard descriptors.
    let fd = unsafe { dup(1) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { dup2(2, 1) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_short_and_distinct() {
        let a = os_name("agent-with-a-rather-long-instance-name", 123_456, "sbx-1-shm");
        let b = os_name("agent-with-a-rather-long-instance-name", 123_457, "sbx-1-shm");
        let c = os_name("other", 123_456, "sbx-1-shm");
        assert!(a.len() <= 31 && a.starts_with('/'));
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn semaphore_post_wait_and_stale_replace() {
        let name = os_name("sandbox-unit", std::process::id(), "sem-test");
        let owner = Semaphore::create(&name).unwrap();
        // A second create with the same name replaces it (stale cleanup) instead of failing.
        let owner2 = Semaphore::create(&name).unwrap();
        let other = Semaphore::open(&name).unwrap();
        owner2.post();
        other.wait();
        drop(owner);
        drop(owner2);
        drop(other);
        assert!(Semaphore::open(&name).is_err(), "name must be unlinked on drop");
    }
}
