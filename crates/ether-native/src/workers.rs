//! Real-time worker pool for multicore track processing (roadmap v2, `multicore`; see
//! `ether_core::parallel` and CONTRACTS.md §11.7).
//!
//! [`WorkerPool`] implements [`ParallelExecutor`]: `workers` threads created up front
//! ([`WorkerPool::new`], non-RT), which the audio thread hands each DAG level's track jobs.
//!
//! # Dispatch (RT, lock-free, no allocation)
//! - One 64-bit claim word packs `epoch:32 | jobs:16 | next:16`. `execute` stores a pointer
//!   to a stack [`Batch`] (the job closure), then publishes a new epoch with `next` = the
//!   first shared index (Release). Everyone (workers and the audio thread) claims indices
//!   with a CAS that only succeeds for that epoch while `next < jobs`, runs the job, and
//!   bumps `done` (Release). The audio thread runs the pinned jobs, helps with the rest,
//!   then spins until `done` covers every shared job (Acquire) and returns.
//! - A worker reads the batch pointer only **after** a successful claim: the epoch is then
//!   still running (its claimed job is unfinished, so `execute` can't have returned), hence
//!   the pointer is that epoch's and the closure is alive. A late worker that wakes after
//!   the epoch finished fails its CAS and never touches the closure. Epochs wrap after 2^32
//!   dispatches (days of audio), far beyond any preemption.
//! - Waiting: workers spin (bounded, [`SPIN_ITERS`]) for the next epoch, so levels dispatched
//!   back to back within a block are picked up without a syscall, then park
//!   (`std::thread::park`: a futex / ulock / dispatch semaphore, never a mutex shared with
//!   non-RT threads on Linux/macOS/Windows). `execute` wakes (`unpark`: one non-blocking
//!   syscall) only the workers that announced they sleep, Dekker-style with SeqCst on both
//!   sides so no wake-up is lost. The audio thread never blocks: it only spins on `done`.
//! - More than 65535 jobs in one level (never in practice) run on the audio thread.
//!
//! # Threads
//! Workers enable flush-to-zero like the audio thread ([`crate::rt::enable_flush_denormals`]),
//! so a job computes the same bits on any thread. Priority, where the OS allows it:
//! - macOS: Mach `THREAD_TIME_CONSTRAINT_POLICY` (the policy Core Audio gives its I/O
//!   thread) with the engine block as period. Joining the device's `os_workgroup` would
//!   additionally keep workers on performance cores with the I/O thread on Apple Silicon;
//!   cpal doesn't expose the workgroup, so that is left for later.
//! - Linux: `SCHED_FIFO` (best effort: needs `CAP_SYS_NICE`/rtkit limits, else normal).
//! - Windows/others: normal priority.
//!
//! # Size
//! [`default_workers`]: `ETHER_WORKERS` if set, else physical cores - 1 (the audio thread
//! works too) capped at [`MAX_DEFAULT_WORKERS`]. The native host also reads
//! `<data_dir>/config/engine.json` (`{"worker_threads": n}`); the env var wins.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use ether_core::parallel::ParallelExecutor;

/// Upper bound of the default worker count (more rarely helps a DAW graph and costs power).
pub const MAX_DEFAULT_WORKERS: usize = 8;
/// Spin iterations (`spin_loop` hints, ~tens of µs) before a worker parks.
pub const SPIN_ITERS: u32 = 1 << 14;
/// Jobs per level the claim word can address.
const MAX_JOBS: usize = u16::MAX as usize;

/// `ETHER_WORKERS` if set (and a number), else physical cores - 1, capped at
/// [`MAX_DEFAULT_WORKERS`].
pub fn default_workers() -> usize {
    env_workers().unwrap_or_else(|| physical_cores().saturating_sub(1).min(MAX_DEFAULT_WORKERS))
}

/// `ETHER_WORKERS`, if set to a number.
pub fn env_workers() -> Option<usize> {
    std::env::var("ETHER_WORKERS").ok()?.trim().parse().ok()
}

/// Physical CPU cores (logical CPUs where the OS doesn't tell).
pub fn physical_cores() -> usize {
    let logical = std::thread::available_parallelism().map_or(1, |n| n.get());
    os::physical_cores().filter(|&n| n > 0).unwrap_or(logical)
}

/// The job batch of one `execute` call, on the audio thread's stack.
struct Batch<'a> {
    job: &'a (dyn Fn(usize) + Sync),
}

