//! Device → UI analysis channel (v0.2, implemented and frozen by contracts-3; CONTRACTS.md
//! §12.4.3). Users: `fx-analysis` (spectrum, tuner), `fx-dynamics` (gain-reduction meters),
//! `racks-modulation` (modulated-value readback, [`crate::modulation::readback`]).
//!
//! # Contract
//! - A node opts in with [`crate::Node::has_analysis`] (queried once when it is added to the
//!   engine) and fills frames in [`crate::Node::analysis`]; it is only asked while watched
//!   (`EngineHandle::watch_analysis`). Nodes accumulate what they need
//!   (FFT input, pitch estimate) inside `process` and only **copy** the latest result out in
//!   `analysis` (RT: no allocation, bounded).
//! - The engine calls `analysis` for every watched live node (bypassed ones too), round-robin, on the audio
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
//! - `Spectrum`: `[min_hz, max_hz, bin_0_db, ..., bin_{n-1}_db]`, `n <= 256`, bins log-spaced
//!   (the device's output); `SpectrumPre`: the same, measured at its input (EQ pre/post
//!   overlay, `graphical-eq`).
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
pub const ANALYSIS_RING: usize = 128;
/// Frames a node may write per pass ([`AnalysisSink`] capacity), e.g. pre + post spectrum.
pub const ANALYSIS_FRAMES_PER_PASS: usize = 4;
/// Most analysis nodes tracked at once (further opted-in nodes are ignored).
pub const MAX_ANALYSIS_NODES: usize = 128;

/// Payload kind (append-only; see the module docs for the encodings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisKind {
    Spectrum,
    Tuner,
    Levels,
    Modulation,
    /// v0.2 (`graphical-eq`): input spectrum.
    SpectrumPre,
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

/// Where a node writes its frames for one pass ([`crate::Node::analysis`]): a fixed set of
/// [`ANALYSIS_FRAMES_PER_PASS`] pre-allocated frames, no allocation.
pub struct AnalysisSink<'a> {
    frames: &'a mut [AnalysisFrame],
    len: usize,
}

impl<'a> AnalysisSink<'a> {
    /// A sink over `frames` (its capacity).
    pub fn new(frames: &'a mut [AnalysisFrame]) -> Self {
        Self { frames, len: 0 }
    }

    /// RT. The next frame, emptied and set to `kind`; `None` when the sink is full.
    pub fn frame(&mut self, kind: AnalysisKind) -> Option<&mut AnalysisFrame> {
        let f = self.frames.get_mut(self.len)?;
        self.len += 1;
        f.begin(kind);
        Some(f)
    }

    /// Frames written so far.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        self.frames.len()
    }

    /// The frames written.
    pub fn written(&self) -> &[AnalysisFrame] {
        &self.frames[..self.len]
    }
}

/// Audio-thread side (owned by the `Engine`).
///
/// Nodes that opt in get a slot (`on_add`); only **watched** slots are collected
/// (`EngineHandle::watch_analysis`, driven by the controller's per-connection, refcounted
/// `Analysis::Watch`). Collection walks the slots round-robin from where the previous pass
/// stopped, so when the ring is full the same nodes are not always the ones dropped.
pub(crate) struct AnalysisRt {
    /// `(node, watched)`.
    slots: [Option<(NodeKey, bool)>; MAX_ANALYSIS_NODES],
    cursor: usize,
    interval: usize,
    elapsed: usize,
    ring: Producer<AnalysisFrame>,
    scratch: Box<[AnalysisFrame; ANALYSIS_FRAMES_PER_PASS]>,
}

impl AnalysisRt {
    /// Non-RT (engine creation).
    pub(crate) fn new(sample_rate: u32) -> (Self, Consumer<AnalysisFrame>) {
        Self::with_ring(sample_rate, ANALYSIS_RING)
    }

