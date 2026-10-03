//! Per-track runtime state of a compiled snapshot: device-chain buffers, fader, pan,
//! mute/solo gate, sends, PDC delay lines, active notes and meters.
//!
//! Everything here is allocated by the compiler (off the audio thread). When a snapshot is
//! swapped in, the audio thread moves the running state (smoother positions, delay-line
//! contents, sounding notes, meter accumulators) over from the previous snapshot's track
//! with the same id ([`TrackRt::inherit`]); that only swaps pointers, never allocates.

use ether_protocol::model::{SendId, TrackId};

use crate::delay::DelayLine;
use crate::event::EventBuffer;
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
    pub channels: (u16, u16),
    /// Events for the current sub-block (sorted before `process`).
    pub events: EventBuffer,
    /// Live param changes waiting for the next sub-block.
    pub pending: EventBuffer,
    /// Sidechain source track index (`ChainEntry::sidechain`, resolved), see
    /// `crate::sidechain`.
    pub sidechain: Option<usize>,
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
            let mut peak = self.peak[ch];
            let mut sum = 0.0f32;
            for &s in buf {
                let a = s.abs();
                peak = peak.max(a);
                sum += s * s;
            }
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
    /// Keeps a constant latency when devices with latency are bypassed.
    pub bypass_delay: DelayLine,
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
    /// Ping-pong buffers for the device chain (current signal in `a`).
    pub a: Stereo,
    pub b: Stereo,
    pub out_events: EventBuffer,
    pub notes: Vec<ActiveNote>,
    /// Last node-param value sent per automation lane (NaN = send again).
    pub auto_last: Vec<f64>,
    /// Same for clip envelopes: `env_base[clip] + envelope index`.
    pub env_last: Vec<f64>,
    pub env_base: Vec<usize>,
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
    /// Sidechain state of this track's sidechained entries (`crate::sidechain`, compiled
    /// for this track only so jobs never share it) and the sources they listen to.
    pub sidechain: crate::sidechain::Taps,
    pub sc_sources: Vec<usize>,
    /// Post-fader output before the PDC output delay, kept when a sidechain listens to
    /// this track (copied into the consumers' taps by their jobs).
    pub tap: Option<Stereo>,
    /// Note ids of clip notes (per track, so they never depend on processing order).
    pub next_note_id: u32,
    /// Resampling scratch for audio clips (audio tracks only; empty otherwise).
    pub src_scratch: Vec<f32>,
    /// Runs on the audio thread (Complex-warped clips: engine-wide stretcher state).
    pub pinned: bool,
    /// Set by the track's job, collected on the audio thread after the last level.
    pub overflow: bool,
    pub underruns: u32,
    // --- v0.2 hooks (contracts-3; each owned by its module's node) ---
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
}

impl TrackRt {
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
        self.bypass_delay.inherit(&mut old.bypass_delay);
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
        // v0.2 (`midi-fx`): a MIDI effect was added, removed or (un)bypassed. Notes it
        // generated would never get their note-offs: release everything once (queued like a
        // live event, delivered at offset 0 of the next block).
        if midi_fx_changed(&self.chain, &old.chain) {
            for c in &mut self.chain {
                c.pending.push(crate::event::ProcessEvent {
                    offset: 0,
                    kind: crate::event::EventKind::AllNotesOff,
                });
            }
        }
        std::mem::swap(&mut self.notes, &mut old.notes);
        self.next_note_id = old.next_note_id;
        self.meter = old.meter;
        self.racks.inherit(&mut old.racks);
        self.chain_racks.inherit(&mut old.chain_racks);
        self.modulation.inherit(&mut old.modulation);
        self.input_tap.inherit(&mut old.input_tap);
        self.vca.inherit(&mut old.vca);
    }
}

/// Whether the MIDI effects (entries without audio, `channels == (0, 0)`) of a chain or
/// their bypass state differ between two snapshots. RT: compares in place.
fn midi_fx_changed(new: &[ChainRt], old: &[ChainRt]) -> bool {
    let mut a = new.iter().filter(|e| e.channels == (0, 0));
    let mut b = old.iter().filter(|e| e.channels == (0, 0));
    loop {
        match (a.next(), b.next()) {
            (None, None) => return false,
            (Some(x), Some(y)) if x.key == y.key && x.enabled == y.enabled => {}
            _ => return true,
        }
    }
}

/// RT. Apply fader (volume × gate) and pan to `a` in place.
pub(crate) fn apply_fader(
    a: &mut Stereo,
    volume: &mut Smoother,
    pan: &mut Smoother,
    gate: &mut Smoother,
    frames: usize,
) {
    apply_fader_range(a, volume, pan, gate, 0, frames);
}

/// RT. [`apply_fader`] on samples `start..end` only (sample-accurate automation ramps the
/// smoothers between grid points, `crate::automation_rt::fader`).
pub(crate) fn apply_fader_range(
    a: &mut Stereo,
    volume: &mut Smoother,
    pan: &mut Smoother,
    gate: &mut Smoother,
    start: usize,
    end: usize,
) {
    let [l, r] = a;
    for (sl, sr) in l[start..end].iter_mut().zip(r[start..end].iter_mut()) {
        let g = volume.tick() * gate.tick();
        let p = pan.tick().clamp(-1.0, 1.0);
        // Balance law: centre is unity on both sides.
        let gl = if p > 0.0 { 1.0 - p } else { 1.0 };
        let gr = if p < 0.0 { 1.0 + p } else { 1.0 };
        *sl *= g * gl;
        *sr *= g * gr;
    }
}

/// RT. `buf *= smoothed gain`. A send's buffer is scaled in its track's job and added to
/// the destination bus later (`d + s * g` rounds exactly like `t = s * g; d + t`: Rust
/// never fuses into FMA), so this matches mixing `src * gain` straight into the bus.
/// `advance = false` leaves `gain` where it was (the left channel; the right advances).
pub(crate) fn scale(buf: &mut [f32], gain: &mut Smoother, advance: bool) {
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
