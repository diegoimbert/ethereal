//! Rack chains: parallel device chains inside instrument / audio-effect / MIDI-effect racks
//! (v0.2, owned by the `racks-modulation` node; model `ether_model::rack`, CONTRACTS.md §12.6).
//!
//! Same engine shape as drum-rack pads (`crate::drum_rack`, pre-wired the same way in
//! `engine.rs`/`graph.rs`): `TrackDesc::chain_racks` lists, per rack node of the track chain,
//! its chains; at the rack's chain entry the engine calls [`ChainRacksRt::run`] **before**
//! the rack node itself, which must route the rack entry's events to the chains (key /
//! velocity / selector zones), run the chains (latency-aligned to the longest), and write
//! their mix into the rack's input (`a`); MIDI effect racks merge the chains' output events
//! into the rack entry's output instead. The rack node (`ether_devices::racks`) then runs as
//! a pass-through (macros are modulation sources, not audio).
//!
//! Pre-wired and working already: chain nodes are indexed (`SnapshotRt::rack_chain_index`),
//! receive live params and automation through [`ChainRacksRt::pending_mut`] /
//! [`ChainRacksRt::events_mut`], and their latency is refreshed. Placeholders: [`run`]
//! (chains are not processed yet) and [`chain_latency`] (0).
//!
//! [`run`]: ChainRacksRt::run

use ether_protocol::model::RackChainId;
use serde::{Deserialize, Serialize};

use crate::config::EngineConfig;
use crate::engine::NodeTable;
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::{ChainEntry, NodeInfo};
use crate::mixer::{ChainRt, MAX_PENDING_EVENTS};
use crate::node::NodeKey;
use crate::transport::TransportInfo;

/// Rack type (signal flow, see `ether_model::rack`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainRackKind {
    Instrument,
    AudioEffect,
    MidiEffect,
}

/// A rack node of a track chain and its chains.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainRackDesc {
    pub rack: NodeKey,
    pub kind: ChainRackKind,
    pub chains: Vec<RackChainDesc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RackChainDesc {
    pub id: RackChainId,
    pub chain: Vec<ChainEntry>,
    /// Linear gain; pan -1..=1. `mute` already includes solo resolution (the controller mutes
    /// non-soloed chains while any chain of the rack is soloed).
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    /// Inclusive zones `(lo, hi)`, 0..=127.
    pub keys: (u8, u8),
    pub velocities: (u8, u8),
    pub select: (u8, u8),
}

/// Latency the rack entry `node` adds on top of the rack node's own (the longest chain).
/// Placeholder: 0.
pub(crate) fn chain_latency(
    racks: &[ChainRackDesc],
    node: NodeKey,
    node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
) -> u32 {
    let _ = (racks, node, node_info);
    0
}

#[derive(Debug)]
struct RackRt {
    key: NodeKey,
    chains: Vec<Vec<ChainRt>>,
}

/// Per-track rack-chain state (in `TrackRt`).
#[derive(Debug, Default)]
pub(crate) struct ChainRacksRt {
    racks: Vec<RackRt>,
}

impl ChainRacksRt {
    /// Non-RT. Preallocates every chain entry's event lists.
    pub(crate) fn compile(
        racks: &[ChainRackDesc],
        node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
        config: &EngineConfig,
    ) -> Self {
        let racks = racks
            .iter()
            .map(|r| RackRt {
                key: r.rack,
                chains: r
                    .chains
                    .iter()
                    .map(|c| {
                        c.chain
                            .iter()
                            .map(|e| ChainRt {
                                key: e.node,
                                enabled: e.enabled,
                                channels: node_info(e.node).map_or((2, 2), |i| i.channels),
                                events: EventBuffer::with_capacity(config.max_events_per_block),
                                pending: EventBuffer::with_capacity(MAX_PENDING_EVENTS),
                                sidechain: None,
                            })
                            .collect()
                    })
                    .collect(),
            })
            .collect();
        Self { racks }
    }

    /// RT. Carry running state (delay lines, gain ramps) over a snapshot swap.
    pub(crate) fn inherit(&mut self, old: &mut ChainRacksRt) {
        let _ = old;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.racks.is_empty()
    }

    /// RT. Whether chain node `key` is a rack with chains on this track.
    pub(crate) fn is_rack(&self, key: NodeKey) -> bool {
        self.racks.iter().any(|r| r.key == key)
    }

    fn entry_mut(&mut self, node: NodeKey) -> Option<&mut ChainRt> {
        self.racks
            .iter_mut()
            .flat_map(|r| r.chains.iter_mut())
            .flat_map(|c| c.iter_mut())
            .find(|e| e.key == node)
    }

    /// RT. Live param changes for a chain node (next sub-block).
    pub(crate) fn pending_mut(&mut self, node: NodeKey) -> Option<&mut EventBuffer> {
        self.entry_mut(node).map(|e| &mut e.pending)
    }

    /// RT. This sub-block's events of a chain node (automation, modulation).
    pub(crate) fn events_mut(&mut self, node: NodeKey) -> Option<&mut EventBuffer> {
        self.entry_mut(node).map(|e| &mut e.events)
    }

    /// RT. Start of a sub-block: pending live params become events (and `AllNotesOff`).
    pub(crate) fn begin_block(&mut self, all_notes_off: bool) {
        for c in self
            .racks
            .iter_mut()
            .flat_map(|r| r.chains.iter_mut())
            .flat_map(|c| c.iter_mut())
        {
            c.events.clear();
            for e in c.pending.as_slice() {
                c.events.push(*e);
            }
            c.pending.clear();
            if all_notes_off {
                c.events.push(ProcessEvent {
                    offset: 0,
                    kind: EventKind::AllNotesOff,
                });
            }
        }
    }

    /// RT. Run rack `rack`'s chains for `n` frames with the rack entry's sorted `events`
    /// and write the result into `input` (see the module docs). `reset` = transport jump.
    /// Returns `true` if an event buffer overflowed. Placeholder: chains are not processed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        rack: NodeKey,
        nodes: &mut NodeTable<'_>,
        events: &[ProcessEvent],
        out_events: &mut EventBuffer,
        info: &TransportInfo,
        sample_rate: f32,
        input: &mut [Vec<f32>; 2],
        n: usize,
        reset: bool,
    ) -> bool {
        let _ = (
            rack,
            nodes,
            events,
            out_events,
            info,
            sample_rate,
            input,
            n,
            reset,
        );
        false
    }
}
