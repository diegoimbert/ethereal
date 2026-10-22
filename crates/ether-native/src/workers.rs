//! Real-time worker pool for multicore track processing (roadmap v2, `multicore`; see
//! `ether_core::parallel` and CONTRACTS.md §11.7).
//!
//! [`WorkerPool`] implements [`ParallelExecutor`]: `workers` threads created up front
//! ([`WorkerPool::new`], non-RT), which the audio thread hands each sub-block's track job
//! graph ([`ParallelExecutor::execute_graph`]), or a flat batch of independent jobs.
//!
//! # Dispatch (RT, lock-free, no allocation)
//! - **Epochs.** `execute*` puts the work (graph + job closure, or flat batch) on its stack,
//!   stores a pointer to it, and opens a new epoch in the state word (`epoch << 1 | open`,
//!   SeqCst). Workers that see an open epoch *enter* it: bump `active`, re-check that the
//!   same epoch is still open, and only then read the work pointer. When every job is done
//!   the audio thread closes the epoch and waits until `active` is 0 before returning, so
//!   no worker ever touches the work (or the snapshot's graph) after `execute*` returned.
//!   A late worker sees the epoch closed (or another one) and backs off untouched.
//! - **Graph.** Everyone (workers and the audio thread) takes the most urgent ready job
//!   ([`JobGraph::pop`], a lock-free bitset), runs it and completes it
//!   ([`JobGraph::complete`]: per-job atomic counters of unfinished sources; the last
//!   source makes its successor ready). There is no barrier: a job starts as soon as its
//!   own sources are done. The audio thread takes pinned jobs too, and keeps working until
//!   the graph finished.
//! - **Waking (as cheap as level dispatch, usually cheaper).** Opening the graph epoch
//!   wakes at most `width - 1` parked workers (the widest dependency level; none for a
//!   graph that is a single chain, which runs on the audio thread) — once per sub-block,
//!   where level dispatch woke workers again for every level. A worker that finds nothing
//!   ready spins ([`SPIN_ITERS`]), then leaves the epoch and waits as usual (spin, then
//!   park). A completion that makes several jobs ready at once bumps a push counter (which
//!   spinning workers watch, to rejoin) and unparks up to that many minus one parked
//!   workers (one non-blocking syscall each, Dekker-style with SeqCst on both sides so no
//!   wake-up is lost). A lost race only ever costs parallelism: the audio thread never
//!   parks and finishes the graph by itself if needed.
//! - **Forks** ([`Fork::join`], `execute_graph_forking`): a running job (an audio rack's
//!   chains, `ether_core::rack_chains`) publishes its sub-jobs in a free slot (one per
//!   thread), unparks parked workers for them, claims sub-jobs itself (`fetch_add`), waits
//!   for the last one, empties the slot and waits until no helper still looks at it (a
//!   per-slot user count, SeqCst on both sides). Idle threads help before taking ready
//!   graph jobs: workers waiting in or outside an epoch (a worker about to park checks for
//!   unclaimed sub-jobs after announcing it sleeps, so no wake-up is lost; one with nothing
//!   left to claim parks as usual instead of spinning on a busy core) and the audio thread
//!   waiting for the graph. Works without an epoch too (a
//!   graph run on the audio thread alone).
//! - **Flat batches** (`execute`, `execute_pinned`): indices claimed with one `fetch_add`
//!   on the batch's counter; the audio thread runs the pinned ones, helps with the rest,
//!   then waits for the shared ones.
//! - **Panics.** Every job runs under `catch_unwind` (free when nothing panics), on the
//!   workers and on the audio thread alike, and always counts as done (a graph job still
//!   releases its successors). The first payload is kept and re-raised with
//!   `resume_unwind` on the audio thread once *every* job of the epoch finished and every
//!   worker left it, so a panic never leaves `execute*` waiting forever, never unwinds the
//!   audio thread past work other threads still use, and reaches the host callback's
//!   existing poison path (`audio.rs`: the stream goes silent).
//! - The audio thread's waits spin, then yield; see `wait_done` for the priority-inversion
//!   note when workers don't get real-time priority.
//!
//! # Threads
//! Workers enable flush-to-zero like the audio thread ([`crate::rt::enable_flush_denormals`]),
//! so a job computes the same bits on any thread. Priority, where the OS allows it:
//! - macOS: Mach `THREAD_TIME_CONSTRAINT_POLICY` (the policy Core Audio gives its I/O
//!   thread) with the engine block as period. Joining the device's `os_workgroup` would
//!   additionally keep workers on performance cores with the I/O thread on Apple Silicon;
//!   cpal doesn't expose the workgroup, so that is left for later.
//! - Linux: `SCHED_FIFO` (best effort: needs `CAP_SYS_NICE`/rtkit limits, else normal).
//! - Windows: MMCSS, the "Pro Audio" task at high priority (what DAWs give their audio
//!   threads; the service raises the thread into the real-time range and keeps it there while
//!   it stays within the system's multimedia budget), else `THREAD_PRIORITY_TIME_CRITICAL`.
//!   Without it a worker woken for a buffer waits behind every runnable normal-priority
//!   thread (the webview, plugin editors) while the audio thread spins for its job.
//! - Others: normal priority.
//!
//! The device callback's thread gets the same promotion where the backend doesn't give it one
//! ([`promote_current_thread`], `audio.rs`).
//!
//! # Size
//! [`default_workers`]: `ETHER_WORKERS` if set, else physical cores - 1 (the audio thread
//! works too) capped at [`MAX_DEFAULT_WORKERS`]. The native host also reads
//! `<data_dir>/config/engine.json` (`{"worker_threads": n}`); the env var wins.

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{JoinHandle, Thread};
use std::time::Duration;

