//! Basic-shape synth: sine/saw/square/triangle oscillator, ADSR, low-pass filter.
//!
//! The instruments pass appends `Fine` (cents), `Glide`, `Velocity` (sensitivity),
//! and the filter's `Env Amount` (the amp envelope sweeps the cutoff, ±6 octaves at
//! ±100 %) and `Key Tracking`; at their defaults the synth renders exactly as before.
//!
//! `Mode` (id 14, appended) is the voice mode, as on the Poly Synth: `Poly`, `Mono` (one
//! voice, every note retriggers the envelope, glides when `Glide` > 0) or `Legato` (one
//! voice; an overlapping note takes over without retriggering and glides, a detached one
//! retriggers without gliding). Mono/legato use last-note priority: releasing the newest
//! key returns to the most recent key still held. Pitch is `Transpose` + `Fine` (global).
//!
//! Polyphonic (fixed [`MAX_VOICES`], oldest-voice stealing). Saw, square and triangle are
//! band-limited with 4-point polyBLEP / polyBLAMP ([`crate::dsp::blep`]); the filter is a per-voice TPT state-variable low-pass
//! (stable under fast modulation). All state is pre-allocated: `process` never allocates.

use std::f32::consts::{PI, TAU};

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::dsp::blep::{blamp, blep};
use crate::util::{self, Adsr, AdsrRates};

/// Maximum simultaneous voices.
pub const MAX_VOICES: usize = 16;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const WAVEFORM: ParamId = ParamId(0);
    pub const TRANSPOSE: ParamId = ParamId(1);
    pub const ATTACK: ParamId = ParamId(2);
    pub const DECAY: ParamId = ParamId(3);
    pub const SUSTAIN: ParamId = ParamId(4);
    pub const RELEASE: ParamId = ParamId(5);
    pub const CUTOFF: ParamId = ParamId(6);
    pub const RESONANCE: ParamId = ParamId(7);
    pub const VOLUME: ParamId = ParamId(8);
    /// Fine tuning, cents.
    pub const FINE: ParamId = ParamId(9);
    /// Portamento time, ms.
    pub const GLIDE: ParamId = ParamId(10);
    /// Velocity → level, percent (100 = the original response).
    pub const VELOCITY: ParamId = ParamId(11);
    /// Amp envelope → cutoff, percent of ±6 octaves.
    pub const FILTER_ENV: ParamId = ParamId(12);
    /// Cutoff follows the key, percent (100 = one octave per octave, around C4).
    pub const KEY_TRACKING: ParamId = ParamId(13);
    /// Voice mode: Poly / Mono / Legato ([`super::VOICE_MODES`]).
    pub const VOICE_MODE: ParamId = ParamId(14);
}

const NUM_PARAMS: usize = 15;
/// Held keys remembered in mono/legato modes (last-note priority).
const STACK: usize = 16;

/// Voice mode labels, in plain-value order (as the Poly Synth's and FM Synth's `Mode`).
pub const VOICE_MODES: [&str; 3] = ["Poly", "Mono", "Legato"];
/// Cutoff sweep of the envelope at `Env Amount` ±100 % (octaves).
const ENV_OCTAVES: f32 = 6.0;
/// Glide and per-voice filter updates run every this many samples.
const CTRL: usize = 16;
/// Time constant of the pitch change of a sounding mono / legato voice that doesn't glide
/// (ms): reads as a jump, but a sine doesn't kink (a -50 dB click on a 220 → 330 Hz
/// jump).
const DECLICK_MS: f32 = 0.5;

