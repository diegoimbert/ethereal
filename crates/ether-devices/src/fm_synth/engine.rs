//! The FM Synth node: voice allocation (poly with stealing, mono, legato), glide, the four operators in
//! the selected algorithm, the voice filter, the LFO and the output stage.
//!
//! **Operators.** Phase modulation: an operator's phase is offset by the sum of its
//! modulators' outputs (`MOD_DEPTH` cycles per unit of output, so a modulator at 0 dB
//! reaches an index of 2π) plus its own feedback (the mean of its last two outputs, which
//! keeps strong feedback from oscillating at Nyquist). Sine reads a table; triangle, saw
//! and square are PolyBLEP/BLAMP shapes (band-limited when unmodulated); noise is white.
//! An operator's output is `wave · envelope · level · velocity gain`; carriers are summed
//! and normalised by `1/√carriers`.
//!
//! **Rendering** runs in control chunks of at most [`CTRL`] samples on the engine's
//! absolute sample clock, split at event offsets (a render is the same for every block
//! size that is a multiple of [`CTRL`]): pitch, filter cutoff and the LFO are evaluated
//! once per chunk and per voice; operators, envelopes and the filter run per sample at the
//! internal rate (2x with `Antialias`, decimated by the saturator's half-band FIR), levels
//! and cutoff interpolated across the chunk. Everything is pre-allocated: `process` never
//! allocates, locks or blocks.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::algorithms::{ALGORITHMS, Algorithm, OPS};
use super::fm_synth::{self as id, op};
use crate::fx_color::oversample::{Stage1, stage1};
use crate::poly_synth::dsp::{
    self, Env, EnvRates, FilterMode, FilterState, LfoShape, Rng, Shape, SineTable,
};
use crate::util;

/// Largest `Voices` value.
pub const MAX_POLYPHONY: usize = 32;
/// Physical voice slots: the polyphony plus room for stolen voices fading out.
const SLOTS: usize = MAX_POLYPHONY + 8;
/// Control-rate chunk (base-rate samples).
const CTRL: usize = 16;
/// Time constant of the pitch change of a sounding mono / legato voice that doesn't glide
/// (ms): reads as a jump, but a sine doesn't kink (a -50 dB click on a 220 → 330 Hz
/// jump).
const DECLICK_MS: f32 = 0.5;
/// Oversampling factor with `Antialias` on.
const OS: usize = 2;
/// Ramp of pitch/filter params (samples): two automation grid intervals.
const RAMP_FAST: u32 = 2 * ether_core::automation_rt::PARAM_GRID as u32;
/// Ramp of gain params (ms): long enough that a knob jump never clicks.
const RAMP_GAIN_MS: f32 = 5.0;
/// Fade-out of a stolen or choked voice (ms).
const STEAL_MS: f32 = 2.0;
/// Held keys remembered in mono/legato modes (last-note priority).
const STACK: usize = 16;
/// Level params at their minimum are off.
const LEVEL_OFF_DB: f64 = -70.0;
/// Phase offset (cycles) per unit of modulator output.
pub const MOD_DEPTH: f32 = 1.0;
/// Phase offset (cycles) per unit of an operator's own output at `Feedback` 100 %.
pub const FB_DEPTH: f32 = 0.25;
/// Filter envelope range at ±100 % (octaves).
const ENV_CUTOFF_OCTAVES: f32 = 8.0;
/// LFO pitch depth at 100 % (semitones; the amount is squared for fine vibrato).
const LFO_PITCH_SEMIS: f32 = 12.0;
/// LFO cutoff range at ±100 % (octaves).
const LFO_CUTOFF_OCTAVES: f32 = 4.0;
/// Highest operator increment (cycles per internal sample).
const MAX_DT: f32 = 0.45;
/// Filter input headroom: the saturating SVF input stage sees the voice at this scale.
const FILTER_PRE: f32 = 0.35;
/// `Tone` at 0 %: output low-pass corner (Hz); 100 % = open.
const TONE_MIN_HZ: f32 = 1000.0;

// --- smoothed params -------------------------------------------------------------------

/// Linear ramp read at chunk boundaries.
#[derive(Clone, Copy, Debug)]
struct Ramp {
    cur: f32,
    target: f32,
    step: f32,
    left: u32,
    /// Value at the start of the current chunk.
    prev: f32,
}

impl Ramp {
    const fn new(v: f32) -> Self {
        Self {
            cur: v,
            target: v,
            step: 0.0,
            left: 0,
            prev: v,
        }
    }

