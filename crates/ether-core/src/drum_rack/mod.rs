//! Engine-side drum racks (roadmap v2; see CONTRACTS.md §11.12).
//!
//! Wired into `graph.rs`/`engine.rs`/`mixer.rs`, so this node only edits
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
//! - **Routing.** Note-ons route by key (transposed to `PAD_PLAY_NOTE`; other keys are
//!   dropped), raw MIDI the same way (note-on/off and poly aftertouch by key; channel-wide
//!   messages such as CC, pitch bend and channel pressure go to every pad). A note-off or
//!   choke goes to the pad its note-on went to, even if the pad's note changed meanwhile
//!   (remembered per rack, [`MAX_HELD`] notes, else routed by key). `AllNotesOff`
//!   reaches every pad chain, from [`RacksRt::begin_block`] (transport) and from the rack
//!   entry's events (a device before the rack switched on or off). A device switched
//!   on or off on a pad releases its notes like on a track chain (`ChainRt::switch_notes`). Pad chains are processed like a
//!   track chain (bypass, reset, note/MIDI output feeding the next device). A pad entry may
//!   be a chain rack (instrument / audio effect / MIDI effect rack, listed in the track's
//!   `chain_racks`): its chains run before its node, as on a track chain.
//! - **Nesting.** A drum rack may sit on an instrument rack's chain (v0.2): `TrackDesc::racks`
//!   lists it too, and `crate::rack_chains` calls [`RacksRt::run_pads`] at its chain entry
//!   (its longest pad chain counts in that chain's latency). No drum rack ever sits under a
//!   drum pad, so pad racks run without drum racks.
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
use crate::engine::NodeTable;
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::{NodeInfo, RackDesc};
use crate::held_notes::HeldNotes;
use crate::mixer::{ChainRt, Stereo, inherit_switches, release_on_midi_fx_change, stereo};
use crate::node::{NodeKey, ProcessContext};
use crate::param::Smoother;
use crate::rack_chains::{ChainRackDesc, ChainRacksRt, entry_latency};
use crate::sidechain::{ScIo, ScPoint};
use crate::transport::TransportInfo;

/// Ramp time of pad volume/pan/mute changes.
pub const PAD_RAMP_MS: f32 = 20.0;
/// Fade-out time of a choked pad before its voices are cut.
pub const CHOKE_FADE_MS: f32 = 3.0;
/// Notes remembered per pad for choking (the most recent ones).
pub const MAX_HITS: usize = 16;

/// Latency (samples) of rack `rack`'s longest pad chain (0 if `rack` is not a rack or has no
/// pads). Unknown pad nodes count 0. A pad entry that is a chain rack (`chain_racks`, v0.2)
/// counts its longest chain too.
pub(crate) fn pad_latency(
    racks: &[RackDesc],
    chain_racks: &[ChainRackDesc],
    rack: NodeKey,
    node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
) -> u32 {
    nested_pad_latency(racks, chain_racks, rack, node_info, 0)
}