use ether_core::parallel::{Fork, JobGraph, ParallelExecutor};

/// Upper bound of the default worker count. Heavy projects keep 15 workers + the audio
/// thread busy (independent tracks, forked rack chains); idle workers park, so a bigger pool
/// costs nothing on light projects.
pub const MAX_DEFAULT_WORKERS: usize = 15;
/// How long `WorkerPool::with_options` waits for its workers to start before panicking.
const WORKER_START_TIMEOUT: Duration = Duration::from_secs(10);

/// Spin iterations (`spin_loop` hints, ~tens of µs) before a worker parks, or before an
/// idle worker leaves a graph epoch.
pub const SPIN_ITERS: u32 = 1 << 12;
/// Spin iterations of the audio thread waiting for the last jobs before it yields.
const WAIT_SPIN_ITERS: u32 = 1 << 14;

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

/// Non-RT (once per thread, before it renders): the real-time promotion the workers get
/// (see the module docs), for another thread that renders device buffers (the device
/// callback, a benchmark's render thread).
pub fn promote_current_thread(period: Duration) {
    os::promote_current_thread(period);
}

/// The work of one `execute*` call, on the audio thread's stack.
enum Work<'a> {
    Flat {
        job: &'a (dyn Fn(usize) + Sync),
        jobs: usize,
        /// Next index to claim (starts after the pinned ones).
        next: AtomicUsize,
        /// Shared jobs finished.
        done: AtomicUsize,
    },
    Graph {
        graph: &'a JobGraph,
        job: &'a (dyn Fn(usize) + Sync),
    },
}

/// The sub-jobs of one [`Fork::join`], on the forking thread's stack.
struct ForkBatch<'a> {
    sub: &'a (dyn Fn(usize) + Sync),
    n: usize,
    /// Next index to claim.
    next: AtomicUsize,
    /// Sub-jobs finished.
    done: AtomicUsize,
}

/// Where a running `join` publishes its batch for helpers (one slot per thread that can
/// fork at once: the workers and the audio thread).
struct ForkSlot {
    batch: AtomicPtr<ForkBatch<'static>>,
    /// Helpers that may be looking at `batch` (the forker waits for 0 before returning).
    users: AtomicUsize,
}

struct Shared {
    /// Fork-join batches of running jobs (`Fork::join`), helped by idle threads.
    forks: Box<[ForkSlot]>,
    /// Batches published in `forks` (idle threads look only when non-zero).
    forks_active: AtomicUsize,
    /// `epoch << 1 | open`.
    state: AtomicU64,
    /// Bumped each time a graph job's completion made others ready (lets idle workers that
    /// left the epoch rejoin it).
    pushes: AtomicU64,
    /// The open epoch's work (valid while that epoch is open and for entered workers; see
    /// the module docs).
    work: AtomicPtr<Work<'static>>,
    /// Workers inside an epoch (entered, not yet left).
    active: AtomicUsize,
    shutdown: AtomicBool,
    /// Per worker: parked (or about to park) and wants an `unpark`.
    sleeping: Box<[AtomicBool]>,
    /// The workers' handles (set once, before the pool is handed out).
    threads: OnceLock<Box<[Thread]>>,
    /// A job panicked in the current epoch (fast check; the payload is in `panic`).
    poisoned: AtomicBool,
    /// First panic payload of the epoch, re-raised on the audio thread by `execute*` once
    /// every job finished. Only touched on the panic path (`try_lock`, never blocks).
    panic: Mutex<Option<Box<dyn Any + Send>>>,
    /// Workers that finished their start-up (thread-local setup, RT promotion) and entered
    /// the wait loop. `with_options` returns only once all did, so no thread-start
    /// allocation can happen after the pool is handed to the engine.
    started: AtomicUsize,
}

#[inline]
fn open(epoch: u64) -> u64 {
    (epoch << 1) | 1
}

#[inline]
fn is_open(state: u64) -> bool {
    state & 1 == 1
}

