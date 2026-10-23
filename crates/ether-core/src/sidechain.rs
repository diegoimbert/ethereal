//! Sidechain routing in the engine (roadmap v2; see CONTRACTS.md §11.10). v0.4: sidechained entries at any depth (drum
//! pads, rack chains, nested racks) and tap points other than the post-fader output.
//!
//! - **Consumers.** Any chain entry with `ChainEntry::sidechain` — on the track chain, a pad
//!   chain or a rack chain (any depth). Its tap point is listed in `TrackDesc::sidechain_taps`
//!   ([`ScPoint`]); unlisted = the source's post-fader output (the v2 meaning).
//! - **Order.** `graph::compile_with` orders every other-track source before its consumer
//!   track (an ordering edge, not a bus). A source on the consumer's own track must be
//!   produced earlier in the same job ([`plan`] walks the track in processing order: a point
//!   not yet produced when the consumer runs is ignored, so the engine never reads a stale
//!   or future signal; the model rejects those choices up front, `ether_model::SidechainTap`).
//! - **Taps** ([`Points`], per source track, only for tapped points): `Track(PreFx)` before
//!   the chain, `Track(PostFx)` after it, `Track(PostFader)` after the
//!   fader and VCA (before the PDC output delay); `After(node)` = that entry's output (any
//!   depth); `ChainIn/ChainOut/ChainMix` = a rack chain or pad's input, aligned output, and
//!   output after its volume/pan/mute (and choke); `ChainRaw` = a rack chain's output before
//!   the alignment, for another chain of the same rack (run after it).
//! - **PDC** ([`plan`]). Walking the consumer's track in processing order with the latency
//!   `L` of the signal at each entry: at a sidechained entry, `L_sc` = the latency of the
//!   tapped point (final: other tracks were planned first, own points earlier in the walk).
//!   If `L_sc > L`, the *main* signal is delayed by `L_sc - L` just before that entry
//!   ([`Taps::align_main`]) and the delay counts as latency of that entry: on the track chain
//!   it adds to the track's chain latency; inside a pad / rack chain it counts like the
//!   node's own latency (`TrackPlan::extra`, folded into `NodeInfo` for the rack runtimes,
//!   so the chain alignment and the rack entry's latency absorb it). Otherwise the sidechain
//!   is delayed by `L - L_sc` ([`Taps::read`]).
//! - **RT.** Buffers and delay lines are allocated at compile; the audio thread copies,
//!   delays and reads only. Other-track sources are copied at the start of the consumer's
//!   job ([`Taps::gather`], the source finished before it); own-track sources at
//!   the read. Snapshot swaps rebuild the delay lines (a sidechain edit may click once).
//! - **Mute/solo.** They only silence what a track contributes to the mix, never what it feeds a
//!   sidechain: the post-fader tap sits before the mute/solo gate (`engine.rs`), so soloing
//!   the consumer keeps it ducking and a muted "ghost" source keeps keying (CONTRACTS §11.10).

use ether_protocol::model::{InputTap, TrackId};
use serde::{Deserialize, Serialize};

use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::event::{EventBuffer, ProcessEvent};
use crate::graph::{NodeInfo, TrackDesc};
use crate::midi_sidechain::{MidiConsumer, MidiPoint, MidiPoints, MidiTapRt};
use crate::mixer::{Stereo, stereo};
use crate::node::NodeKey;

/// Where a sidechain taps its source track (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ScPoint {
    /// A point of the track itself (`crate::bus_tap` points).
    Track(InputTap),
    /// The output of a chain entry of the track (any depth), e.g. its instrument.
    After(NodeKey),
    /// The input of a rack chain or drum pad of the track (its id's ULID as `u128`).
    ChainIn(u128),
    /// Its output, aligned with the rack's other chains, before its volume/pan/mute.
    ChainOut(u128),
    /// Its output before the alignment (a rack chain's): what another chain of the same rack
    /// keys from (it runs after this one; the aligned output comes after every chain).
    ChainRaw(u128),
    /// Its output after its volume/pan/mute (and a pad's choke).
    ChainMix(u128),
}

impl ScPoint {
    /// The v2 point: the source's post-fader output.
    pub const POST_FADER: ScPoint = ScPoint::Track(InputTap::PostFader);
}

/// `TrackDesc::sidechain_taps`: the tap of the sidechained entry `node` (any depth).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidechainTapDesc {
    pub node: NodeKey,
    pub point: ScPoint,
}

