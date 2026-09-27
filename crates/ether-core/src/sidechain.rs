//! Sidechain routing in the engine (roadmap v2, owned by the `sidechain` node; see
//! `docs/ROADMAP.md` and CONTRACTS.md §11.10).
//!
//! Implemented in base-24 (signal path + PDC), so the sidechain node only needs the device
//! side (`Node::process_sidechain` in the compressor/limiter), the controller command and
//! the UI:
//!
//! - **Order.** `graph::compile_with` orders every sidechain source before its consumer
//!   track (the `ChainEntry::sidechain` edges join the topological sort, not the buses).
//! - **Tap.** The source's post-fader output **before** its PDC output delay, whose
//!   latency is `out_lat(source)` (final: the source was compiled first). `engine.rs` calls
//!   [`Taps::write`] there.
//! - **PDC** ([`plan`]). Walking the consumer's chain with `L = in_lat(T)`: at an entry
//!   with a sidechain, `L_sc = out_lat(source)`. If `L_sc > L`, the *main* signal is
//!   delayed by `L_sc - L` just before that entry ([`Taps::align_main`], called by
//!   `engine.rs` before every entry) and that delay counts into T's chain latency (so T's
//!   output latency grows and downstream PDC absorbs it). Otherwise the sidechain is delayed
//!   by `L - L_sc` ([`Taps::read`]). Then `L += latency(entry)`. Both paths are aligned at
//!   the entry whether T plays clips, receives buses or both.
//! - Snapshot swaps rebuild the delay lines (a sidechain edit may click once).

use ether_protocol::model::TrackId;

use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::graph::{NodeInfo, TrackDesc};
use crate::mixer::{Stereo, stereo};

/// One sidechained chain entry of a track, as planned by [`plan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EntryPlan {
    pub entry: usize,
    pub source: usize,
    /// Delay of the main signal just before the entry (counted into the chain latency).
    pub main_delay: u32,
    /// Delay of the tapped sidechain signal.
    pub sc_delay: u32,
}

/// Non-RT. PDC plan of a track's sidechained entries (see the module docs). `in_lat` is the
/// track's final input latency; `out_lat[source]` is final for every source (processing
/// order). Entries whose source is unknown or the track itself are skipped.
pub(crate) fn plan(
    track: usize,
    desc: &TrackDesc,
    chain_info: &[NodeInfo],
    in_lat: u32,
    source_index: &dyn Fn(TrackId) -> Option<usize>,
    out_lat: &[u32],
) -> Vec<EntryPlan> {
    let mut out = Vec::new();
    let mut lat = in_lat;
    for (k, e) in desc.chain.iter().enumerate() {
        if let Some(source) = e.sidechain.and_then(source_index).filter(|&s| s != track) {
            let l_sc = out_lat[source];
            let (main_delay, sc_delay) = if l_sc > lat {
                (l_sc - lat, 0)
            } else {
                (0, lat - l_sc)
            };
            lat += main_delay;
            out.push(EntryPlan {
                entry: k,
                source,
                main_delay,
                sc_delay,
            });
        }
        lat += chain_info.get(k).map_or(0, |i| i.latency);
    }
    out
}

#[derive(Debug)]
struct EntryRt {
    track: usize,
    entry: usize,
    main: DelayLine,
    sc: DelayLine,
    /// The delayed sidechain for the current sub-block.
    buf: Stereo,
}

/// Per-snapshot sidechain state: tap buffers of the sources and per-entry delay lines.
#[derive(Debug, Default)]
pub(crate) struct Taps {
    /// Per track index: the tap buffer if something listens to it.
    taps: Vec<Option<Stereo>>,
    /// Sorted by `(track, entry)`.
    entries: Vec<EntryRt>,
}

impl Taps {
    /// Non-RT. `plans[i]` is [`plan`] of track `i`.
    pub(crate) fn compile(plans: &[Vec<EntryPlan>], config: &EngineConfig) -> Self {
        let frames = config.max_block_size;
        let mut taps: Vec<Option<Stereo>> = (0..plans.len()).map(|_| None).collect();
        let mut entries = Vec::new();
        for (track, p) in plans.iter().enumerate() {
            for e in p {
                if taps[e.source].is_none() {
                    taps[e.source] = Some(stereo(frames));
                }
                entries.push(EntryRt {
                    track,
                    entry: e.entry,
                    main: DelayLine::new(e.main_delay as usize),
                    sc: DelayLine::new(e.sc_delay as usize),
                    buf: stereo(frames),
                });
            }
        }
        Self { taps, entries }
    }

    fn find(&mut self, track: usize, entry: usize) -> Option<&mut EntryRt> {
        self.entries
            .binary_search_by(|e| (e.track, e.entry).cmp(&(track, entry)))
            .ok()
            .map(|i| &mut self.entries[i])
    }

    /// RT. Record track `track`'s post-fader output (before its PDC output delay) for this
    /// sub-block (`n` frames), if anything listens to it.
    pub(crate) fn write(&mut self, track: usize, out: &Stereo, n: usize) {
        if let Some(Some(tap)) = self.taps.get_mut(track) {
            tap[0][..n].copy_from_slice(&out[0][..n]);
            tap[1][..n].copy_from_slice(&out[1][..n]);
        }
    }

    /// RT. Delay the main signal before chain entry `(track, entry)` as planned (every
    /// entry, bypassed or not, so the chain latency never depends on bypass state).
    pub(crate) fn align_main(&mut self, track: usize, entry: usize, main: &mut Stereo, n: usize) {
        if self.entries.is_empty() {
            return;
        }
        if let Some(e) = self.find(track, entry)
            && e.main.delay() > 0
        {
            let [l, r] = main;
            e.main.process(&mut l[..n], &mut r[..n]);
        }
    }

    /// RT. The aligned sidechain signal for chain entry `(track, entry)` fed by `source`.
    pub(crate) fn read(
        &mut self,
        track: usize,
        entry: usize,
        source: usize,
        n: usize,
    ) -> Option<[&[f32]; 2]> {
        let Self { taps, entries } = self;
        let tap = taps.get(source)?.as_ref()?;
        let i = entries
            .binary_search_by(|e| (e.track, e.entry).cmp(&(track, entry)))
            .ok()?;
        let e = &mut entries[i];
        e.buf[0][..n].copy_from_slice(&tap[0][..n]);
        e.buf[1][..n].copy_from_slice(&tap[1][..n]);
        {
            let [l, r] = &mut e.buf;
            e.sc.process(&mut l[..n], &mut r[..n]);
        }
        Some([&e.buf[0][..n], &e.buf[1][..n]])
    }
}
