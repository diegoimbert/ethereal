//! Anticipative processing (CONTRACTS.md §11.7, docs/ARCHITECTURE.md "Multicore scheduling").
//!
//! Tracks nobody listens to live render ahead of the output, so the real-time side of the
//! graph (the master chain, live tracks) overlaps the next buffer's tracks instead of
//! running alone at the end of each buffer.
//!
//! - **Classification** (per snapshot, [`classify`]): live tracks are monitored, armed, or
//!   hold an external device (`crate::hw_io`). The real-time set is everything downstream
//!   of a live track, of a track feeding one, or of a track writing the hardware output
//!   (master). Every other track is *anticipated*. Every source of an anticipated track is
//!   anticipated too, so the anticipated tracks of a buffer never wait for a real-time one.
//! - **Time.** Anticipated tracks run up to one buffer (the host's next block, at most
//!   `max_block_size`) ahead of the output. The transport is decided on their side
//!   (`engine::plan_sub`, where transport controls apply): each sub-block they render is
//!   queued ([`PlanQueue`]) and the real-time side renders the same plans one buffer
//!   later, so both sides see the same timeline. Transport controls therefore take effect
//!   one buffer later everywhere, and live parameter, fader and mute changes one buffer
//!   later on anticipated tracks.
//! - **Held output** ([`Held`]): what other tracks read from an anticipated track (its
//!   output, sends, post-fader tap, sidechain and input-tap points) is copied at the end of
//!   its job into `back`; between buffers `back` moves to `front`, where real-time jobs read
//!   it while the anticipated job overwrites the track's own buffers with the next buffer.
//!   `front`/`back` are separate from the tracks' state, so no job reads what another
//!   writes.
//! - **Roles** of a track in a buffer, from whether it was ahead and whether it should be:
//!   ahead→ahead, ahead→real-time (it renders nothing this buffer: its readers take the
//!   held output), real-time→real-time, real-time→ahead (it renders this buffer in real
//!   time and then the next one ahead: twice, once). Changes apply only when the queue
//!   holds no more than this buffer; otherwise they wait a buffer, unless the graph forces
//!   them (then the queue is dropped: a jump of up to one buffer on the anticipated tracks).

use ether_protocol::model::{SendId, TrackId};

use crate::bus_tap::TapBuffers;
use crate::engine::SubPlan;
use crate::graph::RenderGraphDesc;
use crate::mixer::{Stereo, TrackRt};
use crate::sidechain::{Points, ScPoint};

/// Sub-blocks the queue can hold (more than any buffer splits into but a pathological loop;
/// when it is full the anticipated side plans less and catches up in real time).
pub(crate) const MAX_PLANS: usize = 64;

/// Non-RT (graph compile). Which tracks are anticipated (see the module docs). `succ[i]`:
/// every track `i` feeds (outputs, sends, sidechains, modulator sidechains, input taps).
pub(crate) fn classify(
    desc: &RenderGraphDesc,
    succ: &[Vec<usize>],
    to_hardware: &[bool],
) -> Vec<bool> {
    let n = desc.tracks.len();
    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (s, ds) in succ.iter().enumerate() {
        for &d in ds {
            pred[d].push(s);
        }
    }
    let live: Vec<bool> = desc
        .tracks
        .iter()
        .map(|t| t.monitor || t.armed || !t.hw_io.is_empty())
        .collect();
    // Live tracks and everything feeding them.
    let mut seed = live.clone();
    let mut stack: Vec<usize> = (0..n).filter(|&i| live[i]).collect();
    while let Some(i) = stack.pop() {
        for &p in &pred[i] {
            if !seed[p] {
                seed[p] = true;
                stack.push(p);
            }
        }
    }
    // Plus the hardware outputs, then everything downstream.
    let mut realtime = seed;
    for (r, &hw) in realtime.iter_mut().zip(to_hardware) {
        *r |= hw;
    }
    // Held output carries no notes: the MIDI sidechain sources of a real-time track are
    // real-time too (`crate::midi_sidechain`), and so is everything downstream of them.
    let mut index: Vec<(TrackId, usize)> = desc
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id, i))
        .collect();
    index.sort();
    let midi_sources: Vec<Vec<usize>> = desc
        .tracks
        .iter()
        .map(|t| {
            t.midi_sidechains
                .iter()
                .filter_map(|m| {
                    index
                        .binary_search_by(|e| e.0.cmp(&m.source))
                        .ok()
                        .map(|k| index[k].1)
                })
                .collect()
        })
        .collect();
    let mut stack: Vec<usize> = (0..n).filter(|&i| realtime[i]).collect();
    while let Some(i) = stack.pop() {
        for &d in succ[i].iter().chain(&midi_sources[i]) {
            if !realtime[d] {
                realtime[d] = true;
                stack.push(d);
            }
        }
    }
    realtime.iter().map(|&r| !r).collect()
}

