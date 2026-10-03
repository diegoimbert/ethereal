//! The multisampler node: zone voices with loops, global amp/filter envelopes and a
//! per-voice filter.
//!
//! - **Voices**: up to [`MAX_VOICES`] (the `Voices` param limits polyphony). A note-on starts
//!   one voice per selected zone ([`ZoneSet::select`]); stealing takes the oldest voice,
//!   releasing ones first.
//! - **Pitch**: `key - root_key + tune_cents/100 + Transpose + Fine` semitones (+ glide from
//!   the previous note), read with 4-point Hermite interpolation (rate capped at 4 octaves
//!   up).
//! - **Loops**: while held and `looping`, a voice wraps `[loop_start, loop_end)`; with a
//!   crossfade `X` the last `X` frames before `loop_end` blend linearly into the `X` frames
//!   before `loop_start`, so the wrap is continuous (`X` is clamped to the loop length and to
//!   `loop_start`). After note-off the voice plays on to the zone end.
//! - **Live zone edits** ([`Node::set_data`] with a boxed [`ZoneSet`]): voices keep their
//!   playback data and are remapped to the new set's sources by media id; a voice whose media
//!   left the set fades out over a few milliseconds.
//!
//! Everything is allocated in `new`/`prepare`; `process` reads the sources in chunks into
//! fixed scratch buffers.

use ether_core::node::NodeData;
use ether_core::protocol::devices::{DeviceDescriptor, ParamInfo};
use ether_core::protocol::model::{MediaId, ParamId};
use ether_core::{
    AudioBuffers, AudioSource, Device, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessStatus, Smoother,
};

use super::multi_sampler as p;
use super::zones::{Hit, MAX_LAYERS, RoundRobin, ZoneSet};
use crate::util::{self, Adsr, AdsrRates};

/// Maximum simultaneous voices (the `Voices` param's maximum).
pub const MAX_VOICES: usize = 64;
/// Maximum playback-rate ratio (4 octaves up).
const MAX_RATE: f64 = 16.0;
/// Render chunk in frames (filter coefficients and pitch update per chunk).
const CHUNK: usize = 32;
/// Source frames read per chunk: `CHUNK · MAX_RATE` plus the interpolation margin.
const SCRATCH: usize = CHUNK * MAX_RATE as usize + 4;
/// Fade for a zone end inside the sample and for voices dropped by a zone edit, in ms.
const FADE_MS: f64 = 2.0;

#[derive(Clone, Copy, Debug)]
struct Voice {
    note_id: u32,
    channel: u8,
    key: u8,
    age: u64,
    /// Media and its index in the current set's sources.
    media: MediaId,
    source: usize,
    /// Read position in source frames.
    pos: f64,
    /// Playback limits in source frames.
    end: f64,
    looping: bool,
    loop_start: f64,
    loop_end: f64,
    xfade: f64,
    /// Zone pitch offset in semitones (`key - root + tune`).
    semis: f64,
    /// Glide offset in semitones, decaying linearly to 0.
    glide: f64,
    glide_step: f64,
    /// Velocity amp × zone gain × crossfade gain.
    amp: f32,
    /// Zone pan gains (balance).
    pan: [f32; 2],
    env: Adsr,
    fenv: Adsr,
    /// Forced fade-out (dropped by a zone edit): remaining frames, 0 = none.
    dying: u32,
    filter: [[Svf; 2]; 2],
    /// Reads the previous zone set's source (its media left the set: fading out).
    from_prev: bool,
}

impl Voice {
    const IDLE: Self = Self {
        note_id: 0,
        channel: 0,
        key: 0,
        age: 0,
        media: MediaId(ether_core::protocol::model::Ulid(0)),
        source: 0,
        pos: 0.0,
        end: 0.0,
        looping: false,
        loop_start: 0.0,
        loop_end: 0.0,
        xfade: 0.0,
        semis: 0.0,
        glide: 0.0,
        glide_step: 0.0,
        amp: 0.0,
        pan: [1.0, 1.0],
        env: Adsr::IDLE,
        fenv: Adsr::IDLE,
        dying: 0,
        filter: [[Svf::ZERO; 2]; 2],
        from_prev: false,
    };

    fn active(&self) -> bool {
        self.env.is_active()
    }
}

