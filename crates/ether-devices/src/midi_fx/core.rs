//! Shared machinery of the MIDI effects: param storage, the block driver (sample-accurate
//! event order, scheduled future events, generated note ids, flushing held notes), musical
//! timing and deterministic randomness. Everything is allocated in `new`; the audio thread
//! only pushes into pre-reserved vectors (never past their capacity).

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{MusicalScale, ParamId, ScaleKind};
use ether_core::{EventBuffer, EventKind, NodeData, ProcessContext, ProcessEvent};

/// Generated note ids: `GENERATED | n` (CONTRACTS.md §12.4.4).
pub(crate) const GENERATED: u32 = 0x8000_0000;
/// Scheduled events (delayed note-ons, note-offs) per instance.
pub(crate) const MAX_SCHEDULED: usize = 1024;
/// Generated notes sounding at once (flushed with note-offs on stop/reset).
pub(crate) const MAX_ACTIVE: usize = 512;
/// Input notes tracked at once (held keys, id mappings).
pub(crate) const MAX_NOTES: usize = 256;

/// Plain param values clamped to the descriptor's ranges.
pub(crate) struct Params {
    desc: DeviceDescriptor,
    values: Vec<f64>,
}

impl Params {
    /// Non-RT.
    pub fn new(desc: DeviceDescriptor) -> Self {
        let values = desc.params.iter().map(|p| p.default).collect();
        Self { desc, values }
    }

    pub fn descriptor(&self) -> &DeviceDescriptor {
        &self.desc
    }

    /// Value of param `id` (dense ids: `id == index`).
    pub fn get(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    /// Enum/stepped value as an index.
    pub fn index(&self, id: ParamId) -> usize {
        let p = &self.desc.params[id.0 as usize];
        (self.get(id) - p.min).round().max(0.0) as usize
    }

    pub fn int(&self, id: ParamId) -> i32 {
        self.get(id).round() as i32
    }

    pub fn on(&self, id: ParamId) -> bool {
        self.get(id) >= 0.5
    }

    pub fn read(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    /// RT. Returns whether `id` exists.
    pub fn set(&mut self, id: ParamId, value: f64) -> bool {
        let i = id.0 as usize;
        let Some(p) = self.desc.params.get(i) else {
            return false;
        };
        let (lo, hi) = (p.min.min(p.max), p.max.max(p.min));
        self.values[i] = if value.is_finite() {
            let v = value.clamp(lo, hi);
            match p.step {
                Some(step) if step > 0.0 => (p.min + ((v - p.min) / step).round() * step).clamp(lo, hi),
                _ => v,
            }
        } else {
            p.default
        };
        true
    }
}

#[derive(Clone, Copy, Debug)]
struct Scheduled {
    at: u64,
    kind: EventKind,
}

#[derive(Clone, Copy, Debug)]
struct Active {
    id: u32,
    channel: u8,
    key: u8,
}

/// Output side: scheduled events, generated ids and the generated notes still sounding.
pub(crate) struct Out {
    queue: Vec<Scheduled>,
    active: Vec<Active>,
    next_id: u32,
}

pub(crate) fn note_id(kind: &EventKind) -> Option<u32> {
    match *kind {
        EventKind::NoteOn { note_id, .. }
        | EventKind::NoteOff { note_id, .. }
        | EventKind::NoteChoke { note_id, .. } => Some(note_id),
        _ => None,
    }
}

impl Out {
    /// Non-RT.
    pub fn new() -> Self {
        Self {
            queue: Vec::with_capacity(MAX_SCHEDULED),
            active: Vec::with_capacity(MAX_ACTIVE),
            next_id: 0,
        }
    }

    /// A fresh generated note id.
    pub fn new_id(&mut self) -> u32 {
        let id = GENERATED | (self.next_id & !GENERATED);
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Push `kind` at `offset`, tracking generated notes (so they can be flushed).
    pub fn emit(&mut self, out: &mut EventBuffer, offset: u32, kind: EventKind) {
        match kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                ..
            } if note_id & GENERATED != 0 => {
                if self.active.len() < self.active.capacity() {
                    self.active.push(Active {
                        id: note_id,
                        channel,
                        key,
                    });
                }
            }
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. }
                if note_id & GENERATED != 0 =>
            {
                if let Some(i) = self.active.iter().position(|a| a.id == note_id) {
                    self.active.swap_remove(i);
                }
            }
            _ => {}
        }
        out.push(ProcessEvent { offset, kind });
    }