/// A track's readable output (see the module docs), up to `max_block_size` frames of it,
/// oldest first.
#[derive(Debug)]
pub(crate) struct Held {
    len: usize,
    a: Stereo,
    sends: Vec<(SendId, Stereo)>,
    tap: Option<Stereo>,
    sc: Points,
    taps: TapBuffers,
}

impl Held {
    /// Non-RT. Empty buffers shaped like `track`'s readable output.
    pub(crate) fn like(track: &TrackRt, frames: usize) -> Self {
        Self {
            len: 0,
            a: crate::mixer::stereo(frames),
            sends: track
                .sends
                .iter()
                .map(|s| (s.id, crate::mixer::stereo(frames)))
                .collect(),
            tap: track.tap.as_ref().map(|_| crate::mixer::stereo(frames)),
            sc: track.sc_points.like(frames),
            taps: track.taps.like(frames),
        }
    }

    /// Frames held.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    fn cap(&self) -> usize {
        self.a[0].len()
    }

    fn bufs_mut(&mut self) -> impl Iterator<Item = &mut Stereo> {
        std::iter::once(&mut self.a)
            .chain(self.sends.iter_mut().map(|s| &mut s.1))
            .chain(self.tap.as_mut())
            .chain(self.sc.iter_mut().map(|p| p.1))
            .chain(self.taps.iter_mut().map(|p| p.1))
    }

    /// RT. Append the first `n` frames of `track`'s current output (the end of its job;
    /// `self` was made [`Held::like`] it). Frames past the capacity are dropped.
    pub(crate) fn append(&mut self, track: &TrackRt, n: usize) {
        let at = self.len;
        let n = n.min(self.cap() - at);
        let src = std::iter::once(&track.a)
            .chain(track.sends.iter().map(|s| &s.buf))
            .chain(track.tap.as_ref())
            .chain(track.sc_points.iter().map(|p| p.1))
            .chain(track.taps.iter().map(|p| p.1));
        for (d, s) in self.bufs_mut().zip(src) {
            for ch in 0..2 {
                d[ch][at..at + n].copy_from_slice(&s[ch][..n]);
            }
        }
        self.len = at + n;
    }

    /// RT. Append everything `other` (same shape) holds and empty it. A swap when `self` is
    /// empty.
    pub(crate) fn take_from(&mut self, other: &mut Held) {
        if self.len == 0 {
            std::mem::swap(self, other);
        } else {
            let at = self.len;
            let n = other.len.min(self.cap() - at);
            for (d, s) in self.bufs_mut().zip(other.bufs_mut()) {
                for ch in 0..2 {
                    d[ch][at..at + n].copy_from_slice(&s[ch][..n]);
                }
            }
            self.len = at + n;
        }
        other.len = 0;
    }

    /// RT. Drop the oldest `n` frames (the real-time side rendered them).
    pub(crate) fn consume(&mut self, n: usize) {
        let n = n.min(self.len);
        let rest = self.len - n;
        if rest > 0 {
            for b in self.bufs_mut() {
                for ch in &mut b[..] {
                    ch.copy_within(n..n + rest, 0);
                }
            }
        }
        self.len = rest;
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }

    /// RT (snapshot swap). Take over the frames `old` (this track in the previous snapshot)
    /// holds: matching outputs keep their signal, new ones start silent.
    pub(crate) fn inherit(&mut self, old: &Held) {
        let n = old.len.min(self.cap());
        let copy = |d: &mut Stereo, s: Option<&Stereo>| {
            for ch in 0..2 {
                match s {
                    Some(s) => d[ch][..n].copy_from_slice(&s[ch][..n]),
                    None => d[ch][..n].fill(0.0),
                }
            }
        };
        copy(&mut self.a, Some(&old.a));
        for (id, d) in &mut self.sends {
            copy(d, old.sends.iter().find(|s| s.0 == *id).map(|s| &s.1));
        }
        if let Some(d) = &mut self.tap {
            copy(d, old.tap.as_ref());
        }
        for (p, d) in self.sc.iter_mut() {
            copy(d, old.sc.get(p));
        }
        for (p, d) in self.taps.iter_mut() {
            copy(d, old.taps.iter().find(|t| t.0 == p).map(|t| t.1));
        }
        self.len = n;
    }

    /// The track's output (`TrackRt::a`).
    pub(crate) fn output(&self) -> &Stereo {
        &self.a
    }

    /// Send `k`'s signal (`SendRt::buf`).
    pub(crate) fn send(&self, k: usize) -> Option<&Stereo> {
        self.sends.get(k).map(|s| &s.1)
    }

    /// The post-fader tap (`TrackRt::tap`).
    pub(crate) fn tap(&self) -> Option<&Stereo> {
        self.tap.as_ref()
    }

    /// A sidechain point (`TrackRt::sc_points`).
    pub(crate) fn sc_point(&self, point: ScPoint) -> Option<&Stereo> {
        self.sc.get(point)
    }

    /// The input-tap points (`TrackRt::taps`).
    pub(crate) fn taps(&self) -> &TapBuffers {
        &self.taps
    }
}