/// Filter modes in param order (`Type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterMode {
    Off,
    Lp24,
    Lp12,
    Hp12,
    Bp12,
}

impl FilterMode {
    fn from_index(i: usize) -> Self {
        match i {
            1 => Self::Lp24,
            2 => Self::Lp12,
            3 => Self::Hp12,
            4 => Self::Bp12,
            _ => Self::Off,
        }
    }
}

/// The multisampler (`BuiltinDeviceType::MultiSampler`).
pub struct MultiSampler {
    descriptor: DeviceDescriptor,
    infos: Vec<ParamInfo>,
    values: [f64; p::COUNT],
    set: Box<ZoneSet>,
    /// The previous set while voices dropped by a zone edit fade out.
    prev: Option<Box<ZoneSet>>,
    sample_rate: f32,
    voices: Box<[Voice; MAX_VOICES]>,
    next_age: u64,
    last_key: Option<u8>,
    rr: RoundRobin,
    amp_rates: AdsrRates,
    filter_rates: AdsrRates,
    volume: Smoother,
    pan: Smoother,
    /// Transpose + Fine, semitones.
    pitch: Smoother,
    /// log2(cutoff Hz).
    cutoff: Smoother,
    resonance: Smoother,
    /// Per source channel read scratch (main and loop-crossfade tail).
    scratch: [[Vec<f32>; 2]; 2],
    /// Interpolated voice output per channel (main, tail).
    frames: [[[f32; CHUNK]; 2]; 2],
    gains: [[f32; CHUNK]; 2],
    hits: [Hit; MAX_LAYERS],
}

impl std::fmt::Debug for MultiSampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiSampler")
            .field("zones", &self.set)
            .field("values", &self.values)
            .finish()
    }
}