struct Shared {
    /// `epoch:32 | jobs:16 | next:16`.
    claim: AtomicU64,
    /// Jobs of the current epoch finished (shared ones only).
    done: AtomicUsize,
    /// The current epoch's batch (valid while that epoch runs; see the module docs).
    batch: AtomicPtr<Batch<'static>>,
    shutdown: AtomicBool,
    /// Per worker: parked (or about to park) and wants an `unpark`.
    sleeping: Box<[AtomicBool]>,
}

#[inline]
fn pack(epoch: u32, jobs: usize, next: usize) -> u64 {
    ((epoch as u64) << 32) | ((jobs as u64) << 16) | next as u64
}

#[inline]
fn unpack(c: u64) -> (u32, usize, usize) {
    ((c >> 32) as u32, ((c >> 16) & 0xffff) as usize, (c & 0xffff) as usize)
}

impl Shared {
    /// Claim the next index of `epoch`, if any is left.
    #[inline]
    fn claim(&self, epoch: u32) -> Option<usize> {
        let mut c = self.claim.load(Ordering::Acquire);
        loop {
            let (e, jobs, next) = unpack(c);
            if e != epoch || next >= jobs {
                return None;
            }
            match self.claim.compare_exchange_weak(
                c,
                pack(e, jobs, next + 1),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(next),
                Err(now) => c = now,
            }
        }
    }

    /// Run claimed jobs of `epoch` until none is left; returns how many ran.
    #[inline]
    fn work(&self, epoch: u32) -> usize {
        let mut ran = 0;
        while let Some(i) = self.claim(epoch) {
            // SAFETY: we hold an unfinished job of `epoch`, so `execute` for it hasn't
            // returned: `batch` points to its live `Batch` (stored before the epoch was
            // published, which our Acquire claim observed).
            let batch = unsafe { &*self.batch.load(Ordering::Acquire) };
            (batch.job)(i);
            self.done.fetch_add(1, Ordering::Release);
            ran += 1;
        }
        ran
    }
}

/// Fixed pool of real-time worker threads (see the module docs).
pub struct WorkerPool {
    shared: Arc<Shared>,
    handles: Vec<JoinHandle<()>>,
    epoch: u32,
}

impl std::fmt::Debug for WorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerPool")
            .field("workers", &self.handles.len())
            .finish()
    }
}

/// Options of [`WorkerPool::with_options`].
#[derive(Clone, Copy, Debug)]
pub struct PoolOptions {
    /// Engine block duration: the period of the macOS time-constraint policy.
    pub period: Duration,
    /// Try to raise the workers to real-time priority.
    pub realtime: bool,
    /// Enable flush-to-zero on the workers (match the audio thread).
    pub flush_denormals: bool,
}

impl Default for PoolOptions {
    fn default() -> Self {
        Self {
            period: Duration::from_micros(1024 * 1_000_000 / 48_000),
            realtime: true,
            flush_denormals: true,
        }
    }
}

impl WorkerPool {
    /// Non-RT. `workers` threads (0 = sequential) at real-time priority where allowed.
    pub fn new(workers: usize, period: Duration) -> Self {
        Self::with_options(
            workers,
            PoolOptions {
                period,
                ..PoolOptions::default()
            },
        )
    }

    /// Non-RT. See [`PoolOptions`].
    pub fn with_options(workers: usize, options: PoolOptions) -> Self {
        let shared = Arc::new(Shared {
            claim: AtomicU64::new(pack(0, 0, 0)),
            done: AtomicUsize::new(0),
            batch: AtomicPtr::new(std::ptr::null_mut()),
            shutdown: AtomicBool::new(false),
            sleeping: (0..workers).map(|_| AtomicBool::new(false)).collect(),
        });
        let mut handles = Vec::with_capacity(workers);
        for w in 0..workers {
            let s = shared.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("ether-worker-{w}"))
                .spawn(move || worker_main(&s, w, options));
            match spawned {
                Ok(h) => handles.push(h),
                Err(e) => {
                    tracing::warn!(%e, "failed to spawn audio worker; continuing with fewer");
                    break;
                }
            }
        }
        Self {
            shared,
            handles,
            epoch: 0,
        }
    }

    fn wake(&self, want: usize) {
        // The first `want` workers: the spinning ones pick the epoch up by themselves.
        for (flag, h) in self.shared.sleeping.iter().zip(&self.handles).take(want) {
            if flag.load(Ordering::SeqCst) {
                h.thread().unpark();
            }
        }
    }
}

