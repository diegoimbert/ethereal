//! Sample-accurate automation: arrangement lanes and clip envelopes applied per sample
//! (CONTRACTS.md §12.7; the v0.1 per-block runtime was extracted from `engine.rs` by
//! the v0.2 contracts, then replaced by sample-accurate automation).
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
//! - **mixer targets** (volume, pan, send levels, VCA gain) are ramped linearly per sample
//!   between knots on the grid and at every breakpoint ([`chunks`]): every sample gets the
//!   curve's value at its own beat, and a step lands whole on its own sample (never
//!   before, no ramp after it: the lane's points are the ramp);
//! - **delay compensation** (issue #270): a target whose signal is late relative to the
//!   timeline (a device after latent ones, a bus fed by latent tracks, the fader after a
//!   latent chain) reads its automation that many samples late, where the timeline was
//!   when its audio was rendered ([`views`], `crate::history`), so automation lands on the
//!   audio it belongs to through loops, locates and tempo changes. A latent device's own
//!   params are read at its input, like its audio;
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
use crate::node::NodeKey;
use crate::param::Smoother;
use crate::sched::{self, Timing};

/// The frozen v0.2 automation grid (samples of the absolute engine clock).
pub const PARAM_GRID: u64 = 32;
/// Knots (event offsets) per target and sub-block beyond which breakpoints are dropped (the
/// grid still covers them): `max_block_size / PARAM_GRID` grid points plus breakpoints.
pub(crate) const MAX_KNOTS: usize = 256;

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
pub(crate) struct Knots {
    at: [(u32, f64); MAX_KNOTS],
    len: usize,
}

impl Knots {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn as_slice(&self) -> &[(u32, f64)] {
        &self.at[..self.len]
    }
}

/// RT. Knots of a lane's points whose sample is before `until` ([`lookahead`]) in a
/// playing view (VCA lanes, `crate::vca`).
pub(crate) fn lane_breakpoints(
    points: &[(f64, f64, ether_protocol::model::CurveShape)],
    timing: &Timing<'_>,
    until: usize,
    knots: &mut Knots,
) {
    let lo = timing.beat_at(-1.0);
    let hi = timing.beat_at(until as f64);
    let first = points.partition_point(|p| p.0 <= lo);
    for p in &points[first..] {
        if p.0 >= hi {
            break;
        }
        let o = timing.sample_at_or_after_upto(p.0, until);
        if o < until {
            knots.insert(o, p.0);
        }
    }
}

/// RT. Index of the first clip that may still play at or after beat `lo`: every clip before
/// it ends at or before `lo`. `ends` is the running maximum of the clips' ends
/// ([`clip_ends`]), so this is a binary search however long the song is.
#[inline]
pub(crate) fn first_live_clip(ends: &[f64], lo: f64) -> usize {
    ends.partition_point(|&e| e <= lo)
}

/// Non-RT. Running maximum of `start + max(length, 0)` over `clips`: the lookup table of
/// [`first_live_clip`], built with the snapshot.
pub(crate) fn clip_ends(clips: &[ClipDesc]) -> Vec<f64> {
    clips
        .iter()
        .scan(f64::NEG_INFINITY, |max, c| {
            *max = max.max(c.start + c.length.max(0.0));
            Some(*max)
        })
        .collect()
}

/// A stretch of the sub-block whose automation is read at `timing`: the sub-block itself, or
/// for a target whose signal is late (plugin delay compensation, issue #270) a piece of the
/// timeline as the engine rendered it that many samples earlier ([`views`]).
pub(crate) struct View<'t> {
    /// Offset of the stretch in the sub-block.
    pub at: usize,
    pub timing: Timing<'t>,
    pub playing: bool,
}

