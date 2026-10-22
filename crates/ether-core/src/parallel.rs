//! Multicore processing contract (roadmap v2; see CONTRACTS.md §11.7).
//!
//! `ether-core` never creates threads. A host that wants parallel processing gives the
//! engine a [`ParallelExecutor`] backed by its own real-time worker threads
//! (`EngineConfig::worker_threads` of them; `ether-native/src/workers.rs`). Partition
//! contract:
//!
//! - **Dependency-driven scheduling.** The compiled snapshot holds a [`JobGraph`]: one job
//!   per track, an edge from every source to the track it feeds (outputs, sends, input
//!   taps, sidechains of the device chain and of modulators). Drum-rack pad chains and rack
//!   chains belong to their track (they run inside its job). Per sub-block the engine hands
//!   the whole graph to [`ParallelExecutor::execute_graph`] once: a job starts as soon as
//!   **its own** sources finished (per-job atomic counters of unfinished sources), not when
//!   a whole level did. So a sidechain from a light track onto the heaviest track delays
//!   that track by the light track only, and the master starts the moment its last input
//!   is done.
//! - **Critical path first.** When more jobs are ready than threads, the one with the
//!   longest remaining path to the end of the graph runs first. Each job has a static cost
//!   estimate (1 + the devices in its chains, rack/pad chains included); its priority is
//!   that cost plus the highest priority of the jobs it feeds. Jobs are numbered by
//!   priority (rank 0 = longest remaining path), and the ready queue is a bitset of ranks,
//!   so taking the most urgent ready job is "lowest set bit" (one `fetch_and` per take,
//!   lock-free, nothing allocated). Rank order is also a topological order (a job's
//!   priority is strictly higher than any job it feeds), which is the sequential order.
//! - **Mixing in a fixed order.** A track's input bus is not written by its sources: the
//!   track's own job *gathers* it at its start, summing its sources' output/send buffers
//!   (all finished, since the job only becomes ready after them) in a list fixed at
//!   compile time: the order in which the sequential engine used to add them (topological
//!   order, then pre-fader sends, post-fader sends, output). The hardware output is summed
//!   on the audio thread after the graph, in the same order. So the floating-point
//!   summation order never depends on scheduling or on the worker count: the output is
//!   bit-identical to sequential processing (worker count 0) and to the pre-multicore
//!   engine.
//! - Each job touches only its own track state and nodes (a node belongs to exactly one
//!   chain or pad chain, checked by the compiler), and reads finished source tracks, so
//!   jobs need no locks. Shared engine state (transport, param queue, rings, hardware
//!   outputs, metronome, preview, recording) is only touched on the audio thread before or
//!   after the graph.
//! - Stopped, devices and plugins still run (tails, auditioned instruments, plugins that
//!   never sleep), so the engine dispatches like while playing. Only when
//!   the previous sub-block did no real work (every node asleep, see silence skipping in
//!   `engine.rs`, and nothing monitored) does it run the jobs itself in rank order (same
//!   bits) and leave the workers parked, so an idle engine costs no worker wake-ups.
//! - **Pinned jobs.** A job flagged pinned in the graph (or the first `pinned` jobs of a
//!   flat [`ParallelExecutor::execute_pinned`] batch) must run on the calling (audio)
//!   thread. None is today: tracks with Complex-warped clips used to (their stretchers
//!   lived in one engine-wide table); each track now owns its stretcher voices
//!   (`crate::warp`) and runs on the workers like the others. Plugins are *not* pinned:
//!   CLAP (`process` is an `[audio-thread]` call, and the spec lets a host run it on any
//!   of its audio/worker threads, never concurrently for one instance), VST3
//!   (`IAudioProcessor::process` from one processing thread at a time) and AU
//!   (`AudioUnitRender` / the AUv3 render block, any thread, not re-entrant) all accept a
//!   worker thread; our hosts keep no thread-local audio state, and a sandboxed plugin's
//!   shared-memory + named-semaphore protocol belongs to its node, so posting/spinning from
//!   a worker is the same as from the audio thread.
//! - The executor must be RT-safe: no allocation, no blocking syscalls on the audio thread;
//!   workers spin or park on futexes/semaphores that the audio thread signals; `execute*`
//!   returns only when every job is done. With 0 workers the audio thread runs every job
//!   itself. Everything a [`JobGraph`] needs at run time (counters, ready bitset) is
//!   allocated when the snapshot compiles, off the audio thread.
//! - **Unsafe sharing (engine side).** `job` is a shared `Fn + Sync`, but each job needs
//!   `&mut` access to its own track (buffers, nodes). The engine hands out disjoint mutable
//!   access through raw pointers (job id = track index, never two jobs on one track or one
//!   node), which is sound only if the executor upholds its contract: every job exactly
//!   once, a job only after every one of its sources finished (with release/acquire
//!   ordering between them: [`JobGraph::complete`] / [`JobGraph::pop`]), all jobs finished
//!   before `execute_graph` returns, and no job running after that. `Node: Send` is what
//!   allows a node to be processed on a worker thread. Every `unsafe` block in `engine.rs`
//!   refers to this.
//! - **wasm32:** `EngineConfig::worker_threads` is ignored and the engine always runs
//!   sequentially (the AudioWorklet has no threads; SharedArrayBuffer workers are out of
//!   scope): `Engine::set_executor` is a no-op there, and the graph runs in rank order
//!   through [`SequentialExecutor`].

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Runs jobs, possibly in parallel, and returns when all are finished.
pub trait ParallelExecutor: Send {
    /// RT. Call `job(i)` exactly once for every `i` in `0..jobs` (any thread, any order).
    /// Every job's writes are visible to the caller when this returns.
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync));

    /// RT. Like [`ParallelExecutor::execute`], but jobs `0..pinned` must run on the calling
    /// thread (state that only the audio thread may touch). The default runs them first,
    /// then the others through `execute`; pools override it to overlap both.
    fn execute_pinned(&mut self, jobs: usize, pinned: usize, job: &(dyn Fn(usize) + Sync)) {
        let pinned = pinned.min(jobs);
        for i in 0..pinned {
            job(i);
        }
        if jobs > pinned {
            self.execute(jobs - pinned, &|i| job(i + pinned));
        }
    }

    /// RT. Call `job(id)` exactly once for every job of `graph`, each only after all its
    /// sources' jobs finished (and their writes are visible), pinned jobs on the calling
    /// thread; return when all finished. Pools drive the graph with [`JobGraph::reset`],
    /// [`JobGraph::pop`] and [`JobGraph::complete`]. The default runs the jobs here, in
    /// rank order (a topological order).
    fn execute_graph(&mut self, graph: &JobGraph, job: &(dyn Fn(usize) + Sync)) {
        for id in graph.order() {
            job(id);
        }
    }

    /// Worker threads available (0 = sequential).
    fn workers(&self) -> usize;
}