fn worker_main(s: &Shared, index: usize, options: PoolOptions) {
    if options.flush_denormals {
        crate::rt::enable_flush_denormals();
    }
    if options.realtime {
        os::promote_current_thread(options.period);
    }
    let mut seen = unpack(s.claim.load(Ordering::Acquire)).0;
    let sleeping = &s.sleeping[index];
    loop {
        // Wait for a new epoch: spin, then park.
        let mut spins = 0u32;
        let epoch = loop {
            if s.shutdown.load(Ordering::Acquire) {
                return;
            }
            let e = unpack(s.claim.load(Ordering::Acquire)).0;
            if e != seen {
                break e;
            }
            if spins < SPIN_ITERS {
                spins += 1;
                std::hint::spin_loop();
                continue;
            }
            sleeping.store(true, Ordering::SeqCst);
            // Re-check after announcing (pairs with the SeqCst publish + flag load in
            // `execute`): either we see the new epoch or `execute` sees our flag.
            if unpack(s.claim.load(Ordering::SeqCst)).0 == seen
                && !s.shutdown.load(Ordering::SeqCst)
            {
                std::thread::park();
            }
            sleeping.store(false, Ordering::SeqCst);
            spins = 0;
        };
        seen = epoch;
        s.work(epoch);
    }
}

impl ParallelExecutor for WorkerPool {
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        self.execute_pinned(jobs, 0, job);
    }

    fn execute_pinned(&mut self, jobs: usize, pinned: usize, job: &(dyn Fn(usize) + Sync)) {
        let pinned = pinned.min(jobs);
        let shared_jobs = jobs - pinned;
        if self.handles.is_empty() || shared_jobs <= 1 || jobs > MAX_JOBS {
            for i in 0..jobs {
                job(i);
            }
            return;
        }
        let s = &*self.shared;
        let batch = Batch { job };
        self.epoch = self.epoch.wrapping_add(1);
        let epoch = self.epoch;
        s.done.store(0, Ordering::Relaxed);
        // The pointer is only dereferenced for this epoch, while `batch` is alive (see
        // `Shared::work`); erase the lifetime to store it.
        s.batch.store(
            (&batch as *const Batch<'_>).cast_mut().cast::<Batch<'static>>(),
            Ordering::Release,
        );
        s.claim.store(pack(epoch, jobs, pinned), Ordering::SeqCst);
        // The audio thread takes one shared job itself (or the pinned ones).
        self.wake(shared_jobs.saturating_sub(if pinned == 0 { 1 } else { 0 }));

        for i in 0..pinned {
            job(i);
        }
        s.work(epoch);
        // Everything is claimed; wait for the workers still running theirs.
        while s.done.load(Ordering::Acquire) < shared_jobs {
            std::hint::spin_loop();
        }
    }

    fn workers(&self) -> usize {
        self.handles.len()
    }
}

impl Drop for WorkerPool {
    /// Non-RT (the engine is dropped off the audio thread): stop and join the workers.
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        for h in &self.handles {
            h.thread().unpark();
        }
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

#[cfg(target_os = "macos")]
mod os {
    use std::time::Duration;

    #[repr(C)]
    struct MachTimebaseInfo {
        numer: u32,
        denom: u32,
    }

    #[repr(C)]
    struct TimeConstraintPolicy {
        period: u32,
        computation: u32,
        constraint: u32,
        preemptible: u32,
    }

    const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;
    const THREAD_TIME_CONSTRAINT_POLICY_COUNT: u32 = 4;

    unsafe extern "C" {
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
        fn pthread_self() -> usize;
        fn pthread_mach_thread_np(thread: usize) -> u32;
        fn thread_policy_set(thread: u32, flavor: u32, info: *mut u32, count: u32) -> i32;
        fn sysctlbyname(
            name: *const std::ffi::c_char,
            oldp: *mut std::ffi::c_void,
            oldlenp: *mut usize,
            newp: *mut std::ffi::c_void,
            newlen: usize,
        ) -> i32;
    }

    /// Time-constraint (real-time) policy for the calling thread: `period` = one engine
    /// block, computation budget half of it, constraint = the period.
    pub(super) fn promote_current_thread(period: Duration) {
        let mut tb = MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: plain out-parameter call.
        if unsafe { mach_timebase_info(&mut tb) } != 0 || tb.numer == 0 {
            return;
        }
        let to_abs = |d: Duration| {
            (d.as_nanos() as u64 * tb.denom as u64 / tb.numer as u64).min(u32::MAX as u64) as u32
        };
        let period = period.max(Duration::from_micros(500));
        let mut policy = TimeConstraintPolicy {
            period: to_abs(period),
            computation: to_abs(period / 2),
            constraint: to_abs(period),
            preemptible: 1,
        };
        // SAFETY: `policy` is the documented 4 × integer_t layout of
        // `thread_time_constraint_policy_data_t`; the thread port is this thread's.
        let r = unsafe {
            thread_policy_set(
                pthread_mach_thread_np(pthread_self()),
                THREAD_TIME_CONSTRAINT_POLICY,
                (&mut policy as *mut TimeConstraintPolicy).cast(),
                THREAD_TIME_CONSTRAINT_POLICY_COUNT,
            )
        };
        if r != 0 {
            tracing::debug!(r, "worker: time-constraint policy refused");
        }
    }

