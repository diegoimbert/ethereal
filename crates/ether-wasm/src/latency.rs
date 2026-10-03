//! Node latency report from the worklet (v0.3, owned by the `web-latency` node;
//! CONTRACTS.md §13.13).
//!
//! Natively `EngineBridge::node_latency` reads `EngineHandle::node_latency`, so the
//! controller tick republishes PDC when a built-in's latency changes (`latency-republish`).
//! On the web the engine lives in the AudioWorklet and the controller's bridge returned
//! `None`. The fix (`web-latency`, shared touches in `proto.rs`, `worklet.rs`, `bridge.rs`):
//! - the worklet sends a [`LatencyReport`] (every live node's current `Node::latency`) at
//!   most every [`REPORT_INTERVAL_MS`], and only when a value changed since its last report;
//! - the Worker's bridge keeps the latest values and answers `node_latency(key)` from them,
//!   so `EngineState::check_latencies` republishes exactly like natively.
//! No allocation on the audio side: the worklet fills a pre-sized buffer.

use ether_core::NodeKey;

/// Minimum time between two reports.
pub const REPORT_INTERVAL_MS: u64 = 50;

/// Worklet → Worker: current latencies (samples) of the nodes whose latency changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LatencyReport {
    pub latencies: Vec<(NodeKey, u32)>,
}
