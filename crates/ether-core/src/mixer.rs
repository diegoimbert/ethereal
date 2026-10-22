//! Per-track runtime state of a compiled snapshot: device-chain buffers, fader, pan,
//! mute/solo gate, sends, PDC delay lines, active notes and meters.
//!
//! Everything here is allocated by the compiler (off the audio thread). When a snapshot is
//! swapped in, the audio thread moves the running state (smoother positions, delay-line
//! contents, sounding notes, meter accumulators) over from the previous snapshot's track
//! with the same id ([`TrackRt::inherit`]); that only swaps pointers, never allocates.

use ether_protocol::model::{ClipId, SendId, TrackId};

use crate::config::EngineConfig;
use crate::delay::{DelayLine, DryLine};
use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::node::NodeKey;
use crate::param::Smoother;

/// Ramp time for fader, pan, send and mute changes.
pub(crate) const MIX_RAMP_MS: f32 = 10.0;
/// Sounding notes tracked per track (for note-offs at note end, stop, loop and locate).
pub(crate) const MAX_ACTIVE_NOTES: usize = 512;
/// Live parameter events queued per chain node between two blocks.
pub(crate) const MAX_PENDING_EVENTS: usize = 128;

pub(crate) type Stereo = [Vec<f32>; 2];

pub(crate) fn stereo(frames: usize) -> Stereo {
    [vec![0.0; frames], vec![0.0; frames]]
}

#[derive(Debug)]
pub(crate) struct ChainRt {
    pub key: NodeKey,
    pub enabled: bool,
    /// Device on/off automation (`ResolvedTarget::NodeEnabled`): overrides `enabled` while
    /// a lane or clip envelope drives it; `None` = the static switch.
    pub auto_enabled: Option<bool>,
    /// Sample of the sub-block where the automation flips the switch. Both switches are
    /// sample-accurate: the audio crossfade turns there, and MIDI thru and note releases
    /// change state there ([`Self::route_midi`]); [`Self::is_enabled`] is the value at the
    /// sub-block start.
    pub auto_switch_at: Option<u32>,
    pub channels: (u16, u16),
    /// Events for the current sub-block (sorted before `process`).
    pub events: EventBuffer,
    /// Live param changes waiting for the next sub-block.
    pub pending: EventBuffer,
    /// The latest param changes that arrived while the device was off (one per param),
    /// delivered where it is on again so it never resumes with stale values (issue #270).
    held: EventBuffer,
    /// The entry reads a sidechain (planned in the track's `crate::sidechain::Taps`).
    pub sidechain: bool,
    /// Whether the device ran the last time its chain reached it ([`Self::switch_notes`];
    /// carried over snapshot swaps by node, [`inherit_switches`]).
    pub was_enabled: bool,
    /// Click-free switching: gain of the processed signal against the bypass path (1 = on),
    /// ramped over [`SWITCH_FADE_MS`]; carried over snapshot swaps.
    pub fade: f32,
    fade_step: f32,
    /// The node ran in the last sub-block (a bypassed one doesn't).
    ran: bool,
    /// Samples the fade-in still waits after the node started running again: its latency,
    /// so a look-ahead device's output is warm (not its stale or empty delay buffer) when
    /// it fades in.
    warmup: u32,
    /// The bypass path: the entry's input delayed by the entry's latency, so switching it
    /// (statically or by automation) never moves the signal in time (a bypassed device's
    /// latency still counts in PDC). None for entries without audio outputs.
    dry: DryLine,
}

/// Crossfade between a device's output and its bypass path when it switches on or off.
pub(crate) const SWITCH_FADE_MS: f32 = 10.0;

/// Distinct params whose changes an off device keeps for when it is on again
/// (`ChainRt::held`; beyond that, the oldest are dropped).
const MAX_HELD_PARAMS: usize = 64;

