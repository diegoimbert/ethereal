//! Engine-side drum racks (roadmap v2, owned by the `drum-rack` node; see
//! `docs/ROADMAP.md`).
//!
//! Wired into `graph.rs`/`engine.rs`/`mixer.rs` (base-17/base-24), so this node only edits
//! this module:
//! - `graph::compile_with` builds a [`RacksRt`] per track from `TrackDesc::racks`
//!   ([`RacksRt::compile`]), indexes every pad-chain node (`SnapshotRt::pad_index`) and adds
//!   [`pad_latency`] (the longest pad chain) to the rack entry's latency (PDC).
//! - Live param changes ([`RacksRt::pending_mut`]), automation ([`RacksRt::events_mut`]) and
//!   the latency refresh (`SnapshotRt::pad_index`) reach pad-chain nodes like chain nodes.
//! - Per sub-block, [`RacksRt::begin_block`] moves pending params into the pad chains'
//!   events (with the chain's), then at every rack chain entry `engine.rs` calls
//!   [`RacksRt::run_pads`] before processing the rack node.
//!
//! Implemented (base-24, basic): note routing by key (transposed to `PAD_PLAY_NOTE`),
//! `AllNotesOff`/raw MIDI to every pad, pad chains processed like a track chain (bypass,
//! reset, note/MIDI output feeding the next device), PDC alignment of every pad to the
//! longest pad chain (the incoming chain audio is delayed the same), pad volume/pan/mute
//! (constant gain, no smoothing yet). **Left to the drum-rack node:** choke groups,
//! smoothing of pad mix changes, carrying delay/voice state across snapshot swaps
//! ([`RacksRt::inherit`]). RT rules apply; must compile on wasm32.

use ether_protocol::model::PAD_PLAY_NOTE;

use crate::buffer::AudioBuffers;
use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::engine::NodeSlot;
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::{NodeInfo, RackDesc};
use crate::mixer::{ChainRt, MAX_PENDING_EVENTS, Stereo, stereo};
use crate::node::{NodeKey, ProcessContext};
use crate::transport::TransportInfo;