/// A sidechained entry of a track (any depth), resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Consumer {
    pub node: NodeKey,
    /// Source track index.
    pub source: usize,
    pub point: ScPoint,
}

/// Non-RT. The sidechained entries of track `track` (`desc`), sorted by node. Unknown
/// sources and the track's own post-FX / post-fader output (a cycle) are skipped.
pub(crate) fn consumers(
    track: usize,
    desc: &TrackDesc,
    source_index: &dyn Fn(TrackId) -> Option<usize>,
) -> Vec<Consumer> {
    let entries = desc
        .chain
        .iter()
        .chain(
            desc.racks
                .iter()
                .flat_map(|r| &r.pads)
                .flat_map(|p| &p.chain),
        )
        .chain(
            desc.chain_racks
                .iter()
                .flat_map(|r| &r.chains)
                .flat_map(|c| &c.chain),
        );
    let mut out: Vec<Consumer> = entries
        .filter_map(|e| {
            let source = e.sidechain.and_then(source_index)?;
            let point = desc
                .sidechain_taps
                .iter()
                .find(|t| t.node == e.node)
                .map_or(ScPoint::POST_FADER, |t| t.point);
            if source == track
                && matches!(
                    point,
                    ScPoint::Track(InputTap::PostFx | InputTap::PostFader)
                )
            {
                return None;
            }
            Some(Consumer {
                node: e.node,
                source,
                point,
            })
        })
        .collect();
    out.sort_by_key(|c| c.node);
    out.dedup_by_key(|c| c.node);
    out
}

/// One planned sidechained entry: an audio sidechain, a MIDI sidechain
/// (`crate::midi_sidechain`) or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EntryPlan {
    pub node: NodeKey,
    /// Delay of the main signal just before the entry (counted as latency).
    pub main_delay: u32,
    pub audio: Option<AudioPlan>,
    pub midi: Option<MidiPlan>,
}

/// The audio sidechain of a planned entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AudioPlan {
    pub source: usize,
    pub point: ScPoint,
    /// Delay of the tapped sidechain signal.
    pub sc_delay: u32,
}

/// The MIDI sidechain of a planned entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MidiPlan {
    pub source: usize,
    pub point: MidiPoint,
    /// Delay of the source's notes.
    pub delay: u32,
}

/// [`plan`] of one track.
#[derive(Clone, Debug, Default)]
pub(crate) struct TrackPlan {
    /// Sorted by node.
    pub entries: Vec<EntryPlan>,
    /// Main-signal delays before pad / rack chain entries (sorted by node): they count as
    /// those nodes' latency.
    pub extra: Vec<(NodeKey, u32)>,
    /// Latency of every track chain entry: its node's plus, for a rack, its longest pad or
    /// chain (with the `extra` delays inside).
    pub top_latency: Vec<u32>,
    /// Main-signal delays before track chain entries (added to the chain latency).
    pub top_main: u32,
    /// Latency of every point of this track (in walk order).
    pub points: Vec<(ScPoint, u32)>,
    /// Latency of the signal at every node's input (after the main-signal delay planned
    /// before it), any depth, sorted by node: automation of the node is delayed by it so it
    /// stays in time with the audio it affects (`crate::automation_rt`, issue #270).
    pub inputs: Vec<(NodeKey, u32)>,
    /// Latency of the post-fx signal (the fader, sends and VCA gain apply there).
    pub post_fx: u32,
}

impl TrackPlan {
    /// Latency of `point` of this track, if it exists.
    pub(crate) fn point_latency(&self, point: ScPoint) -> Option<u32> {
        self.points.iter().find(|(p, _)| *p == point).map(|e| e.1)
    }

    /// Latency of MIDI `point` of this track (the audio's there, `crate::midi_sidechain`):
    /// the pre-FX latency, a top-level entry's input latency, the post-FX latency.
    pub(crate) fn midi_point_latency(&self, point: MidiPoint) -> Option<u32> {
        match point {
            MidiPoint::Input => self.point_latency(ScPoint::Track(InputTap::PreFx)),
            MidiPoint::Before(node) => self.inputs.iter().find(|e| e.0 == node).map(|e| e.1),
            MidiPoint::End => self.point_latency(ScPoint::Track(InputTap::PostFx)),
        }
    }