/// Runs every job on the calling thread (the default; v0.1 behaviour).
#[derive(Debug, Default, Clone, Copy)]
pub struct SequentialExecutor;

impl ParallelExecutor for SequentialExecutor {
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        for i in 0..jobs {
            job(i);
        }
    }

    fn execute_pinned(&mut self, jobs: usize, _pinned: usize, job: &(dyn Fn(usize) + Sync)) {
        self.execute(jobs, job);
    }

    fn workers(&self) -> usize {
        0
    }
}

const WORD: usize = 64;

/// A DAG of jobs scheduled by dependency, most urgent first (see the module docs).
///
/// Built off the audio thread ([`JobGraph::new`]); then, per run, `reset` on the calling
/// thread, `pop`/`complete` from any thread until [`JobGraph::finished`]. Jobs are
/// addressed by **rank** (0 = highest priority) in `pop`/`complete`, and carry the caller's
/// **id** ([`JobGraph::id`], a track index for the engine).
#[derive(Debug)]
pub struct JobGraph {
    /// Rank → id.
    ids: Box<[u32]>,
    /// Rank → priority (cost of the longest path from this job to the end of the graph).
    priority: Box<[u64]>,
    /// Successor ranks of rank `r`: `succ[succ_start[r]..succ_start[r + 1]]`.
    succ_start: Box<[u32]>,
    succ: Box<[u32]>,
    /// Rank → number of distinct sources.
    deps: Box<[u32]>,
    /// Ready-at-start bitsets (sources-free jobs), shared and pinned.
    roots: Box<[u64]>,
    roots_pinned: Box<[u64]>,
    /// Pinned ranks (bitset).
    pinned: Box<[u64]>,
    /// Upper estimate of useful parallelism: the widest dependency level.
    width: usize,
    // --- run-time state (atomics, reset per run) ---
    pending: Box<[AtomicU32]>,
    ready: Box<[AtomicU64]>,
    ready_pinned: Box<[AtomicU64]>,
    done: AtomicUsize,
}

