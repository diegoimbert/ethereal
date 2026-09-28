//! Sample-accurate automation: arrangement lanes and clip envelopes applied per sample
//! (CONTRACTS.md §12.7; the v0.1 per-block runtime was extracted from `engine.rs` by
//! contracts-3, then replaced by the `sample-accurate-automation` node).
//!
//! Everything is placed on the engine's sample clock, never on sub-block boundaries, so an
//! offline render is the same for every block size:
//! - **node params** get `EventKind::Param` events on an absolute grid (`sample_time`
//!   multiples of [`PARAM_GRID`]) plus one at the sample of every breakpoint (lane or
//!   envelope point, clip start/end/loop point), each evaluated at the exact beat of its
//!   sample (`sched::Timing::beat_at`, exact on tempo ramps), and only when the value
//!   changed. A breakpoint lands on the first sample whose beat is at or after it, so a
//!   `Step` point switches exactly there. After a timeline jump (play, locate, loop wrap) or
//!   a live change of an automated param, the current value is re-sent at offset 0;
//! - **mixer targets** (volume, pan, send levels) are ramped linearly per sample to the
//!   value at each grid point (`Smoother::ramp_to`, one ramp per grid interval; the ramp
//!   of the interval that contains the loop end aims at the loop end);
//! - **precedence** (CONTRACTS.md §4) is resolved per event/grid point: an unmuted clip
//!   whose envelope targets the param drives it where the clip plays, the arrangement lane
//!   elsewhere;
//! - while **stopped**, everything is evaluated at the position (v0.1 behaviour: mixer
//!   targets smoothed with the fader ramp, node params sent at offset 0 when they change).
//!
//! RT-safe: no allocation (knots live in a fixed stack buffer), bounded by the automation
//! data overlapping the sub-block.

use crate::automation::{evaluate, gain_from_plain, to_plain};
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::{AutomationDesc, ClipDesc, ParamMapping, ResolvedTarget, TrackDesc};
use crate::mixer::{ChainRt, SendRt, Stereo, apply_fader_range, scale};
use crate::param::Smoother;
use crate::sched::{self, EVENT_SHIFT, Timing};

/// The frozen v0.2 automation grid (samples of the absolute engine clock).
pub const PARAM_GRID: u64 = 32;
/// Knots (event offsets) per target and sub-block beyond which breakpoints are dropped (the
/// grid still covers them): `max_block_size / PARAM_GRID` grid points plus breakpoints.
const MAX_KNOTS: usize = 256;

/// Offsets of the grid points inside the sub-block (modulation uses the same grid).
pub(crate) fn grid(timing: &Timing<'_>) -> std::iter::StepBy<std::ops::Range<usize>> {
    let first = ((PARAM_GRID - timing.sample_time % PARAM_GRID) % PARAM_GRID) as usize;
    (first..timing.frames).step_by(PARAM_GRID as usize)
}

/// Which mixer targets automation drives in this (played) sub-block: the fader stage ramps
/// them per sample ([`fader`], [`send_level`]).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MixerDrive {
    pub volume: bool,
    pub pan: bool,
    pub sends: bool,
}

/// Event offsets of one target in the sub-block, sorted and unique, each with the beat it
/// must be evaluated at least at (a breakpoint's own beat; `-inf` for grid points).
struct Knots {
    at: [(u32, f64); MAX_KNOTS],
    len: usize,
}

impl Knots {
    fn new() -> Self {
        Self {
            at: [(0, f64::NEG_INFINITY); MAX_KNOTS],
            len: 0,
        }
    }

    fn insert(&mut self, offset: usize, min_beat: f64) {
        let o = offset as u32;
        let knots = &mut self.at[..self.len];
        match knots.binary_search_by(|k| k.0.cmp(&o)) {
            Ok(i) => knots[i].1 = knots[i].1.max(min_beat),
            Err(i) if self.len < MAX_KNOTS => {
                self.at.copy_within(i..self.len, i + 1);
                self.at[i] = (o, min_beat);
                self.len += 1;
            }
            Err(_) => {}
        }
    }