/// [`pad_latency`] of a drum rack at rack nesting `depth` (a drum rack on an instrument
/// rack's chain: `crate::rack_chains::entry_latency`).
pub(crate) fn nested_pad_latency(
    racks: &[RackDesc],
    chain_racks: &[ChainRackDesc],
    rack: NodeKey,
    node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
    depth: usize,
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
                        .map(|e| entry_latency(chain_racks, racks, e.node, node_info, depth + 1))
                        .fold(0u32, u32::saturating_add)
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

/// Sounding notes remembered per rack, so their note-offs find their pad.
pub const MAX_HELD: usize = 128;
/// Note id under which raw MIDI note-ons (which have none) are remembered.
const RAW_NOTE_ID: u32 = u32::MAX;

/// RT. Push `kind` at `offset` into the pad chain's first entry. Returns `true` on overflow.
fn push_to(pad: &mut PadRt, offset: u32, kind: EventKind) -> bool {
    match pad.chain.first_mut() {
        Some(first) => !first.events.push(ProcessEvent { offset, kind }),
        None => false,
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
    /// Sounding notes and the pad each note-on went to.
    held: HeldNotes<DrumPadId>,
}

impl RackRt {
    /// RT. Route one of the rack entry's events to its pads (see the module docs). Returns
    /// `true` if an event list was full.
    fn route(&mut self, e: &ProcessEvent) -> bool {
        let Self { pads, held, .. } = self;
        let offset = e.offset;
        let mut overflow = false;
        // A note-off or choke: to the pad its note-on went to, else by key.
        let mut end = |note_id: u32, channel: u8, key: u8, kind: EventKind| {
            let mut overflow = false;
            let found = held.off(note_id, channel, key, |id| {
                if let Some(pad) = pads.iter_mut().find(|p| p.id == id) {
                    overflow |= push_to(pad, offset, kind);
                }
            });
            if !found {
                for pad in pads.iter_mut().filter(|p| p.note == key) {
                    overflow |= push_to(pad, offset, kind);
                }
            }
            overflow
        };
        match e.kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => {
                let kind = EventKind::NoteOn {
                    note_id,
                    channel,
                    key: PAD_PLAY_NOTE,
                    velocity,
                };
                for pad in pads.iter_mut().filter(|p| p.note == key) {
                    held.on(note_id, channel, key, pad.id);
                    overflow |= push_to(pad, offset, kind);
                }
            }
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                velocity,
            } => {
                let kind = EventKind::NoteOff {
                    note_id,
                    channel,
                    key: PAD_PLAY_NOTE,
                    velocity,
                };
                overflow |= end(note_id, channel, key, kind);
            }
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => {
                let kind = EventKind::NoteChoke {
                    note_id,
                    channel,
                    key: PAD_PLAY_NOTE,
                };
                overflow |= end(note_id, channel, key, kind);
            }
            EventKind::Midi { data } => {
                let (channel, key) = (data[0] & 0x0f, data[1]);
                let kind = EventKind::Midi {
                    data: [data[0], PAD_PLAY_NOTE, data[2]],
                };
                match data[0] & 0xf0 {
                    0x90 if data[2] > 0 => {
                        for pad in pads.iter_mut().filter(|p| p.note == key) {
                            held.on(RAW_NOTE_ID, channel, key, pad.id);
                            overflow |= push_to(pad, offset, kind);
                        }
                    }
                    0x80 | 0x90 => overflow |= end(RAW_NOTE_ID, channel, key, kind),
                    0xa0 => {
                        for pad in pads.iter_mut().filter(|p| p.note == key) {
                            overflow |= push_to(pad, offset, kind);
                        }
                    }
                    // Channel-wide messages: every pad.
                    _ => {
                        for pad in pads.iter_mut() {
                            overflow |= push_to(pad, offset, e.kind);
                        }
                    }
                }
            }
            EventKind::AllNotesOff => {
                held.clear();
                for pad in pads.iter_mut() {
                    overflow |= push_to(pad, offset, e.kind);
                }
            }
            // The rack node's own params; per-note expressions don't reach pads.
            _ => {}
        }
        overflow
    }
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
    /// `node_info` includes the planned sidechain alignment delays (`crate::sidechain`);
    /// `has_sc(node)`: the pad entry `node` reads a sidechain.
    pub(crate) fn compile(
        racks: &[RackDesc],
        chain_racks: &[ChainRackDesc],
        node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
        has_sc: &dyn Fn(NodeKey) -> bool,
        config: &EngineConfig,
    ) -> Self {
        let frames = config.max_block_size;
        let sr = config.sample_rate.max(1) as f32;
        let all = racks;
        let racks = racks
            .iter()
            .map(|r| {
                let longest = pad_latency(all, chain_racks, r.rack, node_info);
                let pads = r
                    .pads
                    .iter()
                    .map(|p| {
                        // Latency this pad adds (bypassed entries delay their input by
                        // theirs, like on a track chain).
                        let lat: u32 = p
                            .chain
                            .iter()
                            .map(|e| entry_latency(chain_racks, all, e.node, node_info, 1))
                            .fold(0, u32::saturating_add);
                        let [gl, gr] = pad_gains(p.volume, p.pan, p.mute);
                        PadRt {
                            id: p.pad,
                            note: p.note,
                            group: p.choke_group,
                            chain: p
                                .chain
                                .iter()
                                .map(|e| {
                                    ChainRt::new(
                                        e.node,
                                        e.enabled,
                                        node_info(e.node).map_or((2, 2), |i| i.channels),
                                        has_sc(e.node),
                                        entry_latency(chain_racks, all, e.node, node_info, 1)
                                            as usize,
                                        config,
                                    )
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
                    held: HeldNotes::with_capacity(MAX_HELD),
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
    /// targets), choke state, remembered hits and the rack's sounding notes. Only swaps; never allocates or frees.
    pub(crate) fn inherit(&mut self, old: &mut RacksRt) {
        for rack in &mut self.racks {
            let Some(o) = old.racks.iter_mut().find(|o| o.key == rack.key) else {
                continue;
            };
            rack.input_delay.inherit(&mut o.input_delay);
            std::mem::swap(&mut rack.held, &mut o.held);
            for pad in &mut rack.pads {
                let Some(op) = o.pads.iter_mut().find(|op| op.id == pad.id) else {
                    continue;
                };
                release_on_midi_fx_change(&mut pad.chain, &op.chain);
                inherit_switches(&mut pad.chain, &mut op.chain);
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

    /// RT. Forget device on/off automation overrides (re-evaluated every sub-block).
    pub(crate) fn clear_auto_enabled(&mut self) {
        for e in self
            .racks
            .iter_mut()
            .flat_map(|r| r.pads.iter_mut())
            .flat_map(|p| p.chain.iter_mut())
        {
            e.clear_auto();
        }
    }

    pub(crate) fn entry_mut(&mut self, node: NodeKey) -> Option<&mut ChainRt> {
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
    /// `sc`: the track's sidechain state (pad entries reading a sidechain, tapped pad
    /// points, `crate::sidechain`). Returns `true` if an event buffer overflowed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_pads(
        &mut self,
        rack: NodeKey,
        nodes: &mut NodeTable<'_>,
        events: &[ProcessEvent],
        info: &TransportInfo,
        sample_rate: f32,
        input: &mut [Vec<f32>; 2],
        n: usize,
        reset: bool,
        sc: &mut ScIo<'_>,
        chain_racks: &mut ChainRacksRt,
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
        if reset {
            r.held.clear();
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
        if r.has_groups {
            // Chokes go in before this block's notes (stable sort keeps them first at equal
            // offsets).
            for pad in r.pads.iter_mut() {
                overflow |= pad.plan_chokes(n, fade_len);
            }
        }
        // Route the rack's notes to the pads.
        for e in events {
            overflow |= r.route(e);
        }
        for pad in r.pads.iter_mut() {
            pad.a[0][..n].fill(0.0);
            pad.a[1][..n].fill(0.0);
            sc.points.write(ScPoint::ChainIn(pad.id.0.0), &pad.a, n);
            for k in 0..pad.chain.len() {
                let (head, tail) = pad.chain.split_at_mut(k + 1);
                let entry = &mut head[k];
                // Off = MIDI thru, like on a track chain (`ChainRt::route_midi`): its notes
                // reach the next entry. One fading out runs without events.
                overflow |= entry.route_midi(tail.first_mut().map(|e| &mut e.events));
                overflow |= entry.events.overflowed();
                // The bypass path keeps the input (`ChainRt::blend`).
                entry.push_dry(&pad.a, n);
                sc.taps.align_main(entry.key, &mut pad.a, n);
                let runs = entry.runs();
                // A chain rack on the pad (v0.2): its chains feed its node's input first,
                // like at a track-chain entry (`crate::rack_chains`).
                let nested = runs && !chain_racks.is_empty() && chain_racks.is_rack(entry.key);
                if nested {
                    overflow |= chain_racks.run(
                        entry.key,
                        nodes,
                        entry.events.as_slice(),
                        &mut pad.out_events,
                        info,
                        sample_rate,
                        &mut pad.a,
                        n,
                        reset,
                        sc,
                        // No drum rack under a drum pad (`ether_model::rack`).
                        None,
                        // Pad chains run on the track's job thread, one after another.
                        &crate::parallel::SerialFork,
                    );
                }
                let Some(node) = nodes.get(entry.key) else {
                    sc.points.write(ScPoint::After(entry.key), &pad.a, n);
                    continue;
                };
                if reset {
                    node.reset();
                }
                if !runs {
                    entry.bypass(&mut pad.a, n);
                    sc.points.write(ScPoint::After(entry.key), &pad.a, n);
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
                    let key = entry.key;
                    match entry
                        .sidechain
                        .then(|| sc.taps.read(key, sc.points, n))
                        .flatten()
                    {
                        Some(side) => node.process_sidechain(&mut ctx, &mut buffers, &side),
                        None => node.process(&mut ctx, &mut buffers),
                    };
                }
                overflow |= pad.out_events.overflowed();
                if nested {
                    chain_racks.finish(entry.key, &mut pad.out_events);
                }
                // Fading out: its MIDI goes nowhere (its input went thru).
                entry.gate_output(&mut pad.out_events);
                match n_out {
                    0 => {}
                    1 => {
                        let [bl, br] = &mut pad.b;
                        br[..n].copy_from_slice(&bl[..n]);
                        entry.blend(&mut pad.b, n);
                        std::mem::swap(&mut pad.a, &mut pad.b);
                    }
                    _ => {
                        entry.blend(&mut pad.b, n);
                        std::mem::swap(&mut pad.a, &mut pad.b);
                    }
                }
                sc.points.write(ScPoint::After(entry.key), &pad.a, n);
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
            sc.points.write(ScPoint::ChainOut(pad.id.0.0), &pad.a, n);
            // Mix: smoothed pad gain × choke fade.
            let [gl, gr] = &mut pad.gain;
            let [sl, sr] = &pad.a;
            let [dl, dr] = input;
            let mut mix_tap = sc.points.buf_mut(ScPoint::ChainMix(pad.id.0.0));
            let mut next_action = 0;
            for k in 0..n {
                while let Some(&(offset, action)) = pad.actions.get(next_action)
                    && offset <= k
                {
                    pad.choke.apply(action, fade_len);
                    next_action += 1;
                }
                let c = pad.choke.tick(fade_len);
                let (vl, vr) = (sl[k] * gl.tick() * c, sr[k] * gr.tick() * c);
                dl[k] += vl;
                dr[k] += vr;
                if let Some([ml, mr]) = mix_tap.as_deref_mut() {
                    ml[k] = vl;
                    mr[k] = vr;
                }
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
