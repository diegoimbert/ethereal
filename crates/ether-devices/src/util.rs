//! Small shared DSP and descriptor helpers.

use ether_core::protocol::devices::{ParamInfo, ParamScale, ParamUnit};
use ether_core::protocol::model::ParamId;

/// Builder for a continuous [`ParamInfo`].
pub(crate) fn param(
    id: u32,
    name: &str,
    group: &str,
    unit: ParamUnit,
    (min, max, default): (f64, f64, f64),
    scale: ParamScale,
) -> ParamInfo {
    ParamInfo {
        step: None,
        remote: None,
        id: ParamId(id),
        name: name.to_owned(),
        group: Some(group.to_owned()),
        unit,
        min,
        max,
        default,
        scale,
        labels: None,
        automatable: true,
        hidden: false,
    }
}

/// Builder for a discrete (enum) [`ParamInfo`] with plain values `0..labels.len()-1`.
pub(crate) fn choice(
    id: u32,
    name: &str,
    group: &str,
    unit: ParamUnit,
    labels: &[&str],
    default: usize,
) -> ParamInfo {
    ParamInfo {
        id: ParamId(id),
        name: name.to_owned(),
        group: Some(group.to_owned()),
        unit,
        min: 0.0,
        max: (labels.len() - 1) as f64,
        default: default as f64,
        scale: ParamScale::Linear,
        labels: Some(labels.iter().map(|s| (*s).to_owned()).collect()),
        automatable: true,
        hidden: false,
        step: Some(1.0),
        remote: None,
    }
}

/// Clamp `value` into the range of the param described by `(min, max)`; NaN maps to `min`.
#[inline]
pub(crate) fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value.is_nan() {
        min
    } else {
        value.clamp(min, max)
    }
}

/// Round an enum param's plain value to an index.
#[inline]
pub(crate) fn index(value: f64, count: usize) -> usize {
    (value.round().max(0.0) as usize).min(count - 1)
}

#[inline]
pub(crate) fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

#[inline]
pub(crate) fn amp_to_db(amp: f32) -> f32 {
    20.0 * amp.max(1e-9).log10()
}

/// MIDI key (fractional) to frequency, A4 = 440 Hz.
#[inline]
pub(crate) fn key_to_hz(key: f32) -> f32 {
    440.0 * 2f32.powf((key - 69.0) / 12.0)
}

/// One-pole coefficient such that an exponential segment covers ~-80 dB (1e-4) in `ms`.
/// Used for decay/release segments.
#[inline]
pub(crate) fn exp_coef(ms: f32, sample_rate: f32) -> f32 {
    let samples = (ms * 0.001 * sample_rate).max(1.0);
    (-9.21 / samples).exp()
}

/// One-pole coefficient with time constant `ms` (reaches ~63 % in `ms`).
#[inline]
pub(crate) fn tau_coef(ms: f32, sample_rate: f32) -> f32 {
    let samples = (ms * 0.001 * sample_rate).max(1.0);
    (-1.0 / samples).exp()
}

/// Amplitude envelope (ADSR). Linear attack, exponential decay and release.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Adsr {
    stage: Stage,
    level: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Precomputed per-sample envelope rates (shared by all voices of a device).
#[derive(Clone, Copy, Debug)]
pub(crate) struct AdsrRates {
    /// Level increment per sample during attack.
    pub attack_step: f32,
    pub decay_coef: f32,
    pub sustain: f32,
    pub release_coef: f32,
}

impl AdsrRates {
    pub(crate) fn new(
        attack_ms: f32,
        decay_ms: f32,
        sustain: f32,
        release_ms: f32,
        sample_rate: f32,
    ) -> Self {
        Self {
            attack_step: 1.0 / (attack_ms * 0.001 * sample_rate).max(1.0),
            decay_coef: exp_coef(decay_ms, sample_rate),
            sustain: sustain.clamp(0.0, 1.0),
            release_coef: exp_coef(release_ms, sample_rate),
        }
    }
}