/// Which side a track's job renders in a pass (`engine.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Lane {
    /// Nothing this pass.
    #[default]
    Idle = 0,
    /// The output's sub-block.
    Now = 1,
    /// The anticipated sub-block.
    Ahead = 2,
}

/// The per-snapshot state (`SnapshotRt::antic`), allocated at compile when the engine
/// anticipates (`EngineConfig::anticipative`).
#[derive(Debug)]
pub(crate) struct AnticRt {
    /// Per track: anticipated by classification.
    pub target: Vec<bool>,
    /// Some track is.
    pub any: bool,
    /// Per track: the tracks it reads (every graph edge into it).
    pub sources: Vec<Vec<u32>>,
    /// The job graph without the edges from anticipated to real-time tracks (those read
    /// held output): the pass that overlaps both sides.
    pub graph: crate::parallel::JobGraph,
    /// Per track (runtime): rendered ahead, its `front` holds the queued frames.
    pub ahead: Vec<bool>,
    /// Per track, this buffer: should end ahead.
    pub eff: Vec<bool>,
    /// Per track, this pass.
    pub lane: Vec<Lane>,
    pub front: Vec<Held>,
    pub back: Vec<Held>,
}

impl AnticRt {
    /// Non-RT (graph compile).
    pub(crate) fn compile(
        desc: &RenderGraphDesc,
        succ: &[Vec<usize>],
        tracks: &[TrackRt],
        cost: &[u64],
        frames: usize,
    ) -> Self {
        let n = tracks.len();
        let to_hw: Vec<bool> = tracks.iter().map(|t| t.to_hardware).collect();
        let target = classify(desc, succ, &to_hw);
        let mut sources: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (s, ds) in succ.iter().enumerate() {
            for &d in ds {
                if !sources[d].contains(&(s as u32)) {
                    sources[d].push(s as u32);
                }
            }
        }
        let cut: Vec<Vec<usize>> = succ
            .iter()
            .enumerate()
            .map(|(s, ds)| {
                ds.iter()
                    .copied()
                    .filter(|&d| !(target[s] && !target[d]))
                    .collect()
            })
            .collect();
        let graph = crate::parallel::JobGraph::new(
            &(0..n).collect::<Vec<_>>(),
            cost,
            &cut,
            &vec![false; n],
        );
        Self {
            any: target.iter().any(|&t| t),
            target,
            sources,
            graph,
            ahead: vec![false; n],
            eff: vec![false; n],
            lane: vec![Lane::Idle; n],
            front: tracks.iter().map(|t| Held::like(t, frames)).collect(),
            back: tracks.iter().map(|t| Held::like(t, frames)).collect(),
        }
    }

