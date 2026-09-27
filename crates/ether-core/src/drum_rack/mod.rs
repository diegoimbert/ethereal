//! Engine-side drum racks (roadmap v2, owned by the `drum-rack` node; see
//! `docs/ROADMAP.md` and CONTRACTS.md §11.12).
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
//! - On a snapshot swap, `TrackRt::inherit` calls [`RacksRt::inherit`].
//!
//! Behaviour:
//! - **Routing.** Notes route by key (transposed to `PAD_PLAY_NOTE`; other keys are
//!   dropped), raw MIDI the same way (note-on/off and poly aftertouch by key; channel-wide
//!   messages such as CC, pitch bend and channel pressure go to every pad), `AllNotesOff`
//!   once per pad chain (from [`RacksRt::begin_block`]). Pad chains are processed like a
//!   track chain (bypass, reset, note/MIDI output feeding the next device).
//! - **PDC.** Every pad is aligned to the longest pad chain; the incoming chain audio is
//!   delayed the same.
//! - **Mix.** Pad volume/pan/mute are smoothed ([`Smoother`], [`PAD_RAMP_MS`]) and the ramps
//!   continue across snapshot swaps, so a mix change never clicks.
//! - **Choke groups.** A note-on for a pad in group `g` chokes every other pad of the rack
//!   in `g`: the choked pad's output fades out over [`CHOKE_FADE_MS`], then its chain gets
//!   a `NoteChoke` for every note it was hit with (up to [`MAX_HITS`]), so voices stop for
//!   good. A hit on a pad that is fading out chokes its old notes right away and restores
//!   its gain.
//! - **Snapshot swaps.** Delay lines, gain ramps, choke state and hit lists are carried
//!   over by rack node and pad id ([`RacksRt::inherit`]; swaps only, never allocates).
//!
//! RT rules apply; compiles on wasm32.

use ether_protocol::model::{DrumPadId, PAD_PLAY_NOTE};

use crate::buffer::AudioBuffers;
use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::engine::NodeSlot;
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::{NodeInfo, RackDesc};
use crate::mixer::{ChainRt, MAX_PENDING_EVENTS, Stereo, stereo};
use crate::node::{NodeKey, ProcessContext};
use crate::param::Smoother;
use crate::transport::TransportInfo;

/// Ramp time of pad volume/pan/mute changes.
pub const PAD_RAMP_MS: f32 = 20.0;
/// Fade-out time of a choked pad before its voices are cut.
pub const CHOKE_FADE_MS: f32 = 3.0;
/// Notes remembered per pad for choking (the most recent ones).
pub const MAX_HITS: usize = 16;

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

/// Route a raw short MIDI message to the pad on `note`: note-on/off and poly aftertouch only
/// if their key is `note` (re-keyed to `PAD_PLAY_NOTE`), channel-wide messages always.
fn midi_route(data: [u8; 3], note: u8) -> Option<[u8; 3]> {
    match data[0] & 0xf0 {
        0x80 | 0x90 | 0xa0 => (data[1] == note).then_some([data[0], PAD_PLAY_NOTE, data[2]]),
        _ => Some(data),
    }
}

/// Linear left/right gains of a pad (volume × balance pan × mute).
fn pad_gains(volume: f32, pan: f32, mute: bool) -> [f32; 2] {
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

/// What happens to a pad at a sample offset of the current block.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    /// Another pad of its choke group was hit.
    Choke,
    /// The pad itself was hit (note id, channel).
    Hit(u32, u8),
}

/// Choke state of a pad (see the module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ChokeState {
    /// Faded out completely: silent until the next hit.
    choked: bool,
    /// Samples left in the fade-out (0 = not fading).
    fade_left: u32,
}

impl ChokeState {
    /// RT. Apply the actions at one sample and return the gain for that sample.
    #[inline]
    fn tick(&mut self, fade_len: u32) -> f32 {
        if self.fade_left > 0 {
            self.fade_left -= 1;
            if self.fade_left == 0 {
                self.choked = true;
            }
            self.fade_left as f32 / fade_len as f32
        } else if self.choked {
            0.0
        } else {
            1.0
        }
    }

    #[inline]
    fn apply(&mut self, action: Action, fade_len: u32) {
        match action {
            Action::Choke => {
                if !self.choked && self.fade_left == 0 {
                    self.fade_left = fade_len;
                }
            }
            Action::Hit(..) => *self = ChokeState::default(),
        }
    }
}

#[derive(Debug)]
struct PadRt {
    id: DrumPadId,
    note: u8,
    group: Option<u8>,
    chain: Vec<ChainRt>,
    /// Smoothed linear gains (volume × pan × mute), left/right.
    gain: [Smoother; 2],
    /// Aligns this pad with the longest pad chain.
    delay: DelayLine,
    a: Stereo,
    b: Stereo,
    out_events: EventBuffer,
    /// This block's choke/hit actions, in offset order (capacity: events per block).
    actions: Vec<(usize, Action)>,
    choke: ChokeState,
    /// Recent hits (note id, channel), oldest first; choked together.
    hits: [(u32, u8); MAX_HITS],
    n_hits: usize,
}