    /// Queue `kind` for absolute sample time `at` (after events already queued at `at`).
    /// Returns `false` when the queue is full.
    pub fn schedule(&mut self, at: u64, kind: EventKind) -> bool {
        if self.queue.len() == self.queue.capacity() {
            return false;
        }
        let i = self.queue.partition_point(|s| s.at <= at);
        self.queue.insert(i, Scheduled { at, kind });
        true
    }

    /// Emit `kind` at absolute time `at` (now when `at <= now`, else scheduled). A note-off
    /// that doesn't fit in the queue is sent now rather than lost.
    pub fn at(&mut self, out: &mut EventBuffer, t: &Timing, now: u32, at: u64, kind: EventKind) {
        if at <= t.abs(now) {
            self.emit(out, now, kind);
        } else if !self.schedule(at, kind) && !matches!(kind, EventKind::NoteOn { .. }) {
            self.emit(out, now, kind);
        }
    }

    /// Emit the queued events due before absolute time `until`.
    pub fn emit_due(&mut self, out: &mut EventBuffer, t: &Timing, until: u64) {
        let n = self.queue.partition_point(|s| s.at < until);
        for i in 0..n {
            let s = self.queue[i];
            let offset = s.at.saturating_sub(t.base).min(u64::from(t.frames.max(1) - 1));
            self.emit(out, offset as u32, s.kind);
        }
        self.queue.drain(..n);
    }

    /// Drop queued events of note `id` (e.g. a choke); returns whether its note-on was
    /// still queued.
    pub fn cancel(&mut self, id: u32) -> bool {
        let pending_on = self
            .queue
            .iter()
            .any(|s| matches!(s.kind, EventKind::NoteOn { note_id, .. } if note_id == id));
        self.queue.retain(|s| note_id(&s.kind) != Some(id));
        pending_on
    }

    /// Release every generated note now and drop everything queued.
    pub fn release_all(&mut self, out: &mut EventBuffer, offset: u32) {
        self.queue.clear();
        while let Some(a) = self.active.pop() {
            out.push(ProcessEvent {
                offset,
                kind: EventKind::NoteOff {
                    note_id: a.id,
                    channel: a.channel,
                    key: a.key,
                    velocity: 0.0,
                },
            });
        }
    }

    pub fn is_idle(&self) -> bool {
        self.queue.is_empty() && self.active.is_empty()
    }
}

/// Timing of the current block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    /// Device sample clock at the block start.
    pub base: u64,
    pub frames: u32,
    pub sr: f64,
    /// Transport running (song beats advance).
    pub playing: bool,
    /// Song position at the block start.
    pub pos0: f64,
    /// Beats per sample (from the tempo even when stopped).
    pub bps: f64,
}

impl Timing {
    pub fn new(ctx: &ProcessContext<'_>, base: u64) -> Self {
        let t = ctx.transport;
        let sr = f64::from(ctx.sample_rate.max(1.0));
        let playing = t.playing && t.beats_per_sample > 0.0;
        let bpm = if t.bpm > 0.0 { t.bpm } else { 120.0 };
        Self {
            base,
            frames: ctx.frames as u32,
            sr,
            playing,
            pos0: t.position,
            bps: if playing {
                t.beats_per_sample
            } else {
                bpm / 60.0 / sr
            },
        }
    }

    pub fn abs(&self, offset: u32) -> u64 {
        self.base + u64::from(offset)
    }

    pub fn ms(&self, ms: f64) -> u64 {
        (ms.max(0.0) * self.sr / 1000.0).round() as u64
    }

    pub fn beats(&self, beats: f64) -> u64 {
        (beats.max(0.0) / self.bps).round() as u64
    }

    /// A stable musical/temporal position of `offset` for seeding randomness: song ticks
    /// (1/960 beat) while playing, else the sample clock.
    pub fn stamp(&self, offset: u32) -> u64 {
        if self.playing {
            ((self.pos0 + f64::from(offset) * self.bps) * 960.0).round() as i64 as u64
        } else {
            self.abs(offset)
        }
    }
}