    /// RT. Whether every source of an `eff` track is `eff` too (the anticipated side never
    /// waits for the real-time side).
    pub(crate) fn closed(&self, eff: &[bool]) -> bool {
        (0..eff.len())
            .filter(|&i| eff[i])
            .all(|i| self.sources[i].iter().all(|&s| eff[s as usize]))
    }
}

/// The sub-blocks rendered ahead and not yet output, oldest first (a ring, nothing
/// allocated after [`PlanQueue::new`]).
#[derive(Debug)]
pub(crate) struct PlanQueue {
    plans: Vec<Option<SubPlan>>,
    head: usize,
    len: usize,
    /// Frames queued.
    pub frames: usize,
}

impl PlanQueue {
    pub(crate) fn new() -> Self {
        Self {
            plans: vec![None; MAX_PLANS],
            head: 0,
            len: 0,
            frames: 0,
        }
    }

    pub(crate) fn is_full(&self) -> bool {
        self.len == MAX_PLANS
    }

    pub(crate) fn front(&self) -> Option<&SubPlan> {
        (self.len > 0)
            .then(|| self.plans[self.head].as_ref())
            .flatten()
    }

    pub(crate) fn push(&mut self, plan: SubPlan) -> bool {
        if self.is_full() {
            return false;
        }
        self.plans[(self.head + self.len) % MAX_PLANS] = Some(plan);
        self.len += 1;
        self.frames += plan.n;
        true
    }

    pub(crate) fn pop(&mut self) -> Option<SubPlan> {
        let plan = self.plans[self.head].take()?;
        self.head = (self.head + 1) % MAX_PLANS;
        self.len -= 1;
        self.frames -= plan.n;
        Some(plan)
    }

    /// Replace the oldest plan (after the real-time side rendered part of it).
    pub(crate) fn replace_front(&mut self, plan: SubPlan) {
        if let Some(p) = self.plans[self.head].as_mut() {
            self.frames = self.frames - p.n + plan.n;
            *p = plan;
        }
    }

    pub(crate) fn clear(&mut self) {
        while self.pop().is_some() {}
    }

    /// Of the plans covering the first `frames` queued frames, the position of the one
    /// covering the most of them (the first such one).
    pub(crate) fn longest_within(&self, frames: usize) -> usize {
        let (mut best, mut best_n, mut at) = (0, 0, 0);
        for k in 0..self.len {
            if at >= frames {
                break;
            }
            let n = self.plans[(self.head + k) % MAX_PLANS].map_or(0, |p| p.n);
            let covered = n.min(frames - at);
            if covered > best_n {
                (best, best_n) = (k, covered);
            }
            at += n;
        }
        best
    }
}

/// RT. The first `m` frames of `plan` and the rest (`0 < m < plan.n`), for a host block
/// shorter than the one planned. The first part keeps the one-shot flags; the timeline of
/// the second continues where the first ends.
pub(crate) fn split(
    plan: &SubPlan,
    m: usize,
    tempo: &crate::tempo::TempoMapRt,
    sr: f64,
) -> (SubPlan, SubPlan) {
    let mut first = *plan;
    let mut rest = *plan;
    first.n = m;
    first.wraps = false;
    let mid = if !plan.playing {
        plan.b0
    } else if let Some((beat, seconds, since)) = plan.exact {
        crate::sched::exact_beat(tempo, sr, (beat, seconds), since, m as f64)
    } else {
        tempo.seconds_to_beats(plan.s0 + m as f64 / sr)
    };
    first.b1 = mid;
    rest.n = plan.n - m;
    rest.sample_time = plan.sample_time + m as u64;
    rest.b0 = mid;
    // As `engine::plan_sub` computes a sub-block starting there.
    rest.s0 = match plan.exact {
        Some((_, seconds, since)) => seconds + (since + m as u64) as f64 / sr,
        None if plan.playing => tempo.beats_to_seconds(mid),
        None => plan.s0,
    };
    rest.exact = plan
        .exact
        .map(|(beat, seconds, since)| (beat, seconds, since + m as u64));
    rest.release_notes = false;
    rest.all_notes_off = false;
    rest.reset_nodes = false;
    rest.relocated = false;
    rest.chase_notes = false;
    rest.jump = false;
    (first, rest)
}