    fn set(&mut self, v: f32, samples: u32) {
        if samples == 0 {
            *self = Self::new(v);
            return;
        }
        self.target = v;
        self.left = samples;
        self.step = (v - self.cur) / samples as f32;
    }

    #[inline]
    fn advance(&mut self, n: u32) {
        self.prev = self.cur;
        if self.left > 0 {
            if n >= self.left {
                self.cur = self.target;
                self.left = 0;
            } else {
                self.cur += self.step * n as f32;
                self.left -= n;
            }
        }
    }
}

/// The operator field of `pid` (`None` for a global param).
fn op_field(pid: ParamId) -> Option<(usize, u32)> {
    (pid.0 < op::STRIDE * OPS as u32).then_some(((pid.0 / op::STRIDE) as usize, pid.0 % op::STRIDE))
}

/// Smoothed value domain of a param: amplitude for levels/volume, log2 Hz for the cutoff,
/// 0..1 for percents, a fraction for `Fine`, semitones for the global fine, plain otherwise.
fn domain(pid: ParamId, v: f64) -> f32 {
    let amp = |v: f64| {
        if v <= LEVEL_OFF_DB {
            0.0
        } else {
            util::db_to_amp(v as f32)
        }
    };
    if let Some((_, field)) = op_field(pid) {
        return match field {
            op::LEVEL => amp(v),
            op::FINE => v as f32 * 0.001,
            op::VELOCITY | op::FEEDBACK => v as f32 * 0.01,
            _ => v as f32,
        };
    }
    match pid {
        id::VOLUME => amp(v),
        id::CUTOFF => (v as f32).max(1.0).log2(),
        id::FINE => v as f32 * 0.01,
        id::LFO_PITCH
        | id::LFO_FILTER
        | id::LFO_VOLUME
        | id::RESONANCE
        | id::FILTER_ENV_AMOUNT
        | id::KEY_TRACKING
        | id::TONE => v as f32 * 0.01,
        _ => v as f32,
    }
}

fn is_gain(pid: ParamId) -> bool {
    pid == id::VOLUME || matches!(op_field(pid), Some((_, op::LEVEL)))
}

/// Does `pid` change an envelope's rates?
fn is_envelope(pid: ParamId) -> bool {
    match op_field(pid) {
        Some((_, f)) => (op::ATTACK..=op::RELEASE).contains(&f),
        None => (id::FILTER_ATTACK.0..=id::FILTER_RELEASE.0).contains(&pid.0),
    }
}

// --- voices ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Voice {
    note_id: u32,
    channel: u8,
    key: u8,
    velocity: f32,
    age: u64,
    /// Sounding (some carrier's envelope still runs).
    alive: bool,
    /// Key held.
    gate: bool,
    /// Released while the sustain pedal was down.
    sustained: bool,
    /// Current (gliding) and target pitch, in MIDI keys.
    pitch: f32,
    target: f32,
    env: [Env; OPS],
    fenv: Env,
    phase: [f32; OPS],
    /// The last two outputs of each operator (feedback).
    last: [[f32; 2]; OPS],
    /// 1 unless fading out (stolen/choked): then decreasing by `fade_step` to 0.
    fade: f32,
    fade_step: f32,
    rng: Rng,
    filt: [FilterState; 2],
    /// Chunk-start filter coefficient (`fresh` = none yet).
    g: f32,
    fresh: bool,
    /// Chunk-start phase increments of the operators (ramped across each chunk).
    dt: [f32; OPS],
    /// The pitch moves to `target` with [`DECLICK_MS`] (a mono note change without glide).
    declick: bool,
}

impl Voice {
    const IDLE: Self = Self {
        note_id: 0,
        channel: 0,
        key: 0,
        velocity: 0.0,
        age: 0,
        alive: false,
        gate: false,
        sustained: false,
        pitch: 60.0,
        target: 60.0,
        env: [Env::IDLE; OPS],
        fenv: Env::IDLE,
        phase: [0.0; OPS],
        last: [[0.0; 2]; OPS],
        fade: 1.0,
        fade_step: 0.0,
        rng: Rng::new(1),
        filt: [FilterState::ZERO; 2],
        g: 0.0,
        fresh: true,
        dt: [0.0; OPS],
        declick: false,
    };

    fn active(&self) -> bool {
        self.alive
    }

    fn fading(&self) -> bool {
        self.fade_step > 0.0
    }

    fn releasing(&self) -> bool {
        !self.gate && !self.sustained
    }

    fn release(&mut self) {
        self.gate = false;
        for e in &mut self.env {
            e.release();
        }
        self.fenv.release();
    }

