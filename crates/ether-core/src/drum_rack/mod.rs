//! Engine-side drum racks (roadmap v2, owned by the `drum-rack` node; see
//! `docs/ROADMAP.md`).
//!
//! Already wired (base-17), so this node doesn't need to edit `graph.rs`/`engine.rs`/
//! `mixer.rs`:
//! - `graph::compile_with` builds a [`RacksRt`] per track from `TrackDesc::racks` with
//!   [`RacksRt::compile`] (preallocate everything there), and `TrackRt::inherit` calls
//!   [`RacksRt::inherit`] on snapshot swaps.
//! - `engine.rs::render_sub`, at every chain entry whose node is a rack
//!   ([`RacksRt::is_rack`]), calls [`RacksRt::run_pads`] *before* processing the rack node:
//!   route the entry's note events by key to the pad chains (transposed to
//!   `ether_model::PAD_PLAY_NOTE`), apply choke groups, process the pad chains' nodes (from
//!   the engine node table), PDC-align them, and mix them through pad volume/pan/mute into
//!   `input` (the rack node's input buffer). The rack node then runs on that input.
//!
//! RT rules apply; must compile on wasm32. Placeholder: racks pass their input through.

use crate::config::EngineConfig;
use crate::engine::NodeSlot;
use crate::event::ProcessEvent;
use crate::graph::{NodeInfo, RackDesc};
use crate::node::NodeKey;
use crate::transport::TransportInfo;

/// Runtime state of a track's racks (pad buffers, smoothers, choke state).
#[derive(Debug, Default)]
pub(crate) struct RacksRt {
    racks: Vec<NodeKey>,
}

impl RacksRt {
    /// Non-RT. `node_info` gives pad-chain node latencies/channels (unknown = skipped).
    pub(crate) fn compile(
        racks: &[RackDesc],
        node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
        config: &EngineConfig,
    ) -> Self {
        let _ = (node_info, config);
        Self {
            racks: racks.iter().map(|r| r.rack).collect(),
        }
    }

    /// RT. Carry running state (smoothers, voices' choke state) from the previous snapshot.
    pub(crate) fn inherit(&mut self, old: &mut RacksRt) {
        let _ = old;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.racks.is_empty()
    }

    /// RT. Whether chain node `key` is a rack of this track.
    pub(crate) fn is_rack(&self, key: NodeKey) -> bool {
        self.racks.contains(&key)
    }

    /// RT. Run rack `rack`'s pads for `n` frames (see the module docs). `events` are the
    /// rack entry's sorted events; `input` is the rack node's input (in: the chain signal
    /// so far; out: what the rack node should receive). `reset` = transport jump.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_pads(
        &mut self,
        rack: NodeKey,
        nodes: &mut [NodeSlot],
        events: &[ProcessEvent],
        info: &TransportInfo,
        sample_rate: f32,
        input: &mut [Vec<f32>; 2],
        n: usize,
        reset: bool,
    ) {
        let _ = (rack, nodes, events, info, sample_rate, input, n, reset);
    }
}