/// RT. Call `f` with the views covering the sub-block for a target whose signal is `delay`
/// samples late relative to the timeline (the latency at the device's input, or after the
/// chain for the fader, sends and VCA gain): the automation it gets at sample `s` is the
/// timeline's at `s - delay`, through loops, locates and tempo changes like the audio
/// (`crate::history`). Without a delay (or stopped), the sub-block itself.
pub(crate) fn views<'t>(
    timing: &Timing<'t>,
    playing: bool,
    delay: u32,
    mut f: impl FnMut(View<'t>),
) {
    match timing.history {
        Some(history) if delay > 0 && playing => {
            let (tempo, sr) = (timing.tempo, timing.sample_rate);
            let from = timing.sample_time as i64 - i64::from(delay);
            history.pieces(from, timing.frames, tempo, sr, |at, plan| {
                f(View {
                    at,
                    timing: plan.view(tempo, sr),
                    playing: plan.playing,
                });
            });
        }
        _ => f(View {
            at: 0,
            timing: *timing,
            playing,
        }),
    }
}

/// `delays` (sorted by node): the latency at `node`'s input (0 when unknown).
fn delay_of(delays: &[(NodeKey, u32)], node: NodeKey) -> u32 {
    delays
        .binary_search_by_key(&node, |e| e.0)
        .map_or(0, |i| delays[i].1)
}

/// The arrangement lane of `target` (the first one with points).
fn lane_of(lanes: &[AutomationDesc], target: ResolvedTarget) -> Option<&AutomationDesc> {
    lanes
        .iter()
        .find(|l| l.resolved == target && !l.points.is_empty())
}

/// Non-RT (graph compile). The targets only clip envelopes automate (no lane with points
/// drives them), each once: [`apply_automation`] keeps one last-sent value per target.
pub(crate) fn envelope_targets(tdesc: &TrackDesc) -> Vec<ResolvedTarget> {
    let mut out: Vec<ResolvedTarget> = Vec::new();
    for env in tdesc.clips.iter().flat_map(|c| &c.envelopes) {
        if env.points.is_empty()
            || lane_of(&tdesc.automation, env.resolved).is_some()
            || out.contains(&env.resolved)
        {
            continue;
        }
        out.push(env.resolved);
    }
    out
}

/// The automation of one track that overlaps a sub-block.
struct Sources<'a> {
    /// Clips starting before the sub-block end, from the first one that may end after `lo`
    /// (every earlier clip ends at or before `lo`: none of them is active).
    clips: &'a [ClipDesc],
    /// Clips ending after this beat take part: one sample before the sub-block start while
    /// playing (a clip ending exactly on the sub-block start hands over at offset 0), the
    /// position while stopped.
    lo: f64,
}

impl<'a> Sources<'a> {
    fn new(tdesc: &'a TrackDesc, ends: &[f64], timing: &Timing<'_>, playing: bool) -> Self {
        let (lo, end) = if playing {
            // Mixer ramps look up to one grid interval past the sub-block end.
            let ahead = timing.beat_at((timing.frames as u64 + PARAM_GRID) as f64);
            (timing.beat_at(-1.0), ahead.max(timing.b1))
        } else {
            (timing.b0, timing.b0 + 1e-9)
        };
        let clips_end = tdesc.clips.partition_point(|c| c.start < end);
        // `start + length > lo` implies `start + max(length, 0) > lo`: the skipped clips
        // are never active.
        let base = first_live_clip(ends, lo).min(clips_end);
        Self {
            clips: &tdesc.clips[base..clips_end],
            lo,
        }
    }

    fn is_active(&self, clip: &ClipDesc) -> bool {
        !clip.muted && clip.start + clip.length > self.lo
    }