    fn trigger(&mut self) {
        self.gate = true;
        self.sustained = false;
        for e in &mut self.env {
            e.trigger();
        }
        self.fenv.trigger();
    }
}

/// An operator's settings for one chunk.
#[derive(Clone, Copy, Debug)]
struct OpChunk {
    on: bool,
    /// Rendered at all (on, and its level is not off for the whole chunk).
    audible: bool,
    wave: usize,
    ratio: f32,
    fixed: Option<f32>,
    /// Level (amplitude) at the chunk start and end.
    level: (f32, f32),
    velocity: f32,
    feedback: f32,
    rates: EnvRates,
}

/// Everything shared by the voices for one chunk.
#[derive(Clone, Copy)]
struct Chunk {
    /// Internal-rate samples of this chunk and the internal rate.
    ni: usize,
    isr: f32,
    alg: Algorithm,
    norm: f32,
    ops: [OpChunk; OPS],
    /// Pitch offset (semitones): transpose, fine, bend, LFO.
    pitch: f32,
    glide_coef: f32,
    /// Glide coefficient of a declicked pitch change over this chunk ([`DECLICK_MS`]).
    declick_coef: f32,
    filter: bool,
    mode: FilterMode,
    two_stage: bool,
    cutoff: f32,
    k: f32,
    k_flat: f32,
    key_tracking: f32,
    fenv_amount: f32,
    lfo_cutoff: f32,
    fenv_rates: EnvRates,
}

/// The FM Synth.
pub struct FmSynth {
    values: [f64; id::COUNT],
    ranges: [(f64, f64, f64); id::COUNT],
    ramps: [Ramp; id::COUNT],
    sr: f32,
    /// Oversampling factor in use (1 or [`OS`]).
    os: usize,
    gain_ramp: u32,
    sine: Box<SineTable>,
    voices: Box<[Voice; SLOTS]>,
    op_rates: [EnvRates; OPS],
    fenv_rates: EnvRates,
    lfo_phase: f32,
    lfo_held: f32,
    /// Output gain (volume × LFO tremolo) at the end of the last chunk.
    out_gain: f32,
    bend_target: f32,
    bend: f32,
    next_age: u64,
    last_pitch: Option<f32>,
    /// Mono/legato: held keys (last = most recent) and the sounding slot.
    stack: [u8; STACK],
    stack_len: usize,
    mono: Option<usize>,
    pedal: bool,
    rng: Rng,
    /// Internal-rate mix of the current chunk.
    buf: [f32; CTRL * OS],
    down: Box<Stage1>,
    tone_z: f32,
}

impl std::fmt::Debug for FmSynth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FmSynth").finish_non_exhaustive()
    }
}

impl Default for FmSynth {
    fn default() -> Self {
        Self::new()
    }
}

impl FmSynth {
    /// Non-RT.
    pub fn new() -> Self {
        let desc = super::descriptor(BuiltinDeviceType::FmSynth);
        let mut values = [0.0; id::COUNT];
        let mut ranges = [(0.0, 0.0, 0.0); id::COUNT];
        for p in &desc.params {
            let i = p.id.0 as usize;
            values[i] = p.default;
            ranges[i] = (p.min, p.max, p.default);
        }
        let rates = EnvRates::new(2.0, 600.0, 1.0, 300.0, 48_000.0);
        let mut s = Self {
            values,
            ranges,
            ramps: [Ramp::new(0.0); id::COUNT],
            sr: 48_000.0,
            os: OS,
            gain_ramp: 240,
            sine: Box::new(SineTable::new()),
            voices: Box::new([Voice::IDLE; SLOTS]),
            op_rates: [rates; OPS],
            fenv_rates: rates,
            lfo_phase: 0.0,
            lfo_held: 0.0,
            out_gain: 0.0,
            bend_target: 0.0,
            bend: 0.0,
            next_age: 0,
            last_pitch: None,
            stack: [0; STACK],
            stack_len: 0,
            mono: None,
            pedal: false,
            rng: Rng::new(0xF4_5EED),
            buf: [0.0; CTRL * OS],
            down: Box::new(stage1()),
            tone_z: 0.0,
        };
        s.sync_all();
        s
    }

    fn v(&self, pid: ParamId) -> f64 {
        self.values[pid.0 as usize]
    }

    fn on(&self, pid: ParamId) -> bool {
        self.v(pid) >= 0.5
    }

    fn idx(&self, pid: ParamId, count: usize) -> usize {
        util::index(self.v(pid), count)
    }

    fn ramp(&self, pid: ParamId) -> &Ramp {
        &self.ramps[pid.0 as usize]
    }

