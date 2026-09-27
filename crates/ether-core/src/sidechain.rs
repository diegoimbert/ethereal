//! Sidechain routing in the engine (roadmap v2, owned by the `sidechain` node; see
//! `docs/ROADMAP.md` and CONTRACTS.md §11.10).
//!
//! Already wired (base-17), so this node doesn't need to edit `graph.rs`/`engine.rs`/
//! `mixer.rs`:
//! - `graph::compile_with` orders every sidechain source before its consumer track (the
//!   `ChainEntry::sidechain` edges join the topological sort, not the bus mixing), resolves
//!   `ChainRt::sidechain` to the source's track index, calls [`required_input_latency`] for
//!   every track before computing its output latency (PDC), and builds [`Taps`] with
//!   [`Taps::compile`].
//! - `engine.rs::render_sub` calls [`Taps::write`] with every track's post-fader output
//!   (after its PDC output delay), and for a chain entry with a sidechain calls
//!   `Node::process_sidechain` with [`Taps::read`] (falling back to `Node::process` when it
//!   returns `None`).
//!
//! Everything here is a placeholder until the node implements it: no taps, no extra
//! latency, so sidechained devices behave as before.

use crate::config::EngineConfig;
use crate::graph::{NodeInfo, TrackDesc};

/// Per-snapshot sidechain buffers: one stereo buffer per tapped source track, plus the PDC
/// delay per consuming chain entry. Allocated by [`Taps::compile`] (non-RT).
#[derive(Debug, Default)]
pub(crate) struct Taps {
    _private: (),
}

impl Taps {
    /// Non-RT. `tracks[i].chain[k].sidechain` is the source of entry `(i, k)`;
    /// `source_index` resolves a track id to its index; `in_lat`/`out_lat` are the compiled
    /// latencies (samples); `chain_info[i][k]` the node infos.
    pub(crate) fn compile(
        tracks: &[TrackDesc],
        source_index: &dyn Fn(ether_protocol::model::TrackId) -> Option<usize>,
        in_lat: &[u32],
        out_lat: &[u32],
        chain_info: &[Vec<NodeInfo>],
        config: &EngineConfig,
    ) -> Self {
        let _ = (tracks, source_index, in_lat, out_lat, chain_info, config);
        Self::default()
    }

    /// RT. Record track `track`'s post-fader output for this sub-block (`n` frames).
    pub(crate) fn write(&mut self, track: usize, out: &[Vec<f32>; 2], n: usize) {
        let _ = (track, out, n);
    }

    /// RT. The (latency-aligned) sidechain signal for chain entry `(track, entry)` fed by
    /// `source`, or `None` (then the node is processed without a sidechain).
    pub(crate) fn read(
        &mut self,
        track: usize,
        entry: usize,
        source: usize,
        n: usize,
    ) -> Option<[&[f32]; 2]> {
        let _ = (track, entry, source, n);
        None
    }
}

/// Non-RT, called by the compiler for each track in processing order: the minimum input
/// latency track `desc` needs so that no sidechain arrives late (CONTRACTS.md §11.10).
/// `out_lat` already holds the output latency of every track processed before. 0 = no
/// constraint (the placeholder).
pub(crate) fn required_input_latency(
    desc: &TrackDesc,
    chain_info: &[NodeInfo],
    source_index: &dyn Fn(ether_protocol::model::TrackId) -> Option<usize>,
    out_lat: &[u32],
) -> u32 {
    let _ = (desc, chain_info, source_index, out_lat);
    0
}
