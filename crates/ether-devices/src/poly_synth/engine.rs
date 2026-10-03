//! The Poly Synth node: voice allocation (poly with stealing, mono, legato), glide, unison,
//! modulation (envelopes, LFOs) and the per-voice signal path.
//!
//! Rendering runs in control chunks of at most [`CTRL`] samples, aligned on the engine's
//! sample clock and split at event offsets (params and notes land on their exact sample; a
//! render is the same for every block size that is a multiple of [`CTRL`]): pitch, filter cutoff, wavetable position and
//! modulation are evaluated once per chunk and per voice; oscillators, envelopes, filters and
//! every gain run per sample, with cutoff (`g`) and gains interpolated linearly across the
//! chunk so modulation and automation never step. Everything is pre-allocated: `process`
//! never allocates, locks or blocks.

use std::f32::consts::FRAC_PI_4;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
    TransportInfo,
};

use super::dsp::{
    self, Env, EnvRates, FilterMode, FilterState, LfoShape, NoiseColor, Rng, Shape, SineTable,
};
use super::poly_synth as id;
use super::tables::{Cycle, FRAMES, Wavetables};
use crate::contract::SYNC_RATE_BEATS;
use crate::util;

/// Largest `Voices` value.
pub const MAX_POLYPHONY: usize = 32;
/// Physical voice slots: the polyphony plus room for stolen voices fading out.
const SLOTS: usize = MAX_POLYPHONY + 8;
/// Largest `Unison` value.
pub const MAX_UNISON: usize = 8;
/// Control-rate chunk (samples).
const CTRL: usize = 16;
/// Ramp of pitch/filter/position params (samples): two automation grid intervals.
const RAMP_FAST: u32 = 2 * ether_core::automation_rt::PARAM_GRID as u32;
/// Ramp of gain params (ms): long enough that a knob jump never clicks.
const RAMP_GAIN_MS: f32 = 5.0;
/// Fade-out of a stolen or choked voice (ms).
const STEAL_MS: f32 = 2.0;
/// Level params at their minimum are off.
const LEVEL_OFF_DB: f64 = -70.0;
/// Held keys remembered in mono/legato modes (last-note priority).
const STACK: usize = 16;

/// Filter envelope / mod envelope cutoff range at ±100 % (octaves).
const ENV_CUTOFF_OCTAVES: f32 = 8.0;
/// Mod envelope pitch range at ±100 % (semitones).
const ENV_PITCH_SEMIS: f32 = 24.0;
/// LFO pitch depth at ±100 % (semitones; the amount is squared for fine vibrato).
const LFO_PITCH_SEMIS: f32 = 12.0;
/// LFO cutoff range at ±100 % (octaves).
const LFO_CUTOFF_OCTAVES: f32 = 4.0;
/// Pulse-width modulation range at ±100 % (fraction of the cycle).
const PW_MOD: f32 = 0.45;
/// Unison detune at 100 % (cents, outermost voice; the knob is squared).
const DETUNE_CENTS: f32 = 100.0;

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

    /// Advance `n` samples (the chunk), keeping the start value in `prev`.
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

/// Smoothed value domain of a param: amplitude for levels/volume, log2 Hz for the cutoff,
/// linear gain for drive, 0..1 for percents, plain otherwise.
fn domain(pid: ParamId, v: f64) -> f32 {
    match pid {
        id::OSC1_LEVEL | id::OSC2_LEVEL | id::SUB_LEVEL | id::NOISE_LEVEL | id::VOLUME => {
            if v <= LEVEL_OFF_DB {
                0.0
            } else {
                util::db_to_amp(v as f32)
            }
        }
        id::CUTOFF => (v as f32).max(1.0).log2(),
        id::DRIVE => util::db_to_amp(v as f32),
        id::OSC1_POSITION
        | id::OSC2_POSITION
        | id::OSC1_PULSE_WIDTH
        | id::OSC2_PULSE_WIDTH
        | id::NOISE_COLOR
        | id::UNISON_DETUNE
        | id::UNISON_SPREAD
        | id::RESONANCE
        | id::KEY_TRACKING
        | id::FILTER_ENV_AMOUNT
        | id::MOD_ENV_AMOUNT
        | id::LFO1_AMOUNT
        | id::LFO2_AMOUNT
        | id::VELOCITY => v as f32 * 0.01,
        _ => v as f32,
    }
}

fn is_gain(pid: ParamId) -> bool {
    matches!(
        pid,
        id::OSC1_LEVEL | id::OSC2_LEVEL | id::SUB_LEVEL | id::NOISE_LEVEL | id::VOLUME | id::DRIVE
    )
}