impl ChainRt {
    /// Non-RT. A chain entry with its event lists. `latency`: how much the bypass path
    /// delays the entry's input (its latency plus a sidechain main delay planned before it).
    pub(crate) fn new(
        key: NodeKey,
        enabled: bool,
        channels: (u16, u16),
        sidechain: bool,
        latency: usize,
        config: &EngineConfig,
    ) -> Self {
        let fade_frames = SWITCH_FADE_MS * 0.001 * config.sample_rate.max(1) as f32;
        Self {
            key,
            enabled,
            auto_enabled: None,
            auto_switch_at: None,
            channels,
            events: EventBuffer::with_capacity(config.max_events_per_block),
            pending: EventBuffer::with_capacity(MAX_PENDING_EVENTS),
            held: EventBuffer::with_capacity(MAX_HELD_PARAMS),
            sidechain,
            was_enabled: enabled,
            fade: if enabled { 1.0 } else { 0.0 },
            fade_step: 1.0 / fade_frames.max(1.0),
            ran: enabled,
            warmup: 0,
            dry: if channels.1 > 0 {
                DryLine::new(latency, config.max_block_size)
            } else {
                DryLine::default()
            },
        }
    }

    /// Whether the device is on this sub-block (automation wins over the static switch).
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.auto_enabled.unwrap_or(self.enabled)
    }

    /// RT. Automation drives the switch this sub-block: `on` at its start, flipping at
    /// sample `at` (if it does).
    pub(crate) fn set_auto(&mut self, on: bool, at: Option<u32>) {
        self.auto_enabled = Some(on);
        self.auto_switch_at = at;
    }

    /// RT. Back to the static switch (automation is re-evaluated every sub-block).
    pub(crate) fn clear_auto(&mut self) {
        self.auto_enabled = None;
        self.auto_switch_at = None;
    }

    /// Whether the node must run this sub-block: on, switched inside it, or still fading
    /// out. Entries without audio outputs switch instantly (no fade).
    #[inline]
    pub(crate) fn runs(&self) -> bool {
        self.is_enabled() || self.auto_switch_at.is_some() || (self.dry.active() && self.fade > 0.0)
    }

    /// Whether the device is off at `offset` of this sub-block (an automated switch flips
    /// at [`Self::auto_switch_at`]).
    #[inline]
    fn off_at(&self, offset: u32) -> bool {
        self.is_enabled() == (offset >= self.auto_switch_at.unwrap_or(u32::MAX))
    }

    /// RT. Call when the chain reaches this entry: releases the notes a switch strands
    /// ([`Self::switch_notes`], and at an automated flip inside the sub-block, on its
    /// sample), sorts the entry's events, then passes those that arrive while the device is
    /// off on to `next` (MIDI thru: an off device is as if absent; its own params stay
    /// behind: the latest per param is delivered where the device is on again). The device
    /// keeps the events of the span it is on. `next`: the next entry's events, or where the
    /// chain's MIDI goes (`None`: nowhere). Returns `true` if an event list was full.
    pub(crate) fn route_midi(&mut self, mut next: Option<&mut EventBuffer>) -> bool {
        let mut overflow = self.switch_notes(next.as_deref_mut());
        if let Some(at) = self.auto_switch_at {
            // The second transition, on the flip's sample (`switch_notes` at the next
            // sub-block then sees none).
            let on = !self.is_enabled();
            self.was_enabled = on;
            let off = ProcessEvent {
                offset: at,
                kind: EventKind::AllNotesOff,
            };
            if on {
                overflow |= !self.events.push(off);
            }
            if let Some(next) = next.as_deref_mut() {
                overflow |= !next.push(off);
            }
        }
        let flip = self.auto_switch_at.unwrap_or(u32::MAX);
        let on = self.is_enabled();
        // Param changes held while off, where the device is on again (before this
        // sub-block's own changes at that sample, which win).
        let on_at = if on {
            Some(0)
        } else {
            (flip < u32::MAX).then_some(flip)
        };
        if let Some(at) = on_at
            && !self.held.is_empty()
        {
            for h in self.held.as_slice() {
                let EventKind::Param { param, .. } = h.kind else {
                    continue;
                };
                let fresh = self.events.as_slice().iter().any(|e| {
                    e.offset == at
                        && matches!(e.kind, EventKind::Param { param: p, .. } if p == param)
                });
                if !fresh {
                    overflow |= !self.events.push(ProcessEvent {
                        offset: at,
                        kind: h.kind,
                    });
                }
            }
            self.held.clear();
        }
        self.events.sort();
        if on && flip == u32::MAX {
            return overflow;
        }
        let held = &mut self.held;
        self.events.retain(|e| {
            if on != (e.offset >= flip) {
                return true;
            }
            match e.kind {
                EventKind::Param { param, .. } => {
                    held.retain(
                        |h| !matches!(h.kind, EventKind::Param { param: p, .. } if p == param),
                    );
                    if !held.push(*e) {
                        // Full: drop the oldest.
                        let mut first = true;
                        held.retain(|_| !std::mem::take(&mut first));
                        held.push(*e);
                    }
                }
                _ => {
                    if let Some(next) = next.as_deref_mut() {
                        overflow |= !next.push(*e);
                    }
                }
            }
            false
        });
        overflow
    }

    /// RT. After the node ran: keep only the MIDI it sent while on (one fading out, or off
    /// before or after an automated flip, sends none: its input went thru).
    pub(crate) fn gate_output(&self, out: &mut EventBuffer) {
        if self.is_enabled() && self.auto_switch_at.is_none() {
            return;
        }
        out.retain(|e| !self.off_at(e.offset));
    }

    /// RT. Keep this sub-block's input for the bypass path. Call before anything writes into
    /// `input` (a sidechain alignment, a rack's chains), on every sub-block.
    #[inline]
    pub(crate) fn push_dry(&mut self, input: &Stereo, n: usize) {
        self.dry.push(input, n);
    }

    /// RT. The entry doesn't run: its output is its delayed input (written into `io`).
    pub(crate) fn bypass(&mut self, io: &mut Stereo, n: usize) {
        if self.dry.active() {
            for (ch, x) in io.iter_mut().enumerate() {
                for (i, s) in x[..n].iter_mut().enumerate() {
                    *s = self.dry.get(ch, i);
                }
            }
        }
        self.fade = 0.0;
        self.ran = false;
    }

    /// RT. The node ran into `out` (both channels): crossfade with the bypass path while
    /// the switch ramps (equal-gain, [`SWITCH_FADE_MS`]; a node that just started running
    /// fades in after its latency); nothing to do once settled on.
    pub(crate) fn blend(&mut self, out: &mut Stereo, n: usize) {
        let on = self.is_enabled();
        let flip = self.auto_switch_at.map_or(usize::MAX, |a| a as usize);
        if !self.ran {
            self.ran = true;
            self.warmup = self.dry.delay() as u32;
        }
        if !self.dry.active() {
            self.fade = if on != (flip < n) { 1.0 } else { 0.0 };
            return;
        }
        if self.fade >= 1.0 && on && flip >= n {
            return;
        }
        let (start, step, hold) = (self.fade, self.fade_step, self.warmup as usize);
        let mut fade = start;
        for (ch, x) in out.iter_mut().enumerate() {
            fade = start;
            for (i, s) in x[..n].iter_mut().enumerate() {
                fade = if on == (i >= flip) {
                    (fade - step).max(0.0)
                } else if i >= hold {
                    (fade + step).min(1.0)
                } else {
                    fade
                };
                let d = self.dry.get(ch, i);
                *s = d + (*s - d) * fade;
            }
        }
        self.fade = fade;
        self.warmup = hold.saturating_sub(n) as u32;
    }

    /// RT. Call when the chain reaches this entry, before its events are sorted. When the
    /// device was switched on or off since it was last reached (static switch or On/Off
    /// automation, on a track chain, a rack chain or a drum pad), release the notes the
    /// switch strands: an off device is MIDI thru, so the notes it generated (on → off)
    /// or passed through (off → on) would never get their note-offs at `next` (the next
    /// entry's events, or where the chain's MIDI goes), and a device that comes back on
    /// missed the note-offs sent while it was off. `AllNotesOff` at offset 0 goes to `next`
    /// on every transition and to the device itself when it comes back on. Returns `true`
    /// if an event list was full.
    pub(crate) fn switch_notes(&mut self, next: Option<&mut EventBuffer>) -> bool {
        let on = self.is_enabled();
        if on == self.was_enabled {
            return false;
        }
        self.was_enabled = on;
        let off = ProcessEvent {
            offset: 0,
            kind: EventKind::AllNotesOff,
        };
        let mut overflow = false;
        if on {
            overflow |= !self.events.push(off);
        }
        if let Some(next) = next {
            overflow |= !next.push(off);
        }
        overflow
    }
}

