//! Arpeggiator: plays the held notes one step at a time on a tempo-synced grid.
//!
//! - Clock: steps sit on the song grid (`Rate`, odd steps late by `Swing`: 100 % = a
//!   triplet shuffle) while the transport runs; stopped, a free-running beat clock at the
//!   current tempo starts on the first key so the first step plays at once.
//! - Pattern: the held keys (sorted, or in played order for `As Played`) over `Octaves`
//!   repeats, each `Distance` semitones above the previous (12 = octaves), walked per
//!   `Style`; `Chord` plays them all each step; `Random` draws deterministically.
//! - The appended styles (Live's): `Up & Down` / `Down & Up` turn round repeating the end
//!   notes (`Up-Down` doesn't); `Con & Diverge` converges, then diverges back; `Pinky Up`
//!   alternates each of the other notes, upwards, with the top one (`Pinky Up-Down`: the
//!   others up then down); `Thumb Up` / `Thumb Up-Down` the same with the bottom note
//!   played before each; `Random Other` plays every note once in a random order before
//!   drawing a new order; `Random Once` keeps one random order until the held notes change.
//! - `Gate` is the step's note length (up to 200 %: legato overlaps); `Hold` latches the last
//!   chord until a new one is played; `Retrigger` restarts the pattern on each new note or
//!   on each beat.
//! - Input notes are consumed; steps use generated ids and every one gets its note-off
//!   (release, bypass, removal, stop and locate included).

use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{EventBuffer, EventKind};

use super::arpeggiator as id;
use super::core::{MidiFx, Out, Params, Timing, hash, midi_fx_node, thru, unit};
use crate::contract::SYNC_RATE_BEATS;

/// Held keys (and latched ones).
const MAX_HELD: usize = 32;
/// Octaves at most.
const MAX_OCTAVES: usize = 4;
/// Pattern length at most (`Pinky Up-Down` / `Thumb Up-Down` pair every note of an up-down
/// walk with the outer note: four times the note set).
const MAX_PATTERN: usize = 4 * MAX_HELD * MAX_OCTAVES;

#[derive(Clone, Copy, Debug)]
struct Held {
    input: u32,
    channel: u8,
    key: u8,
    velocity: f32,
    /// Press order (`As Played`).
    order: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Up,
    Down,
    UpDown,
    DownUp,
    Converge,
    Diverge,
    AsPlayed,
    Random,
    Chord,
    // Appended (Live's styles).
    UpAndDown,
    DownAndUp,
    ConAndDiverge,
    PinkyUp,
    PinkyUpDown,
    ThumbUp,
    ThumbUpDown,
    RandomOther,
    RandomOnce,
}

/// By `Style` index (stable, append-only).
const STYLES: [Style; 18] = [
    Style::Up,
    Style::Down,
    Style::UpDown,
    Style::DownUp,
    Style::Converge,
    Style::Diverge,
    Style::AsPlayed,
    Style::Random,
    Style::Chord,
    Style::UpAndDown,
    Style::DownAndUp,
    Style::ConAndDiverge,
    Style::PinkyUp,
    Style::PinkyUpDown,
    Style::ThumbUp,
    Style::ThumbUpDown,
    Style::RandomOther,
    Style::RandomOnce,
];

/// A pattern step: key, velocity, channel.
type Step = (u8, f32, u8);

/// `src` up, then back down: without its end notes again (`Up-Down`) or with them
/// (`Up & Down`).
fn turn(src: &[Step], out: &mut Vec<Step>, repeat_ends: bool) {
    out.extend_from_slice(src);
    let n = src.len();
    if repeat_ends {
        out.extend(src.iter().rev());
    } else if n > 2 {
        out.extend(src[1..n - 1].iter().rev());
    }
}

/// Outside in: low, high, low + 1, high - 1, ...
fn converge(src: &[Step], out: &mut Vec<Step>) {
    let n = src.len();
    out.extend((0..n).map(|i| src[if i % 2 == 0 { i / 2 } else { n - 1 - i / 2 }]));
}