/// Level below which a releasing envelope is considered finished (-100 dB).
const ENV_FLOOR: f32 = 1e-5;

impl Adsr {
    pub(crate) const IDLE: Self = Self {
        stage: Stage::Idle,
        level: 0.0,
    };

    /// Start (or retrigger from the current level, avoiding clicks).
    pub(crate) fn trigger(&mut self) {
        self.stage = Stage::Attack;
    }

    pub(crate) fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub(crate) fn kill(&mut self) {
        *self = Self::IDLE;
    }

    pub(crate) fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    pub(crate) fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }

    /// Current level (0..1), without advancing.
    #[inline]
    pub(crate) fn level(&self) -> f32 {
        self.level
    }

    #[inline]
    pub(crate) fn tick(&mut self, r: &AdsrRates) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += r.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level = r.sustain + (self.level - r.sustain) * r.decay_coef;
                if (self.level - r.sustain).abs() < ENV_FLOOR {
                    self.level = r.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                // Follow sustain changes smoothly.
                self.level = r.sustain + (self.level - r.sustain) * r.decay_coef;
                if self.level < ENV_FLOOR {
                    self.kill();
                }
            }
            Stage::Release => {
                self.level *= r.release_coef;
                if self.level < ENV_FLOOR {
                    self.kill();
                }
            }
        }
        self.level
    }
}

/// Split a block at event offsets: calls `render(start, end)` for each event-free span and
/// `event(kind)` for each event, in order. Offsets past `frames` are clamped.
#[inline]
pub(crate) fn split_at_events<S>(
    state: &mut S,
    events: &[ether_core::ProcessEvent],
    frames: usize,
    mut render: impl FnMut(&mut S, usize, usize),
    mut event: impl FnMut(&mut S, &ether_core::EventKind),
) {
    let mut pos = 0;
    for ev in events {
        let off = (ev.offset as usize).min(frames);
        if off > pos {
            render(state, pos, off);
            pos = off;
        }
        event(state, &ev.kind);
    }
    if pos < frames {
        render(state, pos, frames);
    }
}

/// Where a latent device applies an event, relative to its offset (see [`LateEvents`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// At its offset: params acting on the input as it arrives (detectors, sends, the
    /// latency itself), notes.
    Now,
    /// `n` samples after its offset: params acting on audio that is already `n` samples
    /// into the device's path (output gain, mix, a delay line's read side).
    Late(usize),
    /// Twice: as is `early` samples after its offset, and [`tagged`] `late` samples after
    /// it. For a param with a part inside the path (an oversampled drive) and a part on
    /// the output (the gain compensating it).
    Split { early: usize, late: usize },
}

/// Bit set on the param id of a [`Route::Split`] event's late copy.
const LATE_TAG: u32 = 1 << 31;

/// The late copy of a param event (its id tagged, see [`untag`]).
#[inline]
fn tagged(kind: ether_core::EventKind) -> ether_core::EventKind {
    match kind {
        ether_core::EventKind::Param { param, value } => ether_core::EventKind::Param {
            param: ParamId(param.0 | LATE_TAG),
            value,
        },
        other => other,
    }
}

/// A param id as [`LateEvents`] delivers it: the plain id and whether this is the late
/// copy of a [`Route::Split`] event.
#[inline]
pub(crate) fn untag(id: ParamId) -> (ParamId, bool) {
    (ParamId(id.0 & !LATE_TAG), id.0 & LATE_TAG != 0)
}

/// Most events a [`LateEvents`] holds (32 bytes each).
const LATE_MAX: usize = 8192;
/// Capacity before `prepare` sizes it.
const LATE_DEFAULT: usize = 256;

