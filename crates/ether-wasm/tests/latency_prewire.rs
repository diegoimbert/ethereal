//! v0.3 prewire (contracts-4) for `web-latency`: the worklet → Worker latency report type
//! exists and the web bridge still reports no node latency (`EngineBridge::node_latency`
//! default), so PDC republish is native-only until the node lands. `web-latency` replaces
//! this with real tests (a latency change in the worklet reaches `node_latency` and
//! triggers a republish).

use ether_wasm::latency::{LatencyReport, REPORT_INTERVAL_MS};

#[test]
fn latency_report_is_prewired() {
    let r = LatencyReport::default();
    assert!(r.latencies.is_empty());
    assert_eq!(REPORT_INTERVAL_MS, 50);
}