/// A MIDI effect's per-event logic, driven by [`run`].
pub(crate) trait MidiFx {
    fn params(&mut self) -> &mut Params;
    fn out(&mut self) -> &mut Out;
    /// Device clock (samples since creation).
    fn clock(&mut self) -> &mut u64;
    /// Set by `reset`: flush held notes at the start of the next block.
    fn flush(&mut self) -> &mut bool;
    /// A param changed (at its offset).
    fn param_changed(&mut self, _id: ParamId) {}
    /// Clear per-note state (after a flush or `AllNotesOff`).
    fn clear_notes(&mut self);
    /// A note/MIDI event (never `Param` or `AllNotesOff`) at `offset`.
    fn event(&mut self, out: &mut EventBuffer, t: &Timing, offset: u32, kind: EventKind);
    /// Generate what happens in `[from, to)` (arpeggiator steps). Default: nothing.
    fn advance(&mut self, _out: &mut EventBuffer, _t: &Timing, _from: u32, _to: u32) {}
    /// End of block (clock bookkeeping).
    fn end_block(&mut self, _t: &Timing) {}
}

/// RT. Process one block: events in order, scheduled events and generated steps in between,
/// params applied at their offsets (never forwarded), `AllNotesOff` releases every generated
/// note and is forwarded. Output is sorted by offset.
pub(crate) fn run<D: MidiFx>(d: &mut D, ctx: &mut ProcessContext<'_>) {
    let base = *d.clock();
    let t = Timing::new(ctx, base);
    let out = &mut *ctx.out_events;
    if ctx.frames == 0 {
        return;
    }
    if std::mem::take(d.flush()) {
        d.out().release_all(out, 0);
        d.clear_notes();
    }
    let last = t.frames - 1;
    let mut cursor = 0u32;
    for e in ctx.events {
        let o = e.offset.min(last);
        if o > cursor {
            span(d, out, &t, cursor, o);
            cursor = o;
        }
        match e.kind {
            EventKind::Param { param, value } => {
                if d.params().set(param, value) {
                    d.param_changed(param);
                }
            }
            EventKind::AllNotesOff => {
                d.out().emit_due(out, &t, t.abs(o) + 1);
                d.out().release_all(out, o);
                d.clear_notes();
                out.push(ProcessEvent {
                    offset: o,
                    kind: EventKind::AllNotesOff,
                });
            }
            kind => {
                // Due events at this sample go first (a note-off before a new note-on).
                d.out().emit_due(out, &t, t.abs(o) + 1);
                d.event(out, &t, o, kind);
            }
        }
    }
    span(d, out, &t, cursor, t.frames);
    d.out().emit_due(out, &t, t.abs(t.frames));
    d.end_block(&t);
    *d.clock() = base + u64::from(t.frames);
    out.sort();
}

fn span<D: MidiFx>(d: &mut D, out: &mut EventBuffer, t: &Timing, from: u32, to: u32) {
    d.out().emit_due(out, t, t.abs(to));
    d.advance(out, t, from, to);
    d.out().emit_due(out, t, t.abs(to));
}

/// Take a pushed `MusicalScale` (`Node::set_data`); the box goes back to be dropped off the
/// audio thread.
pub(crate) fn take_scale(data: NodeData, scale: &mut MusicalScale) -> Option<NodeData> {
    match data.downcast::<MusicalScale>() {
        Ok(s) => {
            *scale = *s;
            Some(s)
        }
        Err(other) => Some(other),
    }
}

/// Scale kinds in `Kind` param order (= `ScaleKind` declaration order).
pub(crate) const SCALE_KINDS: [ScaleKind; 14] = [
    ScaleKind::Chromatic,
    ScaleKind::Major,
    ScaleKind::Minor,
    ScaleKind::HarmonicMinor,
    ScaleKind::MelodicMinor,
    ScaleKind::MajorPentatonic,
    ScaleKind::MinorPentatonic,
    ScaleKind::Blues,
    ScaleKind::Dorian,
    ScaleKind::Phrygian,
    ScaleKind::Lydian,
    ScaleKind::Mixolydian,
    ScaleKind::Locrian,
    ScaleKind::WholeTone,
];