// --- modulation routing ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModEnvTarget {
    Off,
    Pitch,
    Osc2Pitch,
    Position,
    PulseWidth,
    Cutoff,
}

impl ModEnvTarget {
    fn from_index(i: usize) -> Self {
        [
            Self::Off,
            Self::Pitch,
            Self::Osc2Pitch,
            Self::Position,
            Self::PulseWidth,
            Self::Cutoff,
        ][i.min(5)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LfoTarget {
    Off,
    Pitch,
    Cutoff,
    Amp,
    Pan,
    Position,
    PulseWidth,
}

impl LfoTarget {
    fn from_index(i: usize) -> Self {
        [
            Self::Off,
            Self::Pitch,
            Self::Cutoff,
            Self::Amp,
            Self::Pan,
            Self::Position,
            Self::PulseWidth,
        ][i.min(6)]
    }
}

/// Sum of the global (LFO) modulation of one chunk.
#[derive(Clone, Copy, Debug, Default)]
struct GlobalMod {
    pitch: f32,
    cutoff: f32,
    position: f32,
    pw: f32,
    /// Amp gain (1 = none) and pan (-1..1) at the chunk end.
    amp: f32,
    pan: f32,
}

#[derive(Clone, Copy, Debug)]
struct Lfo {
    phase: f32,
    held: f32,
    /// Sync: index of the current cycle (sample & hold redraws when it changes).
    cycle: i64,
}

// --- voices ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Voice {
    note_id: u32,
    channel: u8,
    key: u8,
    velocity: f32,
    age: u64,
    /// Key held.
    gate: bool,
    /// Released while the sustain pedal was down.
    sustained: bool,
    /// Current (gliding) and target pitch, in MIDI keys.
    pitch: f32,
    target: f32,
    amp: Env,
    fenv: Env,
    menv: Env,
    /// 1 unless fading out (stolen/choked): then decreasing by `fade_step` to 0.
    fade: f32,
    fade_step: f32,
    phase1: [f32; MAX_UNISON],
    phase2: [f32; MAX_UNISON],
    sub_phase: f32,
    rng: Rng,
    noise_z: f32,
    filt: [FilterState; 2],
    /// Chunk-start values of the interpolated controls (`fresh` = none yet).
    g: f32,
    gain: f32,
    pan: (f32, f32),
    fresh: bool,
}

impl Voice {
    const IDLE: Self = Self {
        note_id: 0,
        channel: 0,
        key: 0,
        velocity: 0.0,
        age: 0,
        gate: false,
        sustained: false,
        pitch: 60.0,
        target: 60.0,
        amp: Env::IDLE,
        fenv: Env::IDLE,
        menv: Env::IDLE,
        fade: 1.0,
        fade_step: 0.0,
        phase1: [0.0; MAX_UNISON],
        phase2: [0.0; MAX_UNISON],
        sub_phase: 0.0,
        rng: Rng::new(1),
        noise_z: 0.0,
        filt: [FilterState::ZERO; 2],
        g: 0.0,
        gain: 0.0,
        pan: (1.0, 1.0),
        fresh: true,
    };

    fn active(&self) -> bool {
        self.amp.is_active()
    }

    fn fading(&self) -> bool {
        self.fade_step > 0.0
    }

    fn release(&mut self) {
        self.gate = false;
        self.amp.release();
        self.fenv.release();
        self.menv.release();
    }

    fn trigger(&mut self) {
        self.gate = true;
        self.sustained = false;
        self.amp.trigger();
        self.fenv.trigger();
        self.menv.trigger();
    }
}

/// Per-chunk oscillator source of one voice.
#[derive(Clone, Copy)]
enum Source<'a> {
    Off,
    Va(Shape),
    Table {
        a: Cycle<'a>,
        b: Cycle<'a>,
        blend: f32,
    },
}

impl Source<'_> {
    #[inline]
    fn at(&self, phase: f32, dt: f32, pw: f32, sine: &SineTable) -> f32 {
        match self {
            Source::Off => 0.0,
            Source::Va(s) => dsp::va(*s, phase, dt, pw, sine),
            Source::Table { a, b, blend } => {
                let x = a.at(phase);
                x + (b.at(phase) - x) * blend
            }
        }
    }
}

