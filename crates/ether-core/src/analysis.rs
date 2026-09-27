//! Device → UI analysis channel (v0.2, implemented and frozen by contracts-3; CONTRACTS.md
//! §12.4.3). Users: `fx-analysis` (spectrum, tuner), `fx-dynamics` (gain-reduction meters),
//! `racks-modulation` (modulated-value readback, [`crate::modulation::readback`]).
//!
//! # Contract
//! - A node opts in with [`crate::Node::has_analysis`] (queried once when it is added to the
//!   engine) and fills frames in [`crate::Node::analysis`]. Nodes accumulate what they need
//!   (FFT input, pitch estimate) inside `process` and only **copy** the latest result out in
//!   `analysis` (RT: no allocation, bounded).
//! - The engine calls `analysis` for every opted-in live node (bypassed ones too) on the audio
//!   thread, after all track jobs of a block, at most [`ANALYSIS_HZ`] times per second
//!   (every `sample_rate / ANALYSIS_HZ` frames), and pushes the frame into a fixed-size SPSC
//!   ring of `Copy` frames ([`ANALYSIS_RING`] slots). When the ring is full, frames are
//!   dropped (latest-wins semantics downstream; nothing blocks, nothing allocates).
//! - The controller drains it with `EngineHandle::poll_analysis` (via
//!   `EngineBridge::poll_analysis`), maps node keys to devices and emits `Event::Analysis`
//!   only for watched devices (`Analysis::Watch`), coalescing to the latest frame per
//!   (device, kind) per tick.
//!
//! # Frame encoding per [`AnalysisKind`] (`data[..len]`)
//! - `Spectrum`: `[min_hz, max_hz, bin_0_db, ..., bin_{n-1}_db]`, `n <= 256`, bins log-spaced.
//! - `Tuner`: `[hz (0 = none), note (-1 = none), cents, confidence 0..1, level_db]`.
//! - `Levels`: device-defined values (e.g. per-band gain reduction in dB).
//! - `Modulation`: triples `[f32::from_bits(param id), base, effective]` (normalized).

use rtrb::{Consumer, Producer, RingBuffer};

use crate::node::{Node, NodeKey};

/// Values per frame.
pub const ANALYSIS_MAX_VALUES: usize = 1024;
/// Frames per second per node (upper bound).
pub const ANALYSIS_HZ: u32 = 30;
/// Ring capacity (frames).
pub const ANALYSIS_RING: usize = 64;
/// Most analysis nodes tracked at once (further opted-in nodes are ignored).
pub const MAX_ANALYSIS_NODES: usize = 128;

/// Payload kind (append-only; see the module docs for the encodings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisKind {
    Spectrum,
    Tuner,
    Levels,
    Modulation,
}

/// One analysis frame (fixed size, `Copy`: travels through the ring by value).
#[derive(Clone, Copy)]
pub struct AnalysisFrame {
    /// Set by the engine (the node that produced it).
    pub node: NodeKey,
    pub kind: AnalysisKind,
    pub len: u16,
    pub data: [f32; ANALYSIS_MAX_VALUES],
}

impl std::fmt::Debug for AnalysisFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisFrame")
            .field("node", &self.node)
            .field("kind", &self.kind)
            .field("values", &self.values())
            .finish()
    }
}

impl AnalysisFrame {
    pub const EMPTY: Self = Self {
        node: NodeKey {
            index: 0,
            generation: 0,
        },
        kind: AnalysisKind::Levels,
        len: 0,
        data: [0.0; ANALYSIS_MAX_VALUES],
    };

    pub fn values(&self) -> &[f32] {
        &self.data[..(self.len as usize).min(ANALYSIS_MAX_VALUES)]
    }

    /// RT. Reset to an empty frame of `kind`.
    pub fn begin(&mut self, kind: AnalysisKind) {
        self.kind = kind;
        self.len = 0;
    }

    /// RT. Append a value; `false` when full.
    pub fn push(&mut self, v: f32) -> bool {
        let i = self.len as usize;
        if i >= ANALYSIS_MAX_VALUES {
            return false;
        }
        self.data[i] = v;
        self.len += 1;
        true
    }
}

/// Audio-thread side (owned by the `Engine`).
pub(crate) struct AnalysisRt {
    keys: [Option<NodeKey>; MAX_ANALYSIS_NODES],
    interval: usize,
    elapsed: usize,
    ring: Producer<AnalysisFrame>,
    scratch: Box<AnalysisFrame>,
}

impl AnalysisRt {
    /// Non-RT (engine creation).
    pub(crate) fn new(sample_rate: u32) -> (Self, Consumer<AnalysisFrame>) {
        let (ring, rx) = RingBuffer::new(ANALYSIS_RING);
        (
            Self {
                keys: [None; MAX_ANALYSIS_NODES],
                interval: (sample_rate / ANALYSIS_HZ).max(1) as usize,
                elapsed: 0,
                ring,
                scratch: Box::new(AnalysisFrame::EMPTY),
            },
            rx,
        )
    }

    /// RT. A node was added to the engine.
    pub(crate) fn on_add(&mut self, key: NodeKey, node: &dyn Node) {
        if node.has_analysis()
            && let Some(slot) = self.keys.iter_mut().find(|k| k.is_none())
        {
            *slot = Some(key);
        }
    }

    /// RT. A node left the engine.
    pub(crate) fn on_remove(&mut self, key: NodeKey) {
        for k in self.keys.iter_mut().filter(|k| **k == Some(key)) {
            *k = None;
        }
    }

    /// RT. Advance the throttle clock by `frames`; `true` when frames are due this block.
    pub(crate) fn due(&mut self, frames: usize) -> bool {
        self.elapsed += frames;
        if self.elapsed >= self.interval {
            self.elapsed = 0;
            true
        } else {
            false
        }
    }

    /// Opted-in node keys.
    pub(crate) fn keys(&self) -> [Option<NodeKey>; MAX_ANALYSIS_NODES] {
        self.keys
    }

    /// RT. Ask `node` for a frame and push it.
    pub(crate) fn collect(&mut self, key: NodeKey, node: &mut dyn Node) {
        self.scratch.begin(AnalysisKind::Levels);
        if node.analysis(&mut self.scratch) {
            self.scratch.node = key;
            let _ = self.ring.push(*self.scratch);
        }
    }

    /// RT. Push a frame (dropped when the ring is full). Used by `crate::modulation::readback`.
    #[allow(dead_code)] // until racks-modulation lands
    pub(crate) fn push(&mut self, frame: &AnalysisFrame) {
        let _ = self.ring.push(*frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_encode_and_bound() {
        let mut f = AnalysisFrame::EMPTY;
        f.begin(AnalysisKind::Tuner);
        for v in [440.0, 69.0, 0.0, 1.0, -12.0] {
            assert!(f.push(v));
        }
        assert_eq!(f.values(), &[440.0, 69.0, 0.0, 1.0, -12.0]);
        f.begin(AnalysisKind::Levels);
        for _ in 0..ANALYSIS_MAX_VALUES {
            assert!(f.push(0.5));
        }
        assert!(!f.push(0.5));
        assert_eq!(f.values().len(), ANALYSIS_MAX_VALUES);
    }
}