/// Param events delayed inside a device by (part of) its latency, sample-accurately.
///
/// A device reporting latency `L` gets its input and its events at the same timeline time,
/// and its output plays `L` samples late. A param that acts on audio already `L` samples
/// into the path (output gain, mix, tone after an oversampler, the read side of a delay)
/// must change at its offset `+ L`, else it is heard `L` samples early. Each block, the
/// device routes its events ([`Route`]) and runs the merged list (this block's
/// immediate events and the delayed ones falling due in it, in time order) through
/// [`split_at_events`] as usual; its smoothing then starts at the delayed point.
///
/// Time is the device's own sample count, which skips forward with the engine's
/// `sample_time` when the device wasn't called (bypassed, asleep): events due then apply
/// at the start of the next call. RT: fixed capacity, sized in [`LateEvents::prepare`];
/// when full, the oldest event applies at once (early, never lost).
#[derive(Debug)]
pub(crate) struct LateEvents {
    /// `(due, event)`, sorted by due time (FIFO among equal times).
    pending: Vec<(u64, ether_core::EventKind)>,
    /// The merged list of the current block (lent out by [`LateEvents::take`]).
    merged: Vec<ether_core::ProcessEvent>,
    clock: u64,
    /// Engine sample time expected at the next call.
    next: Option<u64>,
}

impl Default for LateEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl LateEvents {
    /// Non-RT.
    pub(crate) fn new() -> Self {
        Self {
            pending: Vec::with_capacity(LATE_DEFAULT),
            merged: Vec::with_capacity(2 * LATE_DEFAULT),
            clock: 0,
            next: None,
        }
    }

    /// Non-RT. Room for the events of `max_latency` samples (keeps the pending ones).
    pub(crate) fn prepare(&mut self, max_latency: usize, config: &ether_core::PrepareConfig) {
        let per_block = config.max_events_per_block.max(16);
        let blocks = max_latency.div_ceil(config.max_block_size.max(1)) + 1;
        let cap = (2 * per_block * blocks).clamp(LATE_DEFAULT, LATE_MAX);
        if self.pending.capacity() < cap {
            self.pending.reserve_exact(cap - self.pending.len());
        }
        let merged = cap + 2 * per_block;
        if self.merged.capacity() < merged {
            self.merged.reserve_exact(merged - self.merged.len());
        }
    }