/// RT. Carry each entry's last on/off state over a snapshot swap (same node), so a switch
/// changed by the new snapshot counts as a transition ([`ChainRt::switch_notes`]) and
/// crossfades from where it was ([`ChainRt::blend`]).
pub(crate) fn inherit_switches(new: &mut [ChainRt], old: &mut [ChainRt]) {
    for e in new {
        if let Some(o) = old.iter_mut().find(|o| o.key == e.key) {
            e.was_enabled = o.was_enabled;
            // The crossfade and the bypass path continue (same delay).
            e.fade = o.fade;
            e.ran = o.ran;
            e.warmup = o.warmup;
            e.dry.inherit(&mut o.dry);
        }
    }
}

/// RT. A snapshot swap added, removed or reordered MIDI effects of a chain: notes they
/// generated would never get their note-offs, so queue `AllNotesOff` to every entry (once,
/// delivered at offset 0 of the next sub-block). Switching one on or off is a transition
/// ([`ChainRt::switch_notes`]).
pub(crate) fn release_on_midi_fx_change(new: &mut [ChainRt], old: &[ChainRt]) {
    if midi_fx_changed(new, old) {
        for c in new.iter_mut() {
            c.pending.push(ProcessEvent {
                offset: 0,
                kind: EventKind::AllNotesOff,
            });
        }
    }
}

