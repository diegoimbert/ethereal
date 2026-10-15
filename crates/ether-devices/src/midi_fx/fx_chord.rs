//! Chord: every note plus up to six shifted copies (Shift 0 = off, duplicates and
//! out-of-range keys skipped), each at its own velocity percentage; `Strum` spreads the
//! chord low to high over up to 400 ms (`Tension` bends the spacing). A note with no active
//! shift passes through untouched; chord notes use generated ids.

use ether_core::protocol::model::BuiltinDeviceType;
use ether_core::{EventBuffer, EventKind};

use super::chord as id;
use super::core::{MAX_NOTES, MidiFx, Out, Params, Timing, midi_fx_node, thru};

/// Shifts (and velocities) per chord.
const SHIFTS: usize = 6;

#[derive(Clone, Copy, Debug)]
struct Voice {
    input: u32,
    id: u32,
    channel: u8,
    key: u8,
    /// Absolute time of its note-on (strummed notes start late).
    on_at: u64,
}

pub struct Chord {
    params: Params,
    out: Out,
    clock: u64,
    flush: bool,
    voices: Vec<Voice>,
}

impl Chord {
    pub fn new() -> Self {
        Self {
            params: Params::new(super::descriptor(BuiltinDeviceType::Chord)),
            out: Out::new(),
            clock: 0,
            flush: false,
            voices: Vec::with_capacity(MAX_NOTES * (SHIFTS + 1)),
        }
    }

    /// Chord notes of `key` at `velocity`, sorted low to high: (key, velocity).
    fn notes(&self, key: u8, velocity: f32, out: &mut [(u8, f32); SHIFTS + 1]) -> usize {
        out[0] = (key, velocity);
        let mut n = 1;
        for i in 0..SHIFTS {
            let shift = self.params.int(ether_core::protocol::model::ParamId(
                id::SHIFT_1.0 + i as u32,
            ));
            let pct = self.params.get(ether_core::protocol::model::ParamId(
                id::VELOCITY_1.0 + i as u32,
            ));
            let k = i32::from(key) + shift;
            if shift == 0 || pct <= 0.0 || !(0..=127).contains(&k) {
                continue;
            }
            let k = k as u8;
            if out[..n].iter().any(|(q, _)| *q == k) {
                continue;
            }
            out[n] = (k, (velocity * (pct / 100.0) as f32).clamp(1.0 / 127.0, 1.0));
            n += 1;
        }
        out[..n].sort_unstable_by_key(|(k, _)| *k);
        n
    }

    /// Strum delay (samples) of chord note `j` of `n`.
    fn strum(&self, t: &Timing, j: usize, n: usize) -> u64 {
        let total = self.params.get(id::STRUM);
        if total <= 0.0 || n < 2 {
            return 0;
        }
        let x = j as f64 / (n - 1) as f64;
        // Tension > 0: gaps shrink (accelerating strum); < 0: they grow.
        let tension = self.params.get(id::STRUM_TENSION) / 100.0;
        t.ms(total * x.powf(4f64.powf(-tension)))
    }

    fn release(
        &mut self,
        out: &mut EventBuffer,
        t: &Timing,
        offset: u32,
        input: u32,
        off: EventKind,
    ) -> bool {
        let mut found = false;
        let mut i = 0;
        while i < self.voices.len() {
            let v = self.voices[i];
            if v.input != input {
                i += 1;
                continue;
            }
            found = true;
            self.voices.swap_remove(i);
            let kind = match off {
                EventKind::NoteChoke { .. } => {
                    if self.out.cancel(v.id) {
                        continue; // never started
                    }
                    EventKind::NoteChoke {
                        note_id: v.id,
                        channel: v.channel,
                        key: v.key,
                    }
                }
                EventKind::NoteOff { velocity, .. } => EventKind::NoteOff {
                    note_id: v.id,
                    channel: v.channel,
                    key: v.key,
                    velocity,
                },
                _ => continue,
            };
            // Never before its (strummed) note-on.
            let at = v.on_at.max(t.abs(offset));
            self.out.at(out, t, offset, at, kind);
        }
        found
    }
}

impl MidiFx for Chord {
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
        self.voices.clear();
    }

    fn event(&mut self, out: &mut EventBuffer, t: &Timing, offset: u32, kind: EventKind) {
        match kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => {
                let mut notes = [(0u8, 0f32); SHIFTS + 1];
                let n = self.notes(key, velocity, &mut notes);
                if n == 1 {
                    // No chord: pass through.
                    thru(out, offset, kind);
                    return;
                }
                // A re-used input id releases its previous chord first.
                self.release(
                    out,
                    t,
                    offset,
                    note_id,
                    EventKind::NoteOff {
                        note_id,
                        channel,
                        key,
                        velocity: 0.0,
                    },
                );
                for (j, &(k, v)) in notes[..n].iter().enumerate() {
                    if self.voices.len() == self.voices.capacity() {
                        break;
                    }
                    let id = self.out.new_id();
                    let on_at = t.abs(offset) + self.strum(t, j, n);
                    self.voices.push(Voice {
                        input: note_id,
                        id,
                        channel,
                        key: k,
                        on_at,
                    });
                    self.out.at(
                        out,
                        t,
                        offset,
                        on_at,
                        EventKind::NoteOn {
                            note_id: id,
                            channel,
                            key: k,
                            velocity: v,
                        },
                    );
                }
            }
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                if !self.release(out, t, offset, note_id, kind) {
                    thru(out, offset, kind);
                }
            }
            other => thru(out, offset, other),
        }
    }
}

midi_fx_node!(Chord);