/// Pitch classes above the root (bit `i` = `i` semitones), as `ui/src/domain/scales.ts`.
pub(crate) fn scale_mask(kind: ScaleKind) -> u16 {
    let steps: &[u8] = match kind {
        ScaleKind::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        ScaleKind::Major => &[0, 2, 4, 5, 7, 9, 11],
        ScaleKind::Minor => &[0, 2, 3, 5, 7, 8, 10],
        ScaleKind::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
        ScaleKind::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
        ScaleKind::MajorPentatonic => &[0, 2, 4, 7, 9],
        ScaleKind::MinorPentatonic => &[0, 3, 5, 7, 10],
        ScaleKind::Blues => &[0, 3, 5, 6, 7, 10],
        ScaleKind::Dorian => &[0, 2, 3, 5, 7, 9, 10],
        ScaleKind::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
        ScaleKind::Lydian => &[0, 2, 4, 6, 7, 9, 11],
        ScaleKind::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
        ScaleKind::Locrian => &[0, 1, 3, 5, 6, 8, 10],
        ScaleKind::WholeTone => &[0, 2, 4, 6, 8, 10],
    };
    steps.iter().fold(0, |m, s| m | (1 << s))
}

/// Direction of [`quantize`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Closest scale note; ties go down.
    Nearest,
    Up,
    Down,
}

/// Snap `key` into `scale` (MIDI range kept: out-of-range candidates are skipped).
pub(crate) fn quantize(key: i32, scale: MusicalScale, dir: Direction) -> i32 {
    let mask = scale_mask(scale.kind);
    let root = i32::from(scale.root % 12);
    let inside = |k: i32| (0..=127).contains(&k) && mask & (1 << (k - root).rem_euclid(12)) != 0;
    if inside(key) {
        return key;
    }
    for d in 1..12 {
        let (down, up) = (key - d, key + d);
        match dir {
            Direction::Nearest | Direction::Down if inside(down) => return down,
            Direction::Nearest | Direction::Up if inside(up) => return up,
            _ => {}
        }
    }
    // Only when the other direction is out of MIDI range: the closest either way.
    for d in 1..12 {
        if inside(key - d) {
            return key - d;
        }
        if inside(key + d) {
            return key + d;
        }
    }
    key.clamp(0, 127)
}

/// Deterministic 32-bit hash (splitmix64 finalizer) of a few words.
pub(crate) fn hash(a: u64, b: u64, c: u64) -> u64 {
    let mut z = a
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(b.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(c.wrapping_mul(0x94D0_49BB_1331_11EB))
        .wrapping_add(0x2545_F491_4F6C_DD1D);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Uniform `0..1` from a hash (`lane` picks independent draws).
pub(crate) fn unit(h: u64, lane: u32) -> f64 {
    let x = hash(h, u64::from(lane), 0x5EED);
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// MIDI velocity (1..=127 scale) to the event's `0..1`.
pub(crate) fn vel01(v: f64) -> f32 {
    (v.clamp(1.0, 127.0) / 127.0) as f32
}

/// `Node` + `Device` for a [`MidiFx`] with fields `params` and `flush` (and `scale` when
/// it takes the pushed `MusicalScale`).
macro_rules! midi_fx_node {
    ($ty:ty $(, $scale:ident)?) => {
        impl ether_core::Node for $ty {
            fn prepare(&mut self, _config: &ether_core::PrepareConfig) {}
            fn reset(&mut self) {
                self.flush = true;
            }
            fn process(
                &mut self,
                ctx: &mut ether_core::ProcessContext<'_>,
                _audio: &mut ether_core::AudioBuffers<'_, '_>,
            ) -> ether_core::ProcessStatus {
                $crate::midi_fx::core::run(self, ctx);
                ether_core::ProcessStatus::Silent
            }
            fn channels(&self) -> (u16, u16) {
                (0, 0)
            }
            $(
                fn set_data(&mut self, data: ether_core::NodeData) -> Option<ether_core::NodeData> {
                    $crate::midi_fx::core::take_scale(data, &mut self.$scale)
                }
            )?
        }

        impl ether_core::Device for $ty {
            fn descriptor(&self) -> ether_core::protocol::devices::DeviceDescriptor {
                self.params.descriptor().clone()
            }
            fn param(&self, id: ether_core::protocol::model::ParamId) -> Option<f64> {
                self.params.read(id)
            }
            fn set_param(&mut self, id: ether_core::protocol::model::ParamId, value: f64) {
                if self.params.set(id, value) {
                    $crate::midi_fx::core::MidiFx::param_changed(self, id);
                }
            }
        }
    };
}
pub(crate) use midi_fx_node;
