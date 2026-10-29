//! The multisampler node: zone voices with loops, global amp/filter envelopes and a
//! per-voice filter.
//!
//! - **Voices**: up to [`MAX_VOICES`] (the `Voices` param limits polyphony). A note-on starts
//!   one voice per selected zone ([`ZoneSet::select`]); stealing takes the oldest voice,
//!   releasing ones first.
//! - **Pitch**: `key - root_key + tune_cents/100 + Transpose + Fine` semitones (+ glide from
//!   the previous note), read band-limited ([`crate::dsp::resample`]: a windowed sinc whose
//!   cutoff follows the rate, so a zone pitched up doesn't alias; rate capped at 4
//!   octaves up).
//! - **Legato** (`Legato` on): a note-on while a note is held moves that note's voices to
//!   the new key instead of starting new ones ([`MultiSampler::legato`]): a voice playing
//!   the sample the new key selects is retuned, the others fade out while the new key's other
//!   samples fade in at the same point of the note ([`LEGATO_FADE_MS`]), with the lead's
//!   envelopes. A note-off of the latest held key goes back to the one held before it.
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
use crate::dsp::resample::{Kernel, MAX_HALF, Taps, Weights};
use crate::util::{self, Adsr, AdsrRates};

/// Maximum simultaneous voices (the `Voices` param's maximum).
pub const MAX_VOICES: usize = 64;
/// Maximum playback-rate ratio (4 octaves up).
const MAX_RATE: f64 = 16.0;
/// Render chunk in frames (filter coefficients and pitch update per chunk).
const CHUNK: usize = 32;
/// Source frames read per chunk: `CHUNK · MAX_RATE` plus the kernel's span on both sides.
const SCRATCH: usize = CHUNK * MAX_RATE as usize + 2 * (MAX_HALF + 1) + 2;
/// Fade for a zone end inside the sample and for voices dropped by a zone edit, in ms.
const FADE_MS: f64 = 2.0;
/// Crossfade between the samples of two keys a legato slide changes, in ms.
pub const LEGATO_FADE_MS: f64 = 20.0;
/// Held keys remembered for legato (beyond that, the oldest is forgotten).
const MAX_HELD: usize = 16;

