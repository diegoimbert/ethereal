//! Note Length: every note gets a fixed length (milliseconds or a tempo-synced value, times
//! `Gate`), from its note-on (`Trigger = Note On`: the input note-off is ignored) or from
//! its note-off (`Trigger = Note Off`: the note starts when the key is released, with the
//! note-on's velocity). Output notes use generated ids.

use ether_core::protocol::model::BuiltinDeviceType;
use ether_core::{EventBuffer, EventKind, ProcessEvent};

use super::core::{MAX_NOTES, MidiFx, Out, Params, Timing, midi_fx_node};
use super::note_length as id;
use crate::contract::SYNC_RATE_BEATS;

#[derive(Clone, Copy, Debug)]
struct Held {
    input: u32,
    /// Generated id (`Note On` trigger) or 0 while waiting for the note-off.
    id: u32,
    channel: u8,
    key: u8,
    velocity: f32,
}

pub(crate) struct NoteLength {
    params: Params,
    out: Out,
    clock: u64,
    flush: bool,
    held: Vec<Held>,
}

impl NoteLength {
    pub fn new() -> Self {
        Self {
            params: Params::new(super::descriptor(BuiltinDeviceType::NoteLength)),
            out: Out::new(),
            clock: 0,
            flush: false,
            held: Vec::with_capacity(MAX_NOTES),
        }
    }

    /// Output note length in samples (at least one).
    pub fn length(&self, t: &Timing) -> u64 {
        let p = &self.params;
        let gate = p.get(id::GATE) / 100.0;
        let len = if p.index(id::MODE) == 1 {
            let i = p.index(id::SYNC_LENGTH).min(SYNC_RATE_BEATS.len() - 1);
            t.beats(SYNC_RATE_BEATS[i] * gate)
        } else {
            t.ms(p.get(id::LENGTH) * gate)
        };
        len.max(1)
    }

    fn start(&mut self, out: &mut EventBuffer, t: &Timing, offset: u32, h: Held) -> u32 {
        let id = self.out.new_id();
        self.out.emit(
            out,
            offset,
            EventKind::NoteOn {
                note_id: id,
                channel: h.channel,
                key: h.key,
                velocity: h.velocity,
            },
        );
        let at = t.abs(offset) + self.length(t);
        self.out.at(
            out,
            t,
            offset,
            at,
            EventKind::NoteOff {
                note_id: id,
                channel: h.channel,
                key: h.key,
                velocity: 0.0,
            },
        );
        id
    }

    fn take(&mut self, input: u32) -> Option<Held> {
        let i = self.held.iter().position(|h| h.input == input)?;
        Some(self.held.swap_remove(i))
    }
}

impl MidiFx for NoteLength {
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
        self.held.clear();
    }

    fn event(&mut self, out: &mut EventBuffer, t: &Timing, offset: u32, kind: EventKind) {
        match kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => {
                self.take(note_id);
                let mut h = Held {
                    input: note_id,
                    id: 0,
                    channel,
                    key,
                    velocity,
                };
                if self.params.index(id::TRIGGER) == 0 {
                    h.id = self.start(out, t, offset, h);
                }
                if self.held.len() < self.held.capacity() {
                    self.held.push(h);
                }
            }
            EventKind::NoteOff { note_id, .. } => match self.take(note_id) {
                // Waiting for its release: start now.
                Some(h) if h.id == 0 => {
                    self.start(out, t, offset, h);
                }
                // Already running on its own length.
                Some(_) => {}
                None => out.push(ProcessEvent { offset, kind }),
            },
            EventKind::NoteChoke { note_id, .. } => match self.take(note_id) {
                Some(h) if h.id != 0 => {
                    self.out.cancel(h.id);
                    self.out.emit(
                        out,
                        offset,
                        EventKind::NoteChoke {
                            note_id: h.id,
                            channel: h.channel,
                            key: h.key,
                        },
                    );
                }
                Some(_) => {}
                None => out.push(ProcessEvent { offset, kind }),
            },
            other => out.push(ProcessEvent {
                offset,
                kind: other,
            }),
        }
    }
}

midi_fx_node!(NoteLength);
