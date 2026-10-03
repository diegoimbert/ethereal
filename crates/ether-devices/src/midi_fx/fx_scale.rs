//! Scale: transpose, then snap every note into a scale (the track/project scale pushed by
//! the controller, or the device's own Root/Kind with `Scale = Custom`). No timing change:
//! note ids pass through; note-offs use the key their note-on got.

use ether_core::protocol::model::{BuiltinDeviceType, MusicalScale};
use ether_core::{EventBuffer, EventKind};

use super::core::{
    Direction, MAX_NOTES, MidiFx, Out, Params, SCALE_KINDS, Timing, midi_fx_node, quantize, thru,
};
use super::scale_quantize as id;

pub struct ScaleQuantize {
    params: Params,
    out: Out,
    clock: u64,
    flush: bool,
    /// Resolved track/project scale (`Node::set_data`); chromatic until pushed.
    scale: MusicalScale,
    /// (input note id, output key) of sounding notes.
    keys: Vec<(u32, u8)>,
}

impl ScaleQuantize {
    pub fn new() -> Self {
        Self {
            params: Params::new(super::descriptor(BuiltinDeviceType::ScaleQuantize)),
            out: Out::new(),
            clock: 0,
            flush: false,
            scale: MusicalScale::default(),
            keys: Vec::with_capacity(MAX_NOTES),
        }
    }

    /// The scale in effect.
    pub fn scale(&self) -> MusicalScale {
        if self.params.index(id::SOURCE) == 2 {
            MusicalScale {
                root: self.params.index(id::ROOT).min(11) as u8,
                kind: SCALE_KINDS[self.params.index(id::KIND).min(SCALE_KINDS.len() - 1)],
            }
        } else {
            self.scale
        }
    }

    /// Output key of input `key`.
    pub fn map(&self, key: u8) -> u8 {
        let dir = match self.params.index(id::DIRECTION) {
            1 => Direction::Up,
            2 => Direction::Down,
            _ => Direction::Nearest,
        };
        let k = (i32::from(key) + self.params.int(id::TRANSPOSE)).clamp(0, 127);
        quantize(k, self.scale(), dir) as u8
    }

    fn take(&mut self, note_id: u32) -> Option<u8> {
        let i = self.keys.iter().position(|(n, _)| *n == note_id)?;
        Some(self.keys.swap_remove(i).1)
    }
}

impl MidiFx for ScaleQuantize {
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
                let key = self.map(key);
                // A re-used id replaces its old mapping.
                self.take(note_id);
                if self.keys.len() < self.keys.capacity() {
                    self.keys.push((note_id, key));
                }
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
            } => EventKind::NoteOff {
                note_id,
                channel,
                key: self.take(note_id).unwrap_or_else(|| self.map(key)),
                velocity,
            },
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => EventKind::NoteChoke {
                note_id,
                channel,
                key: self.take(note_id).unwrap_or_else(|| self.map(key)),
            },
            other => other,
        };
        thru(out, offset, kind);
    }
}

midi_fx_node!(ScaleQuantize, scale);