    /// The main-signal delay planned before entry `node` (0 if none).
    pub(crate) fn main_delay(&self, node: NodeKey) -> u32 {
        self.entries
            .binary_search_by_key(&node, |e| e.node)
            .map_or(0, |i| self.entries[i].main_delay)
    }

    /// `node_info` with the planned main-signal delays added to the nodes' latency.
    pub(crate) fn extra_latency(&self, node: NodeKey) -> u32 {
        self.extra
            .binary_search_by_key(&node, |e| e.0)
            .map_or(0, |i| self.extra[i].1)
    }
}

struct Walk<'a> {
    track: usize,
    desc: &'a TrackDesc,
    node_info: &'a dyn Fn(NodeKey) -> Option<NodeInfo>,
    consumers: &'a [Consumer],
    midi: &'a [MidiConsumer],
    other: &'a dyn Fn(usize, ScPoint) -> Option<u32>,
    other_midi: &'a dyn Fn(usize, MidiPoint) -> Option<u32>,
    plan: TrackPlan,
}

impl Walk<'_> {
    fn node_latency(&self, node: NodeKey) -> u32 {
        (self.node_info)(node).map_or(0, |i| i.latency)
    }

    /// Plans entry `node` (if sidechained) with the main signal at latency `lat`; returns
    /// the main-signal delay. With both an audio and a MIDI sidechain, the main signal waits
    /// for the later of the two and each is delayed by the rest.
    fn consume(&mut self, node: NodeKey, lat: u32) -> u32 {
        // A point not produced (yet): no sidechain (the model rejects these).
        let audio = self
            .consumers
            .binary_search_by_key(&node, |c| c.node)
            .ok()
            .and_then(|i| {
                let c = self.consumers[i];
                let l = if c.source == self.track {
                    self.plan.point_latency(c.point)
                } else {
                    (self.other)(c.source, c.point)
                };
                Some((c, l?))
            });
        let midi = self
            .midi
            .binary_search_by_key(&node, |c| c.node)
            .ok()
            .and_then(|i| {
                let c = self.midi[i];
                let l = if c.source == self.track {
                    match c.point {
                        // The entry's own input: the main signal's latency here.
                        MidiPoint::Before(k) if k == node => Some(lat),
                        p => self.plan.midi_point_latency(p),
                    }
                } else {
                    (self.other_midi)(c.source, c.point)
                };
                Some((c, l?))
            });
        if audio.is_none() && midi.is_none() {
            return 0;
        }
        let main_delay = audio
            .map_or(0, |a| a.1.saturating_sub(lat))
            .max(midi.map_or(0, |m| m.1.saturating_sub(lat)));
        let at = lat.saturating_add(main_delay);
        self.plan.entries.push(EntryPlan {
            node,
            main_delay,
            audio: audio.map(|(c, l)| AudioPlan {
                source: c.source,
                point: c.point,
                sc_delay: at - l,
            }),
            midi: midi.map(|(c, l)| MidiPlan {
                source: c.source,
                point: c.point,
                delay: at - l,
            }),
        });
        main_delay
    }

    /// Walks a pad / rack chain starting at latency `lat`. Every entry counts, bypassed or
    /// not (a bypassed entry delays its input by its latency, and automation may switch it
    /// on), like `drum_rack::pad_latency` / `rack_chains::chain_latency`. Returns its latency.
    fn chain(&mut self, entries: &[crate::graph::ChainEntry], lat: u32, depth: usize) -> u32 {
        let mut pos = lat;
        let mut total = 0u32;
        for e in entries {
            let main = self.consume(e.node, pos);
            if main > 0 {
                self.plan.extra.push((e.node, main));
            }
            self.plan.inputs.push((e.node, pos.saturating_add(main)));
            let nested = self.rack(e.node, pos.saturating_add(main), depth + 1);
            let l = self
                .node_latency(e.node)
                .saturating_add(main)
                .saturating_add(nested);
            total = l.saturating_add(total);
            pos = pos.saturating_add(l);
            self.plan.points.push((ScPoint::After(e.node), pos));
        }
        total
    }

    /// The pads / chains of rack entry `node` (input at latency `lat`); returns the longest
    /// (0 when `node` is no rack).
    fn rack(&mut self, node: NodeKey, lat: u32, depth: usize) -> u32 {
        if depth > crate::rack_chains::MAX_NESTING {
            return 0;
        }
        let desc = self.desc;
        let mut ids: Vec<u128> = Vec::new();
        let mut longest = 0;
        if let Some(r) = desc.racks.iter().find(|r| r.rack == node) {
            for p in &r.pads {
                self.plan.points.push((ScPoint::ChainIn(p.pad.0.0), lat));
                longest = longest.max(self.chain(&p.chain, lat, depth));
                ids.push(p.pad.0.0);
            }
        } else if let Some(r) = desc.chain_racks.iter().find(|r| r.rack == node) {
            // In run order: a chain keyed by another one of the rack comes first.
            for c in &r.chains {
                self.plan.points.push((ScPoint::ChainIn(c.id.0.0), lat));
                let l = self.chain(&c.chain, lat, depth);
                self.plan
                    .points
                    .push((ScPoint::ChainRaw(c.id.0.0), lat.saturating_add(l)));
                longest = longest.max(l);
                ids.push(c.id.0.0);
            }
        }
        for id in ids {
            self.plan
                .points
                .push((ScPoint::ChainOut(id), lat.saturating_add(longest)));
            self.plan
                .points
                .push((ScPoint::ChainMix(id), lat.saturating_add(longest)));
        }
        longest
    }
}