    /// No event waiting (a device may only report silence then).
    #[inline]
    pub(crate) fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }

    /// The value of the last pending event of `param`: what `Device::param` reports
    /// (the latest value sent, also while it waits to act).
    pub(crate) fn latest(&self, param: ParamId) -> Option<f64> {
        self.pending.iter().rev().find_map(|(_, k)| match *k {
            ether_core::EventKind::Param { param: p, value } if p == param => Some(value),
            _ => None,
        })
    }

    /// `stored` (a device's own value of `param`, `None` if it has no such param) or the
    /// latest pending one ([`LateEvents::latest`]).
    pub(crate) fn readback(&self, param: ParamId, stored: Option<f64>) -> Option<f64> {
        stored.map(|v| self.latest(param).unwrap_or(v))
    }

    /// Drop the pending events of `param` (a `Device::set_param` overrides them).
    pub(crate) fn cancel(&mut self, param: ParamId) {
        self.pending.retain(|(_, k)| match k {
            ether_core::EventKind::Param { param: p, .. } => untag(*p).0 != param,
            _ => true,
        });
    }

    #[inline]
    fn insert(&mut self, due: u64, kind: ether_core::EventKind) {
        if self.pending.len() == self.pending.capacity() {
            // Full: the oldest applies now.
            let (_, k) = self.pending.remove(0);
            if self.merged.len() < self.merged.capacity() {
                self.merged.push(ether_core::ProcessEvent { offset: 0, kind: k });
            }
        }
        let at = self.pending.partition_point(|(d, _)| *d <= due);
        self.pending.insert(at, (due, kind));
    }

    /// RT. This block's events, merged (see the type docs): give the list back with
    /// [`LateEvents::restore`].
    pub(crate) fn take(
        &mut self,
        ctx: &ether_core::ProcessContext<'_>,
        mut route: impl FnMut(&ether_core::EventKind) -> Route,
    ) -> Vec<ether_core::ProcessEvent> {
        let now = ctx.transport.sample_time;
        if let Some(expected) = self.next
            && now > expected
        {
            self.clock += now - expected;
        }
        self.next = Some(now + ctx.frames as u64);
        self.merged.clear();
        let frames = ctx.frames as u64;
        let start = self.clock;
        for ev in ctx.events {
            let at = start + u64::from(ev.offset).min(frames);
            match route(&ev.kind) {
                Route::Now => self.insert(at, ev.kind),
                Route::Late(n) => self.insert(at + n as u64, ev.kind),
                Route::Split { early, late } => {
                    self.insert(at + early as u64, ev.kind);
                    self.insert(at + late as u64, tagged(ev.kind));
                }
            }
        }
        let end = start + frames;
        let due = self.pending.partition_point(|(d, _)| *d < end);
        for &(at, kind) in &self.pending[..due] {
            if self.merged.len() < self.merged.capacity() {
                self.merged.push(ether_core::ProcessEvent {
                    offset: at.saturating_sub(start) as u32,
                    kind,
                });
            }
        }
        self.pending.drain(..due);
        self.clock = end;
        std::mem::take(&mut self.merged)
    }

    /// RT. Give back the list [`LateEvents::take`] lent.
    #[inline]
    pub(crate) fn restore(&mut self, merged: Vec<ether_core::ProcessEvent>) {
        self.merged = merged;
    }

    /// RT. Apply every pending event now, in order (`reset`: the params end where the
    /// events left them). `late` picks the queue out of the device.
    pub(crate) fn flush<S>(
        state: &mut S,
        late: impl Fn(&mut S) -> &mut Self,
        mut apply: impl FnMut(&mut S, &ether_core::EventKind),
    ) {
        let mut pending = std::mem::take(&mut late(state).pending);
        for (_, kind) in &pending {
            apply(state, kind);
        }
        pending.clear();
        late(state).pending = pending;
    }

    /// RT. [`split_at_events`] over this block's merged events (see the type docs).
    pub(crate) fn split<S>(
        state: &mut S,
        late: impl Fn(&mut S) -> &mut Self,
        ctx: &ether_core::ProcessContext<'_>,
        route: impl FnMut(&ether_core::EventKind) -> Route,
        render: impl FnMut(&mut S, usize, usize),
        event: impl FnMut(&mut S, &ether_core::EventKind),
    ) {
        let events = late(state).take(ctx, route);
        split_at_events(state, &events, ctx.frames, render, event);
        late(state).restore(events);
    }
}

/// Level under which an effect's signal counts as silent: -120 dBFS.
pub(crate) const SILENCE_FLOOR: f32 = 1e-6;

/// Hold of [`Tail::status`] for effects without delay lines (filters ringing out,
/// smoothers): 50 ms.
pub(crate) const BASE_HOLD_SECONDS: f32 = 0.05;

/// `true` when the first `frames` samples of every channel are digital silence.
#[inline]
pub(crate) fn all_zero(channels: &[&[f32]], frames: usize) -> bool {
    channels
        .iter()
        .all(|ch| ch[..frames.min(ch.len())].iter().all(|&s| s == 0.0))
}

/// `true` when every sample of `channels` is under [`SILENCE_FLOOR`].
#[inline]
pub(crate) fn below_floor<'a>(channels: impl IntoIterator<Item = &'a [f32]>) -> bool {
    channels
        .into_iter()
        .all(|ch| ch.iter().all(|s| s.abs() < SILENCE_FLOOR))
}