impl Shared {
    /// Run one job, catching a panic (so it always counts as done and the audio thread
    /// never unwinds past work other threads still use). The payload is kept for `execute*`.
    #[inline]
    fn run(&self, job: &(dyn Fn(usize) + Sync), i: usize) {
        if let Err(payload) = std::panic::catch_unwind(AssertUnwindSafe(|| job(i))) {
            self.poisoned.store(true, Ordering::Release);
            if let Ok(mut slot) = self.panic.try_lock()
                && slot.is_none()
            {
                *slot = Some(payload);
            }
        }
    }

    /// Claim and run flat jobs until none is left.
    #[inline]
    fn work_flat(
        &self,
        job: &(dyn Fn(usize) + Sync),
        jobs: usize,
        next: &AtomicUsize,
        done: &AtomicUsize,
    ) {
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= jobs {
                return;
            }
            self.run(job, i);
            done.fetch_add(1, Ordering::Release);
        }
    }

    /// Take, run and complete one ready graph job; false if none was ready.
    #[inline]
    fn graph_step(&self, graph: &JobGraph, job: &(dyn Fn(usize) + Sync), caller: bool) -> bool {
        let Some(r) = graph.pop(caller) else {
            return false;
        };
        self.run(job, graph.id(r));
        let readied = graph.complete(r);
        if readied > 0 {
            // SeqCst: pairs with an idle worker's announce + re-check (`worker_main`).
            self.pushes.fetch_add(1, Ordering::SeqCst);
            if readied > 1 {
                // This thread takes one of them; wake parked workers for the others.
                self.wake_any(readied - 1);
            }
        }
        true
    }

    /// A worker inside a graph epoch: work until the graph finished, or until nothing was
    /// ready for [`SPIN_ITERS`] (then leave; a later push brings us back). Returns the
    /// push count seen before leaving.
    fn work_graph(&self, graph: &JobGraph, job: &(dyn Fn(usize) + Sync)) -> u64 {
        let mut spins = 0u32;
        loop {
            // A running job's sub-jobs first: that job was the most urgent when it started,
            // and its successors wait for it.
            if self.help_forks() || self.graph_step(graph, job, false) {
                spins = 0;
                continue;
            }
            if graph.finished() {
                return self.pushes.load(Ordering::SeqCst);
            }
            if spins < SPIN_ITERS {
                spins += 1;
                std::hint::spin_loop();
                continue;
            }
            // Read the push count *before* the last look: a push after it wakes us again.
            let seen = self.pushes.load(Ordering::SeqCst);
            if graph.has_ready(false) {
                spins = 0;
                continue;
            }
            return seen;
        }
    }

    /// Help with the published fork batches, if any; whether this thread ran a sub-job.
    #[inline]
    fn help_forks(&self) -> bool {
        if self.forks_active.load(Ordering::Acquire) == 0 {
            return false;
        }
        let mut ran = false;
        for slot in self.forks.iter() {
            if slot.batch.load(Ordering::Acquire).is_null() {
                continue;
            }
            // Counted first, then re-read (pairs with the forker's clear + `users` check:
            // either it sees us, or we see the slot empty).
            slot.users.fetch_add(1, Ordering::SeqCst);
            let b = slot.batch.load(Ordering::SeqCst);
            if !b.is_null() {
                // SAFETY: we are counted in `users`, so the batch's `join` hasn't returned.
                ran |= self.run_fork(unsafe { &*b });
            }
            slot.users.fetch_sub(1, Ordering::Release);
        }
        ran
    }

    /// Whether a published fork batch still has unclaimed sub-jobs (a worker about to park
    /// must not sleep through them).
    fn fork_claimable(&self) -> bool {
        self.forks_active.load(Ordering::SeqCst) > 0
            && self.forks.iter().any(|slot| {
                slot.users.fetch_add(1, Ordering::SeqCst);
                let b = slot.batch.load(Ordering::SeqCst);
                // SAFETY: counted in `users`, so a non-null batch is alive (see `help_forks`).
                let open = !b.is_null() && unsafe { (*b).next.load(Ordering::Relaxed) < (*b).n };
                slot.users.fetch_sub(1, Ordering::Release);
                open
            })
    }

    /// Claim and run sub-jobs of `b` until none is left; whether any ran here.
    #[inline]
    fn run_fork(&self, b: &ForkBatch<'_>) -> bool {
        let mut ran = false;
        loop {
            // Cheap check first: a finished batch costs no write.
            if b.next.load(Ordering::Relaxed) >= b.n {
                return ran;
            }
            let i = b.next.fetch_add(1, Ordering::Relaxed);
            if i >= b.n {
                return ran;
            }
            self.run(b.sub, i);
            b.done.fetch_add(1, Ordering::Release);
            ran = true;
        }
    }

    /// Unpark up to `n` parked workers (any of them).
    fn wake_any(&self, mut n: usize) {
        let Some(threads) = self.threads.get() else {
            return;
        };
        for (flag, t) in self.sleeping.iter().zip(threads.iter()) {
            if n == 0 {
                return;
            }
            if flag.load(Ordering::SeqCst) {
                t.unpark();
                n -= 1;
            }
        }
    }
}

