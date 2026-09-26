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

    /// RT-safe stable insertion sort by offset (event lists are small and nearly sorted).
    pub fn sort(&mut self) {
        let ev = &mut self.events;
        for i in 1..ev.len() {
            let mut j = i;
            while j > 0 && ev[j - 1].offset > ev[j].offset {
                ev.swap(j - 1, j);
                j -= 1;
            }
        }
    }
}