/// Waveform labels, in plain-value order (0 = sine ... 3 = triangle).
pub const WAVEFORMS: [&str; 4] = ["Sine", "Saw", "Square", "Triangle"];

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::{choice, param};
    let time = ParamScale::Power { exponent: 3.0 };
    vec![
        choice(0, "Waveform", "Oscillator", ParamUnit::None, &WAVEFORMS, 1),
        param(
            1,
            "Transpose",
            "Oscillator",
            ParamUnit::Semitones,
            (-24.0, 24.0, 0.0),
            ParamScale::Linear,
        )
        .with_step(1.0),
        param(
            2,
            "Attack",
            "Envelope",
            ParamUnit::Milliseconds,
            (0.0, 5000.0, 5.0),
            time,
        ),
        param(
            3,
            "Decay",
            "Envelope",
            ParamUnit::Milliseconds,
            (1.0, 10000.0, 300.0),
            time,
        ),
        param(
            4,
            "Sustain",
            "Envelope",
            ParamUnit::Percent,
            (0.0, 100.0, 70.0),
            ParamScale::Linear,
        ),
        param(
            5,
            "Release",
            "Envelope",
            ParamUnit::Milliseconds,
            (1.0, 10000.0, 250.0),
            time,
        ),
        param(
            6,
            "Cutoff",
            "Filter",
            ParamUnit::Hertz,
            (20.0, 20000.0, 8000.0),
            ParamScale::Log,
        ),
        param(
            7,
            "Resonance",
            "Filter",
            ParamUnit::Percent,
            (0.0, 100.0, 10.0),
            ParamScale::Linear,
        ),
        param(
            8,
            "Volume",
            "Output",
            ParamUnit::Decibels,
            (-60.0, 6.0, -12.0),
            ParamScale::Linear,
        ),
        param(
            9,
            "Fine",
            "Oscillator",
            ParamUnit::Cents,
            (-100.0, 100.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            10,
            "Glide",
            "Oscillator",
            ParamUnit::Milliseconds,
            (0.0, 2000.0, 0.0),
            time,
        ),
        param(
            11,
            "Velocity",
            "Output",
            ParamUnit::Percent,
            (0.0, 100.0, 100.0),
            ParamScale::Linear,
        ),
        param(
            12,
            "Env Amount",
            "Filter",
            ParamUnit::Percent,
            (-100.0, 100.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            13,
            "Key Tracking",
            "Filter",
            ParamUnit::Percent,
            (0.0, 100.0, 0.0),
            ParamScale::Linear,
        ),
        choice(14, "Mode", "Voice", ParamUnit::None, &VOICE_MODES, 0),
    ]
}

/// Descriptor of the synth type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: Some(layout()),
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Synth,
        },
        name: "Synth".to_owned(),
        category: DeviceCategory::Instrument,
        params: param_infos(),
        audio_inputs: 0,
        audio_outputs: 2,
        midi_input: true,
        sidechain_inputs: 0,
        implemented: true,
    }
}

/// Declarative panel (v0.2; drawn by the shared device renderer).
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{Widget, WidgetSize::*};
    let mut osc = item(
        Widget::Oscillator {
            shape: params::WAVEFORM,
            position: None,
        },
        Small,
    );
    osc.colspan = 2;
    let mut filter = item(
        Widget::FilterCurve {
            cutoff: params::CUTOFF,
            resonance: params::RESONANCE,
            mode: None,
            drive: None,
            gain: None,
        },
        Large,
    );
    filter.colspan = 2;
    let env = item(
        Widget::Envelope {
            attack: params::ATTACK,
            decay: params::DECAY,
            sustain: params::SUSTAIN,
            release: params::RELEASE,
            delay: None,
            hold: None,
        },
        Medium,
    );
    layout(vec![
        section(
            "oscillator",
            Some("Oscillator"),
            1,
            2,
            vec![
                osc,
                knob(params::TRANSPOSE, Small),
                knob(params::FINE, Small),
            ],
        ),
        section(
            "voice",
            Some("Voice"),
            1,
            1,
            vec![
                item(
                    Widget::Choice {
                        param: params::VOICE_MODE,
                    },
                    Small,
                ),
                knob(params::GLIDE, Small),
            ],
        ),
        section(
            "filter",
            Some("Filter"),
            2,
            2,
            vec![
                filter,
                knob(params::FILTER_ENV, Small),
                knob(params::KEY_TRACKING, Small),
            ],
        ),
        section("envelope", Some("Envelope"), 2, 1, vec![env]),
        section(
            "output",
            Some("Output"),
            1,
            1,
            vec![knob(params::VOLUME, Large), knob(params::VELOCITY, Small)],
        ),
    ])
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    note_id: u32,
    channel: u8,
    key: u8,
    velocity: f32,
    /// Allocation order, for stealing the oldest voice.
    age: u64,
    phase: f32,
    /// Phase increment per sample (set at the start of each render span, then ramped by
    /// `dt_step` per sample towards each control update's value while gliding).
    dt: f32,
    dt_step: f32,
    /// Sounding pitch (keys, gliding toward `key`).
    pitch: f32,
    /// The pitch moves to `key` with [`DECLICK_MS`] (a mono note change without glide).
    declick: bool,
    env: Adsr,
    // SVF integrator states.
    ic1: f32,
    ic2: f32,
    /// Own filter coefficients while the cutoff is modulated per voice.
    coefs: [f32; 3],
}