impl MultiSampler {
    /// Non-RT. A multisampler with default params playing `set`.
    pub fn new(set: ZoneSet) -> Self {
        let descriptor = super::descriptor(super::BuiltinDeviceType::MultiSampler);
        let infos = descriptor.params.clone();
        let mut values = [0.0; p::COUNT];
        for (v, info) in values.iter_mut().zip(&infos) {
            *v = info.default;
        }
        let sr = 48_000.0;
        let mut s = Self {
            descriptor,
            infos,
            values,
            set: Box::new(set),
            prev: None,
            sample_rate: sr,
            voices: Box::new([Voice::IDLE; MAX_VOICES]),
            next_age: 0,
            last_key: None,
            rr: RoundRobin::default(),
            amp_rates: AdsrRates::new(1.0, 200.0, 1.0, 100.0, sr),
            filter_rates: AdsrRates::new(1.0, 400.0, 0.0, 300.0, sr),
            volume: Smoother::new(1.0, 20.0, sr),
            pan: Smoother::new(0.0, 20.0, sr),
            pitch: Smoother::new(0.0, 10.0, sr),
            cutoff: Smoother::new(20000f32.log2(), 20.0, sr),
            resonance: Smoother::new(0.0, 20.0, sr),
            scratch: [
                [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
                [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
            ],
            frames: [[[0.0; CHUNK]; 2]; 2],
            gains: [[0.0; CHUNK]; 2],
            hits: [Hit::default(); MAX_LAYERS],
        };
        s.sync_all();
        s
    }

    /// The zones this instance plays.
    pub fn zones(&self) -> &ZoneSet {
        &self.set
    }

    /// Voices currently sounding.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    fn sync_all(&mut self) {
        let sr = self.sample_rate;
        self.volume = Smoother::new(self.volume_amp(), 20.0, sr);
        self.pan = Smoother::new(self.value(p::PAN) as f32, 20.0, sr);
        self.pitch = Smoother::new(self.pitch_target(), 10.0, sr);
        self.cutoff = Smoother::new((self.value(p::CUTOFF) as f32).log2(), 20.0, sr);
        self.resonance = Smoother::new(self.value(p::RESONANCE) as f32 * 0.01, 20.0, sr);
        self.update_rates();
        self.rr.random = util::index(self.value(p::ROUND_ROBIN), 2) == 1;
    }

    fn volume_amp(&self) -> f32 {
        let db = self.value(p::VOLUME) as f32;
        if db <= -70.0 {
            0.0
        } else {
            util::db_to_amp(db)
        }
    }

    fn pitch_target(&self) -> f32 {
        (self.value(p::TRANSPOSE).round() + self.value(p::FINE)) as f32
    }

    fn update_rates(&mut self) {
        let vals = self.values;
        let v = |id: ParamId| vals[id.0 as usize] as f32;
        self.amp_rates = AdsrRates::new(
            v(p::AMP_ATTACK),
            v(p::AMP_DECAY),
            v(p::AMP_SUSTAIN) * 0.01,
            v(p::AMP_RELEASE),
            self.sample_rate,
        );
        self.filter_rates = AdsrRates::new(
            v(p::FILTER_ATTACK),
            v(p::FILTER_DECAY),
            v(p::FILTER_SUSTAIN) * 0.01,
            v(p::FILTER_RELEASE),
            self.sample_rate,
        );
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(info) = self.infos.get(id.0 as usize) else {
            return;
        };
        let mut v = util::clamp(value, info.min, info.max);
        if info.step.is_some() {
            v = v.round();
        }
        self.values[id.0 as usize] = v;
        let set = |s: &mut Smoother, t: f32| {
            if smooth {
                s.set_target(t)
            } else {
                s.set_immediate(t)
            }
        };
        match id {
            p::VOLUME => {
                let t = self.volume_amp();
                set(&mut self.volume, t)
            }
            p::PAN => set(&mut self.pan, v as f32),
            p::TRANSPOSE | p::FINE => {
                let t = self.pitch_target();
                set(&mut self.pitch, t)
            }
            p::CUTOFF => set(&mut self.cutoff, (v as f32).log2()),
            p::RESONANCE => set(&mut self.resonance, v as f32 * 0.01),
            p::ROUND_ROBIN => self.rr.random = util::index(v, 2) == 1,
            p::AMP_ATTACK
            | p::AMP_DECAY
            | p::AMP_SUSTAIN
            | p::AMP_RELEASE
            | p::FILTER_ATTACK
            | p::FILTER_DECAY
            | p::FILTER_SUSTAIN
            | p::FILTER_RELEASE => self.update_rates(),
            _ => {}
        }
    }

    fn max_voices(&self) -> usize {
        (self.value(p::VOICES) as usize).clamp(1, MAX_VOICES)
    }

    fn free_slot(&self) -> usize {
        let limit = self.max_voices();
        let active = self.voices.iter().filter(|v| v.active()).count();
        if active < limit
            && let Some(i) = self.voices.iter().position(|v| !v.active())
        {
            return i;
        }
        // Steal: releasing voices first, then the oldest.
        self.voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.active())
            .min_by_key(|(_, v)| (!v.env.is_releasing(), v.age))
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn note_on(&mut self, note_id: u32, channel: u8, key: u8, velocity: f32) {
        let n = self.set.select(key, velocity, &mut self.rr, &mut self.hits);
        let sr = self.sample_rate as f64;
        let sens = (self.value(p::VELOCITY) * 0.01) as f32;
        let vel_amp = 1.0 - sens + sens * velocity.clamp(0.0, 1.0);
        let glide_ms = self.value(p::GLIDE);
        let glide_from = self
            .last_key
            .filter(|_| glide_ms > 0.0 && self.voices.iter().any(|v| v.active()));
        self.last_key = Some(key);
        for h in 0..n {
            let hit = self.hits[h];
            let zr = &self.set.zones[hit.zone as usize];
            let Some(src) = zr.source else {
                continue;
            };
            let (media, source) = (&self.set.sources[src].0, &self.set.sources[src].1);
            let z = &zr.zone;
            let len = source.frames() as f64;
            let start = (z.start.0 * sr).clamp(0.0, len);
            let end = z.end.map_or(len, |e| (e.0 * sr).clamp(0.0, len));
            if end.is_nan() || end <= start {
                continue;
            }
            let loop_start = (z.loop_start.0 * sr).clamp(start, end);
            let loop_end = (z.loop_end.0 * sr).clamp(loop_start, end);
            let loop_len = loop_end - loop_start;
            let looping = z.looping && loop_len >= 1.0;
            let xfade = (z.loop_crossfade.0 * sr).clamp(0.0, loop_len.min(loop_start));
            let semis = key as f64 - z.root_key as f64 + z.tune_cents as f64 / 100.0;
            let pan = balance(z.pan.0 as f64);
            let amp = vel_amp * util::db_to_amp(z.gain.0) * hit.gain;
            source.prefetch_hint(start as u64);
            let media = *media;
            let slot = self.free_slot();
            self.next_age += 1;
            let glide = glide_from.map_or(0.0, |k| k as f64 - key as f64);
            let glide_frames = (glide_ms * 0.001 * sr).max(1.0);
            let v = &mut self.voices[slot];
            *v = Voice {
                note_id,
                channel,
                key,
                age: self.next_age,
                media,
                source: src,
                pos: start,
                end,
                looping,
                loop_start,
                loop_end,
                xfade,
                semis,
                glide,
                glide_step: glide / glide_frames,
                amp,
                pan,
                ..Voice::IDLE
            };
            v.env.trigger();
            v.fenv.trigger();
        }
    }

    fn for_matching(&mut self, note_id: u32, channel: u8, key: u8, f: impl Fn(&mut Voice)) {
        let by_id = self
            .voices
            .iter()
            .any(|v| v.active() && v.note_id == note_id);
        for v in self.voices.iter_mut().filter(|v| v.active()) {
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
            } => self.for_matching(note_id, channel, key, |v| {
                v.env.release();
                v.fenv.release();
            }),
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => self.for_matching(note_id, channel, key, |v| v.env.kill()),
            EventKind::AllNotesOff => {
                for v in self.voices.iter_mut() {
                    v.env.release();
                    v.fenv.release();
                }
            }
            EventKind::Param { param, value } => self.apply_param(param, value, true),
            // Per-note expression (MPE) is ignored until the v0.3 expression nodes wire it in.
            EventKind::Midi { .. } | EventKind::NoteExpression { .. } => {}
        }
    }

