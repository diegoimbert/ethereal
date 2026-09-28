//! Arpeggiator: plays the held notes one step at a time on a tempo-synced grid.
//!
//! - Clock: steps sit on the song grid (`Rate`, odd steps late by `Swing`: 100 % = a
//!   triplet shuffle) while the transport runs; stopped, a free-running beat clock at the
//!   current tempo starts on the first key so the first step plays at once.
//! - Pattern: the held keys (sorted, or in played order for `As Played`) over `Octaves`
//!   octaves, walked per `Style`; `Chord` plays them all each step; `Random` draws
//!   deterministically.
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
/// Pattern length at most (up-down doubles the note set).
const MAX_PATTERN: usize = 2 * MAX_HELD * MAX_OCTAVES;

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
}

const STYLES: [Style; 9] = [
    Style::Up,
    Style::Down,
    Style::UpDown,
    Style::DownUp,
    Style::Converge,
    Style::Diverge,
    Style::AsPlayed,
    Style::Random,
    Style::Chord,
];

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
    /// Scratch: the current pattern (key, velocity, channel) and a sort buffer.
    pattern: Vec<(u8, f32, u8)>,
    sorted: Vec<Held>,
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
            sorted: Vec::with_capacity(MAX_HELD),
        }
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
        let fixed = self.params.index(id::VELOCITY_MODE) == 1;
        let fixed_vel =
            (self.params.get(id::FIXED_VELOCITY) / 127.0).clamp(1.0 / 127.0, 1.0) as f32;
        self.pattern.clear();
        for o in 0..octaves {
            for h in &self.sorted {
                let k = i32::from(h.key) + 12 * o;
                if k > 127 {
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
        match style {
            Style::Down => self.pattern.reverse(),
            Style::UpDown | Style::DownUp => {
                if style == Style::DownUp {
                    self.pattern.reverse();
                }
                // ... then back without repeating the ends.
                for i in (1..n - 1).rev() {
                    let e = self.pattern[i];
                    self.pattern.push(e);
                }
            }
            Style::Converge | Style::Diverge => {
                // Outside in: low, high, low + 1, high - 1, ... (in place via the tail).
                for i in 0..n {
                    let j = if i % 2 == 0 { i / 2 } else { n - 1 - i / 2 };
                    let e = self.pattern[j];
                    self.pattern.push(e);
                }
                self.pattern.drain(..n);
                if style == Style::Diverge {
                    self.pattern.reverse();
                }
            }
            _ => {}
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