/// Everything shared by the voices for one chunk.
#[derive(Clone, Copy)]
struct Chunk {
    n: usize,
    sr: f32,
    unison: usize,
    uni_ratio: [f32; MAX_UNISON],
    uni_pan: [(f32, f32); MAX_UNISON],
    uni_norm: f32,
    stereo: bool,
    type1: usize,
    type2: usize,
    table1: usize,
    table2: usize,
    offset1: f32,
    offset2: f32,
    pos1: f32,
    pos2: f32,
    pw1: f32,
    pw2: f32,
    lvl1: (f32, f32),
    lvl2: (f32, f32),
    sub: (f32, f32),
    sub_div: f32,
    noise: (f32, f32),
    noise_color: NoiseColor,
    mode: FilterMode,
    cutoff: f32,
    k: f32,
    drive: f32,
    drive_comp: f32,
    key_tracking: f32,
    fenv_amount: f32,
    menv_target: ModEnvTarget,
    menv_amount: f32,
    velocity: f32,
    glide_coef: f32,
    bend: f32,
    lfo: GlobalMod,
    amp_rates: EnvRates,
    fenv_rates: EnvRates,
    menv_rates: EnvRates,
}

/// The Poly Synth.
pub struct PolySynth {
    values: [f64; id::COUNT],
    ranges: [(f64, f64, f64); id::COUNT],
    ramps: [Ramp; id::COUNT],
    sr: f32,
    gain_ramp: u32,
    tables: &'static Wavetables,
    sine: Box<SineTable>,
    voices: Box<[Voice; SLOTS]>,
    amp_rates: EnvRates,
    fenv_rates: EnvRates,
    menv_rates: EnvRates,
    lfos: [Lfo; 2],
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
}

impl std::fmt::Debug for PolySynth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolySynth").finish_non_exhaustive()
    }
}

impl Default for PolySynth {
    fn default() -> Self {
        Self::new()
    }
}

impl PolySynth {
    /// Non-RT (generates the shared wavetables on first use).
    pub fn new() -> Self {
        let desc = super::descriptor(BuiltinDeviceType::PolySynth);
        let mut values = [0.0; id::COUNT];
        let mut ranges = [(0.0, 0.0, 0.0); id::COUNT];
        for p in &desc.params {
            let i = p.id.0 as usize;
            values[i] = p.default;
            ranges[i] = (p.min, p.max, p.default);
        }
        let mut s = Self {
            values,
            ranges,
            ramps: [Ramp::new(0.0); id::COUNT],
            sr: 48_000.0,
            gain_ramp: 240,
            tables: Wavetables::get(),
            sine: Box::new(SineTable::new()),
            voices: Box::new([Voice::IDLE; SLOTS]),
            amp_rates: EnvRates::new(5.0, 200.0, 0.7, 300.0, 48_000.0),
            fenv_rates: EnvRates::new(5.0, 400.0, 0.0, 300.0, 48_000.0),
            menv_rates: EnvRates::new(5.0, 400.0, 0.0, 300.0, 48_000.0),
            lfos: [Lfo {
                phase: 0.0,
                held: 0.0,
                cycle: -1,
            }; 2],
            bend_target: 0.0,
            bend: 0.0,
            next_age: 0,
            last_pitch: None,
            stack: [0; STACK],
            stack_len: 0,
            mono: None,
            pedal: false,
            rng: Rng::new(0x5EED_1234),
        };
        s.sync_all();
        s
    }

    fn v(&self, pid: ParamId) -> f64 {
        self.values[pid.0 as usize]
    }

    fn idx(&self, pid: ParamId, count: usize) -> usize {
        util::index(self.v(pid), count)
    }

    /// An integer param `1..=max` (unison, voices).
    fn count(&self, pid: ParamId, max: usize) -> usize {
        (self.v(pid).round().max(1.0) as usize).min(max)
    }

    fn ramp(&self, pid: ParamId) -> &Ramp {
        &self.ramps[pid.0 as usize]
    }

    /// Snap every derived value to the params (no smoothing).
    fn sync_all(&mut self) {
        for i in 0..id::COUNT {
            self.ramps[i] = Ramp::new(domain(ParamId(i as u32), self.values[i]));
        }
        self.update_rates();
    }