    /// Snap every derived value to the params (no smoothing).
    fn sync_all(&mut self) {
        for i in 0..id::COUNT {
            self.ramps[i] = Ramp::new(domain(ParamId(i as u32), self.values[i]));
        }
        self.out_gain = self.ramp(id::VOLUME).cur;
        self.os = if self.on(id::ANTIALIAS) { OS } else { 1 };
        self.update_rates();
    }

    fn update_rates(&mut self) {
        let isr = self.sr * self.os as f32;
        let env = |s: &Self, a: ParamId| {
            EnvRates::new(
                s.v(a) as f32,
                s.v(ParamId(a.0 + 1)) as f32,
                s.v(ParamId(a.0 + 2)) as f32 * 0.01,
                s.v(ParamId(a.0 + 3)) as f32,
                isr,
            )
        };
        for o in 0..OPS {
            self.op_rates[o] = env(self, id::op_param(o, op::ATTACK));
        }
        self.fenv_rates = env(self, id::FILTER_ATTACK);
    }

    /// RT-safe. Store a clamped plain value (NaN → default); `smooth` ramps it.
    fn apply_param(&mut self, pid: ParamId, value: f64, smooth: bool) {
        let i = pid.0 as usize;
        if i >= id::COUNT {
            return;
        }
        let (min, max, default) = self.ranges[i];
        let v = if value.is_nan() {
            default
        } else {
            value.clamp(min.min(max), max.max(min))
        };
        self.values[i] = v;
        let samples = match (smooth, is_gain(pid)) {
            (false, _) => 0,
            (true, true) => self.gain_ramp,
            (true, false) => RAMP_FAST,
        };
        self.ramps[i].set(domain(pid, v), samples);
        if pid == id::ANTIALIAS {
            let os = if v >= 0.5 { OS } else { 1 };
            if os != self.os {
                self.os = os;
                self.down.reset();
                // Fade-steps are per internal sample.
                for voice in self.voices.iter_mut().filter(|v| v.fading()) {
                    voice.fade_step = 1.0 / (STEAL_MS * 0.001 * self.sr * os as f32).max(1.0);
                }
                self.update_rates();
            }
        } else if is_envelope(pid) {
            self.update_rates();
        }
    }

    // --- notes -------------------------------------------------------------------------

