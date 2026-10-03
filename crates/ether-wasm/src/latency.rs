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
//!
//! No allocation on the audio side: the worklet fills a pre-sized buffer.
//!
//! **Baseline.** A node's latency when the worklet adds it is what the worklet's engine
//! compiles PDC with until it changes, and is never reported: the Worker answers `None`
//! for it (as the controller's last publish recorded), so a node that keeps its initial
//! latency never causes a republish. Only a *change* is reported (as `Some`), and the
//! Worker's `None` → `Some(x)` difference is the trigger, exactly like a native change.
//!
//! Wire format ([`crate::proto::REPORT_LATENCY`]):
//! `[L][count u16][count x (index u32, generation u32, latency u32)]`, keys being the
//! Worker's virtual node keys.

use std::collections::BTreeMap;

use ether_core::{EngineHandle, NodeKey};

use crate::proto::{DecodeError, REPORT_LATENCY};

/// Minimum time between two reports.
pub const REPORT_INTERVAL_MS: u64 = 50;

/// Bytes of the report header (tag + count).
const HEADER: usize = 1 + 2;
/// Bytes per reported node.
const ENTRY: usize = 4 + 4 + 4;

/// Worklet → Worker: current latencies (samples) of the nodes whose latency changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LatencyReport {
    pub latencies: Vec<(NodeKey, u32)>,
}

impl LatencyReport {
    /// Encoded size of a report of `nodes` entries (to pre-size buffers).
    pub fn encoded_len(nodes: usize) -> usize {
        HEADER + nodes * ENTRY
    }

    /// Encode into `out` (cleared first). Doesn't allocate if `out` has enough capacity.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        begin(out);
        for &(key, latency) in &self.latencies {
            push(out, key, latency);
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let (&tag, rest) = bytes.split_first().ok_or(DecodeError::Empty)?;
        if tag != REPORT_LATENCY {
            return Err(DecodeError::Tag(tag));
        }
        let count = rest.get(..2).ok_or(DecodeError::Truncated)?;
        let count = u16::from_le_bytes([count[0], count[1]]) as usize;
        let body = &rest[2..];
        if body.len() != count * ENTRY {
            return Err(DecodeError::Truncated);
        }
        let latencies = body.as_chunks::<ENTRY>().0.iter().map(entry).collect();
        Ok(Self { latencies })
    }
}

/// **RT.** Start a report in `out` (cleared; count 0).
fn begin(out: &mut Vec<u8>) {
    out.clear();
    out.push(REPORT_LATENCY);
    out.extend_from_slice(&0u16.to_le_bytes());
}

/// **RT.** Decode one entry.
fn entry(e: &[u8; ENTRY]) -> (NodeKey, u32) {
    let u32_at = |i: usize| u32::from_le_bytes([e[i], e[i + 1], e[i + 2], e[i + 3]]);
    (
        NodeKey {
            index: u32_at(0),
            generation: u32_at(4),
        },
        u32_at(8),
    )
}

/// **RT.** Append one entry and bump the count (no allocation within capacity).
fn push(out: &mut Vec<u8>, key: NodeKey, latency: u32) {
    let count = u16::from_le_bytes([out[1], out[2]]) + 1;
    out[1..3].copy_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&key.index.to_le_bytes());
    out.extend_from_slice(&key.generation.to_le_bytes());
    out.extend_from_slice(&latency.to_le_bytes());
}

/// Render blocks between two scans at `sample_rate` with `block`-frame quanta (rounded up,
/// so reports are never closer than [`REPORT_INTERVAL_MS`]).
pub fn interval_blocks(sample_rate: u32, block: usize) -> u32 {
    let frames = REPORT_INTERVAL_MS * u64::from(sample_rate);
    (frames.div_ceil(1000 * block as u64)).max(1) as u32
}

/// Worklet side: the last latency reported per node (virtual key), the scan clock and the
/// pre-sized report buffer. [`Self::add`]/[`Self::remove`] run while applying control
/// frames (they may allocate); [`Self::tick`] and [`Self::commit`] are RT.
pub struct LatencyTracker {
    /// Virtual key → latency the Worker knows (the baseline until a report is delivered).
    reported: BTreeMap<NodeKey, u32>,
    interval: u32,
    since: u32,
    buf: Vec<u8>,
}

impl LatencyTracker {
    /// `max_nodes`: the engine's node capacity (a report holds at most that many entries).
    pub fn new(sample_rate: u32, block: usize, max_nodes: usize) -> Self {
        Self {
            reported: BTreeMap::new(),
            interval: interval_blocks(sample_rate, block),
            since: 0,
            buf: Vec::with_capacity(LatencyReport::encoded_len(max_nodes.min(u16::MAX as usize))),
        }
    }