/// The sidechained entries of a track, for [`plan`]: audio ([`consumers`]) and MIDI
/// (`crate::midi_sidechain::consumers`).
pub(crate) struct Consumers<'a> {
    pub audio: &'a [Consumer],
    pub midi: &'a [MidiConsumer],
}

/// Non-RT. PDC plan of track `track` (see the module docs). `in_lat` is its final input
/// latency; `consumers` its sidechained entries; `other(s, point)` / `other_midi(s, point)`
/// the latency of a point of an earlier-processed track (final by processing order).
pub(crate) fn plan(
    track: usize,
    desc: &TrackDesc,
    node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
    consumers: Consumers<'_>,
    in_lat: u32,
    other: &dyn Fn(usize, ScPoint) -> Option<u32>,
    other_midi: &dyn Fn(usize, MidiPoint) -> Option<u32>,
) -> TrackPlan {
    let mut w = Walk {
        track,
        desc,
        node_info,
        consumers: consumers.audio,
        midi: consumers.midi,
        other,
        other_midi,
        plan: TrackPlan::default(),
    };
    let mut lat = in_lat;
    w.plan.points.push((ScPoint::Track(InputTap::PreFx), lat));
    for e in &desc.chain {
        // Every entry, bypassed or not, so the chain latency never depends on bypass state.
        let main = w.consume(e.node, lat);
        lat = lat.saturating_add(main);
        w.plan.top_main = w.plan.top_main.saturating_add(main);
        w.plan.inputs.push((e.node, lat));
        let l = w
            .node_latency(e.node)
            .saturating_add(w.rack(e.node, lat, 1));
        w.plan.top_latency.push(l);
        lat = lat.saturating_add(l);
        w.plan.points.push((ScPoint::After(e.node), lat));
    }
    w.plan.points.push((ScPoint::Track(InputTap::PostFx), lat));
    w.plan
        .points
        .push((ScPoint::Track(InputTap::PostFader), lat));
    w.plan.post_fx = lat;
    w.plan.entries.sort_by_key(|e| e.node);
    w.plan.extra.sort_by_key(|e| e.0);
    w.plan.inputs.sort_by_key(|e| e.0);
    w.plan.inputs.dedup_by_key(|e| e.0);
    w.plan
}

/// Source side: the tapped points of a track (only those some consumer reads).
#[derive(Debug, Default)]
pub(crate) struct Points {
    bufs: Vec<(ScPoint, Stereo)>,
    /// The tapped MIDI points (`crate::midi_sidechain`).
    pub midi: MidiPoints,
}

impl Points {
    /// Non-RT. `midi`: the tapped MIDI points; `no_chain`: the track has no chain entry.
    pub(crate) fn compile(
        points: &[ScPoint],
        midi: &[MidiPoint],
        no_chain: bool,
        config: &EngineConfig,
    ) -> Self {
        let mut points = points.to_vec();
        points.sort();
        points.dedup();
        Self {
            bufs: points
                .into_iter()
                .map(|p| (p, stereo(config.max_block_size)))
                .collect(),
            midi: MidiPoints::compile(midi, no_chain, config.max_events_per_block),
        }
    }