    fn render(&mut self, outputs: &mut [&mut [f32]], start: usize, end: usize) {
        for ch in outputs.iter_mut() {
            ch[start..end].fill(0.0);
        }
        let sr = self.sample_rate as f64;
        let fade = (FADE_MS * 0.001 * sr).max(1.0);
        let mode = FilterMode::from_index(util::index(self.value(p::FILTER_TYPE), 5));
        let key_track = self.value(p::KEY_TRACKING) as f32 * 0.01;
        let env_amt = self.value(p::FILTER_ENV_AMOUNT) as f32 * 0.01;
        let nyq = (self.sample_rate * 0.45).min(20_000.0);
        let mut c0 = start;
        while c0 < end {
            let n = CHUNK.min(end - c0);
            let mut pitch = 0.0;
            let mut cutoff = 0.0;
            let mut res = 0.0;
            for k in 0..n {
                let vol = self.volume.tick();
                let [l, r] = balance(self.pan.tick() as f64);
                self.gains[0][k] = vol * l;
                self.gains[1][k] = vol * r;
                pitch = self.pitch.tick();
                cutoff = self.cutoff.tick();
                res = self.resonance.tick();
            }
            let k_damp = (2.0 - 1.95 * res.clamp(0.0, 1.0)) as f64;
            for vi in 0..MAX_VOICES {
                if !self.voices[vi].active() {
                    continue;
                }
                let set = if self.voices[vi].from_prev {
                    self.prev.as_deref()
                } else {
                    Some(&*self.set)
                };
                let Some((_, source)) = set.and_then(|s| s.sources.get(self.voices[vi].source))
                else {
                    self.voices[vi].env.kill();
                    continue;
                };
                let source: &dyn AudioSource = &**source;
                let src_ch = (source.channels() as usize).min(2);
                if src_ch == 0 {
                    self.voices[vi].env.kill();
                    continue;
                }
                let v = &mut self.voices[vi];
                let semis = v.semis + v.glide + pitch as f64;
                let rate = 2f64.powf(semis / 12.0).min(MAX_RATE);
                v.glide = if v.glide.abs() <= (v.glide_step * n as f64).abs() {
                    0.0
                } else {
                    v.glide - v.glide_step * n as f64
                };
                // Filter coefficients for this chunk (envelope sampled at chunk start).
                let coefs = if mode == FilterMode::Off {
                    None
                } else {
                    let fenv = v.fenv.tick(&self.filter_rates);
                    let oct =
                        cutoff + key_track * (v.key as f32 - 60.0) / 12.0 + env_amt * fenv * 6.0;
                    let fc = 2f32.powf(oct).clamp(20.0, nyq) as f64;
                    Some(svf_coefs(mode, fc, k_damp, sr))
                };
                // Render the voice in segments that never cross a loop boundary.
                let mut k = 0;
                while k < n && v.active() {
                    let held = !v.env.is_releasing() && v.dying == 0;
                    let looping = v.looping && held;
                    let xf_start = v.loop_end - v.xfade;
                    let limit = if looping {
                        if v.pos < xf_start {
                            xf_start
                        } else {
                            v.loop_end
                        }
                    } else {
                        v.end
                    };
                    if v.pos >= limit && !looping {
                        v.env.kill();
                        break;
                    }
                    let left = ((limit - v.pos) / rate).ceil().max(1.0);
                    let m = (left as usize).min(n - k);
                    let in_xf = looping && v.xfade > 0.0 && v.pos >= xf_start;
                    fetch(
                        source,
                        src_ch,
                        v.pos,
                        rate,
                        m,
                        &mut self.scratch[0],
                        &mut self.frames[0],
                    );
                    if in_xf {
                        let len = v.loop_end - v.loop_start;
                        fetch(
                            source,
                            src_ch,
                            v.pos - len,
                            rate,
                            m,
                            &mut self.scratch[1],
                            &mut self.frames[1],
                        );
                    }
                    let end_fade_from = if v.end < source.frames() as f64 {
                        v.end - fade * rate
                    } else {
                        v.end
                    };
                    for j in 0..m {
                        let mut s = [self.frames[0][0][j], self.frames[0][1][j]];
                        if in_xf {
                            // Linear: continuous at both ends of the crossfade.
                            let t = ((v.pos - xf_start) / v.xfade).clamp(0.0, 1.0) as f32;
                            for (c, x) in s.iter_mut().enumerate() {
                                *x = *x * (1.0 - t) + self.frames[1][c][j] * t;
                            }
                        }
                        let mut a = v.env.tick(&self.amp_rates) * v.amp;
                        if !looping && v.pos > end_fade_from {
                            a *= ((v.end - v.pos) / (v.end - end_fade_from)).clamp(0.0, 1.0) as f32;
                        }
                        if v.dying > 0 {
                            a *= v.dying as f32 / fade as f32;
                            v.dying -= 1;
                            if v.dying == 0 {
                                v.env.kill();
                            }
                        }
                        for (c, x) in s.iter_mut().enumerate() {
                            let mut y = *x;
                            if let Some(cf) = &coefs {
                                y = v.filter[0][c].tick(y as f64, cf) as f32;
                                if mode == FilterMode::Lp24 {
                                    y = v.filter[1][c].tick(y as f64, cf) as f32;
                                }
                            }
                            *x = y * a * v.pan[c];
                        }
                        let o = c0 + k + j;
                        if outputs.len() >= 2 {
                            outputs[0][o] += s[0] * self.gains[0][k + j];
                            outputs[1][o] += s[1] * self.gains[1][k + j];
                        } else if let Some(out) = outputs.first_mut() {
                            out[o] += 0.5 * (s[0] + s[1]) * self.gains[0][k + j];
                        }
                        v.pos += rate;
                        if !v.active() {
                            break;
                        }
                    }
                    if looping && v.pos >= v.loop_end {
                        v.pos -= v.loop_end - v.loop_start;
                    }
                    k += m;
                }
            }
            c0 += n;
        }
        for ch in outputs.iter_mut() {
            for x in &mut ch[start..end] {
                *x = crate::dsp::flush32(*x);
            }
        }
    }
}

