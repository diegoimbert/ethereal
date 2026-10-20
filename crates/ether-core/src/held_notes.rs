//! Open-note bookkeeping for routing decisions that can change while a note sounds (stuck
//! notes).
//!
//! A note-off must reach whatever its note-on reached, even when the route changed in
//! between: a track that stopped monitoring live input (`crate::recording`), a drum pad
//! whose note was changed (`crate::drum_rack`). [`HeldNotes`] remembers each note-on that was
//! routed (note id, channel, key) with a tag saying where it went; the note-off takes it
//! back. Device on/off transitions are handled on the chain entries themselves
//! (`ChainRt::switch_notes`, `crate::mixer`).
//!
//! RT: fixed capacity, allocated once (non-RT); when full the oldest note is forgotten (its
//! note-off then falls back to the current route).

/// One routed, still-sounding note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeldNote<T> {
    pub note_id: u32,
    pub channel: u8,
    pub key: u8,
    pub tag: T,
}

#[derive(Debug, Default)]
pub(crate) struct HeldNotes<T> {
    notes: Vec<HeldNote<T>>,
}

impl<T: Copy> HeldNotes<T> {
    /// Non-RT: allocates room for `capacity` notes.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            notes: Vec::with_capacity(capacity),
        }
    }

    /// RT. Remember a note-on routed to `tag` (the oldest note is forgotten when full).
    pub(crate) fn on(&mut self, note_id: u32, channel: u8, key: u8, tag: T) {
        if self.notes.capacity() == 0 {
            return;
        }
        if self.notes.len() == self.notes.capacity() {
            self.notes.remove(0);
        }
        self.notes.push(HeldNote {
            note_id,
            channel,
            key,
            tag,
        });
    }

    /// RT. Forget every note matching a note-off (same id, channel and key) and call `f`
    /// with each one's tag. Returns whether any matched.
    pub(crate) fn off(&mut self, note_id: u32, channel: u8, key: u8, mut f: impl FnMut(T)) -> bool {
        let mut found = false;
        let mut i = 0;
        while i < self.notes.len() {
            let n = self.notes[i];
            if n.note_id == note_id && n.channel == channel && n.key == key {
                self.notes.remove(i);
                f(n.tag);
                found = true;
            } else {
                i += 1;
            }
        }
        found
    }

    /// RT. Forget every note, oldest first, calling `f` with each.
    pub(crate) fn drain(&mut self, mut f: impl FnMut(HeldNote<T>)) {
        for n in self.notes.drain(..) {
            f(n);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.notes.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_offs_take_back_their_note_ons() {
        let mut h = HeldNotes::with_capacity(2);
        h.on(1, 0, 60, 'a');
        h.on(2, 0, 62, 'b');
        let mut got = Vec::new();
        assert!(h.off(1, 0, 60, |t| got.push(t)));
        assert_eq!(got, ['a']);
        // Unknown note: nothing matches.
        assert!(!h.off(1, 0, 60, |t| got.push(t)));
        // Full: the oldest is forgotten.
        h.on(3, 0, 64, 'c');
        h.on(4, 0, 65, 'd');
        assert!(!h.off(2, 0, 62, |_| {}));
        let mut rest = Vec::new();
        h.drain(|n| rest.push(n.tag));
        assert_eq!(rest, ['c', 'd']);
        assert!(h.is_empty());
    }

    #[test]
    fn zero_capacity_remembers_nothing() {
        let mut h = HeldNotes::<()>::default();
        h.on(1, 0, 60, ());
        assert!(h.is_empty());
    }
}
