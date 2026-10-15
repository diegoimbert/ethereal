//! Engine and node configuration.

use serde::{Deserialize, Serialize};

/// Fixed at engine creation. Changing sample rate or max block size = new engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngineConfig {
    pub sample_rate: u32,
    /// Largest block `process` will ever be called with. Hosts with larger device buffers
    /// call `process` several times.
    pub max_block_size: usize,
    pub input_channels: u16,
    pub output_channels: u16,
    /// Capacity of the node table (devices + plugins + internal nodes).
    pub max_nodes: usize,
    /// Per-node, per-block event capacity (notes + param changes). Overflow drops events and
    /// raises a flag reported in [`crate::EngineOutputs`].
    pub max_events_per_block: usize,
    /// Capacities of the lock-free rings.
    pub param_queue_capacity: usize,
    pub control_queue_capacity: usize,
    pub output_queue_capacity: usize,
    /// Roadmap v2 (`multicore`): worker threads the host may use to process independent
    /// tracks in parallel, in addition to the audio thread. 0 (default) = everything on the
    /// audio thread (the v0.1 behaviour). The engine never spawns threads: the host provides
    /// a [`crate::parallel::ParallelExecutor`] through `Engine::set_executor`
    /// (CONTRACTS.md §11.7). Ignored on wasm32.
    pub worker_threads: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            max_block_size: 1024,
            input_channels: 2,
            output_channels: 2,
            max_nodes: 4096,
            max_events_per_block: 1024,
            param_queue_capacity: 4096,
            control_queue_capacity: 1024,
            output_queue_capacity: 4096,
            worker_threads: 0,
        }
    }
}

/// Passed to [`crate::Node::prepare`] (non-RT; nodes may allocate here).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrepareConfig {
    pub sample_rate: f32,
    pub max_block_size: usize,
    pub max_events_per_block: usize,
}