impl Voice {
    const IDLE: Self = Self {
        note_id: 0,
        channel: 0,
        key: 0,
        velocity: 0.0,
        age: 0,
        phase: 0.0,
        dt: 0.0,
        dt_step: 0.0,
        pitch: 0.0,
        declick: false,
        env: Adsr::IDLE,
        ic1: 0.0,
        ic2: 0.0,
        coefs: [0.0; 3],
    };
}

#[inline]
fn oscillator(waveform: usize, phase: f32, dt: f32) -> f32 {
    match waveform {
        0 => (TAU * phase).sin(),
        1 => 2.0 * phase - 1.0 - 2.0 * blep(phase, dt),
        2 => {
            let naive = if phase < 0.5 { 1.0 } else { -1.0 };
            let mut half = phase + 0.5;
            if half >= 1.0 {
                half -= 1.0;
            }
            naive + 2.0 * blep(phase, dt) - 2.0 * blep(half, dt)
        }
        _ => {
            // Corners at 0 (slope +4 per cycle after) and 0.5: 8·dt per sample each.
            let mut half = phase + 0.5;
            if half >= 1.0 {
                half -= 1.0;
            }
            let k = 8.0 * dt;
            1.0 - 4.0 * (phase - 0.5).abs() + k * blamp(phase, dt) - k * blamp(half, dt)
        }
    }
}

/// TPT state-variable low-pass coefficients `(a1, a2, a3)` for `cutoff` (Hz) and `res`
/// (0..1).
#[inline]
fn svf_coefs(cutoff: f32, res: f32, sample_rate: f32) -> [f32; 3] {
    let fc = cutoff.clamp(10.0, sample_rate * 0.45);
    let g = (PI * fc / sample_rate).tan();
    // Damping: Butterworth (k = √2) at 0 %, close to self-oscillation at 100 %.
    let k = std::f32::consts::SQRT_2 * (1.0 - res) + 0.05 * res;
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    [a1, a2, g * a2]
}

/// The built-in polyphonic synth.
pub struct Synth {
    values: [f64; NUM_PARAMS],
    infos: Vec<ParamInfo>,
    sample_rate: f32,
    voices: [Voice; MAX_VOICES],
    next_age: u64,
    rates: AdsrRates,
    cutoff: Smoother,
    volume: Smoother,
    // Filter coefficients for the current cutoff/resonance.
    a1: f32,
    a2: f32,
    a3: f32,
    coef_cutoff: f32,
    coef_res: f32,
    /// Pitch of the last note played (glide start).
    last_pitch: Option<f32>,
    /// Mono/legato: held keys (last = most recent) and the sounding slot.
    stack: [u8; STACK],
    stack_len: usize,
    mono: Option<usize>,
}

impl std::fmt::Debug for Synth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Synth")
            .field("values", &self.values)
            .finish()
    }
}

impl Default for Synth {
    fn default() -> Self {
        Self::new()
    }
}