    /// A free slot for a new voice (steals when the polyphony is exhausted).
    fn allocate(&mut self, limit: usize) -> usize {
        let sounding = self
            .voices
            .iter()
            .filter(|v| v.active() && !v.fading())
            .count();
        if sounding >= limit {
            // Steal: the oldest releasing voice, else the oldest.
            let victim = self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.active() && !v.fading())
                .min_by_key(|(_, v)| (!v.releasing(), v.age))
                .map(|(i, _)| i);
            if let Some(i) = victim {
                self.fade_out(i);
            }
        }
        if let Some(i) = self.voices.iter().position(|v| !v.active()) {
            return i;
        }
        // Every slot busy (fading voices): reuse the quietest fading one.
        self.voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.fading())
            .min_by(|a, b| a.1.fade.total_cmp(&b.1.fade))
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn fade_out(&mut self, i: usize) {
        let step = 1.0 / (STEAL_MS * 0.001 * self.sr * self.os as f32).max(1.0);
        let v = &mut self.voices[i];
        if v.active() && !v.fading() {
            v.fade_step = step;
        }
        if self.mono == Some(i) {
            self.mono = None;
        }
    }

    fn glide_on(&self) -> bool {
        self.v(id::GLIDE) > 0.0
    }

    /// 0 = poly, 1 = mono, 2 = legato.
    fn mode(&self) -> usize {
        self.idx(id::VOICE_MODE, super::VOICE_MODES.len())
    }

    fn note_on(&mut self, note_id: u32, channel: u8, key: u8, velocity: f32) {
        let mode = self.mode();
        if mode == 0 {
            let limit = (self.v(id::VOICES).round().max(1.0) as usize).min(MAX_POLYPHONY);
            let slot = self.allocate(limit);
            self.start_voice(slot, note_id, channel, key, velocity);
        } else {
            let was_held = self.stack_len > 0;
            self.push_key(key);
            match self.mono.filter(|&i| self.voices[i].active()) {
                Some(i) => {
                    let legato = mode == 2 && was_held && self.voices[i].gate;
                    let glide = self.glide_on() && (mode == 1 || legato);
                    let v = &mut self.voices[i];
                    v.note_id = note_id;
                    v.channel = channel;
                    v.key = key;
                    v.target = f32::from(key);
                    v.declick = !glide;
                    if legato {
                        v.gate = true;
                        v.sustained = false;
                    } else {
                        v.velocity = velocity.clamp(0.0, 1.0);
                        v.trigger();
                    }
                }
                None => {
                    let slot = self.allocate(1);
                    self.start_voice(slot, note_id, channel, key, velocity);
                    self.mono = Some(slot);
                }
            }
        }
        self.last_pitch = Some(f32::from(key));
    }

    fn push_key(&mut self, key: u8) {
        self.remove_key(key);
        if self.stack_len == STACK {
            self.stack.copy_within(1.., 0);
            self.stack_len -= 1;
        }
        self.stack[self.stack_len] = key;
        self.stack_len += 1;
    }

    fn remove_key(&mut self, key: u8) {
        if let Some(p) = self.stack[..self.stack_len].iter().position(|&k| k == key) {
            self.stack.copy_within(p + 1..self.stack_len, p);
            self.stack_len -= 1;
        }
    }

    /// Start a fresh voice in `slot` (glides from the last note, except in legato mode
    /// where only overlapping notes glide).
    fn start_voice(&mut self, slot: usize, note_id: u32, channel: u8, key: u8, velocity: f32) {
        self.next_age += 1;
        let start = match self.last_pitch {
            Some(p) if self.glide_on() && self.mode() != 2 => p,
            _ => f32::from(key),
        };
        let seed = self.rng.next_u32();
        let mut v = Voice {
            note_id,
            channel,
            key,
            velocity: velocity.clamp(0.0, 1.0),
            age: self.next_age,
            alive: true,
            pitch: start,
            target: f32::from(key),
            rng: Rng::new(seed),
            ..Voice::IDLE
        };
        v.trigger();
        self.voices[slot] = v;
    }

    fn note_off(&mut self, note_id: u32, channel: u8, key: u8) {
        if self.mode() != 0 {
            self.remove_key(key);
            // The mono voice sounding this key (still held).
            let sounding = self.mono.filter(|&i| {
                let v = &self.voices[i];
                v.active() && v.gate && v.key == key
            });
            if let Some(i) = sounding {
                if self.stack_len > 0 {
                    // Back to the most recent held key (mono retriggers, legato slides).
                    let back = self.stack[self.stack_len - 1];
                    let glide = self.glide_on();
                    let retrigger = self.mode() == 1;
                    let v = &mut self.voices[i];
                    v.key = back;
                    v.target = f32::from(back);
                    v.declick = !glide;
                    if retrigger {
                        v.trigger();
                    }
                    self.last_pitch = Some(f32::from(back));
                } else if self.pedal {
                    self.voices[i].sustained = true;
                    self.voices[i].gate = false;
                } else {
                    self.voices[i].release();
                }
                return;
            }
        }
        // Poly (or a voice left from before a mode change).
        let by_id = self
            .voices
            .iter()
            .any(|v| v.active() && v.gate && v.note_id == note_id);
        let pedal = self.pedal;
        for v in self.voices.iter_mut().filter(|v| v.active() && v.gate) {
            let hit = if by_id {
                v.note_id == note_id
            } else {
                v.key == key && v.channel == channel
            };
            if hit {
                if pedal {
                    v.gate = false;
                    v.sustained = true;
                } else {
                    v.release();
                }
            }
        }
    }

    fn choke(&mut self, note_id: u32, channel: u8, key: u8) {
        let by_id = self
            .voices
            .iter()
            .any(|v| v.active() && v.note_id == note_id);
        for i in 0..SLOTS {
            let v = &self.voices[i];
            let hit = v.active()
                && if by_id {
                    v.note_id == note_id
                } else {
                    v.key == key && v.channel == channel
                };
            if hit {
                self.fade_out(i);
            }
        }
        self.remove_key(key);
    }

    fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.release();
            v.sustained = false;
        }
        self.stack_len = 0;
        self.pedal = false;
    }

    fn set_pedal(&mut self, down: bool) {
        self.pedal = down;
        if !down {
            for v in self.voices.iter_mut().filter(|v| v.sustained) {
                v.sustained = false;
                v.release();
            }
        }
    }

    fn midi(&mut self, data: [u8; 3]) {
        match data[0] & 0xF0 {
            0xE0 => {
                let raw = (i32::from(data[2] & 0x7F) << 7) | i32::from(data[1] & 0x7F);
                self.bend_target = (raw - 8192) as f32 / 8192.0;
            }
            0xB0 => match data[1] {
                64 => self.set_pedal(data[2] >= 64),
                120 => {
                    for i in 0..SLOTS {
                        self.fade_out(i);
                    }
                    self.stack_len = 0;
                }
                123 => self.all_notes_off(),
                _ => {}
            },
            _ => {}
        }
    }

    fn handle_event(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => {
                if velocity > 0.0 {
                    self.note_on(note_id, channel, key, velocity);
                } else {
                    self.note_off(note_id, channel, key);
                }
            }
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                ..
            } => self.note_off(note_id, channel, key),
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => self.choke(note_id, channel, key),
            EventKind::AllNotesOff => self.all_notes_off(),
            EventKind::Param { param, value } => self.apply_param(param, value, true),
            EventKind::Midi { data } => self.midi(data),
            // Per-note expressions are not mapped (yet).
            EventKind::NoteExpression { .. } => {}
        }
    }

    // --- rendering ---------------------------------------------------------------------

    /// Advance the LFO by `n` samples; returns its value (-1..1).
    fn tick_lfo(&mut self, n: usize) -> f32 {
        let shape = LfoShape::from_index(self.idx(id::LFO_SHAPE, 5));
        self.lfo_phase += self.v(id::LFO_RATE) as f32 * n as f32 / self.sr;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= self.lfo_phase.floor();
            self.lfo_held = self.rng.bipolar();
        }
        shape.at(self.lfo_phase, self.lfo_held)
    }

    fn chunk(&mut self, n: usize, lfo: f32) -> Chunk {
        for r in self.ramps.iter_mut() {
            r.advance(n as u32);
        }
        let bend_coef = 1.0 - (-(n as f32) / (0.003 * self.sr)).exp();
        self.bend += (self.bend_target - self.bend) * bend_coef;
        let isr = self.sr * self.os as f32;
        let alg = ALGORITHMS[self.idx(id::ALGORITHM, ALGORITHMS.len())];
        let mut ops = [OpChunk {
            on: false,
            audible: false,
            wave: 0,
            ratio: 1.0,
            fixed: None,
            level: (0.0, 0.0),
            velocity: 0.0,
            feedback: 0.0,
            rates: self.op_rates[0],
        }; OPS];
        for (o, c) in ops.iter_mut().enumerate() {
            let f = |field| id::op_param(o, field);
            let level = self.ramp(f(op::LEVEL));
            let on = self.on(f(op::ON));
            let coarse = self.v(f(op::COARSE)).round();
            let coarse = if coarse < 1.0 { 0.5 } else { coarse as f32 };
            *c = OpChunk {
                on,
                audible: on && (level.prev > 0.0 || level.cur > 0.0),
                wave: self.idx(f(op::WAVE), super::WAVES.len()),
                ratio: coarse * (1.0 + self.ramp(f(op::FINE)).cur),
                fixed: self
                    .on(f(op::FIXED))
                    .then(|| self.ramp(f(op::FREQUENCY)).cur),
                level: (level.prev, level.cur),
                velocity: self.ramp(f(op::VELOCITY)).cur,
                feedback: self.ramp(f(op::FEEDBACK)).cur,
                rates: self.op_rates[o],
            };
        }
        let carriers = alg.carriers.count_ones().max(1) as f32;
        let lfo_pitch = self.ramp(id::LFO_PITCH).cur;
        let glide_ms = self.v(id::GLIDE) as f32;
        let glide_coef = if glide_ms <= 0.0 {
            0.0
        } else {
            // ~95 % of the way in the glide time.
            (-(n as f32) * 3.0 / (glide_ms * 0.001 * self.sr)).exp()
        };
        let ty = self.idx(id::FILTER_TYPE, super::FILTER_TYPES.len());
        let res = self.ramp(id::RESONANCE).cur;
        Chunk {
            ni: n * self.os,
            isr,
            alg,
            norm: 1.0 / carriers.sqrt(),
            ops,
            pitch: self.v(id::TRANSPOSE) as f32
                + self.ramp(id::FINE).cur
                + self.bend * self.v(id::BEND_RANGE) as f32
                + lfo * lfo_pitch * lfo_pitch * LFO_PITCH_SEMIS,
            glide_coef,
            declick_coef: (-(n as f32) / (DECLICK_MS * 0.001 * self.sr)).exp(),
            filter: self.on(id::FILTER_ON),
            mode: [FilterMode::Low12, FilterMode::High12, FilterMode::Band12][ty / 2],
            two_stage: ty % 2 == 1,
            cutoff: self.ramp(id::CUTOFF).cur,
            k: dsp::svf_damping(res),
            k_flat: dsp::svf_damping(0.0),
            key_tracking: self.ramp(id::KEY_TRACKING).cur,
            fenv_amount: self.ramp(id::FILTER_ENV_AMOUNT).cur,
            lfo_cutoff: lfo * self.ramp(id::LFO_FILTER).cur * LFO_CUTOFF_OCTAVES,
            fenv_rates: self.fenv_rates,
        }
    }

    fn render(&mut self, outputs: &mut [&mut [f32]], sample_time: u64, start: usize, end: usize) {
        let mut pos = start;
        while pos < end {
            // Chunks sit on the absolute sample clock: renders don't depend on block sizes.
            let phase = ((sample_time + pos as u64) % CTRL as u64) as usize;
            let n = (end - pos).min(CTRL - phase);
            let lfo = self.tick_lfo(n);
            let c = self.chunk(n, lfo);
            let buf = &mut self.buf[..c.ni];
            buf.fill(0.0);
            let sine = &*self.sine;
            for v in self.voices.iter_mut().filter(|v| v.active()) {
                render_voice(v, &c, sine, buf);
            }
            // Output stage: decimate, tone, volume × tremolo.
            let tone = self.ramp(id::TONE).cur;
            let tone_a = (tone < 0.999).then(|| {
                let fc = TONE_MIN_HZ * (20_000.0 / TONE_MIN_HZ).powf(tone);
                1.0 - (-std::f32::consts::TAU * fc.min(self.sr * 0.45) / self.sr).exp()
            });
            let depth = self.ramp(id::LFO_VOLUME).cur;
            let gain_end = self.ramp(id::VOLUME).cur * (1.0 - depth * (1.0 - lfo) * 0.5);
            let step = (gain_end - self.out_gain) / n as f32;
            let mut g = self.out_gain;
            for i in 0..n {
                let mut x = if self.os == OS {
                    self.down.down([self.buf[2 * i], self.buf[2 * i + 1]])
                } else {
                    self.buf[i]
                };
                match tone_a {
                    Some(a) => {
                        self.tone_z += (x - self.tone_z) * a;
                        x = self.tone_z;
                    }
                    None => self.tone_z = x,
                }
                g += step;
                let y = x * g;
                for ch in outputs.iter_mut() {
                    ch[pos + i] = y;
                }
            }
            self.out_gain = gain_end;
            self.tone_z = dsp::flush(self.tone_z);
            pos += n;
        }
    }

    fn any_active(&self) -> bool {
        self.voices.iter().any(Voice::active)
    }

    /// Voices currently sounding (tests, meters).
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }
}