/// Balance pan gains for `pan` in -1..=1 (centre = unity on both sides).
fn balance(pan: f64) -> [f32; 2] {
    let p = pan.clamp(-1.0, 1.0) as f32;
    [(1.0 - p).min(1.0), (1.0 + p).min(1.0)]
}

/// TPT state-variable filter coefficients (Simper): `y = m0·v0 + m1·v1 + m2·v2`.
#[derive(Clone, Copy, Debug)]
struct Coefs {
    a1: f64,
    a2: f64,
    a3: f64,
    m0: f64,
    m1: f64,
    m2: f64,
}

/// Per-channel SVF state.
#[derive(Clone, Copy, Debug)]
struct Svf {
    ic1: f64,
    ic2: f64,
}

impl Svf {
    const ZERO: Self = Self { ic1: 0.0, ic2: 0.0 };

    #[inline]
    fn tick(&mut self, v0: f64, c: &Coefs) -> f64 {
        let v3 = v0 - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = crate::dsp::flush(2.0 * v1 - self.ic1);
        self.ic2 = crate::dsp::flush(2.0 * v2 - self.ic2);
        c.m0 * v0 + c.m1 * v1 + c.m2 * v2
    }
}

fn svf_coefs(mode: FilterMode, fc: f64, k: f64, sr: f64) -> Coefs {
    let g = (std::f64::consts::PI * fc / sr).tan();
    let (m0, m1, m2) = match mode {
        FilterMode::Lp24 | FilterMode::Lp12 | FilterMode::Off => (0.0, 0.0, 1.0),
        FilterMode::Hp12 => (1.0, -k, -1.0),
        FilterMode::Bp12 => (0.0, k, 0.0),
    };
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    Coefs {
        a1,
        a2,
        a3: g * a2,
        m0,
        m1,
        m2,
    }
}