impl PadRt {
    fn remember_hit(&mut self, note_id: u32, channel: u8) {
        if self.n_hits == MAX_HITS {
            self.hits.copy_within(1.., 0);
            self.n_hits -= 1;
        }
        self.hits[self.n_hits] = (note_id, channel);
        self.n_hits += 1;
    }

    /// RT. Choke every remembered note at `offset` (into the first chain entry's events).
    fn choke_hits(&mut self, offset: usize) -> bool {
        let mut overflow = false;
        if let Some(first) = self.chain.first_mut() {
            for &(note_id, channel) in &self.hits[..self.n_hits] {
                overflow |= !first.events.push(ProcessEvent {
                    offset: offset as u32,
                    kind: EventKind::NoteChoke {
                        note_id,
                        channel,
                        key: PAD_PLAY_NOTE,
                    },
                });
            }
        }
        self.n_hits = 0;
        overflow
    }

    /// RT. Walk this block's actions (without changing the gain state) and emit the chain's
    /// `NoteChoke`s: when a fade-out completes, or earlier when the pad is hit mid-fade.
    /// Remembers this block's hits.
    fn plan_chokes(&mut self, n: usize, fade_len: u32) -> bool {
        let mut overflow = false;
        let mut st = self.choke;
        // Sample where the running fade reaches 0 (relative to the block start).
        let mut fade_end = (st.fade_left > 0).then(|| st.fade_left as usize - 1);
        for i in 0..self.actions.len() {
            let (offset, action) = self.actions[i];
            if let Some(end) = fade_end
                && end < offset
            {
                overflow |= self.choke_hits(end);
                st.choked = true;
                fade_end = None;
            }
            match action {
                Action::Choke => {
                    if !st.choked && fade_end.is_none() {
                        fade_end = Some(offset + fade_len as usize - 1);
                    }
                }
                Action::Hit(note_id, channel) => {
                    if fade_end.take().is_some() {
                        overflow |= self.choke_hits(offset);
                    }
                    st.choked = false;
                    self.remember_hit(note_id, channel);
                }
            }
        }
        if let Some(end) = fade_end
            && end < n
        {
            overflow |= self.choke_hits(end);
        }
        overflow
    }
}

#[derive(Debug)]
struct RackRt {
    key: NodeKey,
    pads: Vec<PadRt>,
    /// Delays the chain audio entering the rack like the pads.
    input_delay: DelayLine,
    /// Any pad has a choke group (skip the choke bookkeeping otherwise).
    has_groups: bool,
}

/// Runtime state of a track's racks.
#[derive(Debug, Default)]
pub(crate) struct RacksRt {
    racks: Vec<RackRt>,
    /// Choke fade length in samples.
    fade_len: u32,
}

impl RacksRt {
    /// Non-RT. Preallocates every pad buffer and event list.
    pub(crate) fn compile(
        racks: &[RackDesc],
        node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
        config: &EngineConfig,
    ) -> Self {
        let frames = config.max_block_size;
        let sr = config.sample_rate.max(1) as f32;
        let racks = racks
            .iter()
            .map(|r| {
                let longest = pad_latency(std::slice::from_ref(r), r.rack, node_info);
                let pads = r
                    .pads
                    .iter()
                    .map(|p| {
                        // Latency this pad actually adds: bypassed entries are skipped by
                        // `run_pads` (the rack's total still counts them, like a track chain
                        // counts bypassed devices).
                        let lat: u32 = p
                            .chain
                            .iter()
                            .filter(|e| e.enabled)
                            .map(|e| node_info(e.node).map_or(0, |i| i.latency))
                            .sum();
                        let [gl, gr] = pad_gains(p.volume, p.pan, p.mute);
                        PadRt {
                            id: p.pad,
                            note: p.note,
                            group: p.choke_group,
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
                            gain: [
                                Smoother::new(gl, PAD_RAMP_MS, sr),
                                Smoother::new(gr, PAD_RAMP_MS, sr),
                            ],
                            delay: DelayLine::new(longest.saturating_sub(lat) as usize),
                            a: stereo(frames),
                            b: stereo(frames),
                            out_events: EventBuffer::with_capacity(config.max_events_per_block),
                            actions: Vec::with_capacity(config.max_events_per_block),
                            choke: ChokeState::default(),
                            hits: [(0, 0); MAX_HITS],
                            n_hits: 0,
                        }
                    })
                    .collect();
                RackRt {
                    key: r.rack,
                    pads,
                    input_delay: DelayLine::new(longest as usize),
                    has_groups: r.pads.iter().any(|p| p.choke_group.is_some()),
                }
            })
            .collect();
        Self {
            racks,
            fade_len: ((CHOKE_FADE_MS * 0.001 * sr) as u32).max(1),
        }
    }