    fn as_slice(&self) -> &[(u32, f64)] {
        &self.at[..self.len]
    }
}

/// The automation of one track that overlaps a sub-block.
struct Sources<'a> {
    lanes: &'a [AutomationDesc],
    /// Clips starting before the sub-block end.
    clips: &'a [ClipDesc],
    /// Clips ending after this beat take part: one sample before the sub-block start while
    /// playing (a clip ending exactly on the sub-block start hands over at offset 0), the
    /// position while stopped.
    lo: f64,
}

impl<'a> Sources<'a> {
    fn new(tdesc: &'a TrackDesc, timing: &Timing<'_>, playing: bool) -> Self {
        let (lo, end) = if playing {
            // Mixer ramps look up to one grid interval past the sub-block end.
            let ahead = timing.beat_at((timing.frames as u64 + PARAM_GRID) as f64);
            (timing.beat_at(-1.0), ahead.max(timing.b1))
        } else {
            (timing.b0, timing.b0 + 1e-9)
        };
        let clips_end = tdesc.clips.partition_point(|c| c.start < end);
        Self {
            lanes: &tdesc.automation,
            clips: &tdesc.clips[..clips_end],
            lo,
        }
    }

    fn is_active(&self, clip: &ClipDesc) -> bool {
        !clip.muted && clip.start + clip.length > self.lo
    }

    /// The arrangement lane of `target` (the first one with points).
    fn lane(&self, target: ResolvedTarget) -> Option<&'a AutomationDesc> {
        self.lanes
            .iter()
            .find(|l| l.resolved == target && !l.points.is_empty())
    }

    /// `(clip index, envelope index, clip, envelope)` of the active clips' envelopes of
    /// `target`.
    fn envelopes(
        &self,
        target: ResolvedTarget,
    ) -> impl Iterator<Item = (usize, usize, &'a ClipDesc, &'a AutomationDesc)> + '_ {
        self.clips
            .iter()
            .enumerate()
            .filter(|(_, c)| self.is_active(c))
            .flat_map(move |(ci, c)| {
                c.envelopes
                    .iter()
                    .enumerate()
                    .filter(move |(_, e)| e.resolved == target && !e.points.is_empty())
                    .map(move |(ei, e)| (ci, ei, c, e))
            })
    }

    /// Normalized value of `target` at timeline beat `t` and its mapping: the last active
    /// clip whose envelope covers `t`, else the lane.
    fn value(
        &self,
        target: ResolvedTarget,
        lane: Option<&'a AutomationDesc>,
        t: f64,
    ) -> Option<(f64, &'a ParamMapping)> {
        let mut out = None;
        for (_, _, clip, env) in self.envelopes(target) {
            if let Some(v) = sched::content_at(clip, t).and_then(|c| evaluate(&env.points, c)) {
                out = Some((v, &env.mapping));
            }
        }
        out.or_else(|| lane.and_then(|l| evaluate(&l.points, t).map(|v| (v, &l.mapping))))
    }

    /// Knots of every breakpoint of `target` whose sample is in the sub-block: lane points,
    /// envelope points on the timeline, and where clips start, end or loop.
    fn breakpoints(
        &self,
        target: ResolvedTarget,
        lane: Option<&'a AutomationDesc>,
        timing: &Timing<'_>,
        knots: &mut Knots,
    ) {
        let (lo, hi) = (self.lo, timing.b1);
        let mut add = |t: f64| {
            if t > lo && t < hi {
                let o = timing.sample_at_or_after(t);
                if o < timing.frames {
                    knots.insert(o, t);
                }
            }
        };
        if let Some(lane) = lane {
            let first = lane.points.partition_point(|p| p.0 <= lo);
            for p in &lane.points[first..] {
                if p.0 >= hi {
                    break;
                }
                add(p.0);
            }
        }
        for (_, _, clip, env) in self.envelopes(target) {
            sched::for_each_piece(clip, lo, hi, |p| {
                add(p.t0);
                add(p.t1);
                let c_end = p.c0 + (p.t1 - p.t0);
                let first = env.points.partition_point(|q| q.0 < p.c0);
                for q in &env.points[first..] {
                    if q.0 >= c_end {
                        break;
                    }
                    add(p.t0 + (q.0 - p.c0));
                }
            });
        }
    }
}

