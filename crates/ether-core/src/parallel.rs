//! Multicore processing contract (roadmap v2, owned by the `multicore` node; see
//! `docs/ROADMAP.md` and CONTRACTS.md §11.7).
//!
//! `ether-core` never creates threads. A host that wants parallel processing gives the
//! engine a [`ParallelExecutor`] backed by its own real-time worker threads
//! (`EngineConfig::worker_threads` of them; `ether-native/src/workers.rs`). Partition
//! contract:
//!
//! - The compiled snapshot groups tracks into **levels** of the processing DAG (outputs,
//!   sends, resampling inputs, sidechains): a track's level is 1 + the max level of its
//!   sources. Tracks of one level have no data dependencies on each other. Drum-rack pad
//!   chains belong to their track (they run inside its job).
//! - Per sub-block the engine runs the levels in order; within a level it calls
//!   [`ParallelExecutor::execute_pinned`] with one job per track: gather the input bus →
//!   monitored input → clips → automation → chain → sends → fader → meters, into the track's
//!   own buffers (its output buffer and one buffer per send).
//! - **Mixing in a fixed order.** A track's input bus is not written by its sources: the
//!   track's own job *gathers* it at its start, summing its sources' output/send buffers
//!   (all finished in earlier levels) in a list fixed at compile time: the order in which the
//!   sequential engine used to add them (topological order, then pre-fader sends, post-fader
//!   sends, output). The hardware output is summed on the audio thread after the last
//!   level, in the same order. So the floating-point summation order never depends on
//!   scheduling or on the worker count: the output is bit-identical to sequential
//!   processing (worker count 0) and to the pre-multicore engine.
//! - Each job touches only its own track state and nodes (a node belongs to exactly one
//!   chain or pad chain, checked by the compiler), and reads finished tracks of earlier
//!   levels, so jobs need no locks. Shared engine state (transport, param queue, rings,
//!   hardware outputs, metronome, preview, recording) is only touched on the audio thread
//!   between levels.
//! - **Pinned jobs.** A few jobs must run on the calling (audio) thread: tracks with
//!   Complex-warped audio clips (their stretchers live in the engine-wide warp state). The
//!   engine lists them first in their level and passes their count as `pinned`.
//!   Plugins are *not* pinned: CLAP (`process` is an `[audio-thread]` call, and the spec
//!   lets a host run it on any of its audio/worker threads, never concurrently for one
//!   instance), VST3 (`IAudioProcessor::process` from one processing thread at a time) and
//!   AU (`AudioUnitRender` / the AUv3 render block, any thread, not re-entrant) all accept a
//!   worker thread; our hosts keep no thread-local audio state, and a sandboxed plugin's
//!   shared-memory + named-semaphore protocol belongs to its node, so posting/spinning from
//!   a worker is the same as from the audio thread.
//! - The executor must be RT-safe: no allocation, no blocking syscalls on the audio thread;
//!   workers spin or park on futexes/semaphores that the audio thread signals; `execute`
//!   returns only when every job is done. With 0 workers the audio thread runs every job
//!   itself.
//! - **Unsafe sharing (engine side).** `job` is a shared `Fn + Sync`, but each job needs
//!   `&mut` access to its own track (buffers, nodes). The engine hands out disjoint mutable
//!   access through raw pointers (index `i` ↔ track `level[i]` only, never two jobs on one
//!   track or one node), which is sound only if the executor upholds its contract: every
//!   index exactly once, all jobs finished (with a release/acquire fence) before `execute`
//!   returns, and no job running after that. `Node: Send` is what allows a node to be
//!   processed on a worker thread. Every `unsafe` block in `engine.rs` refers to this.
//! - **wasm32:** `EngineConfig::worker_threads` is ignored and the engine always runs
//!   sequentially (the AudioWorklet has no threads; SharedArrayBuffer workers are out of
//!   scope): `Engine::set_executor` is a no-op there.

/// Runs `jobs` independent jobs, possibly in parallel, and returns when all are finished.
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
