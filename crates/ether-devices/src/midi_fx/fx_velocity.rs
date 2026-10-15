//! Velocity: input range → curve (drive, compand) → output range, plus random; `Gate`
//! drops notes outside the input range (and their note-offs), `Fixed` plays every note at
//! `Out High`. No timing change: note ids pass through.

use ether_core::protocol::model::BuiltinDeviceType;
use ether_core::{EventBuffer, EventKind};

use super::core::{MAX_NOTES, MidiFx, Out, Params, Timing, hash, midi_fx_node, thru, unit, vel01};
use super::velocity as id;

pub struct Velocity {
    params: Params,
    out: Out,
    clock: u64,
    flush: bool,
    /// Note ids dropped by `Gate` (their note-offs are dropped too).
    dropped: Vec<u32>,
    /// Notes seen, for per-note randomness.
    count: u64,
}

impl Velocity {
    pub fn new() -> Self {
        Self {
            params: Params::new(super::descriptor(BuiltinDeviceType::Velocity)),
            out: Out::new(),
            clock: 0,
            flush: false,
            dropped: Vec::with_capacity(MAX_NOTES),
            count: 0,
        }
    }

    /// Map a MIDI velocity (1..=127); `None` = dropped by `Gate`.
    pub fn map(&self, v: f64, noise: f64) -> Option<f64> {
        let p = &self.params;
        let mode = p.index(id::MODE);
        let (out_lo, out_hi) = (p.get(id::OUT_LOW), p.get(id::OUT_HIGH));
        let random = p.get(id::RANDOM);
        let y = if mode == 2 {
            out_hi
        } else {
            let (a, b) = (p.get(id::IN_LOW), p.get(id::IN_HIGH));
            let (lo, hi) = (a.min(b), a.max(b));
            if mode == 1 && (v < lo || v > hi) {
                return None;
            }
            let mut x = if hi > lo {
                (v.clamp(lo, hi) - lo) / (hi - lo)
            } else {
                1.0
            };
            // Drive: > 0 lifts soft notes (x^(1/4) at 100 %), < 0 pushes them down.
            let drive = p.get(id::DRIVE) / 100.0;
            x = x.powf(4f64.powf(-drive));
            // Compand: > 0 expands away from the middle, < 0 compresses toward it.
            let c = p.get(id::COMPAND) / 100.0;
            let u = 2.0 * x - 1.0;
            let u = u.signum() * u.abs().powf(4f64.powf(-c));
            x = (u + 1.0) / 2.0;
            out_lo + x * (out_hi - out_lo)
        };
        Some((y + (2.0 * noise - 1.0) * random).round().clamp(1.0, 127.0))
    }
}

impl MidiFx for Velocity {
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
        self.dropped.clear();
    }

    fn event(&mut self, out: &mut EventBuffer, t: &Timing, offset: u32, kind: EventKind) {
        let target = self.params.index(id::TARGET);
        let kind = match kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } if target != 1 => {
                self.count += 1;
                let noise = unit(hash(t.stamp(offset), u64::from(key), self.count), 0);
                let mapped = self.map(f64::from(velocity) * 127.0, noise);
                let Some(v) = mapped else {
                    if self.dropped.len() < self.dropped.capacity() {
                        self.dropped.push(note_id);
                    }
                    return;
                };
                EventKind::NoteOn {
                    note_id,
                    channel,
                    key,
                    velocity: vel01(v),
                }
            }
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. }
                if self.dropped.contains(&note_id) =>
            {
                if let Some(i) = self.dropped.iter().position(|d| *d == note_id) {
                    self.dropped.swap_remove(i);
                }
                return;
            }
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                velocity,
            } if target != 0 => {
                let noise = unit(hash(t.stamp(offset), u64::from(key), note_id.into()), 1);
                // `Gate` never drops a note-off of a note it let through.
                let v = self
                    .map(f64::from(velocity) * 127.0, noise)
                    .unwrap_or(f64::from(velocity) * 127.0);
                EventKind::NoteOff {
                    note_id,
                    channel,
                    key,
                    velocity: vel01(v),
                }
            }
            other => other,
        };
        thru(out, offset, kind);
    }
}

midi_fx_node!(Velocity);