/// Silence reporting for effects. The engine skips a node that returned
/// [`ProcessStatus::Silent`] while its input stays digital silence and no event reaches it
/// (its output is then silence), so an effect may only say so once nothing it holds can
/// come out any more: its input has been silent (and event-free) for `hold` samples, longer
/// than any internal delay (delay lines, pre-delay, look-ahead) could keep a signal
/// inaudible before it re-emerges, its output (or wet signal) stayed under
/// [`SILENCE_FLOOR`] all that time, and its own state settled (followers released,
/// parameter glides done). Effects that make sound out of silence (DC, noise) never get
/// there, so they keep being processed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Tail {
    /// Samples of silent, event-free input with a quiet output (saturating).
    quiet: usize,
    /// Engine sample time expected at the next `process` (`None` before the first one).
    next: Option<u64>,
}

impl Tail {
    /// RT. At the start of `process`: samples skipped since the previous call (the engine
    /// skipped the asleep node, or it was bypassed), 0 if none. Free-running LFOs advance by
    /// that much so skipping never shifts their phase.
    #[inline]
    pub(crate) fn skipped(&mut self, ctx: &ether_core::ProcessContext<'_>) -> u64 {
        let now = ctx.transport.sample_time;
        let gap = match self.next {
            Some(expected) if now > expected => now - expected,
            _ => 0,
        };
        self.next = Some(now + ctx.frames as u64);
        gap
    }

    /// RT. At the end of `process`: the status to return (see the type docs). `events`:
    /// the block had events; `input_silent`: its inputs were digital silence; `quiet`: the
    /// watched signal stayed under [`SILENCE_FLOOR`]; `settled`: the device's state is idle.
    #[inline]
    pub(crate) fn status(
        &mut self,
        events: bool,
        input_silent: bool,
        quiet: bool,
        settled: bool,
        frames: usize,
        hold: usize,
    ) -> ether_core::ProcessStatus {
        if events || !input_silent || !quiet {
            self.quiet = 0;
            return ether_core::ProcessStatus::Continue;
        }
        self.quiet = self.quiet.saturating_add(frames);
        if settled && self.quiet >= hold.max(1) {
            ether_core::ProcessStatus::Silent
        } else {
            ether_core::ProcessStatus::Continue
        }
    }

    /// RT. [`Tail::status`] watching the device's outputs.
    #[inline]
    pub(crate) fn status_of(
        &mut self,
        ctx: &ether_core::ProcessContext<'_>,
        audio: &ether_core::AudioBuffers<'_, '_>,
        settled: bool,
        hold: usize,
    ) -> ether_core::ProcessStatus {
        let n = ctx.frames;
        let silent_in = all_zero(audio.inputs, n);
        let quiet = silent_in && below_floor(audio.outputs.iter().map(|o| &o[..n.min(o.len())]));
        self.status(!ctx.events.is_empty(), silent_in, quiet, settled, n, hold)
    }
}