impl Synth {
    /// Non-RT. A synth with default parameters.
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        for (v, info) in values.iter_mut().zip(&infos) {
            *v = info.default;
        }
        let mut s = Self {
            values,
            infos,
            sample_rate: 48_000.0,
            voices: [Voice::IDLE; MAX_VOICES],
            next_age: 0,
            rates: AdsrRates::new(5.0, 300.0, 0.7, 250.0, 48_000.0),
            cutoff: Smoother::new(8000.0, 20.0, 48_000.0),
            volume: Smoother::new(0.25, 20.0, 48_000.0),
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            coef_cutoff: -1.0,
            coef_res: -1.0,
            last_pitch: None,
            stack: [0; STACK],
            stack_len: 0,
            mono: None,
        };
        s.sync_all();
        s
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    /// Recompute all derived state from `values` without smoothing.
    fn sync_all(&mut self) {
        self.cutoff = Smoother::new(self.value(params::CUTOFF) as f32, 20.0, self.sample_rate);
        self.volume = Smoother::new(
            util::db_to_amp(self.value(params::VOLUME) as f32),
            20.0,
            self.sample_rate,
        );
        self.update_rates();
        self.coef_cutoff = -1.0;
        self.update_coefs(self.cutoff.current());
    }

    fn update_rates(&mut self) {
        self.rates = AdsrRates::new(
            self.value(params::ATTACK) as f32,
            self.value(params::DECAY) as f32,
            self.value(params::SUSTAIN) as f32 * 0.01,
            self.value(params::RELEASE) as f32,
            self.sample_rate,
        );
    }

    #[inline]
    fn update_coefs(&mut self, cutoff: f32) {
        let res = self.value(params::RESONANCE) as f32 * 0.01;
        if cutoff == self.coef_cutoff && res == self.coef_res {
            return;
        }
        self.coef_cutoff = cutoff;
        self.coef_res = res;
        [self.a1, self.a2, self.a3] = svf_coefs(cutoff, res, self.sample_rate);
    }