/// RT. Apply the track's automation for the sub-block (see the module docs): node-param
/// events into the chain / drum-pad / rack-chain event buffers, mixer targets while stopped.
/// Returns the mixer targets the fader stage must ramp ([`fader`], [`send_level`]).
///
/// `jump`: the timeline jumped before this sub-block (play, locate, loop wrap). `auto_last`
/// / `env_last` (indexed by lane / `env_base[clip] + envelope`) hold the last plain value
/// sent per node-param target (NaN = send again).
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_automation(
    tdesc: &TrackDesc,
    timing: &Timing<'_>,
    playing: bool,
    jump: bool,
    auto_last: &mut [f64],
    env_last: &mut [f64],
    env_base: &[usize],
    volume: &mut Smoother,
    pan: &mut Smoother,
    sends: &mut [SendRt],
    chain: &mut [ChainRt],
    racks: &mut crate::drum_rack::RacksRt,
    chain_racks: &mut crate::rack_chains::ChainRacksRt,
    modulation: &mut crate::modulation::ModulationRt,
) -> MixerDrive {
    let mut drive = MixerDrive::default();
    if tdesc.automation.is_empty() && tdesc.clips.iter().all(|c| c.envelopes.is_empty()) {
        return drive;
    }
    let src = Sources::new(tdesc, timing, playing);
    // Envelopes of clips that stopped playing send again next time.
    for (ci, clip) in tdesc.clips.iter().enumerate() {
        if !clip.envelopes.is_empty() && !(ci < src.clips.len() && src.is_active(clip)) {
            let base = env_base[ci];
            env_last[base..base + clip.envelopes.len()].fill(f64::NAN);
        }
    }
    let mut node = NodeParams {
        src: &src,
        timing,
        playing,
        jump,
        chain,
        racks,
        chain_racks,
        modulation,
    };
    let mut mixer = |target: ResolvedTarget, lane: Option<&AutomationDesc>| {
        if playing {
            match target {
                ResolvedTarget::TrackVolume => drive.volume = true,
                ResolvedTarget::TrackPan => drive.pan = true,
                ResolvedTarget::Send { .. } => drive.sends = true,
                ResolvedTarget::Node { .. } => {}
            }
        } else if let Some((v, mapping)) = src.value(target, lane, timing.b0) {
            set_mixer_target(target, mapping, v, volume, pan, sends);
        }
    };
    // Each target once: lanes first (the lane's slot keeps the last value, also while an
    // envelope drives it), then targets only clip envelopes automate.
    for (li, lane) in tdesc.automation.iter().enumerate() {
        let target = lane.resolved;
        if lane.points.is_empty() || src.lane(target).is_some_and(|l| !std::ptr::eq(l, lane)) {
            continue;
        }
        match target {
            ResolvedTarget::Node { .. } => node.walk(target, Some(lane), &mut auto_last[li]),
            _ => mixer(target, Some(lane)),
        }
    }
    for (ci, ei, _, env) in src.envelopes_all() {
        let target = env.resolved;
        if src.lane(target).is_some() || src.first_envelope(target) != Some((ci, ei)) {
            continue;
        }
        match target {
            ResolvedTarget::Node { .. } => {
                node.walk(target, None, &mut env_last[env_base[ci] + ei]);
            }
            _ => mixer(target, None),
        }
    }
    drive
}