/// Samples in `seconds` (rounded up).
#[inline]
pub(crate) fn samples(seconds: f32, sample_rate: f32) -> usize {
    (seconds.max(0.0) * sample_rate).ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_reports_silence_after_the_hold() {
        use ether_core::ProcessStatus::{Continue, Silent};
        let mut t = Tail::default();
        // Signal: never silent.
        assert_eq!(t.status(false, false, false, true, 64, 100), Continue);
        // Silent input, quiet output: silent once the hold is covered.
        assert_eq!(t.status(false, true, true, true, 64, 100), Continue);
        assert_eq!(t.status(false, true, true, true, 64, 100), Silent);
        // An event restarts the hold.
        assert_eq!(t.status(true, true, true, true, 64, 100), Continue);
        assert_eq!(t.status(false, true, true, true, 64, 100), Continue);
        // Not settled: keep going.
        assert_eq!(t.status(false, true, true, false, 64, 100), Continue);
        assert_eq!(t.status(false, true, true, true, 64, 100), Silent);
        // Output above the floor restarts it too.
        assert_eq!(t.status(false, true, false, true, 64, 100), Continue);
        assert_eq!(t.status(false, true, true, true, 64, 100), Continue);
    }

    /// The events [`LateEvents`] delivers for a block of `frames` at `time`.
    fn late_block(
        late: &mut LateEvents,
        time: u64,
        frames: usize,
        events: &[ether_core::ProcessEvent],
        route: impl FnMut(&ether_core::EventKind) -> Route,
    ) -> Vec<(u32, u32, bool)> {
        let transport = ether_core::TransportInfo {
            sample_time: time,
            ..ether_core::TransportInfo::STOPPED
        };
        let mut out_events = ether_core::EventBuffer::with_capacity(4);
        let ctx = ether_core::ProcessContext {
            sample_rate: 48_000.0,
            frames,
            transport: &transport,
            events,
            out_events: &mut out_events,
        };
        let merged = late.take(&ctx, route);
        let got = merged
            .iter()
            .map(|e| match e.kind {
                ether_core::EventKind::Param { param, .. } => {
                    let (id, tag) = untag(param);
                    (e.offset, id.0, tag)
                }
                _ => (e.offset, u32::MAX, false),
            })
            .collect();
        late.restore(merged);
        got
    }

    fn ev(offset: u32, id: u32) -> ether_core::ProcessEvent {
        ether_core::ProcessEvent {
            offset,
            kind: ether_core::EventKind::Param {
                param: ParamId(id),
                value: 0.0,
            },
        }
    }

    #[test]
    fn late_events_fall_due_after_their_delay_in_time_order() {
        let mut late = LateEvents::new();
        let route = |k: &ether_core::EventKind| match k {
            ether_core::EventKind::Param { param, .. } if param.0 == 0 => Route::Now,
            ether_core::EventKind::Param { param, .. } if param.0 == 1 => Route::Late(100),
            _ => Route::Split { early: 10, late: 70 },
        };
        // Block 0..64: id 0 now, id 1 due at 105, id 2 at 30 (+ its late copy at 90).
        let got = late_block(&mut late, 0, 64, &[ev(5, 0), ev(5, 1), ev(20, 2)], route);
        assert_eq!(got, [(5, 0, false), (30, 2, false)]);
        assert!(!late.is_idle());
        // Block 64..128, with a new immediate event at 100.
        let got = late_block(&mut late, 64, 64, &[ev(36, 0)], route);
        assert_eq!(got, [(26, 2, true), (36, 0, false), (41, 1, false)]);
        assert!(late.is_idle());
    }

    #[test]
    fn late_events_skip_ahead_with_the_engine_and_cancel() {
        let mut late = LateEvents::new();
        let route = |_: &ether_core::EventKind| Route::Late(100);
        assert!(late_block(&mut late, 0, 64, &[ev(10, 1), ev(12, 2)], route).is_empty());
        // Not called for 64..192 (asleep / bypassed): due at 110, both at the next call's
        // start; a `set_param` dropped id 2.
        late.cancel(ParamId(2));
        let got = late_block(&mut late, 192, 64, &[], route);
        assert_eq!(got, [(0, 1, false)]);
        assert!(late.is_idle());
    }
    #[test]
    fn adsr_shape() {
        let r = AdsrRates::new(1.0, 10.0, 0.5, 10.0, 1000.0);
        let mut e = Adsr::IDLE;
        e.trigger();
        assert_eq!(e.tick(&r), 1.0);
        for _ in 0..100 {
            e.tick(&r);
        }
        assert!((e.tick(&r) - 0.5).abs() < 1e-3);
        e.release();
        for _ in 0..20 {
            e.tick(&r);
        }
        assert!(!e.is_active());
    }

    #[test]
    fn db_roundtrip() {
        assert!((db_to_amp(-6.0206) - 0.5).abs() < 1e-4);
        assert!((amp_to_db(0.5) + 6.0206).abs() < 1e-3);
        assert!((key_to_hz(69.0) - 440.0).abs() < 1e-3);
    }
}