/// Each of `notes` paired with `pivot`, the pivot after it (pinky) or before it (thumb).
fn alternate(notes: &[Step], pivot: Step, pivot_first: bool, out: &mut Vec<Step>) {
    for &e in notes {
        if pivot_first {
            out.extend([pivot, e]);
        } else {
            out.extend([e, pivot]);
        }
    }
}

pub struct Arpeggiator {
    params: Params,
    out: Out,
    clock: u64,
    flush: bool,
    /// Notes the arpeggio plays (latched ones included with `Hold`).
    notes: Vec<Held>,
    /// Input ids of the keys physically down.
    down: Vec<u32>,
    presses: u64,
    /// Next step of the pattern.
    step: u64,
    /// Steps played (random draws).
    played: u64,
    /// Free-running clock (transport stopped): beat at the block start.
    free_beat: f64,
    /// Device-clock sample of the last step (a step on a block boundary never plays twice).
    last_at: Option<u64>,
    /// Scratch: the current pattern, a shaping buffer and a sort buffer.
    pattern: Vec<Step>,
    shaped: Vec<Step>,
    sorted: Vec<Held>,
    /// `Random Other` / `Random Once`: the order (pattern indices) and what it was drawn for
    /// (seed, pattern length).
    order: Vec<usize>,
    order_of: Option<(u64, usize)>,
}

impl Arpeggiator {
    pub fn new() -> Self {
        Self {
            params: Params::new(super::descriptor(BuiltinDeviceType::Arpeggiator)),
            out: Out::new(),
            clock: 0,
            flush: false,
            notes: Vec::with_capacity(MAX_HELD),
            down: Vec::with_capacity(MAX_HELD),
            presses: 0,
            step: 0,
            played: 0,
            free_beat: 0.0,
            last_at: None,
            pattern: Vec::with_capacity(MAX_PATTERN),
            shaped: Vec::with_capacity(MAX_PATTERN),
            sorted: Vec::with_capacity(MAX_HELD),
            order: Vec::with_capacity(MAX_PATTERN),
            order_of: None,
        }
    }

    /// The held notes (as pressed: a new press is a new set), independent of their order.
    fn signature(&self) -> u64 {
        self.notes.iter().fold(0u64, |acc, h| {
            acc.wrapping_add(hash(u64::from(h.input), u64::from(h.key), 0x51))
        })
    }

    /// Index `i` of a random order of `0..n` drawn from `seed` (kept while both stay).
    fn ordered(&mut self, seed: u64, n: usize, i: usize) -> usize {
        if self.order_of != Some((seed, n)) {
            self.order.clear();
            self.order.extend(0..n);
            // Fisher-Yates.
            for k in (1..n).rev() {
                let j = (unit(hash(seed, k as u64, 0x5F), 0) * (k + 1) as f64) as usize;
                self.order.swap(k, j.min(k));
            }
            self.order_of = Some((seed, n));
        }
        self.order[i % n]
    }

    fn style(&self) -> Style {
        STYLES[self.params.index(id::STYLE).min(STYLES.len() - 1)]
    }

    fn rate(&self) -> f64 {
        SYNC_RATE_BEATS[self.params.index(id::RATE).min(SYNC_RATE_BEATS.len() - 1)]
    }