#[inline]
fn wrap01(p: f32) -> f32 {
    p - p.floor()
}

fn render_voice(v: &mut Voice, c: &Chunk, sine: &SineTable, buf: &mut [f32]) {
    let ni = c.ni;
    let glide = if v.declick {
        c.declick_coef
    } else {
        c.glide_coef
    };
    v.pitch = v.target + (v.pitch - v.target) * glide;
    if v.declick && (v.pitch - v.target).abs() < 1e-4 {
        v.pitch = v.target;
        v.declick = false;
    }
    let key = v.pitch + c.pitch;
    let base_hz = 440.0 * dsp::exp2((key - 69.0) / 12.0);
    let inv_ni = 1.0 / ni as f32;
    let mut dt = [0.0f32; OPS];
    let mut dt_step = [0.0f32; OPS];
    let mut lvl = [0.0f32; OPS];
    let mut lvl_step = [0.0f32; OPS];
    let mut fb = [0.0f32; OPS];
    let vel2 = v.velocity * v.velocity;
    for (o, op) in c.ops.iter().enumerate() {
        let hz = op.fixed.unwrap_or(base_hz * op.ratio);
        dt[o] = (hz / c.isr).clamp(0.0, MAX_DT);
        // Ramp from the last chunk's increment: glides and vibrato don't step.
        let start = if v.fresh { dt[o] } else { v.dt[o] };
        dt_step[o] = (dt[o] - start) * inv_ni;
        v.dt[o] = dt[o];
        dt[o] = start;
        let vg = 1.0 - op.velocity + op.velocity * vel2;
        lvl[o] = op.level.0 * vg;
        lvl_step[o] = (op.level.1 * vg - lvl[o]) * inv_ni;
        fb[o] = op.feedback * FB_DEPTH * 0.5;
    }
    // Filter coefficient (interpolated across the chunk).
    let g_end = if c.filter {
        let cutoff = c.cutoff
            + c.key_tracking * (f32::from(v.key) - 60.0) / 12.0
            + v.fenv.level * c.fenv_amount * ENV_CUTOFF_OCTAVES
            + c.lfo_cutoff;
        dsp::prewarp(dsp::exp2(cutoff.clamp(3.0, 15.0)), c.isr)
    } else {
        0.0
    };
    // A fresh voice (or a filter just switched on) starts at its cutoff.
    if v.fresh || v.g == 0.0 {
        v.g = g_end;
        v.fresh = false;
    }
    let g_step = (g_end - v.g) * inv_ni;
    let mut g = v.g;
    let alg = &c.alg;
    for s in buf.iter_mut().take(ni) {
        let mut outs = [0.0f32; OPS];
        let mut sum = 0.0f32;
        for o in (0..OPS).rev() {
            let opc = &c.ops[o];
            if !opc.on {
                continue;
            }
            let e = v.env[o].tick(&opc.rates);
            lvl[o] += lvl_step[o];
            dt[o] += dt_step[o];
            if !opc.audible {
                continue;
            }
            let mut pm = 0.0f32;
            let m = alg.modulators[o];
            if m != 0 {
                for (j, out) in outs.iter().enumerate().skip(o + 1) {
                    if m & (1 << j) != 0 {
                        pm += out;
                    }
                }
                pm *= MOD_DEPTH;
            }
            if fb[o] > 0.0 {
                pm += fb[o] * (v.last[o][0] + v.last[o][1]);
            }
            let p = if pm == 0.0 {
                v.phase[o]
            } else {
                wrap01(v.phase[o] + pm)
            };
            let raw = match opc.wave {
                0 => sine.at(p),
                1 => dsp::va(Shape::Triangle, p, dt[o], 0.5, sine),
                2 => dsp::va(Shape::Saw, p, dt[o], 0.5, sine),
                3 => dsp::va(Shape::Square, p, dt[o], 0.5, sine),
                _ => v.rng.bipolar(),
            };
            let y = raw * e * lvl[o];
            v.last[o] = [y, v.last[o][0]];
            outs[o] = y;
            if alg.carriers & (1 << o) != 0 {
                sum += y;
            }
            let np = v.phase[o] + dt[o];
            v.phase[o] = if np >= 1.0 { np - 1.0 } else { np };
        }
        let mut y = sum * c.norm;
        v.fenv.tick(&c.fenv_rates);
        if c.filter {
            g += g_step;
            let x = y * FILTER_PRE;
            let x = if c.two_stage {
                v.filt[1].tick(c.mode, x, g, c.k_flat, 1.0)
            } else {
                x
            };
            y = v.filt[0].tick(c.mode, x, g, c.k, 1.0) * (1.0 / FILTER_PRE);
        }
        *s += y * v.fade;
        if v.fade_step > 0.0 {
            v.fade -= v.fade_step;
            if v.fade <= 0.0 {
                v.alive = false;
                break;
            }
        }
    }
    v.g = g_end;
    for f in &mut v.filt {
        f.flush();
    }
    for l in &mut v.last {
        l[0] = dsp::flush(l[0]);
        l[1] = dsp::flush(l[1]);
    }
    if v.alive {
        // Sounding while a carrier that is on still has its envelope running.
        v.alive = (0..OPS).any(|o| c.ops[o].on && alg.is_carrier(o) && v.env[o].is_active());
    }
}

