//! MIDI sidechains (CONTRACTS.md §11.10 "MIDI sidechain"): a device's note input fed from a
//! MIDI track's notes (Live's "MIDI From" on effects: Shifter's MIDI pitch mode, vocoders,
//! plugin effects with a note input).
//!
//! - **Consumers.** Chain entries listed in `TrackDesc::midi_sidechains` (any depth: track
//!   chain, drum pads, rack chains). A consumer hears only its source's notes: the notes
//!   reaching it through its own chain are replaced (its params and all-notes-offs stay).
//! - **Points** ([`MidiPoint`], source side, [`MidiPoints`] inside `sidechain::Points`):
//!   `Input` = the notes entering the track's chain (clips, monitored and preview input),
//!   `Before(node)` = the notes reaching a top-level chain entry (the controller's "post FX":
//!   the first device that isn't a MIDI effect), `End` = the notes leaving the chain (a
//!   chain of MIDI effects only). A track without devices still schedules its notes when
//!   its `Input` is tapped ([`MidiPoints::head`]).
//! - **Order.** An edge source → consumer track in the job graph, like an audio sidechain;
//!   a source on the consumer's own track is produced earlier in the same job (a point not
//!   produced yet is read empty).
//! - **PDC.** A point's latency is the latency of the audio signal there (`Input` = the
//!   track's pre-FX latency, `Before(n)` = at `n`'s input, `End` = post-FX). At the
//!   consumer the main signal or the notes are delayed by the difference, exactly like an
//!   audio sidechain (`crate::sidechain::plan`; an entry with both takes the larger main
//!   delay and delays each side by the rest). Notes are delayed by [`EventDelay`].
//! - **Anticipation.** Held output carries no notes: a MIDI sidechain source of a
//!   real-time track renders in real time too (`crate::anticipation::classify`).
//! - **RT.** Buffers are allocated at compile; jobs copy events only.

use ether_protocol::model::TrackId;
use serde::{Deserialize, Serialize};

use crate::event::{EventBuffer, EventKind, ProcessEvent};
use crate::graph::TrackDesc;
use crate::node::NodeKey;

/// Where a MIDI sidechain taps its source track's notes (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MidiPoint {
    /// The notes entering the chain (clips, live and preview input).
    Input,
    /// The notes reaching this top-level chain entry.
    Before(NodeKey),
    /// The notes leaving the chain's last entry.
    End,
}

/// `TrackDesc::midi_sidechains`: entry `node` (any depth) hears `source`'s notes at `point`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiSidechainDesc {
    pub node: NodeKey,
    pub source: TrackId,
    pub point: MidiPoint,
}

/// A MIDI-sidechained entry, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MidiConsumer {
    pub node: NodeKey,
    /// Source track index.
    pub source: usize,
    pub point: MidiPoint,
}

/// Non-RT. The MIDI-sidechained entries of track `track`, sorted by node (unknown sources,
/// and nodes that aren't in the track's chains, skipped; the own track's `End` would come
/// after every consumer, so it is skipped too).
pub(crate) fn consumers(
    track: usize,
    desc: &TrackDesc,
    source_index: &dyn Fn(TrackId) -> Option<usize>,
) -> Vec<MidiConsumer> {
    let in_track = |node: NodeKey| {
        desc.chain.iter().any(|e| e.node == node)
            || desc
                .racks
                .iter()
                .flat_map(|r| &r.pads)
                .flat_map(|p| &p.chain)
                .any(|e| e.node == node)
            || desc
                .chain_racks
                .iter()
                .flat_map(|r| &r.chains)
                .flat_map(|c| &c.chain)
                .any(|e| e.node == node)
    };
    let mut out: Vec<MidiConsumer> = desc
        .midi_sidechains
        .iter()
        .filter(|m| in_track(m.node))
        .filter_map(|m| {
            let source = source_index(m.source)?;
            if source == track && m.point == MidiPoint::End {
                return None;
            }
            Some(MidiConsumer {
                node: m.node,
                source,
                point: m.point,
            })
        })
        .collect();
    out.sort_by_key(|c| c.node);
    out.dedup_by_key(|c| c.node);
    out
}

