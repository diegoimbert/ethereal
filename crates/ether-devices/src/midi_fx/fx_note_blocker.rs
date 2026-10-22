//! Note Blocker: drops every note-on (and the per-note expressions and note-offs of the notes
//! it dropped), so nothing after it plays; with `Block Everything` on, every other MIDI
//! message (CC, pitch bend, aftertouch, program change) goes too. `AllNotesOff` always passes.
//!
//! No stuck notes: a note-off of a note it never saw (held before the device was added or
//! switched on) passes, and the engine sends `AllNotesOff` past it when it is added
//! (`mixer::release_on_midi_fx_change`) and on every on/off switch, static or automated
//! (`ChainRt::switch_notes`), on track chains, rack chains and drum pads. Turning it off never
//! retriggers notes held upstream: they play again from their next note-on.

use ether_core::protocol::model::BuiltinDeviceType;
use ether_core::{EventBuffer, EventKind};

use super::core::{MAX_NOTES, MidiFx, Out, Params, Timing, midi_fx_node, thru};
use super::note_blocker as id;

pub struct NoteBlocker {
    params: Params,
    out: Out,
    clock: u64,
    flush: bool,
    /// Ids of the notes dropped and not ended yet (their note-offs are dropped too); the
    /// oldest goes when full (its note-off then passes, which releases nothing).
    dropped: Vec<u32>,
}

impl NoteBlocker {
    pub fn new() -> Self {
        Self {
            params: Params::new(super::descriptor(BuiltinDeviceType::NoteBlocker)),
            out: Out::new(),
            clock: 0,
            flush: false,
            dropped: Vec::with_capacity(MAX_NOTES),
        }
    }

    fn drop_note(&mut self, note_id: u32) {
        if self.dropped.contains(&note_id) {
            return;
        }
        if self.dropped.len() == self.dropped.capacity() {
            self.dropped.remove(0);
        }
        self.dropped.push(note_id);
    }

    /// Whether `note_id` was dropped (forgetting it).
    fn ended(&mut self, note_id: u32) -> bool {
        match self.dropped.iter().position(|d| *d == note_id) {
            Some(i) => {
                self.dropped.remove(i);
                true
            }
            None => false,
        }
    }
}

/// Whether a raw MIDI message is a note-off (`0x8n`, or `0x9n` at velocity 0).
fn raw_note_off(data: [u8; 3]) -> bool {
    match data[0] & 0xF0 {
        0x80 => true,
        0x90 => data[2] == 0,
        _ => false,
    }
}

impl MidiFx for NoteBlocker {
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
    /// Kept across `AllNotesOff` and resets: the note-offs of dropped notes still come.
    fn clear_notes(&mut self) {}

    fn event(&mut self, out: &mut EventBuffer, _t: &Timing, offset: u32, kind: EventKind) {
        match kind {
            EventKind::NoteOn { note_id, .. } => self.drop_note(note_id),
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                if !self.ended(note_id) {
                    thru(out, offset, kind);
                }
            }
            // Per-note expressions only shape notes, none of which play.
            EventKind::NoteExpression { .. } => {}
            EventKind::Midi { data } => {
                let note_on = data[0] & 0xF0 == 0x90 && data[2] > 0;
                let pass = if self.params.on(id::BLOCK_EVERYTHING) {
                    raw_note_off(data)
                } else {
                    !note_on
                };
                if pass {
                    thru(out, offset, kind);
                }
            }
            other => thru(out, offset, other),
        }
    }
}

midi_fx_node!(NoteBlocker);
