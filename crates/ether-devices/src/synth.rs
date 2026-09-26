//! Basic-shape synth: sine/saw/square/triangle oscillator, ADSR, low-pass filter.
//!
//! Polyphonic (fixed [`MAX_VOICES`], oldest-voice stealing). Saw and square are
//! band-limited with polyBLEP; the filter is a per-voice TPT state-variable low-pass
//! (stable under fast modulation). All state is pre-allocated: `process` never allocates.

use std::f32::consts::{PI, TAU};

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

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
}

const NUM_PARAMS: usize = 9;

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
        ),
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
    ]
}

/// Descriptor of the synth type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Synth,
        },
        name: "Synth".to_owned(),
        category: DeviceCategory::Instrument,
        params: param_infos(),
        audio_inputs: 0,
        audio_outputs: 2,
        midi_input: true,
    }
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
    /// Phase increment per sample (set at the start of each render span).
    dt: f32,
    env: Adsr,
    // SVF integrator states.
    ic1: f32,
    ic2: f32,
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
        env: Adsr::IDLE,
        ic1: 0.0,
        ic2: 0.0,
    };
}

/// polyBLEP residual for a discontinuity at phase 0 (phase in 0..1, `dt` = increment).
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[inline]
fn oscillator(waveform: usize, phase: f32, dt: f32) -> f32 {
    match waveform {
        0 => (TAU * phase).sin(),
        1 => 2.0 * phase - 1.0 - poly_blep(phase, dt),
        2 => {
            let naive = if phase < 0.5 { 1.0 } else { -1.0 };
            let mut half = phase + 0.5;
            if half >= 1.0 {
                half -= 1.0;
            }
            naive + poly_blep(phase, dt) - poly_blep(half, dt)
        }
        _ => 1.0 - 4.0 * (phase - 0.5).abs(),
    }
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
        let fc = cutoff.clamp(10.0, self.sample_rate * 0.45);
        let g = (PI * fc / self.sample_rate).tan();
        // Damping: Butterworth (k = √2) at 0 %, close to self-oscillation at 100 %.
        let k = std::f32::consts::SQRT_2 * (1.0 - res) + 0.05 * res;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
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

    fn note_on(&mut self, note_id: u32, channel: u8, key: u8, velocity: f32) {
        let slot = self
            .voices
            .iter()
            .position(|v| !v.env.is_active())
            .or_else(|| {
                // Steal the oldest releasing voice, else the oldest voice.
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
            .unwrap_or(0);
        self.next_age += 1;
        let v = &mut self.voices[slot];
        *v = Voice {
            note_id,
            channel,
            key,
            velocity: velocity.clamp(0.0, 1.0),
            age: self.next_age,
            ..Voice::IDLE
        };
        v.env.trigger();
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
            } => {
                // Only release voices that haven't been released already.
                self.for_matching(note_id, channel, key, |v| {
                    if !v.env.is_releasing() {
                        v.env.release();
                    }
                });
            }
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => self.for_matching(note_id, channel, key, |v| v.env.kill()),
            EventKind::AllNotesOff => {
                for v in &mut self.voices {
                    v.env.release();
                }
            }
            EventKind::Param { param, value } => self.apply_param(param, value, true),
            EventKind::Midi { .. } => {}
        }
    }

    fn render(&mut self, outputs: &mut [&mut [f32]], start: usize, end: usize) {
        let waveform = util::index(self.value(params::WAVEFORM), WAVEFORMS.len());
        let transpose = self.value(params::TRANSPOSE) as f32;
        let sr = self.sample_rate;
        for v in self.voices.iter_mut().filter(|v| v.env.is_active()) {
            v.dt = (util::key_to_hz(v.key as f32 + transpose) / sr).min(0.49);
        }
        for i in start..end {
            let cutoff = self.cutoff.tick();
            self.update_coefs(cutoff);
            let gain = self.volume.tick();
            let (a1, a2, a3) = (self.a1, self.a2, self.a3);
            let rates = self.rates;
            let mut sum = 0.0;
            for v in self.voices.iter_mut().filter(|v| v.env.is_active()) {
                let dt = v.dt;
                let osc = oscillator(waveform, v.phase, dt);
                v.phase += dt;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
                // TPT state-variable low-pass.
                let v3 = osc - v.ic2;
                let v1 = a1 * v.ic1 + a2 * v3;
                let v2 = v.ic2 + a2 * v.ic1 + a3 * v3;
                v.ic1 = 2.0 * v1 - v.ic1;
                v.ic2 = 2.0 * v2 - v.ic2;
                sum += v2 * v.env.tick(&rates) * v.velocity;
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