    /// RT-safe. Store a clamped plain value; `smooth` ramps continuous params.
    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(info) = self.infos.get(id.0 as usize) else {
            return;
        };
        let v = util::clamp(value, info.min, info.max);
        self.values[id.0 as usize] = v;
        match id {
            params::CUTOFF => {
                if smooth {
                    self.cutoff.set_target(v as f32);
                } else {
                    self.cutoff.set_immediate(v as f32);
                }
            }
            params::VOLUME => {
                let amp = util::db_to_amp(v as f32);
                if smooth {
                    self.volume.set_target(amp);
                } else {
                    self.volume.set_immediate(amp);
                }
            }
            params::ATTACK | params::DECAY | params::SUSTAIN | params::RELEASE => {
                self.update_rates();
            }
            params::RESONANCE => {
                let c = self.cutoff.current();
                self.update_coefs(c);
            }
            _ => {}
        }
    }

    fn glide_on(&self) -> bool {
        self.value(params::GLIDE) > 0.0
    }

    /// 0 = poly, 1 = mono, 2 = legato.
    fn mode(&self) -> usize {
        util::index(self.value(params::VOICE_MODE), VOICE_MODES.len())
    }

    /// A slot for a new voice: a free one, else the oldest releasing, else the oldest.
    fn allocate(&self) -> usize {
        self.voices
            .iter()
            .position(|v| !v.env.is_active())
            .or_else(|| {
                let releasing = self
                    .voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.env.is_releasing())
                    .min_by_key(|(_, v)| v.age)
                    .map(|(i, _)| i);
                releasing.or_else(|| {
                    self.voices
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, v)| v.age)
                        .map(|(i, _)| i)
                })
            })
            .unwrap_or(0)
    }

    /// Start a fresh voice in `slot` (glides from the last note, except in legato mode
    /// where only overlapping notes glide).
    fn start_voice(&mut self, slot: usize, note_id: u32, channel: u8, key: u8, velocity: f32) {
        self.next_age += 1;
        let pitch = match self.last_pitch {
            Some(p) if self.glide_on() && self.mode() != 2 => p,
            _ => f32::from(key),
        };
        let v = &mut self.voices[slot];
        *v = Voice {
            note_id,
            channel,
            key,
            velocity: velocity.clamp(0.0, 1.0),
            age: self.next_age,
            pitch,
            ..Voice::IDLE
        };
        v.env.trigger();
    }

    fn note_on(&mut self, note_id: u32, channel: u8, key: u8, velocity: f32) {
        let mode = self.mode();
        if mode == 0 {
            let slot = self.allocate();
            self.start_voice(slot, note_id, channel, key, velocity);
        } else {
            let was_held = self.stack_len > 0;
            self.push_key(key);
            match self.mono.filter(|&i| self.voices[i].env.is_active()) {
                Some(i) => {
                    let legato = mode == 2 && was_held && !self.voices[i].env.is_releasing();
                    let glide = self.glide_on() && (mode == 1 || legato);
                    self.next_age += 1;
                    let v = &mut self.voices[i];
                    v.note_id = note_id;
                    v.channel = channel;
                    v.key = key;
                    v.age = self.next_age;
                    v.declick = !glide;
                    if !legato {
                        v.velocity = velocity.clamp(0.0, 1.0);
                        v.env.trigger();
                    }
                }
                None => {
                    let slot = self.allocate();
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
            if let Some(i) = self.mono.filter(|&i| self.voices[i].env.is_active()) {
                let v = &self.voices[i];
                if v.key == key && !v.env.is_releasing() {
                    if self.stack_len > 0 {
                        // Back to the most recent held key (mono retriggers, legato slides).
                        let back = self.stack[self.stack_len - 1];
                        let glide = self.glide_on();
                        let v = &mut self.voices[i];
                        v.key = back;
                        v.declick = !glide;
                        if mode == 1 {
                            v.env.trigger();
                        }
                        self.last_pitch = Some(f32::from(back));
                    } else {
                        self.voices[i].env.release();
                    }
                    return;
                }
            }
        }
        // Poly (or a voice left from before a mode change): release the matching voices
        // that haven't been released already.
        self.for_matching(note_id, channel, key, |v| {
            if !v.env.is_releasing() {
                v.env.release();
            }
        });
    }

    /// Voices matching `note_id`, or if none does, the key/channel (hosts without note ids).
    fn for_matching(&mut self, note_id: u32, channel: u8, key: u8, f: impl Fn(&mut Voice)) {
        let by_id = self
            .voices
            .iter()
            .any(|v| v.env.is_active() && v.note_id == note_id);
        for v in self.voices.iter_mut().filter(|v| v.env.is_active()) {
            let hit = if by_id {
                v.note_id == note_id
            } else {
                v.key == key && v.channel == channel
            };
            if hit {
                f(v);
            }
        }
    }

    fn handle_event(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => self.note_on(note_id, channel, key, velocity),
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
            } => {
                self.remove_key(key);
                self.for_matching(note_id, channel, key, |v| v.env.kill());
            }
            EventKind::AllNotesOff => {
                self.stack_len = 0;
                for v in &mut self.voices {
                    v.env.release();
                }
            }
            EventKind::Param { param, value, step } => self.apply_param(param, value, !step),
            EventKind::Midi { .. } | EventKind::NoteExpression { .. } => {}
        }
    }

    fn render(&mut self, outputs: &mut [&mut [f32]], start: usize, end: usize) {
        let waveform = util::index(self.value(params::WAVEFORM), WAVEFORMS.len());
        let transpose = self.value(params::TRANSPOSE) as f32;
        let fine = self.value(params::FINE) as f32 * 0.01;
        let sr = self.sample_rate;
        let glide_ms = self.value(params::GLIDE) as f32;
        // Per CTRL samples: ~95 % of the way in the glide time.
        let glide_coef = if glide_ms > 0.0 {
            (-(CTRL as f32) * 3.0 / (glide_ms * 0.001 * sr)).exp()
        } else {
            0.0
        };
        let declick_coef = (-(CTRL as f32) / (DECLICK_MS * 0.001 * sr)).exp();
        let sens = self.value(params::VELOCITY) as f32 * 0.01;
        let env_oct = self.value(params::FILTER_ENV) as f32 * 0.01 * ENV_OCTAVES;
        let tracking = self.value(params::KEY_TRACKING) as f32 * 0.01;
        // Per-voice cutoffs only when something modulates them (else the shared filter).
        let per_voice = env_oct != 0.0 || tracking != 0.0;
        let res = self.value(params::RESONANCE) as f32 * 0.01;
        let mut gliding = false;
        for v in self.voices.iter_mut().filter(|v| v.env.is_active()) {
            if glide_ms <= 0.0 && !v.declick {
                v.pitch = f32::from(v.key);
            }
            gliding |= v.pitch != f32::from(v.key);
            v.dt = (util::key_to_hz(v.pitch + transpose + fine) / sr).min(0.49);
            v.dt_step = 0.0;
        }
        for i in start..end {
            let cutoff = self.cutoff.tick();
            self.update_coefs(cutoff);
            let gain = self.volume.tick();
            let (a1, a2, a3) = (self.a1, self.a2, self.a3);
            let rates = self.rates;
            // Control rate (every CTRL samples of the span): glide and per-voice cutoffs.
            let ctrl = (i - start).is_multiple_of(CTRL);
            let mut sum = 0.0;
            for v in self.voices.iter_mut().filter(|v| v.env.is_active()) {
                if ctrl && gliding {
                    let target = f32::from(v.key);
                    let coef = if v.declick { declick_coef } else { glide_coef };
                    v.pitch = target + (v.pitch - target) * coef;
                    if (v.pitch - target).abs() < 1e-3 {
                        v.pitch = target;
                        v.declick = false;
                    }
                    // Ramp to the new increment over this control period (no steps).
                    let dt = (util::key_to_hz(v.pitch + transpose + fine) / sr).min(0.49);
                    v.dt_step = (dt - v.dt) / CTRL as f32;
                }
                if v.dt_step != 0.0 {
                    v.dt += v.dt_step;
                }
                if ctrl && per_voice {
                    let octaves =
                        env_oct * v.env.level() + tracking * (f32::from(v.key) - 60.0) / 12.0;
                    v.coefs = svf_coefs(cutoff * octaves.exp2(), res, sr);
                }
                let [b1, b2, b3] = if per_voice { v.coefs } else { [a1, a2, a3] };
                let dt = v.dt;
                let osc = oscillator(waveform, v.phase, dt);
                v.phase += dt;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
                // TPT state-variable low-pass.
                let v3 = osc - v.ic2;
                let v1 = b1 * v.ic1 + b2 * v3;
                let v2 = v.ic2 + b2 * v.ic1 + b3 * v3;
                v.ic1 = 2.0 * v1 - v.ic1;
                v.ic2 = 2.0 * v2 - v.ic2;
                // Velocity sensitivity (100 %: the level is the velocity, as before).
                let vel = if sens >= 1.0 {
                    v.velocity
                } else {
                    1.0 - sens + sens * v.velocity
                };
                sum += v2 * v.env.tick(&rates) * vel;
            }
            let out = sum * gain;
            for ch in outputs.iter_mut() {
                ch[i] = out;
            }
        }
    }

    fn any_active(&self) -> bool {
        self.voices.iter().any(|v| v.env.is_active())
    }
}

impl Node for Synth {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        self.voices = [Voice::IDLE; MAX_VOICES];
        self.stack_len = 0;
        self.mono = None;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let was_active = self.any_active();
        let has_notes = ctx
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::NoteOn { .. }));
        if !was_active && !has_notes {
            // Still apply param events so state stays in sync.
            for e in ctx.events {
                self.handle_event(&e.kind);
            }
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        let outputs = &mut *audio.outputs;
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(outputs, a, b),
            |s, kind| s.handle_event(kind),
        );
        ProcessStatus::Continue
    }

    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

impl Device for Synth {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