    fn update_rates(&mut self) {
        let sr = self.sr;
        let env = |s: &Self, a: ParamId, d: ParamId, su: ParamId, r: ParamId| {
            EnvRates::new(
                s.v(a) as f32,
                s.v(d) as f32,
                s.v(su) as f32 * 0.01,
                s.v(r) as f32,
                sr,
            )
        };
        self.amp_rates = env(
            self,
            id::AMP_ATTACK,
            id::AMP_DECAY,
            id::AMP_SUSTAIN,
            id::AMP_RELEASE,
        );
        self.fenv_rates = env(
            self,
            id::FILTER_ATTACK,
            id::FILTER_DECAY,
            id::FILTER_SUSTAIN,
            id::FILTER_RELEASE,
        );
        self.menv_rates = env(
            self,
            id::MOD_ATTACK,
            id::MOD_DECAY,
            id::MOD_SUSTAIN,
            id::MOD_RELEASE,
        );
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
        if (id::AMP_ATTACK.0..=id::MOD_RELEASE.0).contains(&pid.0) {
            self.update_rates();
        }
    }

    // --- notes -------------------------------------------------------------------------

    fn glide_on(&self) -> bool {
        self.v(id::GLIDE) > 0.0
    }

    fn mode(&self) -> usize {
        self.idx(id::VOICE_MODE, 3)
    }