impl<'a> Sources<'a> {
    /// Every envelope with points of the active clips.
    fn envelopes_all(
        &self,
    ) -> impl Iterator<Item = (usize, usize, &'a ClipDesc, &'a AutomationDesc)> + '_ {
        self.clips
            .iter()
            .enumerate()
            .filter(|(_, c)| self.is_active(c))
            .flat_map(|(ci, c)| {
                c.envelopes
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| !e.points.is_empty())
                    .map(move |(ei, e)| (ci, ei, c, e))
            })
    }

    fn first_envelope(&self, target: ResolvedTarget) -> Option<(usize, usize)> {
        self.envelopes(target).next().map(|(ci, ei, _, _)| (ci, ei))
    }
}

/// Node-param automation of one track for one sub-block.
struct NodeParams<'s, 'a, 't, 'm> {
    src: &'s Sources<'a>,
    timing: &'t Timing<'t>,
    playing: bool,
    jump: bool,
    chain: &'m mut [ChainRt],
    racks: &'m mut crate::drum_rack::RacksRt,
    chain_racks: &'m mut crate::rack_chains::ChainRacksRt,
    modulation: &'m mut crate::modulation::ModulationRt,
}

impl NodeParams<'_, '_, '_, '_> {
    /// Send `target`'s value at every knot where it changed.
    fn walk(&mut self, target: ResolvedTarget, lane: Option<&AutomationDesc>, last: &mut f64) {
        let ResolvedTarget::Node { node, param } = target else {
            return;
        };
        let timing = self.timing;
        let mut knots = Knots::new();
        if !self.playing || self.jump || last.is_nan() {
            knots.insert(0, f64::NEG_INFINITY);
        }
        if self.playing {
            for g in grid(timing) {
                knots.insert(g, f64::NEG_INFINITY);
            }
            self.src.breakpoints(target, lane, timing, &mut knots);
        }
        // Track-chain node, a device on a drum pad or on a rack chain.
        let events: &mut EventBuffer = match self.chain.iter_mut().find(|c| c.key == node) {
            Some(entry) => &mut entry.events,
            None => match self.racks.events_mut(node) {
                Some(events) => events,
                None => match self.chain_racks.events_mut(node) {
                    Some(events) => events,
                    None => return,
                },
            },
        };
        for &(o, min_beat) in knots.as_slice() {
            let t = if self.playing {
                timing.beat_at(o as f64).max(min_beat)
            } else {
                timing.b0
            };
            let Some((v, mapping)) = self.src.value(target, lane, t) else {
                continue;
            };
            let plain = to_plain(mapping, v);
            if plain == *last {
                continue;
            }
            *last = plain;
            // Modulated params: automation sets the base (`crate::modulation`).
            if !self.modulation.intercept(node, param, plain) {
                events.push(ProcessEvent {
                    offset: o,
                    kind: EventKind::Param {
                        param,
                        value: plain,
                    },
                });
            }
        }
    }
}

/// Stopped: drive a mixer smoother towards the value (v0.1: only when the target changes,
/// so a manual move never sticks while a lane is enabled).
fn set_mixer_target(
    target: ResolvedTarget,
    mapping: &ParamMapping,
    v: f64,
    volume: &mut Smoother,
    pan: &mut Smoother,
    sends: &mut [SendRt],
) {
    let Some((smoother, value)) = mixer_value(target, mapping, v, volume, pan, sends) else {
        return;
    };
    if smoother.target() != value {
        smoother.set_target(value);
    }
}

/// The smoother of a mixer target and its value for normalized `v`.
fn mixer_value<'s>(
    target: ResolvedTarget,
    mapping: &ParamMapping,
    v: f64,
    volume: &'s mut Smoother,
    pan: &'s mut Smoother,
    sends: &'s mut [SendRt],
) -> Option<(&'s mut Smoother, f32)> {
    let plain = to_plain(mapping, v);
    match target {
        ResolvedTarget::TrackVolume => Some((volume, gain_from_plain(mapping, plain))),
        ResolvedTarget::TrackPan => Some((pan, plain.clamp(-1.0, 1.0) as f32)),
        ResolvedTarget::Send { send } => sends
            .iter_mut()
            .find(|s| s.id == send)
            .map(|s| (&mut s.level, gain_from_plain(mapping, plain))),
        ResolvedTarget::Node { .. } => None,
    }
}

