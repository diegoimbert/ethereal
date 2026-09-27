//! Multicore processing contract (roadmap v2, owned by the `multicore` node; see
//! `docs/ROADMAP.md` and CONTRACTS.md §11.7).
//!
//! `ether-core` never creates threads. A host that wants parallel processing gives the
//! engine a [`ParallelExecutor`] backed by its own real-time worker threads
//! (`EngineConfig::worker_threads` of them). Partition contract:
//!
//! - The compiled snapshot groups tracks into **levels** of the processing DAG (routing,
//!   sends, resampling inputs, sidechains): a track's level is 1 + the max level of its
//!   sources. Tracks of one level have no data dependencies on each other.
//! - Per sub-block the engine runs the levels in order; within a level it calls
//!   [`ParallelExecutor::execute`] with one job per track (clips → automation → chain →
//!   fader → meters into the track's own buffers). Mixing into destination buses happens
//!   after the level completes, on the audio thread, in a fixed order, so results are
//!   bit-identical to sequential processing.
//! - Each job touches only its own track state and nodes (a node belongs to exactly one
//!   chain), so jobs need no locks. Shared engine state (transport, param queue, rings) is
//!   only touched on the audio thread between levels.
//! - The executor must be RT-safe: no allocation, no blocking syscalls; workers spin or
//!   wait on futexes/semaphores that the audio thread signals; `execute` returns only when
//!   every job is done. With 0 workers the audio thread runs every job itself.

/// Runs `jobs` independent jobs, possibly in parallel, and returns when all are finished.
pub trait ParallelExecutor: Send {
    /// RT. Call `job(i)` exactly once for every `i` in `0..jobs` (any thread, any order).
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync));

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

    fn workers(&self) -> usize {
        0
    }
}