    pub(super) fn physical_cores() -> Option<usize> {
        let mut n: i32 = 0;
        let mut len = std::mem::size_of::<i32>();
        // SAFETY: NUL-terminated name; `n`/`len` describe a valid i32 out buffer.
        let r = unsafe {
            sysctlbyname(
                c"hw.physicalcpu".as_ptr(),
                (&mut n as *mut i32).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (r == 0 && n > 0).then_some(n as usize)
    }
}

#[cfg(target_os = "linux")]
mod os {
    use std::time::Duration;

    #[repr(C)]
    struct SchedParam {
        sched_priority: i32,
    }

    const SCHED_FIFO: i32 = 1;

    unsafe extern "C" {
        fn pthread_self() -> std::ffi::c_ulong;
        fn pthread_setschedparam(thread: std::ffi::c_ulong, policy: i32, param: *const SchedParam)
        -> i32;
    }

    /// `SCHED_FIFO` below typical audio-thread priorities; silently stays normal without
    /// the rights to do so.
    pub(super) fn promote_current_thread(_period: Duration) {
        let param = SchedParam { sched_priority: 60 };
        // SAFETY: valid thread handle (this thread) and param struct.
        let r = unsafe { pthread_setschedparam(pthread_self(), SCHED_FIFO, &param) };
        if r != 0 {
            tracing::debug!(r, "worker: SCHED_FIFO refused");
        }
    }

    /// Distinct `(physical id, core id)` pairs of `/proc/cpuinfo`.
    pub(super) fn physical_cores() -> Option<usize> {
        let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        let mut cores = std::collections::BTreeSet::new();
        let mut phys = None;
        for line in info.lines() {
            let mut kv = line.splitn(2, ':').map(str::trim);
            match (kv.next(), kv.next()) {
                (Some("physical id"), Some(v)) => phys = v.parse::<u32>().ok(),
                (Some("core id"), Some(v)) => {
                    if let Ok(c) = v.parse::<u32>() {
                        cores.insert((phys.unwrap_or(0), c));
                    }
                }
                _ => {}
            }
        }
        (!cores.is_empty()).then_some(cores.len())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod os {
    pub(super) fn promote_current_thread(_period: std::time::Duration) {}

    pub(super) fn physical_cores() -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn claim_word_round_trips() {
        assert_eq!(unpack(pack(7, 300, 12)), (7, 300, 12));
        assert_eq!(unpack(pack(u32::MAX, MAX_JOBS, MAX_JOBS)), (u32::MAX, MAX_JOBS, MAX_JOBS));
    }

    #[test]
    fn runs_every_job_exactly_once() {
        for workers in [0, 1, 2, 4] {
            let mut pool = WorkerPool::with_options(
                workers,
                PoolOptions {
                    realtime: false,
                    ..Default::default()
                },
            );
            assert_eq!(pool.workers(), workers);
            for jobs in [0, 1, 2, 3, 17, 64] {
                for pinned in [0, 1, 3] {
                    let hits: Vec<AtomicU32> = (0..jobs).map(|_| AtomicU32::new(0)).collect();
                    pool.execute_pinned(jobs, pinned, &|i| {
                        hits[i].fetch_add(1, Ordering::Relaxed);
                    });
                    assert!(hits.iter().all(|h| h.load(Ordering::Relaxed) == 1));
                }
            }
        }
    }

    #[test]
    fn pinned_jobs_run_on_the_caller() {
        let mut pool = WorkerPool::with_options(
            3,
            PoolOptions {
                realtime: false,
                ..Default::default()
            },
        );
        let me = std::thread::current().id();
        for _ in 0..50 {
            let wrong = AtomicU32::new(0);
            pool.execute_pinned(12, 4, &|i| {
                if i < 4 && std::thread::current().id() != me {
                    wrong.fetch_add(1, Ordering::Relaxed);
                }
            });
            assert_eq!(wrong.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn default_workers_is_bounded() {
        let n = physical_cores().saturating_sub(1).min(MAX_DEFAULT_WORKERS);
        assert!(n <= MAX_DEFAULT_WORKERS);
        assert!(physical_cores() >= 1);
    }
}