/// Chunks `[start, end)` of the sub-block between grid points, with the knot the ramp of
/// each aims at: the next grid point (possibly past the sub-block, which the next
/// sub-block re-targets from where this one stopped), or the loop end when the timeline
/// wraps first. Yields `(start, end, knot offset, knot beat)`.
fn chunks<'a>(timing: &'a Timing<'a>) -> impl Iterator<Item = (usize, usize, usize, f64)> + 'a {
    let n = timing.frames;
    let g = PARAM_GRID as usize;
    let mut c = 0;
    std::iter::from_fn(move || {
        if c >= n {
            return None;
        }
        let phase = ((timing.sample_time + c as u64) % PARAM_GRID) as usize;
        let next = c + (g - phase);
        let (knot, beat) = if next >= n && timing.wraps {
            (n, timing.b1 - EVENT_SHIFT)
        } else {
            (next, timing.beat_at(next as f64))
        };
        let chunk = (c, next.min(n), knot, beat);
        c = next.min(n);
        Some(chunk)
    })
}

/// RT. Fader stage (playing): when automation drives volume or pan, apply the fader in
/// grid chunks with each driven smoother ramped to the value at the chunk's knot. Returns
/// `false` (nothing done) otherwise: the caller runs the plain fader.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fader(
    tdesc: &TrackDesc,
    timing: &Timing<'_>,
    drive: MixerDrive,
    a: &mut Stereo,
    volume: &mut Smoother,
    pan: &mut Smoother,
    gate: &mut Smoother,
) -> bool {
    if !drive.volume && !drive.pan {
        return false;
    }
    let src = Sources::new(tdesc, timing, true);
    let vol_lane = src.lane(ResolvedTarget::TrackVolume);
    let pan_lane = src.lane(ResolvedTarget::TrackPan);
    for (start, end, knot, beat) in chunks(timing) {
        let len = (knot - start) as u32;
        if drive.volume
            && let Some((v, m)) = src.value(ResolvedTarget::TrackVolume, vol_lane, beat)
        {
            volume.ramp_to(gain_from_plain(m, to_plain(m, v)), len);
        }
        if drive.pan
            && let Some((v, m)) = src.value(ResolvedTarget::TrackPan, pan_lane, beat)
        {
            pan.ramp_to(to_plain(m, v).clamp(-1.0, 1.0) as f32, len);
        }
        apply_fader_range(a, volume, pan, gate, start, end);
    }
    true
}

/// RT. Send-level stage (playing): when automation drives this send, scale its buffer in
/// grid chunks with the level ramped to the value at each knot. Returns `false` (nothing
/// done) otherwise: the caller scales with the plain smoother.
pub(crate) fn send_level(
    tdesc: &TrackDesc,
    timing: &Timing<'_>,
    drive: MixerDrive,
    send: &mut SendRt,
) -> bool {
    if !drive.sends {
        return false;
    }
    let target = ResolvedTarget::Send { send: send.id };
    let src = Sources::new(tdesc, timing, true);
    let lane = src.lane(target);
    if lane.is_none() && src.envelopes(target).next().is_none() {
        return false;
    }
    let [l, r] = &mut send.buf;
    for (start, end, knot, beat) in chunks(timing) {
        if let Some((v, m)) = src.value(target, lane, beat) {
            send.level
                .ramp_to(gain_from_plain(m, to_plain(m, v)), (knot - start) as u32);
        }
        scale(&mut l[start..end], &mut send.level, false);
        scale(&mut r[start..end], &mut send.level, true);
    }
    true
}