    fn find(&self, point: ScPoint) -> Option<usize> {
        self.bufs.binary_search_by(|e| e.0.cmp(&point)).ok()
    }

    /// RT. Record `point`'s signal for this sub-block (`n` frames), if it is tapped.
    #[inline]
    pub(crate) fn write(&mut self, point: ScPoint, a: &Stereo, n: usize) {
        if self.bufs.is_empty() {
            return;
        }
        if let Some(i) = self.find(point) {
            let buf = &mut self.bufs[i].1;
            buf[0][..n].copy_from_slice(&a[0][..n]);
            buf[1][..n].copy_from_slice(&a[1][..n]);
        }
    }

    /// RT. The buffer of `point`, if tapped (written sample by sample by a mix loop).
    pub(crate) fn buf_mut(&mut self, point: ScPoint) -> Option<&mut Stereo> {
        if self.bufs.is_empty() {
            return None;
        }
        let i = self.find(point)?;
        Some(&mut self.bufs[i].1)
    }

    /// No point of this track is tapped.
    pub(crate) fn is_empty(&self) -> bool {
        self.bufs.is_empty()
    }

    /// RT. `point`'s last recorded signal.
    pub(crate) fn get(&self, point: ScPoint) -> Option<&Stereo> {
        self.find(point).map(|i| &self.bufs[i].1)
    }

    /// Non-RT. The same points with `frames`-long silent buffers (`crate::anticipation`;
    /// no MIDI: held output carries no notes).
    pub(crate) fn like(&self, frames: usize) -> Self {
        Self {
            bufs: self
                .bufs
                .iter()
                .map(|(p, _)| (*p, stereo(frames)))
                .collect(),
            midi: MidiPoints::default(),
        }
    }

    /// The tapped points and their buffers, in point order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (ScPoint, &Stereo)> {
        self.bufs.iter().map(|(p, b)| (*p, b))
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (ScPoint, &mut Stereo)> {
        self.bufs.iter_mut().map(|(p, b)| (*p, b))
    }
}

#[derive(Debug)]
struct EntryRt {
    node: NodeKey,
    main: DelayLine,
    audio: Option<AudioRt>,
    midi: Option<MidiTapRt>,
}

#[derive(Debug)]
struct AudioRt {
    source: usize,
    point: ScPoint,
    /// The source is this track (read at the entry, not gathered).
    own: bool,
    sc: DelayLine,
    /// The (then delayed) sidechain for the current sub-block.
    buf: Stereo,
}

/// Consumer side: per sidechained entry of a track, its delay lines and signal buffer.
#[derive(Debug, Default)]
pub(crate) struct Taps {
    /// Sorted by node.
    entries: Vec<EntryRt>,
}

impl Taps {
    /// Non-RT. `track` is the consumer's index; `plan` its [`plan`].
    pub(crate) fn compile(track: usize, plan: &TrackPlan, config: &EngineConfig) -> Self {
        let frames = config.max_block_size;
        Self {
            entries: plan
                .entries
                .iter()
                .map(|e| EntryRt {
                    node: e.node,
                    main: DelayLine::new(e.main_delay as usize),
                    audio: e.audio.map(|a| AudioRt {
                        source: a.source,
                        point: a.point,
                        own: a.source == track,
                        sc: DelayLine::new(a.sc_delay as usize),
                        buf: stereo(frames),
                    }),
                    midi: e.midi.map(|m| {
                        MidiTapRt::new(
                            track,
                            m.source,
                            m.point,
                            m.delay,
                            config.max_events_per_block,
                        )
                    }),
                })
                .collect(),
        }
    }