#[allow(clippy::needless_range_loop)]
/// Read `m` output frames from `pos` at `rate` (4-point Hermite) into `out[ch][..m]`.
fn fetch(
    source: &dyn AudioSource,
    src_ch: usize,
    pos: f64,
    rate: f64,
    m: usize,
    scratch: &mut [Vec<f32>; 2],
    out: &mut [[f32; CHUNK]; 2],
) {
    let first = pos.floor() as i64 - 1;
    let last = (pos + (m.saturating_sub(1)) as f64 * rate).floor() as i64 + 2;
    let len = ((last - first + 1).max(4) as usize).min(SCRATCH);
    for ch in 0..src_ch {
        let buf = &mut scratch[ch][..len];
        if first < 0 {
            let skip = ((-first) as usize).min(len);
            buf[..skip].fill(0.0);
            source.read(ch as u16, 0, &mut buf[skip..]);
            // Edge: repeat the first sample before the start.
            let f0 = if skip < len { buf[skip] } else { 0.0 };
            buf[..skip].fill(f0);
        } else {
            source.read(ch as u16, first as u64, buf);
        }
    }
    for j in 0..m {
        let x = pos + j as f64 * rate;
        let idx = x - first as f64;
        let i = (idx.floor() as usize).clamp(1, len - 3);
        let t = (idx - i as f64) as f32;
        for (ch, o) in out.iter_mut().enumerate() {
            let b = &scratch[ch.min(src_ch - 1)];
            let (y0, y1, y2, y3) = (b[i - 1], b[i], b[i + 1], b[i + 2]);
            let c1 = 0.5 * (y2 - y0);
            let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
            o[j] = ((c3 * t + c2) * t + c1) * t + y1;
        }
    }
}

impl Node for MultiSampler {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        *self.voices = [Voice::IDLE; MAX_VOICES];
        self.rr.reset();
        self.last_key = None;
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
        if !self.voices.iter().any(|v| v.active()) && !(has_notes && !self.set.sources.is_empty()) {
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

    /// Takes a boxed [`ZoneSet`]; sounding voices keep playing (remapped by media id, or
    /// faded out when their media left the set). Returns the previous set (dropped off the
    /// audio thread).
    fn set_data(&mut self, data: NodeData) -> Option<NodeData> {
        match data.downcast::<ZoneSet>() {
            Ok(new) => {
                let fade = (FADE_MS * 0.001 * self.sample_rate as f64).max(1.0) as u32;
                for v in self.voices.iter_mut().filter(|v| v.active()) {
                    if v.from_prev {
                        // Its set is being retired now (two edits within the fade).
                        v.env.kill();
                        continue;
                    }
                    match new.source_of(v.media) {
                        Some(i) => v.source = i,
                        None => {
                            v.from_prev = true;
                            v.dying = fade;
                        }
                    }
                }
                let old = std::mem::replace(&mut self.set, new);
                // Keep the replaced set while its voices fade; hand back the one before.
                self.prev.replace(old).map(|b| b as NodeData)
            }
            Err(data) => Some(data),
        }
    }
}

impl Device for MultiSampler {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
