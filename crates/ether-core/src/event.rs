//! Sample-accurate events delivered to nodes.

use ether_protocol::model::ParamId;

/// An event at a sample offset within the current (sub-)block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessEvent {
    /// Offset in samples from the start of the block, `< frames`.
    pub offset: u32,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventKind {
    /// `note_id` is unique among currently sounding notes of this node (CLAP note ids,
    /// voice stealing/choke). `velocity` 0..=1.
    NoteOn {
        note_id: u32,
        channel: u8,
        key: u8,
        velocity: f32,
    },
    NoteOff {
        note_id: u32,
        channel: u8,
        key: u8,
        velocity: f32,
    },
    /// Immediately silence a voice (no release).
    NoteChoke { note_id: u32, channel: u8, key: u8 },
    /// Release every note (transport stop / loop jump / clip stop).
    AllNotesOff,
    /// Set parameter to a *plain* value at this offset (automation, UI, modulation).
    /// Nodes smooth continuous params themselves (see [`crate::param::Smoother`]).
    Param { param: ParamId, value: f64 },
    /// Raw short MIDI message (CC, pitch bend, aftertouch) for instruments/plugins.
    Midi { data: [u8; 3] },
    /// v0.3 (`midi-expression` / `mpe`, CONTRACTS.md §13.2): per-note expression of a
    /// sounding note (CLAP note expression model). `value` is in the kind's range
    /// (`ether_model::NoteExpressionKind::range`: pitch in semitones, pressure/timbre 0..=1).
    /// Nodes that don't support it ignore it; MIDI effects forward it with the note.
    NoteExpression {
        note_id: u32,
        channel: u8,
        key: u8,
        expression: ether_protocol::model::NoteExpressionKind,
        value: f32,
    },
}

/// Fixed-capacity event list. Allocated once (non-RT), then only cleared and pushed on the
/// audio thread. Events must be pushed in non-decreasing `offset` order, or sorted with
/// [`EventBuffer::sort`] (stable, in-place, no allocation).
#[derive(Debug)]
pub struct EventBuffer {
    events: Vec<ProcessEvent>,
    overflowed: bool,
}

impl EventBuffer {
    /// Non-RT: allocates.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
            overflowed: false,
        }
    }

    /// RT-safe. Returns `false` (and records the overflow) when full.
    pub fn push(&mut self, event: ProcessEvent) -> bool {
        if self.events.len() == self.events.capacity() {
            self.overflowed = true;
            return false;
        }
        self.events.push(event);
        true
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.overflowed = false;
    }

    pub fn as_slice(&self) -> &[ProcessEvent] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// RT-safe, in place, no allocation: stable insertion sort by offset (event lists are
    /// small and nearly sorted), then **note-offs before note-ons at the same offset**: two
    /// notes that touch (one ends on the sample the next starts) never overlap, so
    /// mono/legato instruments retrigger instead of sliding and a same-pitch repeat is not
    /// swallowed, as in every other DAW.
    ///
    /// At one offset, an ending event (note-off, choke, all-notes-off) moves ahead of the
    /// note-ons and note expressions of *other* notes, never ahead of its own note's on or
    /// expressions (an on and its off on one sample stay a short note, not a stuck one),
    /// nor ahead of params or raw MIDI (CC, pedal), and ending events keep their order
    /// among themselves. Lists without such a tie are left exactly as the offset sort
    /// leaves them.
    pub fn sort(&mut self) {
        let ev = &mut self.events;
        for i in 1..ev.len() {
            let mut j = i;
            while j > 0 && ev[j - 1].offset > ev[j].offset {
                ev.swap(j - 1, j);
                j -= 1;
            }
        }
        for i in 1..ev.len() {
            let Some(ends) = ending(&ev[i].kind) else {
                continue;
            };
            let offset = ev[i].offset;
            // Leftmost slot reachable: past other notes' ons/expressions and other offs.
            let mut k = i;
            while k > 0 && ev[k - 1].offset == offset {
                let passable = match starting(&ev[k - 1].kind) {
                    Some(note) => ends != Some(note),
                    None => ending(&ev[k - 1].kind).is_some(),
                };
                if !passable {
                    break;
                }
                k -= 1;
            }
            // ... but after the offs already there (stable among offs).
            while k < i && ending(&ev[k].kind).is_some() {
                k += 1;
            }
            if k < i {
                ev[k..=i].rotate_right(1);
            }
        }
    }
}

/// The note `(note_id, channel, key)` an event starts or shapes (note-on, note expression).
#[inline]
fn starting(kind: &EventKind) -> Option<(u32, u8, u8)> {
    match *kind {
        EventKind::NoteOn {
            note_id,
            channel,
            key,
            ..
        }
        | EventKind::NoteExpression {
            note_id,
            channel,
            key,
            ..
        } => Some((note_id, channel, key)),
        _ => None,
    }
}