/// Fixed pool of real-time worker threads (see the module docs).
pub struct WorkerPool {
    shared: Arc<Shared>,
    handles: Vec<JoinHandle<()>>,
    epoch: u64,
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
            forks: (0..=workers)
                .map(|_| ForkSlot {
                    batch: AtomicPtr::new(std::ptr::null_mut()),
                    users: AtomicUsize::new(0),
                })
                .collect(),
            forks_active: AtomicUsize::new(0),
            state: AtomicU64::new(0),
            pushes: AtomicU64::new(0),
            work: AtomicPtr::new(std::ptr::null_mut()),
            active: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            sleeping: (0..workers).map(|_| AtomicBool::new(false)).collect(),
            threads: OnceLock::new(),
            poisoned: AtomicBool::new(false),
            panic: Mutex::new(None),
            started: AtomicUsize::new(0),
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
        let _ = shared
            .threads
            .set(handles.iter().map(|h| h.thread().clone()).collect());
        // Wait until every spawned worker is in its loop (non-RT: this is construction).
        // Bounded: a worker that died before signalling (panic in RT setup) is a bug, not
        // something to spin on forever.
        let deadline = std::time::Instant::now() + WORKER_START_TIMEOUT;
        while shared.started.load(Ordering::Acquire) < handles.len() {
            if let Some(h) = handles.iter().find(|h| h.is_finished()) {
                panic!(
                    "audio worker {:?} exited before starting",
                    h.thread().name().unwrap_or("?")
                );
            }
            assert!(
                std::time::Instant::now() < deadline,
                "audio workers did not start within {WORKER_START_TIMEOUT:?} ({} of {})",
                shared.started.load(Ordering::Acquire),
                handles.len()
            );
            std::thread::yield_now();
        }
        Self {
            shared,
            handles,
            epoch: 0,
        }
    }

    /// Wake the first `want` workers if parked: the spinning ones pick the epoch up by
    /// themselves.
    fn wake(&self, want: usize) {
        for (flag, h) in self.shared.sleeping.iter().zip(&self.handles).take(want) {
            if flag.load(Ordering::SeqCst) {
                h.thread().unpark();
            }
        }
    }

    /// Open a new epoch for `work` (on the caller's stack until `close`).
    fn open(&mut self, work: &Work<'_>) {
        self.epoch = self.epoch.wrapping_add(1) & (u64::MAX >> 1);
        // The pointer is only dereferenced by workers that entered this epoch, and `close`
        // waits for all of them to leave before `work` goes away; erase the lifetime.
        self.shared.work.store(
            (work as *const Work<'_>).cast_mut().cast::<Work<'static>>(),
            Ordering::Release,
        );
        self.shared.state.store(open(self.epoch), Ordering::SeqCst);
    }

    /// Close the epoch and wait until no worker is inside it; then re-raise a job's panic
    /// here, on the audio thread (the host's callback turns it into a silent stream).
    fn close(&self) {
        let s = &*self.shared;
        s.state.store(self.epoch << 1, Ordering::SeqCst);
        // Entered workers leave right away: their work is finished (bounded by their
        // scheduling latency, like `wait_done`).
        wait_until(|| s.active.load(Ordering::SeqCst) == 0);
        if s.poisoned.swap(false, Ordering::AcqRel) {
            let payload = s.panic.try_lock().ok().and_then(|mut p| p.take());
            std::panic::resume_unwind(payload.unwrap_or_else(|| Box::new("audio job panicked")));
        }
    }
}

/// The audio thread's bounded waits: spin first (jobs are short), then yield so a
/// preempted worker can finish on a busy core. If the audio thread is real-time and a
/// worker is not (e.g. SCHED_FIFO refused on Linux), this wait is a priority inversion
/// bounded by that worker's scheduling latency.
#[inline]
fn wait_until(mut done: impl FnMut() -> bool) {
    let mut spins = 0u32;
    while !done() {
        if spins < WAIT_SPIN_ITERS {
            spins += 1;
            std::hint::spin_loop();
        } else {
            std::thread::yield_now();
        }
    }
}