    /// The active clips' envelopes of `target`.
    fn envelopes(
        &self,
        target: ResolvedTarget,
    ) -> impl Iterator<Item = (&'a ClipDesc, &'a AutomationDesc)> + '_ {
        self.clips
            .iter()
            .filter(|c| self.is_active(c))
            .flat_map(move |c| {
                c.envelopes
                    .iter()
                    .filter(move |e| e.resolved == target && !e.points.is_empty())
                    .map(move |e| (c, e))
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
        for (clip, env) in self.envelopes(target) {
            if let Some(v) = sched::content_at(clip, t).and_then(|c| evaluate(&env.points, c)) {
                out = Some((v, &env.mapping));
            }
        }
        out.or_else(|| lane.and_then(|l| evaluate(&l.points, t).map(|v| (v, &l.mapping))))
    }

    /// Knots of every breakpoint of `target` whose sample is before `until` (the sub-block
    /// length, or past it for mixer ramps looking ahead, [`lookahead`]): lane points,
    /// envelope points on the timeline, and where clips start, end or loop.
    fn breakpoints(
        &self,
        target: ResolvedTarget,
        lane: Option<&'a AutomationDesc>,
        timing: &Timing<'_>,
        until: usize,
        knots: &mut Knots,
    ) {
        let lo = self.lo;
        let hi = if until > timing.frames {
            timing.beat_at(until as f64)
        } else {
            timing.b1
        };
        let mut add = |t: f64| {
            if t > lo && t < hi {
                let o = timing.sample_at_or_after_upto(t, until);
                if o < until {
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
        for (clip, env) in self.envelopes(target) {
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
/// (indexed by lane) / `env_last` (indexed by `env_targets`, [`envelope_targets`]) hold the
/// last plain value sent per node-param target (NaN = send again). `ends`: [`clip_ends`]
/// of the track's clips. `delays`: the latency at each node's input (sorted by node), by
/// which its automation is delayed ([`views`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_automation(
    tdesc: &TrackDesc,
    ends: &[f64],
    timing: &Timing<'_>,
    playing: bool,
    jump: bool,
    auto_last: &mut [f64],
    env_targets: &[ResolvedTarget],
    env_last: &mut [f64],
    delays: &[(NodeKey, u32)],
    volume: &mut Smoother,
    pan: &mut Smoother,
    sends: &mut [SendRt],
    chain: &mut [ChainRt],
    racks: &mut crate::drum_rack::RacksRt,
    chain_racks: &mut crate::rack_chains::ChainRacksRt,
    modulation: &mut crate::modulation::ModulationRt,
) -> MixerDrive {
    let mut drive = MixerDrive::default();
    if tdesc.automation.is_empty() && env_targets.is_empty() {
        // Modulated switches override the entries' switches every sub-block: start from the
        // static ones (`ModulationRt::render` reads them as the base).
        if modulation.drives_switches() {
            for e in chain.iter_mut() {
                e.clear_auto();
            }
            racks.clear_auto_enabled();
            chain_racks.clear_auto_enabled();
        }
        return drive;
    }
    // Device on/off overrides are re-evaluated below (none where nothing drives them).
    for e in chain.iter_mut() {
        e.clear_auto();
    }
    racks.clear_auto_enabled();
    chain_racks.clear_auto_enabled();
    let src = Sources::new(tdesc, ends, timing, playing);
    let mut node = NodeParams {
        tdesc,
        ends,
        delays,
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
                ResolvedTarget::Node { .. } | ResolvedTarget::NodeEnabled { .. } => {}
            }
        } else if let Some((v, mapping)) = src.value(target, lane, timing.b0) {
            set_mixer_target(target, mapping, v, volume, pan, sends);
        }
    };
    // Each target once: lanes first (the lane's slot keeps the last value, also while an
    // envelope drives it), then targets only clip envelopes automate.
    for (li, lane) in tdesc.automation.iter().enumerate() {
        let target = lane.resolved;
        if lane.points.is_empty()
            || lane_of(&tdesc.automation, target).is_some_and(|l| !std::ptr::eq(l, lane))
        {
            continue;
        }
        match target {
            ResolvedTarget::Node { .. } => node.walk(target, Some(lane), &mut auto_last[li]),
            ResolvedTarget::NodeEnabled { .. } => node.switch(target, Some(lane)),
            _ => mixer(target, Some(lane)),
        }
    }
    for (&target, last) in env_targets.iter().zip(env_last.iter_mut()) {
        match target {
            ResolvedTarget::Node { .. } => node.walk(target, None, last),
            ResolvedTarget::NodeEnabled { .. } => node.switch(target, None),
            _ => mixer(target, None),
        }
    }
    drive
}

/// Node-param automation of one track for one sub-block.
struct NodeParams<'a, 't, 'm> {
    tdesc: &'a TrackDesc,
    ends: &'a [f64],
    delays: &'a [(NodeKey, u32)],
    timing: &'t Timing<'t>,
    playing: bool,
    jump: bool,
    chain: &'m mut [ChainRt],
    racks: &'m mut crate::drum_rack::RacksRt,
    chain_racks: &'m mut crate::rack_chains::ChainRacksRt,
    modulation: &'m mut crate::modulation::ModulationRt,
}

impl NodeParams<'_, '_, '_> {
    /// Device on/off: the value at the sub-block start overrides the device's switch for
    /// this sub-block, wherever the device is. While playing, the first knot inside the
    /// sub-block (breakpoints and grid points) where the value crosses 0.5 is where it
    /// switches (`ChainRt::auto_switch_at`: sample-accurate crossfade, MIDI thru and note
    /// releases). Read at the device's input latency ([`views`]).
    fn switch(&mut self, target: ResolvedTarget, lane: Option<&AutomationDesc>) {
        let ResolvedTarget::NodeEnabled { node } = target else {
            return;
        };
        let (tdesc, ends) = (self.tdesc, self.ends);
        let mut on: Option<bool> = None;
        let mut at = None;
        views(
            self.timing,
            self.playing,
            delay_of(self.delays, node),
            |v| {
                if at.is_some() {
                    return;
                }
                let src = Sources::new(tdesc, ends, &v.timing, v.playing);
                let start = src.value(target, lane, v.timing.b0).map(|(x, _)| x >= 0.5);
                let Some(state) = on.or(start) else {
                    return;
                };
                if on.is_none() {
                    on = Some(state);
                } else if start.is_some_and(|s| s != state) {
                    at = Some(v.at as u32);
                    return;
                }
                if !v.playing {
                    return;
                }
                let mut knots = Knots::new();
                for g in grid(&v.timing) {
                    knots.insert(g, f64::NEG_INFINITY);
                }
                src.breakpoints(target, lane, &v.timing, v.timing.frames, &mut knots);
                at = knots.as_slice().iter().find_map(|&(o, min_beat)| {
                    let t = v.timing.beat_at(o as f64).max(min_beat);
                    let (x, _) = src.value(target, lane, t)?;
                    (o > 0 && (x >= 0.5) != state).then_some(v.at as u32 + o)
                });
            },
        );
        let Some(on) = on else {
            return;
        };
        let entry = match self.chain.iter_mut().find(|c| c.key == node) {
            Some(e) => Some(e),
            None => match self.racks.entry_mut(node) {
                Some(e) => Some(e),
                None => self.chain_racks.entry_mut(node),
            },
        };
        if let Some(e) = entry {
            e.set_auto(on, at);
        }
    }

    /// Send `target`'s value at every knot where it changed, read at the device's input
    /// latency ([`views`]).
    fn walk(&mut self, target: ResolvedTarget, lane: Option<&AutomationDesc>, last: &mut f64) {
        let ResolvedTarget::Node { node, param } = target else {
            return;
        };
        let delay = delay_of(self.delays, node);
        let Self {
            tdesc,
            ends,
            timing,
            playing,
            jump,
            chain,
            racks,
            chain_racks,
            modulation,
            ..
        } = self;
        // Track-chain node, a device on a drum pad or on a rack chain.
        let events: &mut EventBuffer = match chain.iter_mut().find(|c| c.key == node) {
            Some(entry) => &mut entry.events,
            None => match racks.events_mut(node) {
                Some(events) => events,
                None => match chain_racks.events_mut(node) {
                    Some(events) => events,
                    None => return,
                },
            },
        };
        views(timing, *playing, delay, |v| {
            let src = Sources::new(tdesc, ends, &v.timing, v.playing);
            let mut knots = Knots::new();
            // Delayed views always read their start (their timeline may jump there).
            if !v.playing || *jump || last.is_nan() || delay > 0 {
                knots.insert(0, f64::NEG_INFINITY);
            }
            if v.playing {
                for g in grid(&v.timing) {
                    knots.insert(g, f64::NEG_INFINITY);
                }
                src.breakpoints(target, lane, &v.timing, v.timing.frames, &mut knots);
            }
            for &(o, min_beat) in knots.as_slice() {
                let t = if v.playing {
                    v.timing.beat_at(o as f64).max(min_beat)
                } else {
                    v.timing.b0
                };
                let Some((x, mapping)) = src.value(target, lane, t) else {
                    // Nothing drives it here (no envelope): send again when one does.
                    *last = f64::NAN;
                    continue;
                };
                let plain = to_plain(mapping, x);
                if plain == *last {
                    continue;
                }
                *last = plain;
                let o = v.at as u32 + o;
                // Modulated params: automation sets the base (`crate::modulation`).
                if !modulation.intercept_at(node, param, plain, o) {
                    events.push(ProcessEvent {
                        offset: o,
                        kind: EventKind::Param {
                            param,
                            value: plain,
                        },
                    });
                }
            }
        });
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
        ResolvedTarget::Node { .. } | ResolvedTarget::NodeEnabled { .. } => None,
    }
}

/// How far past the sub-block mixer ramps look for breakpoints: up to the next grid point
/// (none past a loop end, where the timeline wraps).
pub(crate) fn lookahead(timing: &Timing<'_>) -> usize {
    if timing.wraps {
        timing.frames
    } else {
        timing.frames + PARAM_GRID as usize
    }
}

/// Ramp chunks `[start, end)` of a view: boundaries on the grid and at `breaks` (sorted
/// sample offsets of breakpoints, possibly past the view, [`lookahead`]). Each chunk's ramp
/// aims at the knot ending it: yields `(start, end, knot, beat)` = ramp over `knot - start`
/// samples (possibly past the view: the next one re-targets from where this one stopped) to
/// the value at `beat`, the beat of sample `knot - 1`, so every sample gets the curve's
/// value at its own beat (linear in between). A breakpoint ends a chunk at the value just
/// before it and is a one-sample chunk of its own at the value at its beat (or later), so
/// a step lands whole on its own sample: never before, and with no ramp after it (issue
/// #302). Stopped views are one chunk at the position.
pub(crate) fn chunks<'a>(
    timing: &'a Timing<'a>,
    playing: bool,
    breaks: &'a [(u32, f64)],
) -> impl Iterator<Item = (usize, usize, usize, f64)> + 'a {
    let n = timing.frames;
    let g = PARAM_GRID as usize;
    let mut c = 0;
    let mut bi = 0;
    std::iter::from_fn(move || {
        if c >= n {
            return None;
        }
        if !playing {
            c = n;
            return Some((0, n, n, timing.b0));
        }
        while bi < breaks.len() && (breaks[bi].0 as usize) < c {
            bi += 1;
        }
        if let Some(&(b, min_beat)) = breaks.get(bi)
            && b as usize == c
        {
            // A breakpoint's own sample has the curve's value there, with no ramp: a step
            // lands whole on its sample (issue #302).
            bi += 1;
            c += 1;
            return Some((c - 1, c, c, timing.beat_at((c - 1) as f64).max(min_beat)));
        }
        let phase = ((timing.sample_time + c as u64) % PARAM_GRID) as usize;
        let mut next = c + (g - phase);
        if let Some(&(b, _)) = breaks.get(bi) {
            next = next.min(b as usize);
        }
        let knot = if next >= n && timing.wraps { n } else { next };
        let beat = timing.beat_at((knot - 1) as f64);
        let chunk = (c, next.min(n), knot, beat);
        c = next.min(n);
        Some(chunk)
    })
}