impl JobGraph {
    /// Non-RT. Jobs `0..ids.len()` (positions) with caller ids `ids`, static costs `cost`
    /// (≥ 1 counts; 0 is taken as 1), edges `succ[p]` (positions this job feeds; duplicates
    /// and self-edges ignored) and the `pinned` flag.
    ///
    /// # Panics
    /// On a cycle, or mismatched lengths (the compiler rejects cycles first).
    pub fn new(ids: &[usize], cost: &[u64], succ: &[Vec<usize>], pinned: &[bool]) -> Self {
        let n = ids.len();
        assert!(cost.len() == n && succ.len() == n && pinned.len() == n);
        let succ: Vec<Vec<usize>> = succ
            .iter()
            .enumerate()
            .map(|(p, s)| {
                let mut s: Vec<usize> = s.iter().copied().filter(|&d| d != p).collect();
                s.sort_unstable();
                s.dedup();
                s
            })
            .collect();
        // Topological order (Kahn, by position for determinism).
        let mut indeg = vec![0u32; n];
        for s in &succ {
            for &d in s {
                indeg[d] += 1;
            }
        }
        let deps_by_pos = indeg.clone();
        let mut stack: Vec<usize> = (0..n).rev().filter(|&p| indeg[p] == 0).collect();
        let mut topo = Vec::with_capacity(n);
        while let Some(p) = stack.pop() {
            topo.push(p);
            for &d in succ[p].iter().rev() {
                indeg[d] -= 1;
                if indeg[d] == 0 {
                    stack.push(d);
                }
            }
        }
        assert_eq!(topo.len(), n, "JobGraph: cycle");
        // Priority = own cost + max priority of the jobs it feeds (reverse topological).
        let mut prio = vec![0u64; n];
        for &p in topo.iter().rev() {
            let tail = succ[p].iter().map(|&d| prio[d]).max().unwrap_or(0);
            prio[p] = cost[p].max(1) + tail;
        }
        // Width: the widest dependency level (level = 1 + max level of the sources).
        let mut level = vec![0usize; n];
        for &p in &topo {
            for &d in &succ[p] {
                level[d] = level[d].max(level[p] + 1);
            }
        }
        let mut per_level = vec![0usize; n.max(1)];
        for &l in &level {
            per_level[l] += 1;
        }
        let width = per_level.iter().copied().max().unwrap_or(0);
        // Ranks: priority descending, then position (stable, deterministic).
        let mut by_rank: Vec<usize> = (0..n).collect();
        by_rank.sort_by_key(|&p| (std::cmp::Reverse(prio[p]), p));
        let mut rank_of = vec![0usize; n];
        for (r, &p) in by_rank.iter().enumerate() {
            rank_of[p] = r;
        }
        let words = n.div_ceil(WORD);
        let mut roots = vec![0u64; words];
        let mut roots_pinned = vec![0u64; words];
        let mut pinned_bits = vec![0u64; words];
        let mut succ_start = Vec::with_capacity(n + 1);
        let mut flat = Vec::new();
        let mut deps = Vec::with_capacity(n);
        for (r, &p) in by_rank.iter().enumerate() {
            succ_start.push(flat.len() as u32);
            let mut s: Vec<u32> = succ[p].iter().map(|&d| rank_of[d] as u32).collect();
            s.sort_unstable();
            flat.extend(s);
            deps.push(deps_by_pos[p]);
            let bit = 1u64 << (r % WORD);
            if pinned[p] {
                pinned_bits[r / WORD] |= bit;
            }
            if deps_by_pos[p] == 0 {
                if pinned[p] {
                    roots_pinned[r / WORD] |= bit;
                } else {
                    roots[r / WORD] |= bit;
                }
            }
        }
        succ_start.push(flat.len() as u32);
        Self {
            ids: by_rank.iter().map(|&p| ids[p] as u32).collect(),
            priority: by_rank.iter().map(|&p| prio[p]).collect(),
            succ_start: succ_start.into(),
            succ: flat.into(),
            deps: deps.into(),
            roots: roots.into(),
            roots_pinned: roots_pinned.into(),
            pinned: pinned_bits.into(),
            width,
            pending: (0..n).map(|_| AtomicU32::new(0)).collect(),
            ready: (0..words).map(|_| AtomicU64::new(0)).collect(),
            ready_pinned: (0..words).map(|_| AtomicU64::new(0)).collect(),
            done: AtomicUsize::new(0),
        }
    }