    /// A free slot for a new voice (poly: steals when the polyphony is exhausted).
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
                .min_by_key(|(_, v)| (!v.amp.is_releasing(), v.age))
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
        let step = 1.0 / (STEAL_MS * 0.001 * self.sr).max(1.0);
        let v = &mut self.voices[i];
        if v.active() && !v.fading() {
            v.fade_step = step;
        }
        if self.mono == Some(i) {
            self.mono = None;
        }
    }

    /// Start a fresh voice in `slot`.
    fn start_voice(&mut self, slot: usize, note_id: u32, channel: u8, key: u8, velocity: f32) {
        self.next_age += 1;
        let unison = self.count(id::UNISON_VOICES, MAX_UNISON);
        let start = match self.last_pitch {
            Some(p) if self.glide_on() && self.mode() != 2 => p,
            _ => f32::from(key),
        };
        let mut rng = Rng::new(self.rng.next_u32());
        let mut v = Voice {
            note_id,
            channel,
            key,
            velocity: velocity.clamp(0.0, 1.0),
            age: self.next_age,
            pitch: start,
            target: f32::from(key),
            rng: Rng::new(rng.next_u32()),
            ..Voice::IDLE
        };
        if unison > 1 {
            // Free-running feel: random start phases so unison voices never stack.
            for u in 0..MAX_UNISON {
                v.phase1[u] = rng.unit();
                v.phase2[u] = rng.unit();
            }
        }
        v.trigger();
        self.voices[slot] = v;
    }

    fn note_on(&mut self, note_id: u32, channel: u8, key: u8, velocity: f32) {
        let mode = self.mode();
        if mode == 0 {
            let limit = self.count(id::VOICES, MAX_POLYPHONY);
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
                    if !glide {
                        v.pitch = v.target;
                    }
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

    fn note_off(&mut self, note_id: u32, channel: u8, key: u8) {
        let mode = self.mode();
        if mode != 0 {
            self.remove_key(key);
            if let Some(i) = self.mono.filter(|&i| self.voices[i].active()) {
                if self.voices[i].key == key && self.voices[i].gate {
                    if self.stack_len > 0 {
                        // Back to the most recent held key.
                        let back = self.stack[self.stack_len - 1];
                        let glide = self.glide_on();
                        let v = &mut self.voices[i];
                        v.key = back;
                        v.target = f32::from(back);
                        if !glide {
                            v.pitch = v.target;
                        }
                        if mode == 1 {
                            v.trigger();
                        }
                        self.last_pitch = Some(f32::from(back));
                    } else if self.pedal {
                        self.voices[i].sustained = true;
                        self.voices[i].gate = false;
                    } else {
                        self.voices[i].release();
                    }
                }
                return;
            }
        }
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
            // v0.3: per-note pitch/pressure/timbre (`mpe` implements the Poly Synth's MPE).
            EventKind::NoteExpression { .. } => {}
        }
    }

    // --- rendering ---------------------------------------------------------------------

    /// Advance both LFOs by `n` samples from `offset` in the block; returns their values.
    fn tick_lfos(&mut self, t: &TransportInfo, offset: usize, n: usize) -> [f32; 2] {
        let mut out = [0.0; 2];
        let sr = self.sr;
        for (l, o) in out.iter_mut().enumerate() {
            let (shape, rate, sync, sync_rate) = if l == 0 {
                (
                    id::LFO1_SHAPE,
                    id::LFO1_RATE,
                    id::LFO1_SYNC,
                    id::LFO1_SYNC_RATE,
                )
            } else {
                (
                    id::LFO2_SHAPE,
                    id::LFO2_RATE,
                    id::LFO2_SYNC,
                    id::LFO2_SYNC_RATE,
                )
            };
            let shape = LfoShape::from_index(self.idx(shape, 5));
            let synced = self.v(sync) >= 0.5;
            let beats = SYNC_RATE_BEATS[self.idx(sync_rate, SYNC_RATE_BEATS.len())];
            let lfo = &mut self.lfos[l];
            let wrapped;
            if synced && t.playing {
                // Locked to the song position.
                let beat = t.position + t.beats_per_sample * (offset + n) as f64;
                let cycles = beat / beats;
                let cycle = cycles.floor() as i64;
                wrapped = cycle != lfo.cycle;
                lfo.cycle = cycle;
                lfo.phase = (cycles - cycles.floor()) as f32;
            } else {
                let hz = if synced {
                    (t.bpm.max(1.0) / 60.0 / beats) as f32
                } else {
                    self.values[rate.0 as usize] as f32
                };
                lfo.phase += hz * n as f32 / sr;
                wrapped = lfo.phase >= 1.0;
                lfo.phase -= lfo.phase.floor();
                lfo.cycle = -1;
            }
            if wrapped {
                lfo.held = self.rng.bipolar();
            }
            *o = shape.at(lfo.phase, lfo.held);
        }
        out
    }

    fn global_mod(&self, lfo: [f32; 2]) -> GlobalMod {
        let mut m = GlobalMod {
            amp: 1.0,
            ..GlobalMod::default()
        };
        for (l, value) in lfo.into_iter().enumerate() {
            let (target, amount) = if l == 0 {
                (id::LFO1_TARGET, id::LFO1_AMOUNT)
            } else {
                (id::LFO2_TARGET, id::LFO2_AMOUNT)
            };
            let a = self.ramp(amount).cur;
            match LfoTarget::from_index(self.idx(target, 7)) {
                LfoTarget::Off => {}
                LfoTarget::Pitch => m.pitch += value * a * a.abs() * LFO_PITCH_SEMIS,
                LfoTarget::Cutoff => m.cutoff += value * a * LFO_CUTOFF_OCTAVES,
                LfoTarget::Amp => {
                    let depth = a.abs();
                    let uni = if a >= 0.0 { value } else { -value };
                    m.amp *= 1.0 - depth * (1.0 - uni) * 0.5;
                }
                LfoTarget::Pan => m.pan += value * a,
                LfoTarget::Position => m.position += value * a,
                LfoTarget::PulseWidth => m.pw += value * a * PW_MOD,
            }
        }
        m.pan = m.pan.clamp(-1.0, 1.0);
        m
    }

    fn chunk(&mut self, n: usize, lfo: [f32; 2]) -> Chunk {
        for r in self.ramps.iter_mut() {
            r.advance(n as u32);
        }
        let bend_coef = 1.0 - (-(n as f32) / (0.003 * self.sr)).exp();
        self.bend += (self.bend_target - self.bend) * bend_coef;
        let unison = self.count(id::UNISON_VOICES, MAX_UNISON);
        let detune = self.ramp(id::UNISON_DETUNE).cur;
        let spread = self.ramp(id::UNISON_SPREAD).cur;
        let mut uni_ratio = [1.0; MAX_UNISON];
        let mut uni_pan = [(1.0, 1.0); MAX_UNISON];
        if unison > 1 {
            let cents = DETUNE_CENTS * detune * detune;
            for u in 0..unison {
                let x = 2.0 * u as f32 / (unison - 1) as f32 - 1.0;
                uni_ratio[u] = dsp::exp2(x * cents / 1200.0);
                uni_pan[u] = pan_gains(x * spread);
            }
        }
        let offset = |s: &Self, oct: ParamId, semi: ParamId, fine: ParamId| {
            s.v(oct) as f32 * 12.0 + s.v(semi) as f32 + s.ramp(fine).cur
        };
        let pair = |r: &Ramp| (r.prev, r.cur);
        let mode = FilterMode::from_index(self.idx(id::FILTER_TYPE, 5));
        let res = self.ramp(id::RESONANCE).cur;
        let drive = self.ramp(id::DRIVE).cur;
        let glide_ms = self.v(id::GLIDE) as f32;
        let glide_coef = if glide_ms <= 0.0 {
            0.0
        } else {
            // ~95 % of the way in the glide time.
            (-(n as f32) * 3.0 / (glide_ms * 0.001 * self.sr)).exp()
        };
        let bend = self.bend * self.v(id::BEND_RANGE) as f32;
        let lfo = self.global_mod(lfo);
        Chunk {
            n,
            sr: self.sr,
            unison,
            uni_ratio,
            uni_pan,
            uni_norm: 1.0 / (unison as f32).sqrt(),
            stereo: unison > 1 && spread > 0.0,
            type1: self.idx(id::OSC1_TYPE, 5),
            type2: self.idx(id::OSC2_TYPE, 5),
            table1: self.idx(id::OSC1_TABLE, 8),
            table2: self.idx(id::OSC2_TABLE, 8),
            offset1: offset(self, id::OSC1_OCTAVE, id::OSC1_SEMITONES, id::OSC1_FINE),
            offset2: offset(self, id::OSC2_OCTAVE, id::OSC2_SEMITONES, id::OSC2_FINE),
            pos1: self.ramp(id::OSC1_POSITION).cur,
            pos2: self.ramp(id::OSC2_POSITION).cur,
            pw1: self.ramp(id::OSC1_PULSE_WIDTH).cur,
            pw2: self.ramp(id::OSC2_PULSE_WIDTH).cur,
            lvl1: pair(self.ramp(id::OSC1_LEVEL)),
            lvl2: pair(self.ramp(id::OSC2_LEVEL)),
            sub: pair(self.ramp(id::SUB_LEVEL)),
            sub_div: if self.idx(id::SUB_OCTAVE, 2) == 0 {
                0.5
            } else {
                0.25
            },
            noise: pair(self.ramp(id::NOISE_LEVEL)),
            noise_color: NoiseColor::new(self.ramp(id::NOISE_COLOR).cur, self.sr),
            mode,
            cutoff: self.ramp(id::CUTOFF).cur,
            k: if mode == FilterMode::Ladder24 {
                dsp::ladder_feedback(res)
            } else {
                dsp::svf_damping(res)
            },
            drive,
            // Keep the level roughly constant as the drive saturates.
            drive_comp: 1.0 / drive.powf(0.6),
            key_tracking: self.ramp(id::KEY_TRACKING).cur,
            fenv_amount: self.ramp(id::FILTER_ENV_AMOUNT).cur,
            menv_target: ModEnvTarget::from_index(self.idx(id::MOD_ENV_TARGET, 6)),
            menv_amount: self.ramp(id::MOD_ENV_AMOUNT).cur,
            velocity: self.ramp(id::VELOCITY).cur,
            glide_coef,
            bend,
            lfo,
            amp_rates: self.amp_rates,
            fenv_rates: self.fenv_rates,
            menv_rates: self.menv_rates,
        }
    }

    fn render(
        &mut self,
        outputs: &mut [&mut [f32]],
        transport: &TransportInfo,
        start: usize,
        end: usize,
    ) {
        let mut pos = start;
        while pos < end {
            // Chunks sit on the absolute sample clock: renders don't depend on block sizes.
            let phase = ((transport.sample_time + pos as u64) % CTRL as u64) as usize;
            let n = (end - pos).min(CTRL - phase);
            let lfo = self.tick_lfos(transport, pos, n);
            let c = self.chunk(n, lfo);
            for ch in outputs.iter_mut() {
                ch[pos..pos + n].fill(0.0);
            }
            let (tables, sine) = (self.tables, &*self.sine);
            for v in self.voices.iter_mut().filter(|v| v.active()) {
                render_voice(v, &c, tables, sine, outputs, pos);
            }
            let vol = self.ramps[id::VOLUME.0 as usize];
            let step = (vol.cur - vol.prev) / n as f32;
            for ch in outputs.iter_mut() {
                let mut g = vol.prev;
                for s in &mut ch[pos..pos + n] {
                    g += step;
                    *s *= g;
                }
            }
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

/// Oscillator source of one oscillator for a voice's chunk.
fn source<'a>(
    tables: &'a Wavetables,
    ty: usize,
    table: usize,
    pos: f32,
    dt_max: f32,
    sr: f32,
    level: (f32, f32),
) -> Source<'a> {
    if level.0 == 0.0 && level.1 == 0.0 {
        return Source::Off;
    }
    match ty {
        0 => Source::Va(Shape::Saw),
        1 => Source::Va(Shape::Square),
        2 => Source::Va(Shape::Triangle),
        3 => Source::Va(Shape::Sine),
        _ => {
            let lvl = Wavetables::level_for(dt_max, sr);
            let x = pos.clamp(0.0, 1.0) * (FRAMES - 1) as f32;
            let f = (x as usize).min(FRAMES - 2);
            Source::Table {
                a: tables.cycle(table, f, lvl),
                b: tables.cycle(table, f + 1, lvl),
                blend: x - f as f32,
            }
        }
    }
}

/// Equal-power pan gains for `pan` in -1..1, exactly unity in the centre.
fn pan_gains(pan: f32) -> (f32, f32) {
    if pan == 0.0 {
        return (1.0, 1.0);
    }
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (
        a.cos() * std::f32::consts::SQRT_2,
        a.sin() * std::f32::consts::SQRT_2,
    )
}

#[inline]
fn key_dt(key: f32, sr: f32) -> f32 {
    (440.0 * dsp::exp2((key - 69.0) / 12.0) / sr).min(0.45)
}

fn render_voice(
    v: &mut Voice,
    c: &Chunk,
    tables: &Wavetables,
    sine: &SineTable,
    outputs: &mut [&mut [f32]],
    pos: usize,
) {
    let n = c.n;
    // Glide.
    v.pitch = v.target + (v.pitch - v.target) * c.glide_coef;
    // Envelope-driven modulation (values at the chunk start).
    let menv = v.menv.level * c.menv_amount;
    let (mut pitch_mod, mut osc2_mod, mut pos_mod, mut pw_mod, mut cut_mod) =
        (0.0, 0.0, 0.0, 0.0, 0.0);
    match c.menv_target {
        ModEnvTarget::Off => {}
        ModEnvTarget::Pitch => pitch_mod = menv * ENV_PITCH_SEMIS,
        ModEnvTarget::Osc2Pitch => osc2_mod = menv * ENV_PITCH_SEMIS,
        ModEnvTarget::Position => pos_mod = menv,
        ModEnvTarget::PulseWidth => pw_mod = menv * PW_MOD,
        ModEnvTarget::Cutoff => cut_mod = menv * ENV_CUTOFF_OCTAVES,
    }
    let base = v.pitch + c.bend + c.lfo.pitch + pitch_mod;
    let key1 = base + c.offset1;
    let key2 = base + c.offset2 + osc2_mod;
    let dt1 = key_dt(key1, c.sr);
    let dt2 = key_dt(key2, c.sr);
    let max_ratio = c.uni_ratio[..c.unison]
        .iter()
        .fold(1.0f32, |m, &r| m.max(r));
    let src1 = source(
        tables,
        c.type1,
        c.table1,
        c.pos1 + pos_mod + c.lfo.position,
        dt1 * max_ratio,
        c.sr,
        c.lvl1,
    );
    let src2 = source(
        tables,
        c.type2,
        c.table2,
        c.pos2 + pos_mod + c.lfo.position,
        dt2 * max_ratio,
        c.sr,
        c.lvl2,
    );
    let pw1 = (c.pw1 + pw_mod + c.lfo.pw).clamp(0.02, 0.98);
    let pw2 = (c.pw2 + pw_mod + c.lfo.pw).clamp(0.02, 0.98);
    let mut dts1 = [0.0f32; MAX_UNISON];
    let mut dts2 = [0.0f32; MAX_UNISON];
    for u in 0..c.unison {
        dts1[u] = (dt1 * c.uni_ratio[u]).min(0.45);
        dts2[u] = (dt2 * c.uni_ratio[u]).min(0.45);
    }
    let sub_dt = dt1 * c.sub_div;
    let sub_on = c.sub.0 > 0.0 || c.sub.1 > 0.0;
    let noise_on = c.noise.0 > 0.0 || c.noise.1 > 0.0;

    // Filter cutoff (log2 Hz) and the velocity response.
    let vel_amt = 1.0 - c.velocity + c.velocity * v.velocity * v.velocity;
    let fenv_vel = 1.0 - 0.5 * c.velocity * (1.0 - v.velocity);
    let cutoff = c.cutoff
        + c.key_tracking * (f32::from(v.key) - 60.0) / 12.0
        + v.fenv.level * c.fenv_amount * ENV_CUTOFF_OCTAVES * fenv_vel
        + c.lfo.cutoff
        + cut_mod;
    let g_end = dsp::prewarp(dsp::exp2(cutoff.clamp(3.0, 15.0)), c.sr);
    let gain_end = vel_amt * c.lfo.amp;
    let pan_end = pan_gains(c.lfo.pan);
    if v.fresh {
        v.g = g_end;
        v.gain = gain_end;
        v.pan = pan_end;
        v.fresh = false;
    }
    let inv_n = 1.0 / n as f32;
    let g_step = (g_end - v.g) * inv_n;
    let gain_step = (gain_end - v.gain) * inv_n;
    let pan_step = ((pan_end.0 - v.pan.0) * inv_n, (pan_end.1 - v.pan.1) * inv_n);
    let l1_step = (c.lvl1.1 - c.lvl1.0) * inv_n;
    let l2_step = (c.lvl2.1 - c.lvl2.0) * inv_n;
    let sub_step = (c.sub.1 - c.sub.0) * inv_n;
    let noise_step = (c.noise.1 - c.noise.0) * inv_n;
    let (mut l1, mut l2, mut sub, mut noise) = (c.lvl1.0, c.lvl2.0, c.sub.0, c.noise.0);
    let (mut g, mut gain, mut pan) = (v.g, v.gain, v.pan);
    // Pre-filter headroom: the saturator sees about ±0.5 per oscillator at drive 0 dB.
    let pre = 0.5 * c.drive_comp.max(0.0);
    let drive = c.drive;
    let post = c.drive_comp;
    let (left, rest) = outputs.split_at_mut(1);
    let out_l = &mut left[0][pos..pos + n];
    let mut out_r = rest.first_mut().map(|r| &mut r[pos..pos + n]);
    for i in 0..n {
        l1 += l1_step;
        l2 += l2_step;
        sub += sub_step;
        noise += noise_step;
        g += g_step;
        gain += gain_step;
        pan.0 += pan_step.0;
        pan.1 += pan_step.1;
        let (mut xl, mut xr) = (0.0f32, 0.0f32);
        for u in 0..c.unison {
            let mut s = 0.0;
            if !matches!(src1, Source::Off) {
                let p = v.phase1[u];
                s += src1.at(p, dts1[u], pw1, sine) * l1;
                let p = p + dts1[u];
                v.phase1[u] = if p >= 1.0 { p - 1.0 } else { p };
            }
            if !matches!(src2, Source::Off) {
                let p = v.phase2[u];
                s += src2.at(p, dts2[u], pw2, sine) * l2;
                let p = p + dts2[u];
                v.phase2[u] = if p >= 1.0 { p - 1.0 } else { p };
            }
            xl += s * c.uni_pan[u].0;
            xr += s * c.uni_pan[u].1;
        }
        xl *= c.uni_norm;
        xr *= c.uni_norm;
        let mut mono = 0.0;
        if sub_on {
            mono += sine.at(v.sub_phase) * sub;
            let p = v.sub_phase + sub_dt;
            v.sub_phase = if p >= 1.0 { p - 1.0 } else { p };
        }
        if noise_on {
            let w = v.rng.bipolar();
            mono += c.noise_color.tick(w, &mut v.noise_z) * noise;
        }
        let env = v.amp.tick(&c.amp_rates);
        v.fenv.tick(&c.fenv_rates);
        v.menv.tick(&c.menv_rates);
        let yl = v.filt[0].tick(c.mode, (xl + mono) * pre, g, c.k, drive);
        let yr = if c.stereo {
            v.filt[1].tick(c.mode, (xr + mono) * pre, g, c.k, drive)
        } else {
            yl
        };
        let a = env * gain * v.fade * post;
        out_l[i] += yl * a * pan.0;
        if let Some(r) = out_r.as_deref_mut() {
            r[i] += yr * a * pan.1;
        }
        if v.fade_step > 0.0 {
            v.fade -= v.fade_step;
            if v.fade <= 0.0 {
                v.amp = Env::IDLE;
                break;
            }
        }
    }
    v.g = g;
    v.gain = gain;
    v.pan = pan;
    v.filt[0].flush();
    v.filt[1].flush();
    v.noise_z = dsp::flush(v.noise_z);
}

impl Node for PolySynth {
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
        self.stack_len = 0;
        self.mono = None;
        self.pedal = false;
        self.last_pitch = None;
        self.bend = self.bend_target;
        for l in &mut self.lfos {
            l.phase = 0.0;
            l.cycle = -1;
        }
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
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        let transport = ctx.transport;
        let outputs = &mut *audio.outputs;
        if outputs.is_empty() {
            return ProcessStatus::Silent;
        }
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(outputs, transport, a, b),
            |s, kind| s.handle_event(kind),
        );
        ProcessStatus::Continue
    }

    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

impl Device for PolySynth {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::PolySynth)
    }

    fn param(&self, pid: ParamId) -> Option<f64> {
        self.values.get(pid.0 as usize).copied()
    }

    fn set_param(&mut self, pid: ParamId, value: f64) {
        self.apply_param(pid, value, false);
    }
}