/// A held note (legato's last-note priority).
#[derive(Clone, Copy, Debug, Default)]
struct Held {
    note_id: u32,
    channel: u8,
    key: u8,
    velocity: f32,
}

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
    /// Where playback started (the zone's start plus `Offset`), source frames: `pos - start`
    /// is how far into the note it is (legato starts another key's sample there).
    start: f64,
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
    /// Forced fade-out (stolen, dropped by a zone edit, left by a legato slide): remaining
    /// frames, 0 = none, out of `fade_len`.
    dying: u32,
    fade_len: u32,
    /// Fade-in of a legato slide's new sample: remaining frames, 0 = none.
    rise: u32,
    filter: [[Svf; 2]; 2],
    /// The filter's `g` at the end of the last chunk (negative: none yet); each chunk ramps
    /// from it per sample (per-chunk coefficients stepped a cutoff jump audibly).
    g: f64,
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
        start: 0.0,
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
        fade_len: 1,
        rise: 0,
        filter: [[Svf::ZERO; 2]; 2],
        g: -1.0,
        from_prev: false,
    };

    fn active(&self) -> bool {
        self.env.is_active()
    }

    /// Held and not fading out: a voice a legato slide moves.
    fn leads(&self, note_id: u32) -> bool {
        self.active() && !self.env.is_releasing() && self.dying == 0 && self.note_id == note_id
    }

    /// Fade out over `frames` (then free).
    fn fade_out(&mut self, frames: u32) {
        self.dying = frames.max(1);
        self.fade_len = self.dying;
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
    /// Transpose + Fine + the bend, semitones.
    pitch: Smoother,
    /// The last pitch bend (-1..=1).
    bend: f32,
    /// log2(cutoff Hz).
    cutoff: Smoother,
    resonance: Smoother,
    /// Per source channel read scratch (main and loop-crossfade tail).
    scratch: [[Vec<f32>; 2]; 2],
    /// Interpolated voice output per channel (main, tail).
    frames: [[[f32; CHUNK]; 2]; 2],
    gains: [[f32; CHUNK]; 2],
    hits: [Hit; MAX_LAYERS],
    /// Held notes, oldest first (legato).
    held: [Held; MAX_HELD],
    held_len: usize,
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
        // Build the shared resampling kernel here, never first on the audio thread.
        let _ = Kernel::get();
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
            bend: 0.0,
            cutoff: Smoother::new(20000f32.log2(), 20.0, sr),
            resonance: Smoother::new(0.0, 20.0, sr),
            scratch: [
                [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
                [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
            ],
            frames: [[[0.0; CHUNK]; 2]; 2],
            gains: [[0.0; CHUNK]; 2],
            hits: [Hit::default(); MAX_LAYERS],
            held: [Held::default(); MAX_HELD],
            held_len: 0,
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

    /// Transpose + fine + the pitch bend, in semitones (the pitch smoother glides to it).
    fn pitch_target(&self) -> f32 {
        (self.value(p::TRANSPOSE).round() + self.value(p::FINE)) as f32
            + self.bend * self.value(p::BEND_RANGE) as f32
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
            p::TRANSPOSE | p::FINE | p::BEND_RANGE => {
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

    /// The slot for a new voice. `Voices` limits the sounding voices; a stolen one fades out
    /// in a free slot rather than being cut (a click).
    fn free_slot(&mut self) -> usize {
        let limit = self.max_voices();
        let sounding = self
            .voices
            .iter()
            .filter(|v| v.active() && v.dying == 0)
            .count();
        let free = self.voices.iter().position(|v| !v.active());
        if sounding < limit {
            if let Some(i) = free {
                return i;
            }
            // Every slot busy, some fading out: take the one closest to silence.
            return self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.dying > 0)
                .min_by_key(|(_, v)| v.dying)
                .map_or(0, |(i, _)| i);
        }
        // Steal a sounding voice: releasing ones first, then the oldest.
        let slot = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.active() && v.dying == 0)
            .min_by_key(|(_, v)| (!v.env.is_releasing(), v.age))
            .map_or(0, |(i, _)| i);
        if let Some(spare) = free {
            let fade = (FADE_MS * 0.001 * self.sample_rate as f64).max(1.0) as u32;
            self.voices[spare] = self.voices[slot];
            self.voices[spare].fade_out(fade);
        }
        slot
    }

    /// The voice `hit` starts for `key` (its envelopes untriggered), or `None` for a zone
    /// without a sample or with nothing to play.
    fn zone_voice(
        &self,
        hit: Hit,
        note_id: u32,
        channel: u8,
        key: u8,
        vel_amp: f32,
    ) -> Option<Voice> {
        let sr = self.sample_rate as f64;
        let zr = &self.set.zones[hit.zone as usize];
        let src = zr.source?;
        let (media, source) = (&self.set.sources[src].0, &self.set.sources[src].1);
        let z = &zr.zone;
        let len = source.frames() as f64;
        // The zone's start, plus the global `Offset`.
        let start = ((z.start.0 + self.value(p::OFFSET) * 0.001) * sr).clamp(0.0, len);
        let end = z.end.map_or(len, |e| (e.0 * sr).clamp(0.0, len));
        if end.is_nan() || end <= start {
            return None;
        }
        let loop_start = (z.loop_start.0 * sr).clamp(start, end);
        let loop_end = (z.loop_end.0 * sr).clamp(loop_start, end);
        let loop_len = loop_end - loop_start;
        let looping = z.looping && loop_len >= 1.0;
        Some(Voice {
            note_id,
            channel,
            key,
            media: *media,
            source: src,
            pos: start,
            start,
            end,
            looping,
            loop_start,
            loop_end,
            xfade: (z.loop_crossfade.0 * sr).clamp(0.0, loop_len.min(loop_start)),
            semis: key as f64 - z.root_key as f64 + z.tune_cents as f64 / 100.0,
            amp: vel_amp * util::db_to_amp(z.gain.0) * hit.gain,
            pan: balance(z.pan.0 as f64),
            ..Voice::IDLE
        })
    }

    fn vel_amp(&self, velocity: f32) -> f32 {
        let sens = (self.value(p::VELOCITY) * 0.01) as f32;
        1.0 - sens + sens * velocity.clamp(0.0, 1.0)
    }

    /// Glide offset (semitones) and its per-frame step from pitch `from` (a key, fractional
    /// mid-glide) to `key`; none with `Glide` at 0.
    fn glide(&self, from: f64, key: u8) -> (f64, f64) {
        let glide_ms = self.value(p::GLIDE);
        if glide_ms <= 0.0 {
            return (0.0, 0.0);
        }
        let glide = from - key as f64;
        (
            glide,
            glide / (glide_ms * 0.001 * self.sample_rate as f64).max(1.0),
        )
    }

    fn legato_on(&self) -> bool {
        self.value(p::LEGATO) >= 0.5
    }

    /// The latest held note.
    fn lead(&self) -> Option<Held> {
        self.held[..self.held_len].last().copied()
    }

    fn hold(&mut self, h: Held) {
        self.unhold(h.note_id, h.channel, h.key);
        if self.held_len == MAX_HELD {
            self.held.copy_within(1.., 0);
            self.held_len -= 1;
        }
        self.held[self.held_len] = h;
        self.held_len += 1;
    }

    /// Forget a held note (by id, or by key and channel when either has no id).
    fn unhold(&mut self, note_id: u32, channel: u8, key: u8) {
        let mut n = 0;
        for i in 0..self.held_len {
            let h = self.held[i];
            let same = if note_id != 0 && h.note_id != 0 {
                h.note_id == note_id
            } else {
                h.key == key && h.channel == channel
            };
            if !same {
                self.held[n] = h;
                n += 1;
            }
        }
        self.held_len = n;
    }

    fn note_on(&mut self, note_id: u32, channel: u8, key: u8, velocity: f32) {
        let lead = self.lead();
        self.hold(Held {
            note_id,
            channel,
            key,
            velocity,
        });
        if self.legato_on()
            && let Some(lead) = lead
            && self.legato(lead.note_id, note_id, channel, key, velocity)
        {
            return;
        }
        let n = self.set.select(key, velocity, &mut self.rr, &mut self.hits);
        let vel_amp = self.vel_amp(velocity);
        let glide_from = self
            .last_key
            .filter(|_| self.voices.iter().any(|v| v.active()));
        self.last_key = Some(key);
        for h in 0..n {
            let Some(mut voice) = self.zone_voice(self.hits[h], note_id, channel, key, vel_amp)
            else {
                continue;
            };
            if let Some((_, source)) = self.set.sources.get(voice.source) {
                source.prefetch_hint(voice.pos as u64);
            }
            (voice.glide, voice.glide_step) =
                glide_from.map_or((0.0, 0.0), |k| self.glide(k as f64, key));
            let slot = self.free_slot();
            self.next_age += 1;
            voice.age = self.next_age;
            let v = &mut self.voices[slot];
            *v = voice;
            v.env.trigger();
            v.fenv.trigger();
        }
    }

    /// Legato: move the held voices of note `from` to `key` (as note `note_id`) without a new
    /// attack. A voice already playing a sample the new key selects is retuned; the others
    /// fade out while the new key's other samples fade in at the same point of the note,
    /// with the lead voice's envelopes and filter state. Returns `false` (nothing moved) when
    /// `from` has no held voice.
    fn legato(&mut self, from: u32, note_id: u32, channel: u8, key: u8, velocity: f32) -> bool {
        let Some(lead) = self.voices.iter().position(|v| v.leads(from)) else {
            return false;
        };
        let r = self.voices[lead];
        let elapsed = r.pos - r.start;
        let (glide, glide_step) = self.glide(r.key as f64 + r.glide, key);
        self.last_key = Some(key);
        let n = self.set.select(key, velocity, &mut self.rr, &mut self.hits);
        let hits = self.hits;
        let vel_amp = self.vel_amp(velocity);
        // 1. Retune the voices whose sample the new key plays too.
        let mut moved = [false; MAX_VOICES];
        let mut retuned = [false; MAX_LAYERS];
        for (h, hit) in hits[..n].iter().enumerate() {
            let Some(zr) = self.set.zones.get(hit.zone as usize) else {
                continue;
            };
            let Some(media) = zr.source.map(|s| self.set.sources[s].0) else {
                continue;
            };
            let semis = key as f64 - zr.zone.root_key as f64 + zr.zone.tune_cents as f64 / 100.0;
            let found = (0..MAX_VOICES).find(|&i| {
                !moved[i] && self.voices[i].leads(from) && self.voices[i].media == media
            });
            if let Some(i) = found {
                let v = &mut self.voices[i];
                (v.note_id, v.channel, v.key, v.semis) = (note_id, channel, key, semis);
                (v.glide, v.glide_step) = (glide, glide_step);
                moved[i] = true;
                retuned[h] = true;
            }
        }
        // 2. The others fade out (before new voices take slots, so none is stolen).
        let xfade = (LEGATO_FADE_MS * 0.001 * self.sample_rate as f64).max(1.0) as u32;
        for (i, v) in self.voices.iter_mut().enumerate() {
            if !moved[i] && v.leads(from) {
                v.fade_out(xfade);
            }
        }
        // 3. The new key's other samples fade in where the note is.
        for (&hit, _) in hits[..n].iter().zip(retuned).filter(|(_, r)| !r) {
            let Some(mut voice) = self.zone_voice(hit, note_id, channel, key, vel_amp) else {
                continue;
            };
            voice.pos = voice.start + elapsed;
            if voice.looping && voice.pos >= voice.loop_end {
                voice.pos = voice.loop_start
                    + (voice.pos - voice.loop_start) % (voice.loop_end - voice.loop_start);
            }
            if voice.pos >= voice.end {
                continue;
            }
            if let Some((_, source)) = self.set.sources.get(voice.source) {
                source.prefetch_hint(voice.pos as u64);
            }
            (voice.glide, voice.glide_step) = (glide, glide_step);
            (voice.env, voice.fenv, voice.filter, voice.g) = (r.env, r.fenv, r.filter, r.g);
            voice.rise = xfade;
            let slot = self.free_slot();
            self.next_age += 1;
            voice.age = self.next_age;
            self.voices[slot] = voice;
        }
        true
    }

    fn note_off(&mut self, note_id: u32, channel: u8, key: u8) {
        let lead = self.lead();
        self.unhold(note_id, channel, key);
        // Legato: letting go of the latest held key goes back to the one held before it.
        if self.legato_on()
            && let (Some(lead), Some(back)) = (lead, self.lead())
            && lead.note_id == note_id
            && lead.key == key
            && self.legato(note_id, back.note_id, back.channel, back.key, back.velocity)
        {
            return;
        }
        self.for_matching(note_id, channel, key, |v| {
            v.env.release();
            v.fenv.release();
        });
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
            } => self.note_off(note_id, channel, key),
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => {
                self.unhold(note_id, channel, key);
                self.for_matching(note_id, channel, key, |v| v.env.kill())
            }
            EventKind::AllNotesOff => {
                self.held_len = 0;
                for v in self.voices.iter_mut() {
                    v.env.release();
                    v.fenv.release();
                }
            }
            EventKind::Param { param, value, step } => self.apply_param(param, value, !step),
            // Per-note expression (MPE) is ignored until the v0.3 expression nodes wire it in.
            EventKind::Midi { data } => {
                if let Some(b) = crate::synth::bend_of(data) {
                    self.bend = b;
                    let t = self.pitch_target();
                    // Nothing sounding: the next note starts at the bend (no slide into it).
                    if self.voices.iter().any(|v| v.active()) {
                        self.pitch.set_target(t);
                    } else {
                        self.pitch.set_immediate(t);
                    }
                }
            }
            EventKind::NoteExpression { .. } => {}
        }
    }

    fn render(&mut self, outputs: &mut [&mut [f32]], start: usize, end: usize) {
        for ch in outputs.iter_mut() {
            ch[start..end].fill(0.0);
        }
        let sr = self.sample_rate as f64;
        let fade = (FADE_MS * 0.001 * sr).max(1.0);
        let legato_fade = (LEGATO_FADE_MS * 0.001 * sr).max(1.0);
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
                // Filter coefficients for this chunk (envelope sampled at chunk start). The
                // 24 dB low-pass resonates in its first stage only: two resonant stages
                // squared the peak (+24 dB at full resonance).
                let (mut coefs, mut coefs2) = (None, None);
                // `g` ramps per sample from `g0` by `g_step` (0: constant).
                let (mut g, mut g_step) = (0.0, 0.0);
                if mode == FilterMode::Off {
                    v.g = -1.0;
                } else {
                    let fenv = v.fenv.tick(&self.filter_rates);
                    let oct =
                        cutoff + key_track * (v.key as f32 - 60.0) / 12.0 + env_amt * fenv * 6.0;
                    let fc = 2f32.powf(oct).clamp(20.0, nyq) as f64;
                    let g_end = svf_g(fc, sr);
                    let g0 = if v.g < 0.0 { g_end } else { v.g };
                    v.g = g_end;
                    g = g0;
                    g_step = (g_end - g0) / n as f64;
                    let second = (mode == FilterMode::Lp24).then(|| svf_coefs(mode, g_end, 2.0));
                    (coefs, coefs2) = (Some(svf_coefs(mode, g_end, k_damp)), second);
                }
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
                        if v.rise > 0 {
                            a *= 1.0 - v.rise as f32 / legato_fade as f32;
                            v.rise -= 1;
                        }
                        if v.dying > 0 {
                            a *= v.dying as f32 / v.fade_len as f32;
                            v.dying -= 1;
                            if v.dying == 0 {
                                v.env.kill();
                            }
                        }
                        if g_step != 0.0 {
                            g += g_step;
                            coefs = Some(svf_coefs(mode, g, k_damp));
                            if coefs2.is_some() {
                                coefs2 = Some(svf_coefs(mode, g, 2.0));
                            }
                        }
                        for (c, x) in s.iter_mut().enumerate() {
                            let mut y = *x;
                            if let Some(cf) = &coefs {
                                y = v.filter[0][c].tick(y as f64, cf) as f32;
                                if let Some(cf2) = &coefs2 {
                                    y = v.filter[1][c].tick(y as f64, cf2) as f32;
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

/// Prewarped `g` of a cutoff `fc` (Hz).
fn svf_g(fc: f64, sr: f64) -> f64 {
    (std::f64::consts::PI * fc / sr).tan()
}

fn svf_coefs(mode: FilterMode, g: f64, k: f64) -> Coefs {
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

/// Read `m` output frames from `pos` at `rate` (band-limited) into `out[ch][..m]`. Frames
/// before the sample's start are 0.
fn fetch(
    source: &dyn AudioSource,
    src_ch: usize,
    pos: f64,
    rate: f64,
    m: usize,
    scratch: &mut [Vec<f32>; 2],
    out: &mut [[f32; CHUNK]; 2],
) {
    let taps = Taps::new(rate);
    let margin = taps.margin() as i64;
    let first = pos.floor() as i64 - margin;
    let last = (pos + (m.saturating_sub(1)) as f64 * rate).floor() as i64 + margin;
    let len = ((last - first + 1).max(1) as usize).min(SCRATCH);
    for (ch, buf) in scratch[..src_ch].iter_mut().enumerate() {
        let buf = &mut buf[..len];
        let skip = ((-first).max(0) as usize).min(len);
        buf[..skip].fill(0.0);
        source.read(ch as u16, first.max(0) as u64, &mut buf[skip..]);
    }
    let kernel = Kernel::get();
    let mut w = Weights::new();
    for j in 0..m {
        kernel.weights(pos + j as f64 * rate - first as f64, &taps, &mut w);
        for (ch, o) in out.iter_mut().enumerate() {
            o[j] = w.apply(&scratch[ch.min(src_ch - 1)][..len]);
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
        self.held_len = 0;
        self.bend = 0.0;
        let t = self.pitch_target();
        self.pitch.set_immediate(t);
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
                            v.fade_out(fade);
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
