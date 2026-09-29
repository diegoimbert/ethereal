//! Rack chains: parallel device chains inside instrument / audio-effect / MIDI-effect racks
//! (v0.2, owned by the `racks-modulation` node; model `ether_model::rack`, CONTRACTS.md §12.6).
//!
//! Same engine shape as drum-rack pads (`crate::drum_rack`, pre-wired the same way in
//! `engine.rs`/`graph.rs`): `TrackDesc::chain_racks` lists, per rack node of the track chain,
//! its chains; at the rack's chain entry the engine calls [`ChainRacksRt::run`] **before**
//! the rack node itself. The rack node (`ether_devices::racks`) then runs as a pass-through
//! (macros are modulation sources, not audio).
//!
//! Behaviour:
//! - **Routing.** Note-ons (and raw MIDI note-ons) reach a chain when their key is in the
//!   chain's key zone, their velocity (`round(v · 127)`) in its velocity zone and the rack's
//!   current chain-selector value in its selector zone. Note-offs, chokes and every other
//!   raw MIDI message reach every chain (a note-off always finds its note, so a zone or
//!   selector change never leaves a note hanging); `AllNotesOff` reaches every chain from
//!   [`ChainRacksRt::begin_block`]. The rack's own `Param` events are not forwarded; the
//!   selector is read from them at its offset (it may be automated or modulated).
//! - **Signal flow.** Instrument racks: chains start silent, their mix is added to the rack
//!   node's input (which is delayed like the chains). Audio effect racks: every chain starts
//!   from the rack's input and the mix replaces it (no chains = pass-through). MIDI effect
//!   racks: chains get the events, their output events are merged (by offset) and replace
//!   the rack node's MIDI-thru output ([`ChainRacksRt::finish`]); audio is untouched.
//! - **Mix.** Chain volume/pan (balance)/mute and the selector gate are smoothed
//!   ([`CHAIN_RAMP_MS`]); a chain outside the selector zone fades out (and stops receiving
//!   note-ons). `mute` already includes solo resolution (controller).
//! - **PDC.** Every chain is aligned to the longest chain ([`chain_latency`], which the rack
//!   entry reports on top of the rack node's own latency).
//! - **Snapshot swaps.** Delay lines, gain ramps and the selector value carry over by rack
//!   node and chain id ([`ChainRacksRt::inherit`]; swaps only, never allocates).
//!
//! RT rules apply; compiles on wasm32.

use ether_protocol::model::{ParamId, RackChainId};
use serde::{Deserialize, Serialize};

use crate::buffer::AudioBuffers;
use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::engine::NodeTable;
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::{ChainEntry, NodeInfo};
use crate::mixer::{ChainRt, MAX_PENDING_EVENTS, Stereo, stereo};
use crate::node::{NodeKey, ProcessContext};
use crate::param::Smoother;
use crate::transport::TransportInfo;

/// Ramp time of chain volume/pan/mute and selector changes.
pub const CHAIN_RAMP_MS: f32 = 20.0;
/// The chain-selector param of every rack (`ether_model::RACK_SELECTOR_PARAM`).
pub const SELECTOR_PARAM: ParamId = ParamId(8);

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
    /// Chain-selector value when the snapshot is new (the document value; live changes,
    /// automation and modulation arrive as the rack node's `Param` events).
    #[serde(default)]
    pub selector: u8,
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

fn in_zone((lo, hi): (u8, u8), v: u8) -> bool {
    (lo..=hi).contains(&v)
}

