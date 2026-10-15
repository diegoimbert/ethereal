//! Random: pitch randomization (`Chance` of shifting by up to `Range` semitones; `Random`
//! picks a shift, `Alternate` alternates +Range / -Range; `Use Scale` snaps the result into
//! the track scale) and humanize (velocity, a timing delay, a longer length). Deterministic:
//! every draw is a hash of `Seed`, the note's song position (ticks; sample clock when
//! stopped) and key, so renders repeat. Delays only (never earlier than the input); notes
//! whose timing changes use generated ids.

use ether_core::protocol::model::{BuiltinDeviceType, MusicalScale};
use ether_core::{EventBuffer, EventKind};

use super::core::{
    Direction, MAX_NOTES, MidiFx, Out, Params, Timing, hash, midi_fx_node, quantize, thru, unit,
};
use super::randomizer as id;

#[derive(Clone, Copy, Debug)]
struct Voice {
    input: u32,
    id: u32,
    channel: u8,
    key: u8,
    /// Absolute times of the input note-on and of the output note-on.
    in_at: u64,
    on_at: u64,
    /// Length humanize draw (0..1).
    stretch: f32,
}

pub struct Randomizer {
    params: Params,
    out: Out,
    clock: u64,
    flush: bool,
    /// Resolved track scale (`Node::set_data`) for `Use Scale`.
    scale: MusicalScale,
    voices: Vec<Voice>,
    /// `Alternate`: the sign of the next shift.
    up: bool,
}

impl Randomizer {
    pub fn new() -> Self {
        Self {
            params: Params::new(super::descriptor(BuiltinDeviceType::Randomizer)),
            out: Out::new(),
            clock: 0,
            flush: false,
            scale: MusicalScale::default(),
            voices: Vec::with_capacity(MAX_NOTES),
            up: true,
        }
    }

    fn take(&mut self, input: u32) -> Option<Voice> {
        let i = self.voices.iter().position(|v| v.input == input)?;
        Some(self.voices.swap_remove(i))
    }

    fn note_on(
        &mut self,
        out: &mut EventBuffer,
        t: &Timing,
        offset: u32,
        (note_id, channel, key, velocity): (u32, u8, u8, f32),
    ) {
        let p = &self.params;
        let h = hash(p.int(id::SEED) as u64, t.stamp(offset), u64::from(key));
        let range = p.int(id::PITCH_RANGE);
        let mut k = i32::from(key);
        if range > 0 && unit(h, 0) * 100.0 < p.get(id::CHANCE) {
            let shift = if p.index(id::PITCH_MODE) == 1 {
                self.up = !self.up;
                if self.up { -range } else { range }
            } else {
                let size = 1 + ((unit(h, 1) * f64::from(range)) as i32).min(range - 1);
                if unit(h, 2) < 0.5 { -size } else { size }
            };
            k = (k + shift).clamp(0, 127);
            if p.on(id::SCALE_AWARE) {
                k = quantize(k, self.scale, Direction::Nearest);
            }
        }
        let mut velocity = velocity;
        let vel = p.get(id::VELOCITY_RANDOM) / 100.0;
        if vel > 0.0 {
            velocity = (f64::from(velocity) + (2.0 * unit(h, 3) - 1.0) * vel * 0.5)
                .clamp(1.0 / 127.0, 1.0) as f32;
        }
        let delay = t.ms(unit(h, 4) * p.get(id::TIMING_RANDOM));
        let stretch = if p.get(id::LENGTH_RANDOM) > 0.0 {
            unit(h, 5) as f32
        } else {
            0.0
        };
        // Same timing: keep the id; anything later gets its own.
        let id = if delay == 0 && stretch == 0.0 {
            note_id
        } else {
            self.out.new_id()
        };
        let in_at = t.abs(offset);
        let v = Voice {
            input: note_id,
            id,
            channel,
            key: k as u8,
            in_at,
            on_at: in_at + delay,
            stretch,
        };
        if self.voices.len() < self.voices.capacity() {
            self.voices.push(v);
        }
        self.out.at(
            out,
            t,
            offset,
            v.on_at,
            EventKind::NoteOn {
                note_id: id,
                channel,
                key: v.key,
                velocity,
            },
        );
    }
}

impl MidiFx for Randomizer {
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
                if let Some(old) = self.take(note_id) {
                    // A re-used id: end the previous note first.
                    self.out.at(
                        out,
                        t,
                        offset,
                        old.on_at.max(t.abs(offset)),
                        EventKind::NoteOff {
                            note_id: old.id,
                            channel: old.channel,
                            key: old.key,
                            velocity: 0.0,
                        },
                    );
                }
                self.note_on(out, t, offset, (note_id, channel, key, velocity));
            }
            EventKind::NoteOff {
                note_id, velocity, ..
            } => match self.take(note_id) {
                Some(v) => {
                    let now = t.abs(offset);
                    let length = now.saturating_sub(v.in_at) as f64;
                    let extra = (length * f64::from(v.stretch) * self.params.get(id::LENGTH_RANDOM)
                        / 100.0) as u64;
                    let at = (now + (v.on_at - v.in_at) + extra).max(v.on_at);
                    self.out.at(
                        out,
                        t,
                        offset,
                        at,
                        EventKind::NoteOff {
                            note_id: v.id,
                            channel: v.channel,
                            key: v.key,
                            velocity,
                        },
                    );
                }
                None => thru(out, offset, kind),
            },
            EventKind::NoteChoke { note_id, .. } => match self.take(note_id) {
                Some(v) => {
                    if !self.out.cancel(v.id) {
                        self.out.emit(
                            out,
                            offset,
                            EventKind::NoteChoke {
                                note_id: v.id,
                                channel: v.channel,
                                key: v.key,
                            },
                        );
                    }
                }
                None => thru(out, offset, kind),
            },
            other => thru(out, offset, other),
        }
    }
}

midi_fx_node!(Randomizer, scale);