/// Whether `e` belongs on a note input (everything but parameter changes).
#[inline]
fn is_note_stream(e: &ProcessEvent) -> bool {
    !matches!(e.kind, EventKind::Param { .. })
}

/// Source side: the tapped MIDI points of a track (only those some consumer reads).
#[derive(Debug, Default)]
pub(crate) struct MidiPoints {
    /// Sorted by point.
    bufs: Vec<(MidiPoint, EventBuffer)>,
    /// A track without devices whose `Input` is tapped: its notes are scheduled here, and
    /// its live input queued in `pending` ([`Self::head`]).
    head: Option<Head>,
}

#[derive(Debug)]
pub(crate) struct Head {
    pub events: EventBuffer,
    pub pending: EventBuffer,
}

impl MidiPoints {
    /// Non-RT. `head`: the track has no chain entry (its notes need a buffer of their own).
    pub(crate) fn compile(points: &[MidiPoint], head: bool, capacity: usize) -> Self {
        let mut points = points.to_vec();
        points.sort();
        points.dedup();
        let head = head && points.contains(&MidiPoint::Input);
        Self {
            bufs: points
                .into_iter()
                .map(|p| (p, EventBuffer::with_capacity(capacity)))
                .collect(),
            head: head.then(|| Head {
                events: EventBuffer::with_capacity(capacity),
                pending: EventBuffer::with_capacity(capacity),
            }),
        }
    }

    fn find(&self, point: MidiPoint) -> Option<usize> {
        self.bufs.binary_search_by(|e| e.0.cmp(&point)).ok()
    }

    /// RT. Start of the track's job: empty the `End` point, move the head's pending input
    /// in (an `AllNotesOff` first when `all_notes_off`). `n`: the sub-block's length.
    pub(crate) fn begin(&mut self, n: usize, all_notes_off: bool) {
        if let Some(i) = self.find(MidiPoint::End) {
            self.bufs[i].1.clear();
        }
        if let Some(h) = &mut self.head {
            h.events.clear();
            for e in h.pending.as_slice() {
                let mut e = *e;
                e.offset = e.offset.min(n.saturating_sub(1) as u32);
                h.events.push(e);
            }
            h.pending.clear();
            if all_notes_off {
                h.events.push(ProcessEvent {
                    offset: 0,
                    kind: EventKind::AllNotesOff,
                });
            }
        }
    }

    /// The head of a track without devices (its clips' notes go there), if it has one.
    pub(crate) fn head(&mut self) -> Option<&mut Head> {
        self.head.as_mut()
    }

    /// RT. Record `point`'s notes for this sub-block (params dropped), if it is tapped.
    #[inline]
    pub(crate) fn write(&mut self, point: MidiPoint, events: &[ProcessEvent]) -> bool {
        if self.bufs.is_empty() {
            return false;
        }
        let Some(i) = self.find(point) else {
            return false;
        };
        let buf = &mut self.bufs[i].1;
        buf.clear();
        let mut overflow = false;
        for e in events.iter().filter(|e| is_note_stream(e)) {
            overflow |= !buf.push(*e);
        }
        buf.sort();
        overflow
    }

    /// RT. Write the `Input` point from the head (a track without devices).
    pub(crate) fn write_head(&mut self) -> bool {
        let Some(h) = self.head.take() else {
            return false;
        };
        let overflow = self.write(MidiPoint::Input, h.events.as_slice());
        self.head = Some(h);
        overflow
    }

    /// RT. The `End` point's buffer (the chain's last entry adds its MIDI to it), if tapped.
    pub(crate) fn end_mut(&mut self) -> Option<&mut EventBuffer> {
        if self.bufs.is_empty() {
            return None;
        }
        let i = self.find(MidiPoint::End)?;
        Some(&mut self.bufs[i].1)
    }