    /// Jobs.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The widest dependency level: no point waking more threads than that.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The id of rank `rank`.
    #[inline]
    pub fn id(&self, rank: usize) -> usize {
        self.ids[rank] as usize
    }

    /// Priority of rank `rank` (longest remaining path cost, its own job included).
    pub fn priority(&self, rank: usize) -> u64 {
        self.priority[rank]
    }

    /// Successor ranks of rank `rank`.
    pub fn successors(&self, rank: usize) -> &[u32] {
        &self.succ[self.succ_start[rank] as usize..self.succ_start[rank + 1] as usize]
    }

    /// Whether rank `rank` must run on the calling thread.
    pub fn is_pinned(&self, rank: usize) -> bool {
        self.pinned[rank / WORD] & (1 << (rank % WORD)) != 0
    }

    /// Ids in rank order: by priority, and a topological order (the sequential order).
    pub fn order(&self) -> impl Iterator<Item = usize> + '_ {
        self.ids.iter().map(|&i| i as usize)
    }

    /// RT, calling thread only, while no other thread touches the graph: arm a new run
    /// (counters to their source counts, the source-free jobs ready). Publish the graph to
    /// other threads with a Release (or stronger) store afterwards.
    pub fn reset(&self) {
        for (p, &d) in self.pending.iter().zip(self.deps.iter()) {
            p.store(d, Ordering::Relaxed);
        }
        for (w, &b) in self.ready.iter().zip(self.roots.iter()) {
            w.store(b, Ordering::Relaxed);
        }
        for (w, &b) in self.ready_pinned.iter().zip(self.roots_pinned.iter()) {
            w.store(b, Ordering::Relaxed);
        }
        self.done.store(0, Ordering::Relaxed);
    }

    /// RT, lock-free. Take the most urgent ready job (pinned ones first when `caller`, the
    /// thread that called `execute_graph`); its sources' writes are visible (Acquire).
    #[inline]
    pub fn pop(&self, caller: bool) -> Option<usize> {
        if caller && let Some(r) = take_lowest(&self.ready_pinned) {
            return Some(r);
        }
        take_lowest(&self.ready)
    }

    /// RT, lock-free. Whether a job is ready (for threads deciding whether to keep spinning).
    #[inline]
    pub fn has_ready(&self, caller: bool) -> bool {
        self.ready.iter().any(|w| w.load(Ordering::Acquire) != 0)
            || (caller
                && self
                    .ready_pinned
                    .iter()
                    .any(|w| w.load(Ordering::Acquire) != 0))
    }

    /// RT, lock-free. Rank `rank` (taken with `pop`) finished: release its writes, make the
    /// successors whose last source it was ready, count it done. Returns how many jobs it
    /// made ready (a pool may wake that many threads, minus the one continuing).
    #[inline]
    pub fn complete(&self, rank: usize) -> usize {
        let mut readied = 0;
        for &s in self.successors(rank) {
            let s = s as usize;
            // AcqRel: the last decrement acquires every source's release (the RMWs on the
            // counter form one release sequence) and passes them on through the bitset.
            if self.pending[s].fetch_sub(1, Ordering::AcqRel) == 1 {
                let set = if self.is_pinned(s) {
                    &self.ready_pinned
                } else {
                    &self.ready
                };
                set[s / WORD].fetch_or(1 << (s % WORD), Ordering::AcqRel);
                readied += 1;
            }
        }
        self.done.fetch_add(1, Ordering::AcqRel);
        readied
    }

    /// RT. Every job of the run finished; their writes are visible (Acquire).
    #[inline]
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Acquire) >= self.ids.len()
    }

    /// RT. Run ready jobs on this thread until none is ready right now; returns how many
    /// ran. `run(id)` must not unwind (pools catch panics around it), or the run never
    /// finishes. A building block for executors (see `ether-native/src/workers.rs`).
    #[inline]
    pub fn drain(&self, caller: bool, mut run: impl FnMut(usize)) -> usize {
        let mut ran = 0;
        while let Some(r) = self.pop(caller) {
            run(self.id(r));
            self.complete(r);
            ran += 1;
        }
        ran
    }
}