/// RT. Fader stage (playing): when automation drives volume or pan, apply the fader in
/// ramp chunks ([`chunks`]) with each driven smoother ramped to its value at the chunk's
/// knot, read `delay` samples late (the latency after the chain, [`views`]). Returns
/// `false` (nothing done) otherwise: the caller runs the plain fader.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fader(
    tdesc: &TrackDesc,
    ends: &[f64],
    timing: &Timing<'_>,
    drive: MixerDrive,
    delay: u32,
    a: &mut Stereo,
    volume: &mut Smoother,
    pan: &mut Smoother,
) -> bool {
    if !drive.volume && !drive.pan {
        return false;
    }
    let vol_lane = lane_of(&tdesc.automation, ResolvedTarget::TrackVolume);
    let pan_lane = lane_of(&tdesc.automation, ResolvedTarget::TrackPan);
    views(timing, true, delay, |v| {
        let src = Sources::new(tdesc, ends, &v.timing, v.playing);
        let mut breaks = Knots::new();
        if v.playing {
            let until = lookahead(&v.timing);
            if drive.volume {
                let target = ResolvedTarget::TrackVolume;
                src.breakpoints(target, vol_lane, &v.timing, until, &mut breaks);
            }
            if drive.pan {
                let target = ResolvedTarget::TrackPan;
                src.breakpoints(target, pan_lane, &v.timing, until, &mut breaks);
            }
        }
        for (start, end, knot, beat) in chunks(&v.timing, v.playing, breaks.as_slice()) {
            let len = (knot - start) as u32;
            if drive.volume
                && let Some((x, m)) = src.value(ResolvedTarget::TrackVolume, vol_lane, beat)
            {
                volume.ramp_to(gain_from_plain(m, to_plain(m, x)), len);
            }
            if drive.pan
                && let Some((x, m)) = src.value(ResolvedTarget::TrackPan, pan_lane, beat)
            {
                pan.ramp_to(to_plain(m, x).clamp(-1.0, 1.0) as f32, len);
            }
            apply_fader_range(a, volume, pan, v.at + start, v.at + end);
        }
    });
    true
}