    /// RT (snapshot swap): MIDI sidechains keep the notes in flight in their delays.
    pub(crate) fn inherit(&mut self, old: &mut Taps) {
        for e in &mut self.entries {
            let Some(m) = &mut e.midi else { continue };
            if let Some(Some(o)) = old.find(e.node).map(|o| o.midi.as_mut()) {
                m.delay.inherit(&mut o.delay);
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The source tracks of the entries (sorted, deduplicated).
    #[cfg(test)]
    pub(crate) fn sources(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self
            .entries
            .iter()
            .filter_map(|e| e.audio.as_ref().map(|a| a.source))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The delay of entry `node`'s MIDI sidechain, if it has one.
    #[cfg(test)]
    pub(crate) fn midi_delay(&self, node: NodeKey) -> Option<u32> {
        let i = self.entries.binary_search_by(|e| e.node.cmp(&node)).ok()?;
        self.entries[i].midi.as_ref().map(|m| m.delay.delay())
    }

    /// RT. Whether entry `node` reads a sidechain or has a planned alignment.
    pub(crate) fn has(&self, node: NodeKey) -> bool {
        self.entries.binary_search_by(|e| e.node.cmp(&node)).is_ok()
    }

    fn find(&mut self, node: NodeKey) -> Option<&mut EntryRt> {
        self.entries
            .binary_search_by(|e| e.node.cmp(&node))
            .ok()
            .map(|i| &mut self.entries[i])
    }

    /// RT. Start of the consumer's job: copy the other-track sources' taps (finished before
    /// this job). `source(s, point)` gives track `s`'s tapped buffer, `midi(s, point)` its
    /// tapped notes. Returns `true` if an event list was full.
    pub(crate) fn gather<'s>(
        &mut self,
        n: usize,
        source: impl Fn(usize, ScPoint) -> Option<&'s Stereo>,
        midi: impl Fn(usize, MidiPoint) -> Option<&'s [ProcessEvent]>,
    ) -> bool {
        let mut overflow = false;
        for e in self.entries.iter_mut() {
            if let Some(a) = e.audio.as_mut().filter(|a| !a.own) {
                match source(a.source, a.point) {
                    Some(src) => {
                        a.buf[0][..n].copy_from_slice(&src[0][..n]);
                        a.buf[1][..n].copy_from_slice(&src[1][..n]);
                    }
                    None => {
                        a.buf[0][..n].fill(0.0);
                        a.buf[1][..n].fill(0.0);
                    }
                }
            }
            if let Some(m) = e.midi.as_mut().filter(|m| !m.own) {
                overflow |= m.load(midi(m.source, m.point));
            }
        }
        overflow
    }

    /// RT. Entry `node`'s MIDI sidechain, every sub-block (bypassed or not, so its delay
    /// stays in time): its source's (delayed) notes replace the notes in `target` (the
    /// entry's events, `None` when it doesn't run). `own` are the consumer track's own points
    /// (for same-track sources, produced earlier in this job). Returns `true` on an overflow.
    #[inline]
    pub(crate) fn feed_midi(
        &mut self,
        node: NodeKey,
        own: &Points,
        n: usize,
        target: Option<&mut EventBuffer>,
    ) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let Some(m) = self.find(node).and_then(|e| e.midi.as_mut()) else {
            return false;
        };
        let mut overflow = false;
        if m.own {
            overflow |= m.load(own.midi.get(m.point));
        }
        overflow | m.feed(n, target)
    }

    /// RT. Delay the main signal before entry `node` as planned (every entry, bypassed or
    /// not, so the delay line never holds stale audio when the entry switches on).
    #[inline]
    pub(crate) fn align_main(&mut self, node: NodeKey, main: &mut Stereo, n: usize) {
        if self.entries.is_empty() {
            return;
        }
        if let Some(e) = self.find(node)
            && e.main.delay() > 0
        {
            let [l, r] = main;
            e.main.process(&mut l[..n], &mut r[..n]);
        }
    }

    /// RT. The aligned sidechain signal of entry `node`. `own` are the consumer track's own
    /// points (for same-track sources, produced earlier in this job).
    pub(crate) fn read(&mut self, node: NodeKey, own: &Points, n: usize) -> Option<[&[f32]; 2]> {
        let e = self.find(node)?.audio.as_mut()?;
        if e.own {
            match own.get(e.point) {
                Some(src) => {
                    e.buf[0][..n].copy_from_slice(&src[0][..n]);
                    e.buf[1][..n].copy_from_slice(&src[1][..n]);
                }
                None => {
                    e.buf[0][..n].fill(0.0);
                    e.buf[1][..n].fill(0.0);
                }
            }
        }
        {
            let [l, r] = &mut e.buf;
            e.sc.process(&mut l[..n], &mut r[..n]);
        }
        Some([&e.buf[0][..n], &e.buf[1][..n]])
    }
}

/// The sidechain state a pad / rack chain runner needs: the track's consumer [`Taps`] and its
/// own tapped [`Points`].
pub(crate) struct ScIo<'a> {
    pub taps: &'a mut Taps,
    pub points: &'a mut Points,
}