    /// Non-RT. With a ring of `capacity` frames (tests).
    pub(crate) fn with_ring(sample_rate: u32, capacity: usize) -> (Self, Consumer<AnalysisFrame>) {
        let (ring, rx) = RingBuffer::new(capacity);
        (
            Self {
                slots: [None; MAX_ANALYSIS_NODES],
                cursor: 0,
                interval: (sample_rate / ANALYSIS_HZ).max(1) as usize,
                elapsed: 0,
                ring,
                scratch: Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]),
            },
            rx,
        )
    }

    /// RT. A node was added to the engine at `key` (any previous occupant of that slot
    /// index, whatever its generation, is forgotten).
    pub(crate) fn on_add(&mut self, key: NodeKey, node: &dyn Node) {
        self.forget(|n| n.index == key.index);
        if node.has_analysis()
            && let Some(slot) = self.slots.iter_mut().find(|k| k.is_none())
        {
            *slot = Some((key, false));
        }
    }

    /// RT. A node left the engine.
    pub(crate) fn on_remove(&mut self, key: NodeKey) {
        self.forget(|n| n == key);
    }

    fn forget(&mut self, matches: impl Fn(NodeKey) -> bool) {
        for k in self.slots.iter_mut() {
            if k.is_some_and(|(n, _)| matches(n)) {
                *k = None;
            }
        }
    }

    /// RT. Start/stop collecting `key` (unknown keys are ignored).
    pub(crate) fn watch(&mut self, key: NodeKey, on: bool) {
        for (n, w) in self.slots.iter_mut().flatten() {
            if *n == key {
                *w = on;
            }
        }
    }

    /// RT. Advance the throttle clock by `frames`; `true` when frames are due this block.
    pub(crate) fn due(&mut self, frames: usize) -> bool {
        self.elapsed += frames;
        if self.elapsed >= self.interval {
            // Keep the remainder so the average rate is `ANALYSIS_HZ` whatever the block
            // size (at most one collection per block).
            self.elapsed = (self.elapsed - self.interval).min(self.interval - 1);
            true
        } else {
            false
        }
    }

    /// RT. One frame from every watched node, round-robin, while the ring has room.
    pub(crate) fn collect_all(&mut self, nodes: &mut [crate::engine::NodeSlot]) {
        let n = MAX_ANALYSIS_NODES;
        for step in 0..n {
            let i = (self.cursor + step) % n;
            let Some((key, true)) = self.slots[i] else {
                continue;
            };
            if self.ring.slots() < ANALYSIS_FRAMES_PER_PASS {
                // Not enough room for a full pass of this node: resume here next time.
                self.cursor = i;
                return;
            }
            if let Some(node) = crate::engine::NodeSlot::get(nodes, key) {
                let mut sink = AnalysisSink::new(&mut self.scratch[..]);
                node.analysis(&mut sink);
                let n = sink.len();
                for f in self.scratch[..n].iter_mut() {
                    f.node = key;
                    let _ = self.ring.push(*f);
                }
            }
        }
        self.cursor = (self.cursor + 1) % n;
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
    use crate::buffer::AudioBuffers;
    use crate::config::PrepareConfig;
    use crate::engine::NodeSlot;
    use crate::node::{ProcessContext, ProcessStatus};

    /// Emits one `Levels` frame tagged with its id.
    struct Tagger(f32);
    impl Node for Tagger {
        fn prepare(&mut self, _: &PrepareConfig) {}
        fn reset(&mut self) {}
        fn process(
            &mut self,
            _: &mut ProcessContext<'_>,
            _: &mut AudioBuffers<'_, '_>,
        ) -> ProcessStatus {
            ProcessStatus::Silent
        }
        fn has_analysis(&self) -> bool {
            true
        }
        fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
            if let Some(f) = out.frame(AnalysisKind::Levels) {
                f.push(self.0);
            }
        }
    }

    /// With a ring too small for every watched node, passes resume where the previous one
    /// stopped: no node starves.
    #[test]
    fn full_ring_is_fair() {
        let (mut rt, mut rx) = AnalysisRt::with_ring(48_000, ANALYSIS_FRAMES_PER_PASS + 1);
        let mut nodes: Vec<NodeSlot> = (0..3)
            .map(|i| NodeSlot {
                generation: 1,
                node: Some(Box::new(Tagger(i as f32))),
            })
            .collect();
        for i in 0..3u32 {
            let key = NodeKey {
                index: i,
                generation: 1,
            };
            rt.on_add(key, nodes[i as usize].node.as_deref().unwrap());
            rt.watch(key, true);
        }
        let mut seen = [0usize; 3];
        for _ in 0..6 {
            rt.collect_all(&mut nodes);
            while let Ok(f) = rx.pop() {
                seen[f.values()[0] as usize] += 1;
            }
        }
        assert!(seen.iter().all(|&n| n >= 3), "{seen:?}");
        // Unwatched nodes are never asked.
        rt.watch(
            NodeKey {
                index: 1,
                generation: 1,
            },
            false,
        );
        rt.collect_all(&mut nodes);
        rt.collect_all(&mut nodes);
        while let Ok(f) = rx.pop() {
            assert_ne!(f.values()[0], 1.0);
        }
    }

    #[test]
    fn sink_is_bounded() {
        let mut frames = [AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS];
        let mut sink = AnalysisSink::new(&mut frames);
        for _ in 0..ANALYSIS_FRAMES_PER_PASS {
            assert!(sink.frame(AnalysisKind::Spectrum).is_some());
        }
        assert!(sink.frame(AnalysisKind::SpectrumPre).is_none());
        assert_eq!(sink.len(), ANALYSIS_FRAMES_PER_PASS);
    }

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