/// RT. Send-level stage (playing): when automation drives this send, scale its buffer in
/// ramp chunks ([`chunks`]) with the level ramped to the value at each knot, read `delay`
/// samples late ([`views`]). Returns `false` (nothing done) otherwise: the caller scales
/// with the plain smoother.
pub(crate) fn send_level(
    tdesc: &TrackDesc,
    ends: &[f64],
    timing: &Timing<'_>,
    drive: MixerDrive,
    delay: u32,
    send: &mut SendRt,
) -> bool {
    if !drive.sends {
        return false;
    }
    let target = ResolvedTarget::Send { send: send.id };
    let lane = lane_of(&tdesc.automation, target);
    if lane.is_none()
        && !tdesc
            .clips
            .iter()
            .any(|c| c.envelopes.iter().any(|e| e.resolved == target))
    {
        return false;
    }
    let SendRt { buf, level, .. } = send;
    let [l, r] = buf;
    views(timing, true, delay, |v| {
        let src = Sources::new(tdesc, ends, &v.timing, v.playing);
        let mut breaks = Knots::new();
        if v.playing {
            src.breakpoints(target, lane, &v.timing, lookahead(&v.timing), &mut breaks);
        }
        for (start, end, knot, beat) in chunks(&v.timing, v.playing, breaks.as_slice()) {
            if let Some((x, m)) = src.value(target, lane, beat) {
                level.ramp_to(gain_from_plain(m, to_plain(m, x)), (knot - start) as u32);
            }
            let (start, end) = (v.at + start, v.at + end);
            scale(&mut l[start..end], level, false);
            scale(&mut r[start..end], level, true);
        }
    });
    true
}