    /// A node was added with `latency` (its baseline, not reported).
    pub fn add(&mut self, key: NodeKey, latency: u32) {
        self.reported.insert(key, latency);
    }

    pub fn remove(&mut self, key: NodeKey) {
        self.reported.remove(&key);
    }

    /// **RT.** Count one rendered block; every [`REPORT_INTERVAL_MS`], compare each node's
    /// current latency with the last reported one and return the encoded report of the
    /// changes (`None`: not due, or nothing changed). After delivering it, call
    /// [`Self::commit`]; an undelivered report is simply rebuilt at the next scan.
    pub fn tick(
        &mut self,
        keys: &BTreeMap<NodeKey, NodeKey>,
        handle: &EngineHandle,
    ) -> Option<&[u8]> {
        self.since += 1;
        if self.since < self.interval {
            return None;
        }
        self.since = 0;
        let capacity = (self.buf.capacity() - HEADER) / ENTRY;
        begin(&mut self.buf);
        let mut n = 0;
        for (virt, real) in keys {
            let (Some(&last), Some(now)) = (self.reported.get(virt), handle.node_latency(*real))
            else {
                continue;
            };
            if now != last && n < capacity {
                push(&mut self.buf, *virt, now);
                n += 1;
            }
        }
        (n > 0).then_some(&self.buf[..])
    }

    /// **RT.** The last report from [`Self::tick`] was delivered: its values are now the
    /// Worker's.
    pub fn commit(&mut self) {
        let Self { reported, buf, .. } = self;
        for e in buf[HEADER..].as_chunks::<ENTRY>().0 {
            let (key, latency) = entry(e);
            if let Some(v) = reported.get_mut(&key) {
                *v = latency;
            }
        }
        begin(buf);
    }
}

/// Worker side: the latest reported latency per live node (virtual key). `None` until the
/// node's latency first changes in the worklet (see the module docs, "Baseline").
#[derive(Debug, Default)]
pub struct LatencyTable {
    values: BTreeMap<NodeKey, Option<u32>>,
}

impl LatencyTable {
    /// The Worker created `key`.
    pub fn track(&mut self, key: NodeKey) {
        self.values.insert(key, None);
    }

    /// The Worker destroyed `key` (a late report for it is ignored).
    pub fn forget(&mut self, key: NodeKey) {
        self.values.remove(&key);
    }

    /// Apply a report: only nodes the Worker still tracks are updated.
    pub fn apply(&mut self, report: &LatencyReport) {
        for &(key, latency) in &report.latencies {
            if let Some(v) = self.values.get_mut(&key) {
                *v = Some(latency);
            }
        }
    }