impl Node for FmSynth {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sr = config.sample_rate.max(1.0);
        self.gain_ramp = ((RAMP_GAIN_MS * 0.001 * self.sr) as u32).max(1);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = Voice::IDLE;
        }
        self.pedal = false;
        self.last_pitch = None;
        self.stack_len = 0;
        self.mono = None;
        self.bend = self.bend_target;
        self.lfo_phase = 0.0;
        self.lfo_held = 0.0;
        self.down.reset();
        self.tone_z = 0.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let has_notes = ctx
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::NoteOn { .. }));
        if !self.any_active() && !has_notes {
            for e in ctx.events {
                self.handle_event(&e.kind);
            }
            // Params settle instantly while silent (no voice can hear the ramp).
            for r in self.ramps.iter_mut() {
                r.advance(u32::MAX);
            }
            self.bend = self.bend_target;
            self.out_gain = self.ramp(id::VOLUME).cur;
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        let sample_time = ctx.transport.sample_time;
        let outputs = &mut *audio.outputs;
        if outputs.is_empty() {
            return ProcessStatus::Silent;
        }
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(outputs, sample_time, a, b),
            |s, kind| s.handle_event(kind),
        );
        ProcessStatus::Continue
    }

    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

impl Device for FmSynth {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::FmSynth)
    }

    fn param(&self, pid: ParamId) -> Option<f64> {
        self.values.get(pid.0 as usize).copied()
    }

    fn set_param(&mut self, pid: ParamId, value: f64) {
        self.apply_param(pid, value, false);
    }
}