fn worker_main(s: &Shared, index: usize, options: PoolOptions) {
    if options.flush_denormals {
        crate::rt::enable_flush_denormals();
    }
    if options.realtime {
        os::promote_current_thread(options.period);
        // Nodes here wait on the device callback's shared deadline.
        ether_core::period::mark_device_thread();
    }
    let mut seen_epoch = s.state.load(Ordering::Acquire) >> 1;
    let mut seen_push = s.pushes.load(Ordering::Acquire);
    let sleeping = &s.sleeping[index];
    // Work to do: an open epoch we haven't entered, or new ready jobs in the one we left.
    let wanted = |st: u64, pushes: u64, seen_epoch: u64, seen_push: u64| {
        is_open(st) && (st >> 1 != seen_epoch || pushes != seen_push)
    };
    s.started.fetch_add(1, Ordering::Release);
    loop {
        // Wait: spin, then park.
        let mut spins = 0u32;
        let st = loop {
            if s.shutdown.load(Ordering::Acquire) {
                return;
            }
            let st = s.state.load(Ordering::Acquire);
            if wanted(st, s.pushes.load(Ordering::Acquire), seen_epoch, seen_push) {
                break st;
            }
            if s.help_forks() {
                spins = 0;
                continue;
            }
            if spins < SPIN_ITERS {
                spins += 1;
                std::hint::spin_loop();
                continue;
            }
            sleeping.store(true, Ordering::SeqCst);
            // Re-check after announcing (pairs with the SeqCst publish / push + flag load
            // on the waking side): either we see the work or the waker sees our flag.
            let st = s.state.load(Ordering::SeqCst);
            let pushes = s.pushes.load(Ordering::SeqCst);
            if !wanted(st, pushes, seen_epoch, seen_push)
                && !s.fork_claimable()
                && !s.shutdown.load(Ordering::SeqCst)
            {
                std::thread::park();
            }
            sleeping.store(false, Ordering::SeqCst);
            spins = 0;
        };
        seen_epoch = st >> 1;
        seen_push = s.pushes.load(Ordering::SeqCst);
        // Enter: counted first, then the epoch must still be the one we saw (pairs with
        // `close`: either it sees us in `active`, or we see the epoch closed).
        s.active.fetch_add(1, Ordering::SeqCst);
        if s.state.load(Ordering::SeqCst) == st {
            // SAFETY: we are counted in `active` and the epoch is still open, so `close`
            // hasn't finished: `work` points to that epoch's live `Work` (stored before the
            // epoch opened, which our SeqCst load observed).
            let work = unsafe { &*s.work.load(Ordering::Acquire) };
            match work {
                Work::Flat {
                    job,
                    jobs,
                    next,
                    done,
                } => s.work_flat(*job, *jobs, next, done),
                Work::Graph { graph, job } => seen_push = s.work_graph(graph, *job),
            }
        }
        s.active.fetch_sub(1, Ordering::Release);
    }
}

impl ParallelExecutor for WorkerPool {
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        self.execute_pinned(jobs, 0, job);
    }

    fn execute_pinned(&mut self, jobs: usize, pinned: usize, job: &(dyn Fn(usize) + Sync)) {
        let pinned = pinned.min(jobs);
        let shared_jobs = jobs - pinned;
        if self.handles.is_empty() || shared_jobs <= 1 {
            for i in 0..jobs {
                job(i);
            }
            return;
        }
        let work = Work::Flat {
            job,
            jobs,
            next: AtomicUsize::new(pinned),
            done: AtomicUsize::new(0),
        };
        self.open(&work);
        // The audio thread takes one shared job itself (or the pinned ones).
        self.wake(shared_jobs.saturating_sub(if pinned == 0 { 1 } else { 0 }));
        let s = &*self.shared;
        for i in 0..pinned {
            s.run(job, i);
        }
        let Work::Flat { next, done, .. } = &work else {
            unreachable!()
        };
        s.work_flat(job, jobs, next, done);
        // Everything is claimed; wait for the workers still running theirs.
        wait_until(|| done.load(Ordering::Acquire) >= shared_jobs);
        self.close();
    }

    fn execute_graph(&mut self, graph: &JobGraph, job: &(dyn Fn(usize) + Sync)) {
        // A chain (or a single job) has nothing to overlap: no wake-up at all.
        if self.handles.is_empty() || graph.len() <= 1 || graph.width() <= 1 {
            for id in graph.order() {
                job(id);
            }
            return;
        }
        graph.reset();
        let work = Work::Graph { graph, job };
        self.open(&work);
        self.wake((graph.width() - 1).min(self.handles.len()));
        let s = &*self.shared;
        // Work until the graph finished (pinned jobs are ours alone).
        let mut spins = 0u32;
        while !graph.finished() {
            if s.help_forks() || s.graph_step(graph, job, true) {
                spins = 0;
            } else if spins < WAIT_SPIN_ITERS {
                spins += 1;
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
        self.close();
    }

    fn execute_graph_forking(&mut self, graph: &JobGraph, job: &(dyn Fn(usize, &dyn Fork) + Sync)) {
        if self.handles.is_empty() {
            self.execute_graph(graph, &|id| job(id, &ether_core::parallel::SerialFork));
            return;
        }
        let shared = self.shared.clone();
        let fork = PoolFork { s: &shared };
        self.execute_graph(graph, &|id| job(id, &fork));
        // A sub-job's panic caught outside an epoch (a graph run here, without one).
        if shared.poisoned.swap(false, Ordering::AcqRel) {
            let payload = shared.panic.try_lock().ok().and_then(|mut p| p.take());
            std::panic::resume_unwind(payload.unwrap_or_else(|| Box::new("audio job panicked")));
        }
    }

    fn workers(&self) -> usize {
        self.handles.len()
    }
}

/// [`Fork`] over the pool: the forking thread publishes its batch in a free slot, wakes
/// parked workers for the other sub-jobs, runs sub-jobs itself, and returns once all
/// finished and no helper looks at the batch any more. Idle threads (workers waiting for
/// work or for ready graph jobs, the audio thread waiting for the graph) help.
struct PoolFork<'a> {
    s: &'a Shared,
}