/// Clear and return the lowest set bit of a bitset (lock-free; one winner per bit).
#[inline]
fn take_lowest(words: &[AtomicU64]) -> Option<usize> {
    for (k, w) in words.iter().enumerate() {
        let mut v = w.load(Ordering::Relaxed);
        while v != 0 {
            let bit = v.trailing_zeros() as usize;
            let mask = 1u64 << bit;
            let prev = w.fetch_and(!mask, Ordering::AcqRel);
            if prev & mask != 0 {
                return Some(k * WORD + bit);
            }
            v = prev & !mask;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a(1) → c(1) → m(5); b(10) → m; d(1) → b (a light sidechain source of a heavy job).
    fn sample() -> JobGraph {
        // positions: 0 a, 1 b, 2 c, 3 m, 4 d
        JobGraph::new(
            &[10, 11, 12, 13, 14],
            &[1, 10, 1, 5, 1],
            &[vec![2], vec![3], vec![3], vec![], vec![1, 1]],
            &[false; 5],
        )
    }

    #[test]
    fn ranks_follow_the_critical_path_and_are_topological() {
        let g = sample();
        assert_eq!(g.len(), 5);
        // d (1 + 10 + 5) first, then b (15), a (7), c (6), m (5).
        assert_eq!(g.order().collect::<Vec<_>>(), vec![14, 11, 10, 12, 13]);
        assert_eq!(g.priority(0), 16);
        assert_eq!(g.width(), 2);
        let pos: Vec<usize> = (0..g.len()).collect();
        for r in pos {
            for &s in g.successors(r) {
                assert!(s as usize > r);
            }
        }
    }

    #[test]
    fn runs_by_dependency_most_urgent_first() {
        let g = sample();
        for _ in 0..3 {
            g.reset();
            let mut seen = Vec::new();
            // Only d and a are ready at first; d is more urgent.
            assert!(g.has_ready(false));
            g.drain(false, |id| seen.push(id));
            assert!(g.finished());
            assert_eq!(seen, vec![14, 11, 10, 12, 13]);
        }
        g.reset();
        assert_eq!(g.pop(false), Some(0)); // d
        assert_eq!(g.pop(false), Some(2)); // a
        assert_eq!(g.pop(false), None); // b waits for d, c for a
        assert_eq!(g.complete(2), 1); // a → c ready
        assert_eq!(g.pop(false), Some(3));
        assert!(!g.finished());
    }

    #[test]
    fn pinned_jobs_only_go_to_the_caller() {
        let g = JobGraph::new(
            &[0, 1, 2],
            &[1, 1, 1],
            &[vec![2], vec![2], vec![]],
            &[false, true, false],
        );
        g.reset();
        assert_eq!(g.pop(false).map(|r| g.id(r)), Some(0));
        assert_eq!(g.pop(false), None);
        let r = g.pop(true).unwrap();
        assert_eq!(g.id(r), 1);
        assert!(g.is_pinned(r));
    }

    #[test]
    fn many_jobs_span_several_words() {
        let n = 200;
        let succ: Vec<Vec<usize>> = (0..n)
            .map(|i| if i + 1 < n { vec![n - 1] } else { vec![] })
            .collect();
        let g = JobGraph::new(
            &(0..n).collect::<Vec<_>>(),
            &vec![1; n],
            &succ,
            &vec![false; n],
        );
        g.reset();
        let mut count = 0;
        g.drain(false, |_| count += 1);
        assert_eq!(count, n);
        assert!(g.finished());
        assert_eq!(g.order().last(), Some(n - 1));
    }

    #[test]
    fn empty_graph_is_finished() {
        let g = JobGraph::new(&[], &[], &[], &[]);
        g.reset();
        assert!(g.finished());
        assert_eq!(g.pop(true), None);
    }
}