    /// RT. Carry running state from the previous snapshot (same rack node, same pad id):
    /// delay lines (when their length is unchanged), gain ramps (continuing towards the new
    /// targets), choke state and remembered hits. Only swaps; never allocates or frees.
    pub(crate) fn inherit(&mut self, old: &mut RacksRt) {
        for rack in &mut self.racks {
            let Some(o) = old.racks.iter_mut().find(|o| o.key == rack.key) else {
                continue;
            };
            rack.input_delay.inherit(&mut o.input_delay);
            for pad in &mut rack.pads {
                let Some(op) = o.pads.iter_mut().find(|op| op.id == pad.id) else {
                    continue;
                };
                pad.delay.inherit(&mut op.delay);
                for (g, og) in pad.gain.iter_mut().zip(&op.gain) {
                    let target = g.target();
                    *g = *og;
                    if og.target() != target {
                        g.set_target(target);
                    }
                }
                pad.choke = op.choke;
                pad.hits = op.hits;
                pad.n_hits = op.n_hits;
            }
        }
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
        let fade_len = self.fade_len.max(1);
        let Some(r) = self.racks.iter_mut().find(|r| r.key == rack) else {
            return false;
        };
        let mut overflow = false;
        {
            let [l, rr] = input;
            r.input_delay.process(&mut l[..n], &mut rr[..n]);
        }
        // Choke groups: collect every pad's hits and chokes for this block (offset order).
        for pad in r.pads.iter_mut() {
            pad.actions.clear();
            if reset {
                pad.choke = ChokeState::default();
                pad.n_hits = 0;
            }
        }
        if r.has_groups {
            for e in events {
                let (note_id, channel, key) = match e.kind {
                    EventKind::NoteOn {
                        note_id,
                        channel,
                        key,
                        ..
                    } => (note_id, channel, key),
                    EventKind::Midi { data } if data[0] & 0xf0 == 0x90 && data[2] > 0 => {
                        (u32::MAX, data[0] & 0x0f, data[1])
                    }
                    _ => continue,
                };
                let Some(hit) = r.pads.iter().position(|p| p.note == key) else {
                    continue;
                };
                let group = r.pads[hit].group;
                for (i, pad) in r.pads.iter_mut().enumerate() {
                    let action = if i == hit {
                        Action::Hit(note_id, channel)
                    } else if group.is_some() && pad.group == group {
                        Action::Choke
                    } else {
                        continue;
                    };
                    if pad.actions.len() < pad.actions.capacity() {
                        pad.actions.push((e.offset as usize, action));
                    } else {
                        overflow = true;
                    }
                }
            }
        }
        for pad in r.pads.iter_mut() {
            if r.has_groups {
                // Chokes go in before this block's notes (stable sort keeps them first at
                // equal offsets).
                overflow |= pad.plan_chokes(n, fade_len);
            }
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
                        EventKind::Midi { data } => match midi_route(data, pad.note) {
                            Some(data) => EventKind::Midi { data },
                            None => continue,
                        },
                        // AllNotesOff reached every pad chain in `begin_block`; params are
                        // the rack node's own.
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
            // Mix: smoothed pad gain × choke fade.
            let [gl, gr] = &mut pad.gain;
            let [sl, sr] = &pad.a;
            let [dl, dr] = input;
            let mut next_action = 0;
            for k in 0..n {
                while let Some(&(offset, action)) = pad.actions.get(next_action)
                    && offset <= k
                {
                    pad.choke.apply(action, fade_len);
                    next_action += 1;
                }
                let c = pad.choke.tick(fade_len);
                dl[k] += sl[k] * gl.tick() * c;
                dr[k] += sr[k] * gr.tick() * c;
            }
        }
        overflow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choke_fade_reaches_zero_and_stays_until_hit() {
        let mut st = ChokeState::default();
        st.apply(Action::Choke, 4);
        let g: Vec<f32> = (0..6).map(|_| st.tick(4)).collect();
        assert_eq!(g, [0.75, 0.5, 0.25, 0.0, 0.0, 0.0]);
        assert!(st.choked);
        st.apply(Action::Hit(1, 0), 4);
        assert_eq!(st.tick(4), 1.0);
    }

    #[test]
    fn gains_follow_balance_pan_and_mute() {
        assert_eq!(pad_gains(0.5, 0.0, false), [0.5, 0.5]);
        assert_eq!(pad_gains(1.0, 1.0, false), [0.0, 1.0]);
        assert_eq!(pad_gains(1.0, -0.5, false), [1.0, 0.5]);
        assert_eq!(pad_gains(1.0, 0.0, true), [0.0, 0.0]);
    }
}
