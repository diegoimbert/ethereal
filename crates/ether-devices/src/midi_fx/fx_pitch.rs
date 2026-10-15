//! Pitch: transpose every note by `Pitch` semitones, then keep it inside the allowed range
//! (`Lowest` ..= `Lowest + Range`, and always 0 ..= 127): `Block` drops a note that lands
//! outside (and its note-off / expressions), `Fold` moves it by octaves into the range,
//! `Limit` clamps it to the range's edge. No timing change: note ids pass through; note-offs,
//! chokes and per-note expressions use the key their note-on got (a param change while a note
//! sounds never strands it).

use ether_core::protocol::model::BuiltinDeviceType;
use ether_core::{EventBuffer, EventKind};

use super::core::{MAX_NOTES, MidiFx, Out, Params, Timing, midi_fx_node, thru};
use super::pitch as id;

/// `Mode` values.
const BLOCK: usize = 0;
const FOLD: usize = 1;

pub struct Pitch {
    params: Params,
    out: Out,
    clock: u64,
    flush: bool,
    /// (input note id, output key; `None` = blocked) of sounding notes.
    keys: Vec<(u32, Option<u8>)>,
}

impl Pitch {
    pub fn new() -> Self {
        Self {
            params: Params::new(super::descriptor(BuiltinDeviceType::Pitch)),
            out: Out::new(),
            clock: 0,
            flush: false,
            keys: Vec::with_capacity(MAX_NOTES),
        }
    }

    /// Output key of input `key`; `None` = blocked (out of range in `Block` mode).
    pub fn map(&self, key: u8) -> Option<u8> {
        let p = &self.params;
        let k = i32::from(key) + p.int(id::PITCH);
        let lo = p.int(id::LOWEST).clamp(0, 127);
        let hi = (lo + p.int(id::RANGE).max(0)).min(127);
        if (lo..=hi).contains(&k) {
            return Some(k as u8);
        }
        let k = match p.index(id::MODE) {
            BLOCK => return None,
            FOLD => {
                // Whole octaves towards the range; a range narrower than an octave the note
                // can't land in clamps it to the nearer edge.
                let folded = if k > hi {
                    k - 12 * (k - hi + 11).div_euclid(12)
                } else {
                    k + 12 * (lo - k + 11).div_euclid(12)
                };
                if (lo..=hi).contains(&folded) {
                    folded
                } else {
                    k.clamp(lo, hi)
                }
            }
            _ => k.clamp(lo, hi),
        };
        Some(k as u8)
    }

    /// The key a sounding note got (removing it), or `key` mapped now for an unknown id.
    fn take(&mut self, note_id: u32, key: u8) -> Option<u8> {
        match self.keys.iter().position(|(n, _)| *n == note_id) {
            Some(i) => self.keys.swap_remove(i).1,
            None => self.map(key),
        }
    }

    fn sounding(&self, note_id: u32, key: u8) -> Option<u8> {
        match self.keys.iter().find(|(n, _)| *n == note_id) {
            Some((_, k)) => *k,
            None => self.map(key),
        }
    }
}

impl MidiFx for Pitch {
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
        self.keys.clear();
    }

    fn event(&mut self, out: &mut EventBuffer, _t: &Timing, offset: u32, kind: EventKind) {
        let kind = match kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => {
                let mapped = self.map(key);
                // A re-used id replaces its old mapping.
                if let Some(i) = self.keys.iter().position(|(n, _)| *n == note_id) {
                    self.keys.swap_remove(i);
                }
                if self.keys.len() < self.keys.capacity() {
                    self.keys.push((note_id, mapped));
                }
                let Some(key) = mapped else { return };
                EventKind::NoteOn {
                    note_id,
                    channel,
                    key,
                    velocity,
                }
            }
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                velocity,
            } => {
                let Some(key) = self.take(note_id, key) else {
                    return;
                };
                EventKind::NoteOff {
                    note_id,
                    channel,
                    key,
                    velocity,
                }
            }
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => {
                let Some(key) = self.take(note_id, key) else {
                    return;
                };
                EventKind::NoteChoke {
                    note_id,
                    channel,
                    key,
                }
            }
            EventKind::NoteExpression {
                note_id,
                channel,
                key,
                expression,
                value,
            } => {
                let Some(key) = self.sounding(note_id, key) else {
                    return;
                };
                EventKind::NoteExpression {
                    note_id,
                    channel,
                    key,
                    expression,
                    value,
                }
            }
            other => other,
        };
        thru(out, offset, kind);
    }
}

midi_fx_node!(Pitch);