    /// Rebuild `pattern` from the held notes.
    fn build(&mut self) {
        let style = self.style();
        self.sorted.clear();
        self.sorted.extend_from_slice(&self.notes);
        if style == Style::AsPlayed {
            self.sorted.sort_unstable_by_key(|h| h.order);
        } else {
            self.sorted.sort_unstable_by_key(|h| (h.key, h.order));
        }
        let octaves = self.params.int(id::OCTAVES).clamp(1, MAX_OCTAVES as i32);
        let distance = self.params.int(id::DISTANCE).clamp(-24, 24);
        let fixed = self.params.index(id::VELOCITY_MODE) == 1;
        let fixed_vel =
            (self.params.get(id::FIXED_VELOCITY) / 127.0).clamp(1.0 / 127.0, 1.0) as f32;
        self.pattern.clear();
        for o in 0..octaves {
            for h in &self.sorted {
                let k = i32::from(h.key) + distance * o;
                if !(0..=127).contains(&k) {
                    continue;
                }
                let v = if fixed { fixed_vel } else { h.velocity };
                self.pattern.push((k as u8, v, h.channel));
            }
        }
        let n = self.pattern.len();
        if n < 2 {
            return;
        }
        // Shapes `pattern` (the notes upwards) into `shaped`, then swaps them.
        let (p, s) = (&mut self.pattern, &mut self.shaped);
        s.clear();
        match style {
            Style::Down => p.reverse(),
            Style::UpDown | Style::UpAndDown => turn(p, s, style == Style::UpAndDown),
            Style::DownUp | Style::DownAndUp => {
                p.reverse();
                turn(p, s, style == Style::DownAndUp);
            }
            Style::Converge => converge(p, s),
            Style::Diverge => {
                converge(p, s);
                s.reverse();
            }
            Style::ConAndDiverge => {
                converge(p, s);
                p.clear();
                turn(s, p, false);
                s.clear();
            }
            Style::PinkyUp => alternate(&p[..n - 1], p[n - 1], false, s),
            Style::ThumbUp => alternate(&p[1..], p[0], true, s),
            Style::PinkyUpDown | Style::ThumbUpDown => {
                let pinky = style == Style::PinkyUpDown;
                let (pivot, others) = if pinky {
                    (p[n - 1], &p[..n - 1])
                } else {
                    (p[0], &p[1..])
                };
                turn(others, s, false);
                p.clear();
                alternate(s, pivot, !pinky, p);
                s.clear();
            }
            Style::Up
            | Style::AsPlayed
            | Style::Random
            | Style::Chord
            | Style::RandomOther
            | Style::RandomOnce => {}
        }
        if !s.is_empty() {
            std::mem::swap(p, s);
        }
    }

    fn play_step(&mut self, out: &mut EventBuffer, t: &Timing, offset: u32, beat: f64) {
        let retrigger = self.params.index(id::RETRIGGER);
        if retrigger == 2 && (beat - beat.round()).abs() < 1e-6 {
            self.step = 0;
        }
        self.build();
        let n = self.pattern.len();
        if n == 0 {
            return;
        }
        let gate = self.params.get(id::GATE) / 100.0;
        let len = t.beats(self.rate() * gate).max(1);
        let off_at = t.abs(offset) + len;
        let style = self.style();
        let (from, count) = match style {
            Style::Chord => (0, n),
            Style::Random => {
                let h = hash(self.played, 0xA4B, n as u64);
                ((unit(h, 0) * n as f64) as usize % n, 1)
            }
            // A new order per pass over the notes (Other) or per set of held notes (Once).
            Style::RandomOther | Style::RandomOnce => {
                let mut seed = self.signature();
                if style == Style::RandomOther {
                    seed = hash(seed, self.step / n as u64, 0x07E);
                }
                let i = (self.step % n as u64) as usize;
                (self.ordered(seed, n, i), 1)
            }
            _ => ((self.step % n as u64) as usize, 1),
        };
        self.step = self.step.wrapping_add(1);
        self.played = self.played.wrapping_add(1);
        for i in from..from + count {
            let (key, velocity, channel) = self.pattern[i];
            let note_id = self.out.new_id();
            self.out.emit(
                out,
                offset,
                EventKind::NoteOn {
                    note_id,
                    channel,
                    key,
                    velocity,
                },
            );
            self.out.at(
                out,
                t,
                offset,
                off_at,
                EventKind::NoteOff {
                    note_id,
                    channel,
                    key,
                    velocity: 0.0,
                },
            );
        }
    }

    /// Beat position and beats per sample of offset 0 of this block.
    fn clock_of(&self, t: &Timing) -> f64 {
        if t.playing { t.pos0 } else { self.free_beat }
    }