    /// `EngineBridge::node_latency`.
    pub fn get(&self, key: NodeKey) -> Option<u32> {
        self.values.get(&key).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(index: u32) -> NodeKey {
        NodeKey {
            index,
            generation: 1,
        }
    }

    #[test]
    fn report_roundtrip_without_realloc() {
        let r = LatencyReport {
            latencies: vec![
                (key(1), 0),
                (key(7), 480),
                (
                    NodeKey {
                        index: u32::MAX,
                        generation: 3,
                    },
                    u32::MAX,
                ),
            ],
        };
        let mut buf = Vec::with_capacity(LatencyReport::encoded_len(3));
        let cap = buf.capacity();
        r.encode_into(&mut buf);
        assert_eq!(
            (buf.len(), buf.capacity()),
            (LatencyReport::encoded_len(3), cap)
        );
        assert_eq!(LatencyReport::decode(&buf).unwrap(), r);
        assert_eq!(
            LatencyReport::decode(&buf[..buf.len() - 1]),
            Err(DecodeError::Truncated)
        );
        assert_eq!(
            LatencyReport::decode(&buf[..2]),
            Err(DecodeError::Truncated)
        );
        assert_eq!(LatencyReport::decode(&[]), Err(DecodeError::Empty));
        assert_eq!(LatencyReport::decode(b"R\0\0"), Err(DecodeError::Tag(b'R')));
        let empty = LatencyReport::default();
        empty.encode_into(&mut buf);
        assert_eq!(LatencyReport::decode(&buf).unwrap(), empty);
    }

    #[test]
    fn interval_is_at_least_50_ms() {
        // 48 kHz / 128: 18.75 blocks -> 19 (50.7 ms).
        assert_eq!(interval_blocks(48_000, 128), 19);
        assert_eq!(interval_blocks(44_100, 128), 18);
        for (sr, block) in [(44_100, 128), (48_000, 128), (96_000, 128), (8_000, 4096)] {
            let ms = interval_blocks(sr, block) as f64 * block as f64 * 1000.0 / sr as f64;
            assert!(ms >= REPORT_INTERVAL_MS as f64 || interval_blocks(sr, block) == 1);
        }
    }

    /// The worklet-side tracker against a real engine: the baseline is not reported, a
    /// change is reported once per interval into the pre-sized buffer (never reallocated),
    /// and an undelivered report is rebuilt at the next scan.
    #[test]
    fn tracker_reports_changes_only() {
        use ether_core::protocol::model::BuiltinDevice;
        use ether_core::{EngineConfig, ParamChange, ParamTarget};
        use ether_devices::fx_dynamics::gate;

        struct NoMedia;
        impl ether_devices::SampleResolver for NoMedia {
            fn resolve(
                &self,
                _: ether_core::protocol::model::MediaId,
            ) -> Option<std::sync::Arc<dyn ether_core::AudioSource>> {
                None
            }
        }
        let mut parts = ether_core::create(EngineConfig {
            sample_rate: 48_000,
            max_block_size: 128,
            max_nodes: 8,
            ..Default::default()
        });
        let mut keys = BTreeMap::new();
        let mut tracker = LatencyTracker::new(48_000, 128, 8);
        for i in 0..3 {
            let mut node = ether_devices::create(&BuiltinDevice::Gate, &NoMedia);
            node.set_param(gate::LOOKAHEAD, 1.0);
            let real = parts.handle.add_node(node).unwrap();
            keys.insert(key(100 + i), real);
            tracker.add(key(100 + i), parts.handle.node_latency(real).unwrap());
        }
        // Only nodes in the running graph are processed (and refresh their latency).
        use ether_core::graph::{ChainEntry, TrackDesc};
        use ether_core::protocol::model::{TrackId, TrackKind, Ulid};
        let master = TrackDesc {
            modulation: Default::default(),
            vca: Default::default(),
            chain_racks: Default::default(),
            frozen: Default::default(),
            input_tap: Default::default(),
            id: TrackId(Ulid(1)),
            kind: TrackKind::Master,
            chain: keys
                .values()
                .map(|&node| ChainEntry {
                    node,
                    enabled: true,
                    sidechain: None,
                })
                .collect(),
            output: None,
            group: None,
            sends: vec![],
            volume: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            audio_input: None,
            monitor: false,
            armed: false,
            clips: vec![],
            automation: vec![],
            racks: Vec::new(),
            expression: Default::default(),
            hw_io: Vec::new(),
        };
        parts
            .handle
            .publish(ether_core::RenderGraphDesc {
                tracks: vec![master],
                ..Default::default()
            })
            .unwrap();
        let mut l = vec![0.0f32; 128];
        let mut r = vec![0.0f32; 128];
        let mut scan = |parts: &mut ether_core::EngineParts, tracker: &mut LatencyTracker| {
            let mut out = None;
            for _ in 0..interval_blocks(48_000, 128) {
                parts
                    .engine
                    .process(&[], &mut [&mut l[..], &mut r[..]], 128);
                if let Some(b) = tracker.tick(&keys, &parts.handle) {
                    out = Some(LatencyReport::decode(b).unwrap());
                }
            }
            out
        };
        assert_eq!(scan(&mut parts, &mut tracker), None, "baseline reported");
        parts
            .handle
            .set_param(ParamChange {
                target: ParamTarget::Node {
                    node: keys[&key(101)],
                    param: gate::LOOKAHEAD,
                },
                value: 10.0,
            })
            .unwrap();
        let (ptr, cap) = (tracker.buf.as_ptr(), tracker.buf.capacity());
        let want = LatencyReport {
            latencies: vec![(key(101), 480)],
        };
        assert_eq!(scan(&mut parts, &mut tracker), Some(want.clone()));
        // Not delivered: the same change is reported again.
        assert_eq!(scan(&mut parts, &mut tracker), Some(want));
        tracker.commit();
        assert_eq!(
            scan(&mut parts, &mut tracker),
            None,
            "delivered change re-reported"
        );
        assert_eq!((tracker.buf.as_ptr(), tracker.buf.capacity()), (ptr, cap));
        tracker.remove(key(101));
        assert_eq!(scan(&mut parts, &mut tracker), None);
    }

    #[test]
    fn table_tracks_live_nodes_only() {
        let mut t = LatencyTable::default();
        t.track(key(1));
        assert_eq!(t.get(key(1)), None, "baseline is unreported");
        let report = LatencyReport {
            latencies: vec![(key(1), 240), (key(2), 5)],
        };
        t.apply(&report);
        assert_eq!(t.get(key(1)), Some(240));
        assert_eq!(t.get(key(2)), None, "untracked (destroyed) node ignored");
        t.forget(key(1));
        t.apply(&report);
        assert_eq!(t.get(key(1)), None);
        assert!(t.values.is_empty());
    }
}