#[derive(Debug)]
pub(crate) struct SendRt {
    pub id: SendId,
    pub target: usize,
    pub pre_fader: bool,
    pub level: Smoother,
    /// PDC: aligns this send with the other inputs of `target`.
    pub delay: DelayLine,
    /// This sub-block's send signal (delayed, level applied), gathered by `target`'s job
    /// (`crate::parallel`).
    pub buf: Stereo,
}

/// One input of a track's bus, gathered at the start of the track's job
/// (`crate::parallel`: fixed order = the sequential engine's summation order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BusInput {
    /// Track `.0`'s output (post PDC output delay, `TrackRt::a`).
    Output(usize),
    /// Send `.1` of track `.0` (`SendRt::buf`).
    Send(usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ActiveNote {
    pub note_id: u32,
    pub key: u8,
    /// Timeline beat of the note-off.
    pub end: f64,
    /// v0.3: the clip note this voice plays (note expressions).
    pub source: NoteSource,
}

/// The clip note a sounding note plays (v0.3): its note expressions
/// (`crate::expression`) are looked up by clip and note index, timed from `start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NoteSource {
    pub clip: ClipId,
    /// Index in the clip's `ClipContentDesc::Midi { notes }` (sorted by start).
    pub note: u32,
    /// Timeline beat of the note start (before chasing: may be before the note-on).
    pub start: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MeterAccum {
    pub peak: [f32; 2],
    pub sum_sq: [f32; 2],
    pub count: u32,
    pub clipped: bool,
}

impl MeterAccum {
    pub(crate) fn add(&mut self, l: &[f32], r: &[f32]) {
        for (ch, buf) in [l, r].into_iter().enumerate() {
            // Eight partial sums and maxima (vectorizable; meters only reach the UI, so the
            // summation order is free).
            let mut peaks = [self.peak[ch]; 8];
            let mut sums = [0.0f32; 8];
            let (chunks, rest) = buf.as_chunks::<8>();
            for c in chunks {
                for k in 0..8 {
                    peaks[k] = peaks[k].max(c[k].abs());
                    sums[k] += c[k] * c[k];
                }
            }
            for (k, &s) in rest.iter().enumerate() {
                peaks[k] = peaks[k].max(s.abs());
                sums[k] += s * s;
            }
            let peak = peaks.iter().copied().fold(self.peak[ch], f32::max);
            let sum: f32 = sums.iter().sum();
            self.peak[ch] = peak;
            self.sum_sq[ch] += sum;
            if peak >= 1.0 {
                self.clipped = true;
            }
        }
        self.count += l.len() as u32;
    }

    pub(crate) fn rms(&self) -> [f32; 2] {
        if self.count == 0 {
            return [0.0; 2];
        }
        let n = self.count as f32;
        [(self.sum_sq[0] / n).sqrt(), (self.sum_sq[1] / n).sqrt()]
    }
}

#[derive(Debug)]
pub(crate) struct TrackRt {
    pub id: TrackId,
    /// Enclosing group (mute propagation).
    pub parent: Option<usize>,
    /// Master bus: output goes to the hardware.
    pub to_hardware: bool,
    pub chain: Vec<ChainRt>,
    pub sends: Vec<SendRt>,
    /// PDC: aligns this track's output with the other inputs of `output`.
    pub output_delay: DelayLine,
    pub volume: Smoother,
    pub pan: Smoother,
    /// 1 = audible, 0 = muted (mute, group mute, or not soloed while something is).
    pub gate: Smoother,
    pub mute: bool,
    /// False when another track is soloed and nothing keeps this one audible.
    pub solo_ok: bool,
    pub audio_input: Option<(u16, u16)>,
    pub monitor: bool,
    /// Live notes (and sustain pedals) this track received and still holds
    /// (`crate::recording`).
    pub live: crate::recording::LiveNotes,
    /// Ping-pong buffers for the device chain (current signal in `a`).
    pub a: Stereo,
    pub b: Stereo,
    pub out_events: EventBuffer,
    pub notes: Vec<ActiveNote>,
    /// Last node-param value sent per automation lane (NaN = send again).
    pub auto_last: Vec<f64>,
    /// Same for the targets only clip envelopes automate (`env_targets`).
    pub env_last: Vec<f64>,
    pub env_targets: Vec<crate::graph::ResolvedTarget>,
    /// Latency at each node's input (sorted by node, any depth): its automation is read
    /// that late (`automation_rt::views`, issue #270).
    pub auto_delays: Vec<(NodeKey, u32)>,
    /// Latency where the fader, sends and VCA gain apply (after the chain).
    pub mixer_delay: u32,
    /// Running maximum of the clips' ends (`automation_rt::clip_ends`): the first clip
    /// that can still play is a binary search away.
    pub clip_ends: Vec<f64>,
    /// The track's job did real work in the last sub-block (processed a node, monitored
    /// input, ran racks or a frozen render). While stopped, the engine dispatches jobs to
    /// the workers only when some track was awake.
    pub awake: bool,
    /// A live param change reached one of this track's nodes: re-send automated node
    /// params so enabled lanes keep driving their targets.
    pub auto_dirty: bool,
    pub meter: MeterAccum,
    /// Compiled latency of this track's output (for tests/diagnostics).
    pub out_latency: u32,
    /// Drum rack pad chains (`crate::drum_rack`).
    pub racks: crate::drum_rack::RacksRt,
    /// Bus inputs, summed in this order at the start of the track's job.
    pub inputs: Vec<BusInput>,
    /// Sidechain state of this track's sidechained entries, any depth (`crate::sidechain`,
    /// compiled for this track only so jobs never share it).
    pub sidechain: crate::sidechain::Taps,
    /// This track's points tapped by device sidechains (any track's, this one included).
    pub sc_points: crate::sidechain::Points,
    /// Post-fader output before the PDC output delay, kept when an envelope follower
    /// listens to this track (copied into the consumers' modulation by their jobs).
    pub tap: Option<Stereo>,
    /// Note ids of clip notes (per track, so they never depend on processing order).
    pub next_note_id: u32,
    /// Resampling scratch for audio clips (audio tracks only; empty otherwise).
    pub src_scratch: Vec<f32>,
    /// Scratch of the Complex warp path (tracks with Complex-warped clips, `crate::warp`).
    pub warp_scratch: Option<crate::warp::WarpScratch>,
    /// Wall time of the track's job (ns, smoothed: up fast, down slowly), native hosts
    /// only (`Engine::process` measures, `crate::parallel` priorities use it). 0 = unknown.
    pub job_ns: f32,
    /// Set by the track's job, collected on the audio thread after the last level.
    pub overflow: bool,
    pub underruns: u32,
    // --- v0.2 hooks (each owned by its module's node) ---
    /// Rack chains (`crate::rack_chains`).
    pub chain_racks: crate::rack_chains::ChainRacksRt,
    /// Modulators and mappings (`crate::modulation`).
    pub modulation: crate::modulation::ModulationRt,
    /// Input from another track (`crate::bus_tap`).
    pub input_tap: crate::bus_tap::InputTapRt,
    /// Tap buffers other tracks read from this one (`crate::bus_tap`).
    pub taps: crate::bus_tap::TapBuffers,
    /// VCA gain/mute (`crate::vca`).
    pub vca: crate::vca::TrackVcaRt,
    // --- v0.3 hooks ---
    /// MIDI expression playback state (`crate::expression`).
    pub expression: crate::expression::ExpressionRt,
}

impl TrackRt {
    /// RT. Mixer smoothers jump to their targets (a resume after a suspension).
    pub(crate) fn settle(&mut self) {
        self.volume.settle();
        self.pan.settle();
        self.gate.settle();
        for send in &mut self.sends {
            send.level.settle();
        }
    }

    /// RT. Carry running state over from the previous snapshot's version of this track.
    pub(crate) fn inherit(&mut self, old: &mut TrackRt) {
        let (vol, pan, gate) = (
            self.volume.current(),
            self.pan.current(),
            self.gate.current(),
        );
        // New targets come from the new desc; start from where the old one was.
        self.volume = old.volume;
        self.volume.set_target(vol);
        self.pan = old.pan;
        self.pan.set_target(pan);
        self.gate = old.gate;
        self.gate.set_target(gate);
        self.output_delay.inherit(&mut old.output_delay);
        for send in &mut self.sends {
            if let Some(o) = old.sends.iter_mut().find(|s| s.id == send.id) {
                let level = send.level.current();
                send.level = o.level;
                send.level.set_target(level);
                if o.target == send.target {
                    send.delay.inherit(&mut o.delay);
                }
            }
        }
        // v0.2 (`midi-fx`): a MIDI effect was added or removed: release everything once.
        // Devices switched on or off release their notes when the chain reaches them.
        release_on_midi_fx_change(&mut self.chain, &old.chain);
        inherit_switches(&mut self.chain, &mut old.chain);
        std::mem::swap(&mut self.notes, &mut old.notes);
        std::mem::swap(&mut self.live, &mut old.live);
        self.next_note_id = old.next_note_id;
        self.meter = old.meter;
        self.job_ns = old.job_ns;
        self.racks.inherit(&mut old.racks);
        self.chain_racks.inherit(&mut old.chain_racks);
        self.modulation.inherit(&mut old.modulation);
        self.input_tap.inherit(&mut old.input_tap);
        self.vca.inherit(&mut old.vca);
        self.expression.inherit(&mut old.expression);
    }
}

/// Whether the MIDI effects (entries without audio, `channels == (0, 0)`) of a chain differ
/// between two snapshots. RT: compares in place.
fn midi_fx_changed(new: &[ChainRt], old: &[ChainRt]) -> bool {
    let mut a = new.iter().filter(|e| e.channels == (0, 0));
    let mut b = old.iter().filter(|e| e.channels == (0, 0));
    loop {
        match (a.next(), b.next()) {
            (None, None) => return false,
            (Some(x), Some(y)) if x.key == y.key => {}
            _ => return true,
        }
    }
}

/// RT. Apply fader (volume) and pan to `a` in place. The mute/solo gate is separate
/// ([`apply_gate`]): sidechain taps sit between the two (see `engine.rs`).
pub(crate) fn apply_fader(
    a: &mut Stereo,
    volume: &mut Smoother,
    pan: &mut Smoother,
    frames: usize,
) {
    apply_fader_range(a, volume, pan, 0, frames);
}

/// RT. [`apply_fader`] on samples `start..end` only (sample-accurate automation ramps the
/// smoothers between grid points, `crate::automation_rt::fader`).
pub(crate) fn apply_fader_range(
    a: &mut Stereo,
    volume: &mut Smoother,
    pan: &mut Smoother,
    start: usize,
    end: usize,
) {
    let [l, r] = a;
    if !volume.is_smoothing() && !pan.is_smoothing() {
        // Settled (the usual case): the same products as below, with constants the
        // compiler can vectorize (`tick` returns `current` and changes nothing).
        let g = volume.current();
        let p = pan.current().clamp(-1.0, 1.0);
        let gl = if p > 0.0 { 1.0 - p } else { 1.0 };
        let gr = if p < 0.0 { 1.0 + p } else { 1.0 };
        let (kl, kr) = (g * gl, g * gr);
        for s in &mut l[start..end] {
            *s *= kl;
        }
        for s in &mut r[start..end] {
            *s *= kr;
        }
        return;
    }
    for (sl, sr) in l[start..end].iter_mut().zip(r[start..end].iter_mut()) {
        let g = volume.tick();
        let p = pan.tick().clamp(-1.0, 1.0);
        // Balance law: centre is unity on both sides.
        let gl = if p > 0.0 { 1.0 - p } else { 1.0 };
        let gr = if p < 0.0 { 1.0 + p } else { 1.0 };
        *sl *= g * gl;
        *sr *= g * gr;
    }
}

/// RT. Apply the mute/solo gate to `a` in place (a no-op while it rests at 1).
pub(crate) fn apply_gate(a: &mut Stereo, gate: &mut Smoother, frames: usize) {
    if !gate.is_smoothing() && gate.current() == 1.0 {
        return;
    }
    let [l, r] = a;
    for (sl, sr) in l[..frames].iter_mut().zip(r[..frames].iter_mut()) {
        let g = gate.tick();
        *sl *= g;
        *sr *= g;
    }
}

/// RT. `buf *= smoothed gain`. A send's buffer is scaled in its track's job and added to
/// the destination bus later (`d + s * g` rounds exactly like `t = s * g; d + t`: Rust
/// never fuses into FMA), so this matches mixing `src * gain` straight into the bus.
/// `advance = false` leaves `gain` where it was (the left channel; the right advances).
pub(crate) fn scale(buf: &mut [f32], gain: &mut Smoother, advance: bool) {
    if !gain.is_smoothing() {
        // Settled: a constant (vectorizable; `tick` would return it and change nothing).
        let g = gain.current();
        for s in buf.iter_mut() {
            *s *= g;
        }
        return;
    }
    if advance {
        for s in buf.iter_mut() {
            *s *= gain.tick();
        }
    } else {
        let mut g = *gain;
        for s in buf.iter_mut() {
            *s *= g.tick();
        }
    }
}