/// Latency (samples) of rack `rack`'s longest pad chain (0 if `rack` is not a rack or has no
/// pads). Unknown pad nodes count 0.
pub(crate) fn pad_latency(
    racks: &[RackDesc],
    rack: NodeKey,
    node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
) -> u32 {
    racks
        .iter()
        .find(|r| r.rack == rack)
        .map(|r| {
            r.pads
                .iter()
                .map(|p| {
                    p.chain
                        .iter()
                        .map(|e| node_info(e.node).map_or(0, |i| i.latency))
                        .sum::<u32>()
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

#[derive(Debug)]
struct PadRt {
    note: u8,
    chain: Vec<ChainRt>,
    /// Linear gains (volume × pan × mute), left/right.
    gain: [f32; 2],
    /// Aligns this pad with the longest pad chain.
    delay: DelayLine,
    a: Stereo,
    b: Stereo,
    out_events: EventBuffer,
}

#[derive(Debug)]
struct RackRt {
    key: NodeKey,
    pads: Vec<PadRt>,
    /// Delays the chain audio entering the rack like the pads.
    input_delay: DelayLine,
}

/// Runtime state of a track's racks.
#[derive(Debug, Default)]
pub(crate) struct RacksRt {
    racks: Vec<RackRt>,
}

impl RacksRt {
    /// Non-RT. Preallocates every pad buffer and event list.
    pub(crate) fn compile(
        racks: &[RackDesc],
        node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
        config: &EngineConfig,
    ) -> Self {
        let frames = config.max_block_size;
        let racks = racks
            .iter()
            .map(|r| {
                let longest = pad_latency(std::slice::from_ref(r), r.rack, node_info);
                let pads = r
                    .pads
                    .iter()
                    .map(|p| {
                        let lat: u32 = p
                            .chain
                            .iter()
                            .map(|e| node_info(e.node).map_or(0, |i| i.latency))
                            .sum();
                        let pan = p.pan.clamp(-1.0, 1.0);
                        let g = if p.mute { 0.0 } else { p.volume.max(0.0) };
                        PadRt {
                            note: p.note,
                            chain: p
                                .chain
                                .iter()
                                .map(|e| ChainRt {
                                    key: e.node,
                                    enabled: e.enabled,
                                    channels: node_info(e.node).map_or((2, 2), |i| i.channels),
                                    events: EventBuffer::with_capacity(config.max_events_per_block),
                                    pending: EventBuffer::with_capacity(MAX_PENDING_EVENTS),
                                    sidechain: None,
                                })
                                .collect(),
                            gain: [g * (1.0 - pan).min(1.0), g * (1.0 + pan).min(1.0)],
                            delay: DelayLine::new((longest - lat) as usize),
                            a: stereo(frames),
                            b: stereo(frames),
                            out_events: EventBuffer::with_capacity(config.max_events_per_block),
                        }
                    })
                    .collect();
                RackRt {
                    key: r.rack,
                    pads,
                    input_delay: DelayLine::new(longest as usize),
                }
            })
            .collect();
        Self { racks }
    }

    /// RT. Carry running state from the previous snapshot (placeholder: none carried yet).
    pub(crate) fn inherit(&mut self, old: &mut RacksRt) {
        let _ = old;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.racks.is_empty()
    }

    /// RT. Whether chain node `key` is a rack of this track.
    pub(crate) fn is_rack(&self, key: NodeKey) -> bool {
        self.racks.iter().any(|r| r.key == key)
    }

    fn entry_mut(&mut self, node: NodeKey) -> Option<&mut ChainRt> {
        self.racks
            .iter_mut()
            .flat_map(|r| r.pads.iter_mut())
            .flat_map(|p| p.chain.iter_mut())
            .find(|e| e.key == node)
    }

    /// RT. Live param changes for a pad-chain node (next sub-block).
    pub(crate) fn pending_mut(&mut self, node: NodeKey) -> Option<&mut EventBuffer> {
        self.entry_mut(node).map(|e| &mut e.pending)
    }

    /// RT. This sub-block's events of a pad-chain node (automation).
    pub(crate) fn events_mut(&mut self, node: NodeKey) -> Option<&mut EventBuffer> {
        self.entry_mut(node).map(|e| &mut e.events)
    }

    /// RT. Start of a sub-block: pending live params become events (and `AllNotesOff`).
    pub(crate) fn begin_block(&mut self, all_notes_off: bool) {
        for pad in self.racks.iter_mut().flat_map(|r| r.pads.iter_mut()) {
            for c in pad.chain.iter_mut() {
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
    }

    /// RT. Run rack `rack`'s pads for `n` frames and mix them into `input` (the rack
    /// node's input). `events` are the rack entry's sorted events; `reset` = transport jump.
    /// Returns `true` if an event buffer overflowed.
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
    ) -> bool {
        let Some(r) = self.racks.iter_mut().find(|r| r.key == rack) else {
            return false;
        };
        let mut overflow = false;
        {
            let [l, rr] = input;
            r.input_delay.process(&mut l[..n], &mut rr[..n]);
        }
        for pad in r.pads.iter_mut() {
            // Route the rack's notes to this pad.
            if let Some(first) = pad.chain.first_mut() {
                for e in events {
                    let kind = match e.kind {
                        EventKind::NoteOn {
                            note_id,
                            channel,
                            key,
                            velocity,
                        } if key == pad.note => EventKind::NoteOn {
                            note_id,
                            channel,
                            key: PAD_PLAY_NOTE,
                            velocity,
                        },
                        EventKind::NoteOff {
                            note_id,
                            channel,
                            key,
                            velocity,
                        } if key == pad.note => EventKind::NoteOff {
                            note_id,
                            channel,
                            key: PAD_PLAY_NOTE,
                            velocity,
                        },
                        EventKind::NoteChoke {
                            note_id,
                            channel,
                            key,
                        } if key == pad.note => EventKind::NoteChoke {
                            note_id,
                            channel,
                            key: PAD_PLAY_NOTE,
                        },
                        k @ (EventKind::AllNotesOff | EventKind::Midi { .. }) => k,
                        _ => continue,
                    };
                    overflow |= !first.events.push(ProcessEvent {
                        offset: e.offset,
                        kind,
                    });
                }
            }
            pad.a[0][..n].fill(0.0);
            pad.a[1][..n].fill(0.0);
            for k in 0..pad.chain.len() {
                let (head, tail) = pad.chain.split_at_mut(k + 1);
                let entry = &mut head[k];
                entry.events.sort();
                overflow |= entry.events.overflowed();
                let Some(node) = NodeSlot::get(nodes, entry.key) else {
                    continue;
                };
                if reset {
                    node.reset();
                }
                if !entry.enabled {
                    continue;
                }
                pad.out_events.clear();
                let (n_in, n_out) = (
                    (entry.channels.0 as usize).min(2),
                    (entry.channels.1 as usize).min(2),
                );
                {
                    let [al, ar] = &pad.a;
                    let [bl, br] = &mut pad.b;
                    let ins: [&[f32]; 2] = [&al[..n], &ar[..n]];
                    let mut outs: [&mut [f32]; 2] = [&mut bl[..n], &mut br[..n]];
                    let mut ctx = ProcessContext {
                        sample_rate,
                        frames: n,
                        transport: info,
                        events: entry.events.as_slice(),
                        out_events: &mut pad.out_events,
                    };
                    let mut buffers = AudioBuffers {
                        inputs: &ins[..n_in],
                        outputs: &mut outs[..n_out],
                    };
                    node.process(&mut ctx, &mut buffers);
                }
                overflow |= pad.out_events.overflowed();
                match n_out {
                    0 => {}
                    1 => {
                        let [bl, br] = &mut pad.b;
                        br[..n].copy_from_slice(&bl[..n]);
                        std::mem::swap(&mut pad.a, &mut pad.b);
                    }
                    _ => std::mem::swap(&mut pad.a, &mut pad.b),
                }
                if let Some(next) = tail.first_mut() {
                    for e in pad.out_events.as_slice() {
                        next.events.push(*e);
                    }
                }
            }
            {
                let [l, rr] = &mut pad.a;
                pad.delay.process(&mut l[..n], &mut rr[..n]);
            }
            for ch in 0..2 {
                let g = pad.gain[ch];
                for (d, s) in input[ch][..n].iter_mut().zip(&pad.a[ch][..n]) {
                    *d += s * g;
                }
            }
        }
        overflow
    }
}