/// Latency the rack entry `node` adds on top of the rack node's own (the longest chain;
/// bypassed entries count, like on a track chain). Unknown chain nodes count 0.
pub(crate) fn chain_latency(
    racks: &[ChainRackDesc],
    node: NodeKey,
    node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
) -> u32 {
    racks
        .iter()
        .find(|r| r.rack == node)
        .map(|r| {
            r.chains
                .iter()
                .map(|c| {
                    c.chain
                        .iter()
                        .map(|e| node_info(e.node).map_or(0, |i| i.latency))
                        .sum::<u32>()
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

/// Linear left/right gains (volume × balance pan × mute).
fn chain_gains(volume: f32, pan: f32, mute: bool) -> [f32; 2] {
    let pan = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let g = if mute || !volume.is_finite() {
        0.0
    } else {
        volume.max(0.0)
    };
    [g * (1.0 - pan).min(1.0), g * (1.0 + pan).min(1.0)]
}

#[derive(Debug)]
struct ChainState {
    id: RackChainId,
    entries: Vec<ChainRt>,
    /// Mix gains without the selector gate.
    gains: [f32; 2],
    /// Smoothed linear gains (mix × selector gate), left/right.
    gain: [Smoother; 2],
    keys: (u8, u8),
    velocities: (u8, u8),
    select: (u8, u8),
    delay: DelayLine,
    a: Stereo,
    b: Stereo,
    out_events: EventBuffer,
}

impl ChainState {
    fn target(&self, selector: u8) -> [f32; 2] {
        if in_zone(self.select, selector) {
            self.gains
        } else {
            [0.0, 0.0]
        }
    }
}

#[derive(Debug)]
struct RackRt {
    key: NodeKey,
    kind: ChainRackKind,
    chains: Vec<ChainState>,
    /// Current chain-selector value.
    selector: u8,
    /// Instrument racks: delays the audio entering the rack like the chains.
    input_delay: DelayLine,
    /// MIDI effect racks: this sub-block's merged chain output (see [`ChainRacksRt::finish`]).
    merged: EventBuffer,
    /// Audio effect racks: the rack's input, copied into each chain.
    dry: Stereo,
}

/// Per-track rack-chain state (in `TrackRt`).
#[derive(Debug, Default)]
pub(crate) struct ChainRacksRt {
    racks: Vec<RackRt>,
}

/// The selector value a `Param` event sets, if it is the rack's selector.
fn selector_event(e: &ProcessEvent) -> Option<u8> {
    match e.kind {
        EventKind::Param { param, value } if param == SELECTOR_PARAM => {
            Some(if value.is_finite() {
                value.round().clamp(0.0, 127.0) as u8
            } else {
                0
            })
        }
        _ => None,
    }
}

/// Whether a note-on (key, 0..=127 velocity) is a "start" event filtered by zones.
fn note_start(kind: &EventKind) -> Option<(u8, u8)> {
    match *kind {
        EventKind::NoteOn { key, velocity, .. } => {
            let v = if velocity.is_finite() {
                (velocity * 127.0).round().clamp(0.0, 127.0) as u8
            } else {
                0
            };
            Some((key, v))
        }
        EventKind::Midi { data } if data[0] & 0xf0 == 0x90 && data[2] > 0 => {
            Some((data[1], data[2]))
        }
        _ => None,
    }
}

impl ChainRacksRt {
    /// Non-RT. Preallocates every chain buffer and event list.
    pub(crate) fn compile(
        racks: &[ChainRackDesc],
        node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
        config: &EngineConfig,
    ) -> Self {
        let frames = config.max_block_size;
        let sr = config.sample_rate.max(1) as f32;
        let racks = racks
            .iter()
            .map(|r| {
                let longest = chain_latency(std::slice::from_ref(r), r.rack, node_info);
                let chains = r
                    .chains
                    .iter()
                    .map(|c| {
                        // Latency this chain actually adds (bypassed entries are skipped).
                        let lat: u32 = c
                            .chain
                            .iter()
                            .filter(|e| e.enabled)
                            .map(|e| node_info(e.node).map_or(0, |i| i.latency))
                            .sum();
                        let gains = chain_gains(c.volume, c.pan, c.mute);
                        let mut st = ChainState {
                            id: c.id,
                            entries: c
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
                            gains,
                            gain: [
                                Smoother::new(0.0, CHAIN_RAMP_MS, sr),
                                Smoother::new(0.0, CHAIN_RAMP_MS, sr),
                            ],
                            keys: c.keys,
                            velocities: c.velocities,
                            select: c.select,
                            delay: DelayLine::new(longest.saturating_sub(lat) as usize),
                            a: stereo(frames),
                            b: stereo(frames),
                            out_events: EventBuffer::with_capacity(config.max_events_per_block),
                        };
                        let [gl, gr] = st.target(r.selector);
                        st.gain[0].set_immediate(gl);
                        st.gain[1].set_immediate(gr);
                        st
                    })
                    .collect();
                RackRt {
                    key: r.rack,
                    kind: r.kind,
                    chains,
                    selector: r.selector.min(127),
                    input_delay: DelayLine::new(if r.kind == ChainRackKind::Instrument {
                        longest as usize
                    } else {
                        0
                    }),
                    merged: EventBuffer::with_capacity(config.max_events_per_block),
                    dry: if r.kind == ChainRackKind::AudioEffect {
                        stereo(frames)
                    } else {
                        [Vec::new(), Vec::new()]
                    },
                }
            })
            .collect();
        Self { racks }
    }

    /// RT. Carry running state over a snapshot swap (same rack node, same chain id): the
    /// selector value, delay lines (same length) and gain ramps (continuing towards the new
    /// targets). Only swaps; never allocates or frees.
    pub(crate) fn inherit(&mut self, old: &mut ChainRacksRt) {
        for rack in &mut self.racks {
            let Some(o) = old.racks.iter_mut().find(|o| o.key == rack.key) else {
                continue;
            };
            rack.selector = o.selector;
            rack.input_delay.inherit(&mut o.input_delay);
            let selector = rack.selector;
            for chain in &mut rack.chains {
                let target = chain.target(selector);
                match o.chains.iter_mut().find(|oc| oc.id == chain.id) {
                    Some(oc) => {
                        chain.delay.inherit(&mut oc.delay);
                        for (g, (og, t)) in chain.gain.iter_mut().zip(oc.gain.iter().zip(target)) {
                            *g = *og;
                            if g.target() != t {
                                g.set_target(t);
                            }
                        }
                    }
                    None => {
                        // A new chain fades in.
                        for (g, t) in chain.gain.iter_mut().zip(target) {
                            g.set_immediate(0.0);
                            g.set_target(t);
                        }
                    }
                }
            }
        }
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
            .flat_map(|c| c.entries.iter_mut())
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
        for r in self.racks.iter_mut() {
            r.merged.clear();
            for c in r.chains.iter_mut().flat_map(|c| c.entries.iter_mut()) {
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

    /// RT. Run rack `rack`'s chains for `n` frames with the rack entry's sorted `events`
    /// and write the result into `input` (see the module docs). `reset` = transport jump.
    /// Returns `true` if an event buffer overflowed. `out_events` is unused: the engine
    /// clears it before the rack node runs; MIDI output is handed over by [`Self::finish`].
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
        let _ = out_events;
        let Some(r) = self.racks.iter_mut().find(|r| r.key == rack) else {
            return false;
        };
        if r.chains.is_empty() {
            // Selector changes still count (for chains added later).
            if let Some(s) = events.iter().rev().find_map(selector_event) {
                r.selector = s;
            }
            return false;
        }
        let mut overflow = false;
        let kind = r.kind;
        let start_selector = r.selector;
        match kind {
            ChainRackKind::Instrument => {
                let [l, rr] = input;
                r.input_delay.process(&mut l[..n], &mut rr[..n]);
            }
            ChainRackKind::AudioEffect => {
                for (dry, inp) in r.dry.iter_mut().zip(input.iter_mut()) {
                    dry[..n].copy_from_slice(&inp[..n]);
                    inp[..n].fill(0.0);
                }
            }
            ChainRackKind::MidiEffect => {}
        }
        for chain in r.chains.iter_mut() {
            // Route the rack's events to this chain.
            if let Some(first) = chain.entries.first_mut() {
                let mut selector = start_selector;
                for e in events {
                    if let Some(s) = selector_event(e) {
                        selector = s;
                        continue;
                    }
                    match e.kind {
                        EventKind::Param { .. } | EventKind::AllNotesOff => continue,
                        ref k => {
                            if let Some((key, vel)) = note_start(k)
                                && !(in_zone(chain.keys, key)
                                    && in_zone(chain.velocities, vel)
                                    && in_zone(chain.select, selector))
                            {
                                continue;
                            }
                        }
                    }
                    overflow |= !first.events.push(*e);
                }
            }
            if kind == ChainRackKind::AudioEffect {
                chain.a[0][..n].copy_from_slice(&r.dry[0][..n]);
                chain.a[1][..n].copy_from_slice(&r.dry[1][..n]);
            } else {
                chain.a[0][..n].fill(0.0);
                chain.a[1][..n].fill(0.0);
            }
            let midi = kind == ChainRackKind::MidiEffect;
            let last = chain.entries.len().saturating_sub(1);
            for k in 0..chain.entries.len() {
                let (head, tail) = chain.entries.split_at_mut(k + 1);
                let entry = &mut head[k];
                entry.events.sort();
                overflow |= entry.events.overflowed();
                let Some(node) = nodes.get(entry.key) else {
                    continue;
                };
                if reset {
                    node.reset();
                }
                chain.out_events.clear();
                if !entry.enabled {
                    // A bypassed MIDI effect passes its notes on.
                    if midi {
                        for e in entry.events.as_slice() {
                            if !matches!(e.kind, EventKind::Param { .. }) {
                                chain.out_events.push(*e);
                            }
                        }
                    }
                } else {
                    let (n_in, n_out) = (
                        (entry.channels.0 as usize).min(2),
                        (entry.channels.1 as usize).min(2),
                    );
                    {
                        let [al, ar] = &chain.a;
                        let [bl, br] = &mut chain.b;
                        let ins: [&[f32]; 2] = [&al[..n], &ar[..n]];
                        let mut outs: [&mut [f32]; 2] = [&mut bl[..n], &mut br[..n]];
                        let mut ctx = ProcessContext {
                            sample_rate,
                            frames: n,
                            transport: info,
                            events: entry.events.as_slice(),
                            out_events: &mut chain.out_events,
                        };
                        let mut buffers = AudioBuffers {
                            inputs: &ins[..n_in],
                            outputs: &mut outs[..n_out],
                        };
                        node.process(&mut ctx, &mut buffers);
                    }
                    overflow |= chain.out_events.overflowed();
                    match n_out {
                        0 => {}
                        1 => {
                            let [bl, br] = &mut chain.b;
                            br[..n].copy_from_slice(&bl[..n]);
                            std::mem::swap(&mut chain.a, &mut chain.b);
                        }
                        _ => std::mem::swap(&mut chain.a, &mut chain.b),
                    }
                }
                if let Some(next) = tail.first_mut() {
                    for e in chain.out_events.as_slice() {
                        next.events.push(*e);
                    }
                } else if midi && k == last {
                    for e in chain.out_events.as_slice() {
                        overflow |= !r.merged.push(*e);
                    }
                }
            }
            if midi {
                continue;
            }
            {
                let [l, rr] = &mut chain.a;
                chain.delay.process(&mut l[..n], &mut rr[..n]);
            }
            // Mix: smoothed gains, re-targeted at selector changes.
            let [gl, gr] = &mut chain.gain;
            let [sl, sr] = &chain.a;
            let [dl, dr] = &mut *input;
            let mut selector = start_selector;
            let mut next = 0;
            for k in 0..n {
                while let Some(e) = events.get(next)
                    && e.offset as usize <= k
                {
                    next += 1;
                    if let Some(s) = selector_event(e)
                        && s != selector
                    {
                        let was = in_zone(chain.select, selector);
                        selector = s;
                        if was != in_zone(chain.select, s) {
                            let t = if in_zone(chain.select, s) {
                                chain.gains
                            } else {
                                [0.0, 0.0]
                            };
                            gl.set_target(t[0]);
                            gr.set_target(t[1]);
                        }
                    }
                }
                dl[k] += sl[k] * gl.tick();
                dr[k] += sr[k] * gr.tick();
            }
        }
        if let Some(s) = events.iter().rev().find_map(selector_event) {
            r.selector = s;
        }
        if kind == ChainRackKind::MidiEffect {
            r.merged.sort();
        }
        overflow
    }

    /// RT. After the rack node `rack` processed: a MIDI effect rack with chains replaces its
    /// node's MIDI-thru output with the chains' merged output. No-op otherwise.
    pub(crate) fn finish(&mut self, rack: NodeKey, out_events: &mut EventBuffer) {
        let Some(r) = self
            .racks
            .iter()
            .find(|r| r.key == rack && r.kind == ChainRackKind::MidiEffect)
        else {
            return;
        };
        if r.chains.is_empty() {
            return;
        }
        out_events.clear();
        for e in r.merged.as_slice() {
            out_events.push(*e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains_follow_balance_pan_and_mute() {
        assert_eq!(chain_gains(0.5, 0.0, false), [0.5, 0.5]);
        assert_eq!(chain_gains(1.0, 1.0, false), [0.0, 1.0]);
        assert_eq!(chain_gains(1.0, 0.0, true), [0.0, 0.0]);
    }

    #[test]
    fn note_starts_carry_key_and_velocity() {
        let on = EventKind::NoteOn {
            note_id: 1,
            channel: 0,
            key: 60,
            velocity: 1.0,
        };
        assert_eq!(note_start(&on), Some((60, 127)));
        assert_eq!(
            note_start(&EventKind::Midi {
                data: [0x90, 40, 0]
            }),
            None
        );
        assert_eq!(
            note_start(&EventKind::Midi {
                data: [0x91, 40, 30]
            }),
            Some((40, 30))
        );
    }
}