/// `Some(note)` for an event ending a note (note-off, choke), `Some(None)` for all-notes-off,
/// `None` otherwise.
#[inline]
fn ending(kind: &EventKind) -> Option<Option<(u32, u8, u8)>> {
    match *kind {
        EventKind::NoteOff {
            note_id,
            channel,
            key,
            ..
        }
        | EventKind::NoteChoke {
            note_id,
            channel,
            key,
        } => Some(Some((note_id, channel, key))),
        EventKind::AllNotesOff => Some(None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_protocol::model::NoteExpressionKind;

    fn on(offset: u32, note_id: u32, key: u8) -> ProcessEvent {
        ProcessEvent {
            offset,
            kind: EventKind::NoteOn {
                note_id,
                channel: 0,
                key,
                velocity: 1.0,
            },
        }
    }

    fn off(offset: u32, note_id: u32, key: u8) -> ProcessEvent {
        ProcessEvent {
            offset,
            kind: EventKind::NoteOff {
                note_id,
                channel: 0,
                key,
                velocity: 0.0,
            },
        }
    }

    fn param(offset: u32) -> ProcessEvent {
        ProcessEvent {
            offset,
            kind: EventKind::Param {
                param: ParamId(1),
                value: 0.5,
            },
        }
    }

    fn sorted(events: &[ProcessEvent]) -> Vec<ProcessEvent> {
        let mut b = EventBuffer::with_capacity(events.len());
        for e in events {
            b.push(*e);
        }
        b.sort();
        b.as_slice().to_vec()
    }

    #[test]
    fn note_offs_before_note_ons_at_the_same_offset() {
        // Different and same pitch: the ending note's off goes first.
        assert_eq!(
            sorted(&[on(5, 2, 64), off(5, 1, 60)]),
            [off(5, 1, 60), on(5, 2, 64)]
        );
        assert_eq!(
            sorted(&[on(5, 2, 60), off(5, 1, 60)]),
            [off(5, 1, 60), on(5, 2, 60)]
        );
        // Several ons/offs, offsets out of order.
        assert_eq!(
            sorted(&[
                on(9, 4, 1),
                on(5, 2, 64),
                on(5, 3, 67),
                off(5, 1, 60),
                off(0, 0, 50)
            ]),
            [
                off(0, 0, 50),
                off(5, 1, 60),
                on(5, 2, 64),
                on(5, 3, 67),
                on(9, 4, 1)
            ]
        );
        // All-notes-off and chokes too.
        let all = ProcessEvent {
            offset: 0,
            kind: EventKind::AllNotesOff,
        };
        assert_eq!(sorted(&[on(0, 2, 64), all]), [all, on(0, 2, 64)]);
        let choke = ProcessEvent {
            offset: 3,
            kind: EventKind::NoteChoke {
                note_id: 1,
                channel: 0,
                key: 60,
            },
        };
        assert_eq!(sorted(&[on(3, 2, 64), choke]), [choke, on(3, 2, 64)]);
    }

    #[test]
    fn a_notes_own_off_stays_after_its_on() {
        // A note starting and ending on one sample stays on → off (no stuck note).
        assert_eq!(
            sorted(&[on(5, 1, 60), off(5, 1, 60)]),
            [on(5, 1, 60), off(5, 1, 60)]
        );
        // ... while other notes' offs still move ahead of the note-ons.
        assert_eq!(
            sorted(&[on(5, 1, 60), on(5, 2, 64), off(5, 1, 60), off(5, 0, 50)]),
            [off(5, 0, 50), on(5, 1, 60), off(5, 1, 60), on(5, 2, 64)]
        );
    }

    #[test]
    fn note_expressions_stay_with_their_note() {
        let expr = |note_id, key| ProcessEvent {
            offset: 5,
            kind: EventKind::NoteExpression {
                note_id,
                channel: 0,
                key,
                expression: NoteExpressionKind::Pressure,
                value: 0.5,
            },
        };
        // The new note's expression follows its note-on; the ending note's off goes first.
        assert_eq!(
            sorted(&[on(5, 2, 64), expr(2, 64), off(5, 1, 60)]),
            [off(5, 1, 60), on(5, 2, 64), expr(2, 64)]
        );
        // The ending note's own last expression stays before its off.
        assert_eq!(
            sorted(&[expr(1, 60), on(5, 2, 64), off(5, 1, 60)]),
            [expr(1, 60), off(5, 1, 60), on(5, 2, 64)]
        );
    }

    #[test]
    fn params_and_other_events_keep_their_order() {
        assert_eq!(
            sorted(&[param(5), off(5, 1, 60)]),
            [param(5), off(5, 1, 60)]
        );
        assert_eq!(
            sorted(&[off(5, 1, 60), param(5)]),
            [off(5, 1, 60), param(5)]
        );
        assert_eq!(sorted(&[on(5, 1, 60), param(5)]), [on(5, 1, 60), param(5)]);
        assert_eq!(
            sorted(&[param(7), param(5), on(5, 1, 60), param(6)]),
            [param(5), on(5, 1, 60), param(6), param(7)]
        );
    }
}