    /// RT. After the chain: the `End` point in order, params dropped.
    pub(crate) fn finish_end(&mut self) {
        if let Some(b) = self.end_mut() {
            b.retain(is_note_stream);
            b.sort();
        }
    }

    /// RT. `point`'s notes of the last sub-block written.
    pub(crate) fn get(&self, point: MidiPoint) -> Option<&[ProcessEvent]> {
        self.find(point).map(|i| self.bufs[i].1.as_slice())
    }
}

/// A fixed delay for an event stream (RT: a ring allocated at compile). Events keep their
/// order; when the ring is full the newest are dropped (reported as an overflow).
#[derive(Debug)]
pub(crate) struct EventDelay {
    delay: u64,
    /// Samples processed so far.
    clock: u64,
    ring: Vec<(u64, EventKind)>,
    head: usize,
    len: usize,
}

impl EventDelay {
    /// Non-RT. `capacity`: events in flight at most (unused when `delay` is 0).
    pub(crate) fn new(delay: u32, capacity: usize) -> Self {
        Self {
            delay: u64::from(delay),
            clock: 0,
            ring: if delay == 0 {
                Vec::new()
            } else {
                vec![(0, EventKind::AllNotesOff); capacity.max(1)]
            },
            head: 0,
            len: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn delay(&self) -> u32 {
        self.delay as u32
    }

    /// RT. Delay `input` (sorted, offsets `< n`) into `out` for this `n`-sample sub-block.
    /// Returns `true` if an event was dropped.
    pub(crate) fn process(
        &mut self,
        input: &[ProcessEvent],
        n: usize,
        out: &mut EventBuffer,
    ) -> bool {
        let mut overflow = false;
        if self.delay == 0 {
            for e in input {
                overflow |= !out.push(*e);
            }
            return overflow;
        }
        let cap = self.ring.len();
        for e in input {
            if self.len == cap {
                overflow = true;
                continue;
            }
            let at = (self.head + self.len) % cap;
            self.ring[at] = (self.clock + u64::from(e.offset) + self.delay, e.kind);
            self.len += 1;
        }
        let end = self.clock + n as u64;
        while self.len > 0 {
            let (due, kind) = self.ring[self.head];
            if due >= end {
                break;
            }
            overflow |= !out.push(ProcessEvent {
                offset: due.saturating_sub(self.clock) as u32,
                kind,
            });
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
        self.clock = end;
        overflow
    }

    /// RT (snapshot swap). Take over `old`'s events in flight when the delay is the same.
    pub(crate) fn inherit(&mut self, old: &mut EventDelay) {
        if old.delay == self.delay && old.ring.len() == self.ring.len() {
            std::mem::swap(self, old);
        }
    }
}

/// Consumer side, per MIDI-sidechained entry (inside `crate::sidechain::Taps`).
#[derive(Debug)]
pub(crate) struct MidiTapRt {
    pub source: usize,
    pub point: MidiPoint,
    /// The source is this track (read at the entry, not gathered).
    pub own: bool,
    pub delay: EventDelay,
    /// The source's notes of this sub-block (gathered, or read from the own points).
    pub input: EventBuffer,
    /// The same, delayed.
    pub out: EventBuffer,
}

impl MidiTapRt {
    /// Non-RT.
    pub(crate) fn new(
        track: usize,
        source: usize,
        point: MidiPoint,
        delay: u32,
        capacity: usize,
    ) -> Self {
        Self {
            source,
            point,
            own: source == track,
            delay: EventDelay::new(delay, capacity.saturating_mul(4)),
            input: EventBuffer::with_capacity(capacity),
            out: EventBuffer::with_capacity(capacity),
        }
    }

    /// RT. Copy the source's notes in (`None`: nothing this sub-block).
    pub(crate) fn load(&mut self, events: Option<&[ProcessEvent]>) -> bool {
        self.input.clear();
        let mut overflow = false;
        for e in events.unwrap_or(&[]) {
            overflow |= !self.input.push(*e);
        }
        overflow
    }

    /// RT. Delay this sub-block's notes and, when `target` (the entry's events) is given,
    /// replace the notes in it with them. Returns `true` on an overflow.
    pub(crate) fn feed(&mut self, n: usize, target: Option<&mut EventBuffer>) -> bool {
        self.out.clear();
        let mut overflow = self.delay.process(self.input.as_slice(), n, &mut self.out);
        if let Some(t) = target {
            t.retain(|e| matches!(e.kind, EventKind::Param { .. } | EventKind::AllNotesOff));
            for e in self.out.as_slice() {
                overflow |= !t.push(*e);
            }
            t.sort();
        }
        overflow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(offset: u32, key: u8) -> ProcessEvent {
        ProcessEvent {
            offset,
            kind: EventKind::NoteOn {
                note_id: u32::from(key),
                channel: 0,
                key,
                velocity: 1.0,
            },
        }
    }

    #[test]
    fn the_delay_keeps_the_sample_positions_across_sub_blocks() {
        let mut d = EventDelay::new(100, 16);
        let mut out = EventBuffer::with_capacity(16);
        // 64-sample sub-blocks: an event at 10 is due at 110 = the second block's 46.
        assert!(!d.process(&[on(10, 60)], 64, &mut out));
        assert!(out.is_empty());
        assert!(!d.process(&[on(30, 62)], 64, &mut out));
        assert_eq!(out.as_slice(), &[on(46, 60)]);
        out.clear();
        assert!(!d.process(&[], 64, &mut out));
        assert!(out.is_empty());
        // 64 + 30 + 100 = 194: the fourth block's 2.
        assert!(!d.process(&[], 64, &mut out));
        assert_eq!(out.as_slice(), &[on(2, 62)]);
    }

    #[test]
    fn no_delay_is_a_copy_and_a_full_ring_drops_the_newest() {
        let mut out = EventBuffer::with_capacity(8);
        let mut d = EventDelay::new(0, 0);
        assert!(!d.process(&[on(3, 60)], 16, &mut out));
        assert_eq!(out.as_slice(), &[on(3, 60)]);
        let mut d = EventDelay::new(10, 1);
        out.clear();
        assert!(d.process(&[on(0, 60), on(1, 61)], 16, &mut out));
        assert_eq!(out.as_slice(), &[on(10, 60)]);
    }

    #[test]
    fn a_fed_entry_hears_only_the_sidechain_notes_and_keeps_its_params() {
        let mut tap = MidiTapRt::new(0, 1, MidiPoint::Input, 0, 8);
        let mut entry = EventBuffer::with_capacity(8);
        entry.push(on(0, 40));
        let param = ProcessEvent {
            offset: 2,
            kind: EventKind::Param {
                param: ether_protocol::model::ParamId(1),
                value: 0.5,
                step: false,
            },
        };
        entry.push(param);
        tap.load(Some(&[on(5, 64)]));
        assert!(!tap.feed(16, Some(&mut entry)));
        assert_eq!(entry.as_slice(), &[param, on(5, 64)]);
    }

    #[test]
    fn points_record_notes_without_params() {
        let mut p = MidiPoints::compile(&[MidiPoint::Input, MidiPoint::End], true, 8);
        p.begin(16, false);
        p.head().unwrap().events.push(on(4, 60));
        p.write_head();
        assert_eq!(p.get(MidiPoint::Input).unwrap(), &[on(4, 60)]);
        p.end_mut().unwrap().push(on(9, 61));
        p.finish_end();
        assert_eq!(p.get(MidiPoint::End).unwrap(), &[on(9, 61)]);
        p.begin(16, false);
        assert!(p.get(MidiPoint::End).unwrap().is_empty());
    }
}