    fn press(&mut self, t: &Timing, offset: u32, h: Held) {
        let fresh = self.notes.is_empty() || (self.params.on(id::HOLD) && self.down.is_empty());
        if fresh {
            self.notes.clear();
            self.step = 0;
            self.last_at = None;
            if !t.playing {
                // Restart the free clock so the first step plays now.
                self.free_beat = -f64::from(offset) * t.bps;
            }
        }
        if self.params.index(id::RETRIGGER) == 1 {
            self.step = 0;
        }
        if let Some(i) = self.notes.iter().position(|n| n.input == h.input) {
            self.notes.swap_remove(i);
        }
        if self.notes.len() < self.notes.capacity() {
            self.notes.push(h);
        }
        if !self.down.contains(&h.input) && self.down.len() < self.down.capacity() {
            self.down.push(h.input);
        }
    }

    fn release(&mut self, input: u32) -> bool {
        let Some(i) = self.down.iter().position(|d| *d == input) else {
            return self.notes.iter().any(|n| n.input == input);
        };
        self.down.swap_remove(i);
        if !self.params.on(id::HOLD) {
            self.notes.retain(|n| n.input != input);
        }
        true
    }
}

impl MidiFx for Arpeggiator {
    fn params(&mut self) -> &mut Params {
        &mut self.params
    }
    fn out(&mut self) -> &mut Out {
        &mut self.out
    }
    fn clock(&mut self) -> &mut u64 {
        &mut self.clock
    }
    fn flush(&mut self) -> &mut bool {
        &mut self.flush
    }
    fn clear_notes(&mut self) {
        self.notes.clear();
        self.down.clear();
        self.step = 0;
    }

    fn param_changed(&mut self, param: ParamId) {
        // Hold off: drop the latched notes that aren't held any more.
        if param == id::HOLD && !self.params.on(id::HOLD) {
            let down = &self.down;
            self.notes.retain(|n| down.contains(&n.input));
        }
    }

    fn event(&mut self, out: &mut EventBuffer, t: &Timing, offset: u32, kind: EventKind) {
        match kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => {
                self.presses += 1;
                let h = Held {
                    input: note_id,
                    channel,
                    key,
                    velocity,
                    order: self.presses,
                };
                self.press(t, offset, h);
            }
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                if !self.release(note_id) {
                    // Not ours (held before the arpeggiator was inserted).
                    thru(out, offset, kind);
                }
            }
            other => thru(out, offset, other),
        }
    }

    fn advance(&mut self, out: &mut EventBuffer, t: &Timing, from: u32, to: u32) {
        if self.notes.is_empty() || to <= from {
            return;
        }
        let b0 = self.clock_of(t);
        let (ba, bb) = (b0 + f64::from(from) * t.bps, b0 + f64::from(to) * t.bps);
        let rate = self.rate();
        let swing = self.params.get(id::SWING) / 100.0 * rate / 3.0;
        const EPS: f64 = 1e-9;
        let mut k = (ba / rate).floor() as i64 - 1;
        loop {
            let tk = k as f64 * rate + if k.rem_euclid(2) == 1 { swing } else { 0.0 };
            // Swing < one step, so step times increase with `k`.
            if tk >= bb - EPS {
                break;
            }
            if tk >= ba - EPS {
                // First sample at or after the step (a hair of slack for rounding).
                let o = from + ((tk - ba) / t.bps - 1e-6).ceil().max(0.0) as u32;
                let o = o.min(to - 1);
                let at = t.abs(o);
                // Rounding at a block boundary can see the same step in both blocks.
                if self.last_at.is_none_or(|last| at > last + 1) {
                    self.last_at = Some(at);
                    self.play_step(out, t, o, tk);
                }
            }
            k += 1;
        }
    }

    fn end_block(&mut self, t: &Timing) {
        if !t.playing {
            self.free_beat += f64::from(t.frames) * t.bps;
        }
    }
}

midi_fx_node!(Arpeggiator);