impl Fork for PoolFork<'_> {
    fn join(&self, n: usize, sub: &(dyn Fn(usize) + Sync)) {
        let s = self.s;
        let serial = || {
            for i in 0..n {
                sub(i);
            }
        };
        if n <= 1 {
            return serial();
        }
        let batch = ForkBatch {
            sub,
            n,
            next: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
        };
        // The pointer is only dereferenced by helpers counted in the slot's `users`, and
        // this function waits for them before `batch` goes away; erase the lifetime.
        let ptr = (&batch as *const ForkBatch<'_>)
            .cast_mut()
            .cast::<ForkBatch<'static>>();
        let Some(slot) = s.forks.iter().find(|slot| {
            slot.batch
                .compare_exchange(
                    std::ptr::null_mut(),
                    ptr,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
        }) else {
            // More concurrent forks than threads: can't happen (one per running job).
            return serial();
        };
        // SeqCst: pairs with a parking worker's announce + re-check (`worker_main`).
        s.forks_active.fetch_add(1, Ordering::SeqCst);
        s.wake_any(n - 1);
        s.run_fork(&batch);
        wait_until(|| batch.done.load(Ordering::Acquire) >= n);
        slot.batch.store(std::ptr::null_mut(), Ordering::SeqCst);
        s.forks_active.fetch_sub(1, Ordering::SeqCst);
        wait_until(|| slot.users.load(Ordering::SeqCst) == 0);
    }

    fn parallel(&self) -> bool {
        true
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
        fn pthread_setschedparam(
            thread: std::ffi::c_ulong,
            policy: i32,
            param: *const SchedParam,
        ) -> i32;
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

#[cfg(windows)]
mod os {
    use std::cell::OnceCell;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Threading::{
        AVRT_PRIORITY_HIGH, AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
        AvSetMmThreadPriority, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    };

    /// `s` as a UTF-16 string (ASCII only).
    const fn wide<const N: usize>(s: &[u8; N]) -> [u16; N] {
        let mut out = [0u16; N];
        let mut i = 0;
        while i < N {
            out[i] = s[i] as u16;
            i += 1;
        }
        out
    }

    /// The MMCSS task of DAW audio threads (`HKLM\...\Multimedia\SystemProfile\Tasks`).
    const PRO_AUDIO: [u16; 10] = wide(b"Pro Audio\0");

    /// This thread's MMCSS registration, reverted when the thread exits.
    struct Task(HANDLE);

    impl Drop for Task {
        fn drop(&mut self) {
            // SAFETY: a handle returned by `AvSetMmThreadCharacteristicsW` on this thread.
            unsafe { AvRevertMmThreadCharacteristics(self.0) };
        }
    }

    thread_local! {
        static TASK: OnceCell<Task> = const { OnceCell::new() };
    }

    /// Join the "Pro Audio" MMCSS task at high priority (once per thread); if MMCSS refuses
    /// (service disabled), `THREAD_PRIORITY_TIME_CRITICAL`.
    pub(super) fn promote_current_thread(_period: Duration) {
        TASK.with(|task| {
            if task.get().is_some() {
                return;
            }
            let mut index = 0u32;
            // SAFETY: NUL-terminated UTF-16 task name; `index` is a valid out parameter.
            let h = unsafe { AvSetMmThreadCharacteristicsW(PRO_AUDIO.as_ptr(), &mut index) };
            if h.is_null() {
                tracing::debug!("audio thread: MMCSS refused, time-critical priority instead");
                // SAFETY: the pseudo-handle of this thread.
                unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
                return;
            }
            // SAFETY: `h` is this thread's live MMCSS handle.
            if unsafe { AvSetMmThreadPriority(h, AVRT_PRIORITY_HIGH) } == 0 {
                tracing::debug!("audio thread: MMCSS priority refused");
            }
            let _ = task.set(Task(h));
        });
    }

    /// Processor cores (`RelationProcessorCore` entries; the current processor group, up
    /// to 64 logical CPUs). Without this, SMT doubled the count.
    pub(super) fn physical_cores() -> Option<usize> {
        use windows_sys::Win32::System::SystemInformation::{
            GetLogicalProcessorInformation, RelationProcessorCore,
            SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
        };
        let size = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
        let mut len = 0u32;
        // SAFETY: a size query (null buffer, `len` out parameter); it fails with
        // ERROR_INSUFFICIENT_BUFFER and sets `len`.
        unsafe { GetLogicalProcessorInformation(std::ptr::null_mut(), &mut len) };
        if len == 0 {
            return None;
        }
        let mut buf =
            vec![SYSTEM_LOGICAL_PROCESSOR_INFORMATION::default(); (len as usize).div_ceil(size)];
        let mut len = (buf.len() * size) as u32;
        // SAFETY: `buf` holds `len` bytes of entries.
        if unsafe { GetLogicalProcessorInformation(buf.as_mut_ptr(), &mut len) } == 0 {
            return None;
        }
        let n = buf[..len as usize / size]
            .iter()
            .filter(|e| e.Relationship == RelationProcessorCore)
            .count();
        (n > 0).then_some(n)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
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

    fn quiet_pool(workers: usize) -> WorkerPool {
        WorkerPool::with_options(
            workers,
            PoolOptions {
                realtime: false,
                ..Default::default()
            },
        )
    }

    /// A panicking job on a worker: every other job still runs, `execute` returns by
    /// re-raising the panic on the caller, and the pool keeps working afterwards.
    #[test]
    fn panic_on_a_worker_is_reraised_on_the_caller() {
        let mut pool = quiet_pool(3);
        let me = std::thread::current().id();
        let ran = AtomicU32::new(0);
        let hit_worker = AtomicU32::new(0);
        for _ in 0..20 {
            ran.store(0, Ordering::Relaxed);
            // Per iteration: an earlier iteration's worker panic says nothing about this one.
            hit_worker.store(0, Ordering::Relaxed);
            let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
                pool.execute(16, &|i| {
                    ran.fetch_add(1, Ordering::Relaxed);
                    // Panic on whichever worker gets a job (the caller never panics here).
                    if i > 0 && std::thread::current().id() != me {
                        hit_worker.fetch_add(1, Ordering::Relaxed);
                        panic!("job {i} failed");
                    }
                })
            }));
            assert_eq!(ran.load(Ordering::Relaxed), 16);
            if hit_worker.load(Ordering::Relaxed) > 0 {
                assert!(r.is_err());
            }
        }
        let n = AtomicU32::new(0);
        pool.execute(8, &|_| {
            n.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(n.load(Ordering::Relaxed), 8);
    }

    /// A panic on the caller (a pinned job) waits for the workers' jobs before unwinding.
    #[test]
    fn panic_on_the_caller_waits_for_the_workers() {
        let mut pool = quiet_pool(2);
        let finished = AtomicU32::new(0);
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
            pool.execute_pinned(6, 1, &|i| {
                if i == 0 {
                    panic!("pinned job failed");
                }
                std::thread::sleep(Duration::from_millis(5));
                finished.fetch_add(1, Ordering::Relaxed);
            })
        }));
        assert!(r.is_err());
        // Nothing still runs on the (now gone) closure: all shared jobs completed first.
        assert_eq!(finished.load(Ordering::Relaxed), 5);
        let n = AtomicU32::new(0);
        pool.execute(4, &|_| {
            n.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(n.load(Ordering::Relaxed), 4);
    }

    /// Deterministic random DAG over `n` jobs (edges only forward in position order).
    fn random_graph(seed: u64, n: usize, pinned_every: usize) -> (JobGraph, Vec<Vec<usize>>) {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = move || {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        let succ: Vec<Vec<usize>> = (0..n)
            .map(|a| (a + 1..n).filter(|_| next() % 5 == 0).collect())
            .collect();
        let mut sources = vec![Vec::new(); n];
        for (a, s) in succ.iter().enumerate() {
            for &b in s {
                sources[b].push(a);
            }
        }
        let cost: Vec<u64> = (0..n).map(|_| 1 + next() % 8).collect();
        let pinned: Vec<bool> = (0..n)
            .map(|i| pinned_every > 0 && i % pinned_every == 1)
            .collect();
        let ids: Vec<usize> = (0..n).collect();
        (JobGraph::new(&ids, &cost, &succ, &pinned), sources)
    }

    /// Every job exactly once, only after all its sources, pinned ones on the caller,
    /// whatever the worker count; repeated runs of one graph.
    #[test]
    fn graph_runs_every_job_once_after_its_sources() {
        let me = std::thread::current().id();
        for workers in [0, 1, 2, 4, 8] {
            let mut pool = quiet_pool(workers);
            for seed in 0..12u64 {
                let n = [1, 2, 7, 40, 70, 130][seed as usize % 6];
                let (graph, sources) = random_graph(seed, n, if seed % 3 == 0 { 5 } else { 0 });
                let pinned: Vec<bool> = {
                    let mut p = vec![false; n];
                    for r in 0..n {
                        p[graph.id(r)] = graph.is_pinned(r);
                    }
                    p
                };
                for _ in 0..20 {
                    let done: Vec<AtomicBool> = (0..n).map(|_| AtomicBool::new(false)).collect();
                    let hits: Vec<AtomicU32> = (0..n).map(|_| AtomicU32::new(0)).collect();
                    let wrong = AtomicU32::new(0);
                    pool.execute_graph(&graph, &|id| {
                        hits[id].fetch_add(1, Ordering::Relaxed);
                        if sources[id]
                            .iter()
                            .any(|&s| !done[s].load(Ordering::Relaxed))
                        {
                            wrong.fetch_add(1, Ordering::Relaxed);
                        }
                        if pinned[id] && std::thread::current().id() != me {
                            wrong.fetch_add(1, Ordering::Relaxed);
                        }
                        // Relaxed on purpose: the graph's own ordering must make it visible.
                        done[id].store(true, Ordering::Relaxed);
                    });
                    let ctx = format!("{workers} workers, seed {seed}");
                    assert!(hits.iter().all(|h| h.load(Ordering::Relaxed) == 1), "{ctx}");
                    assert_eq!(wrong.load(Ordering::Relaxed), 0, "{ctx}");
                }
            }
        }
    }

    /// Wide graphs really spread over the workers.
    #[test]
    fn graph_jobs_run_on_workers() {
        let mut pool = quiet_pool(3);
        let me = std::thread::current().id();
        let n = 32;
        let succ: Vec<Vec<usize>> = (0..n)
            .map(|i| if i + 1 < n { vec![n - 1] } else { vec![] })
            .collect();
        let ids: Vec<usize> = (0..n).collect();
        let graph = JobGraph::new(&ids, &vec![1; n], &succ, &vec![false; n]);
        let elsewhere = AtomicU32::new(0);
        for _ in 0..20 {
            pool.execute_graph(&graph, &|_| {
                std::thread::sleep(Duration::from_micros(200));
                if std::thread::current().id() != me {
                    elsewhere.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        assert!(elsewhere.load(Ordering::Relaxed) > 0);
    }

    /// A panicking graph job: its successors still run, the panic is re-raised on the
    /// caller after everything finished, and the pool keeps working.
    #[test]
    fn graph_panic_is_reraised_after_every_job() {
        let mut pool = quiet_pool(3);
        let (graph, _) = random_graph(7, 40, 0);
        let ran = AtomicU32::new(0);
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
            pool.execute_graph(&graph, &|id| {
                ran.fetch_add(1, Ordering::Relaxed);
                if id == 3 {
                    panic!("job {id} failed");
                }
            })
        }));
        assert!(r.is_err());
        assert_eq!(ran.load(Ordering::Relaxed), 40);
        let n = AtomicU32::new(0);
        pool.execute_graph(&graph, &|_| {
            n.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(n.load(Ordering::Relaxed), 40);
    }

    /// Jobs that fork (several at once, from workers and the audio thread): every sub-job
    /// runs exactly once, before its job returns, and the pool keeps working after.
    #[test]
    fn forked_sub_jobs_run_once_before_their_job_returns() {
        use ether_core::parallel::JobGraph;
        let mut pool = quiet_pool(3);
        // Four independent jobs into a fifth.
        let graph = JobGraph::new(
            &[0, 1, 2, 3, 4],
            &[1, 1, 1, 1, 1],
            &[vec![4], vec![4], vec![4], vec![4], vec![]],
            &[false; 5],
        );
        let subs = 13;
        let hits: Vec<AtomicU32> = (0..5 * subs).map(|_| AtomicU32::new(0)).collect();
        let late = AtomicU32::new(0);
        for _ in 0..200 {
            hits.iter().for_each(|h| h.store(0, Ordering::Relaxed));
            pool.execute_graph_forking(&graph, &|id, fork| {
                assert!(fork.parallel());
                fork.join(subs, &|i| {
                    hits[id * subs + i].fetch_add(1, Ordering::Relaxed);
                });
                if (0..subs).any(|i| hits[id * subs + i].load(Ordering::Relaxed) != 1) {
                    late.fetch_add(1, Ordering::Relaxed);
                }
            });
            assert!(hits.iter().all(|h| h.load(Ordering::Relaxed) == 1));
        }
        assert_eq!(late.load(Ordering::Relaxed), 0);
        // A single-job graph (no epoch) forks too.
        let one = JobGraph::new(&[0], &[1], &[vec![]], &[false]);
        let n = AtomicU32::new(0);
        pool.execute_graph_forking(&one, &|_, fork| {
            fork.join(9, &|_| {
                n.fetch_add(1, Ordering::Relaxed);
            });
        });
        assert_eq!(n.load(Ordering::Relaxed), 9);
    }

    #[test]
    fn default_workers_is_bounded() {
        let n = physical_cores().saturating_sub(1).min(MAX_DEFAULT_WORKERS);
        assert!(n <= MAX_DEFAULT_WORKERS);
        assert!(physical_cores() >= 1);
    }
}
