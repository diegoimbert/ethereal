//! `midi-fx` MIDI in -> MIDI out tests: Arpeggiator, Chord, Scale, Note Length, Velocity,
//! Random. Every `process` call runs under `assert_no_alloc` (debug builds abort on an
//! allocation on the audio path). Includes the stuck-note tests (release, `AllNotesOff`,
//! `reset` = stop/bypass/removal flush) and block-size independence.

use std::collections::BTreeMap;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, MusicalScale, ParamId, ScaleKind,
};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::midi_fx::{
    self, GENERATED, arpeggiator as arp, chord, note_length as nl, randomizer as rnd,
    scale_quantize as sq, velocity as vel,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
/// Samples per beat at 120 bpm.
const BEAT: usize = 24_000;
/// A sixteenth at 120 bpm.
const SIXTEENTH: usize = BEAT / 4;

fn device(ty: BuiltinDeviceType) -> Box<dyn Device> {
    let mut d = midi_fx::create(&BuiltinDevice::new(ty));
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: 1024,
        max_events_per_block: 256,
    });
    d
}

fn with(ty: BuiltinDeviceType, params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = device(ty);
    for &(p, v) in params {
        d.set_param(p, v);
    }
    d
}

fn on(id: u32, key: u8, velocity: f32) -> EventKind {
    EventKind::NoteOn {
        note_id: id,
        channel: 0,
        key,
        velocity,
    }
}

fn off(id: u32, key: u8) -> EventKind {
    EventKind::NoteOff {
        note_id: id,
        channel: 0,
        key,
        velocity: 0.0,
    }
}

/// Render `frames` samples in blocks of `block` with `events` at absolute frames (sorted),
/// transport `playing` at 120 bpm from beat 0. Returns `(absolute frame, event)`.
fn render_with(
    d: &mut dyn Device,
    frames: usize,
    block: usize,
    playing: bool,
    events: &[(usize, EventKind)],
) -> Vec<(usize, EventKind)> {
    let mut out_events = EventBuffer::with_capacity(512);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(256);
    let mut result = Vec::new();
    let mut pos = 0;
    while pos < frames {
        let n = block.min(frames - pos);
        block_events.clear();
        for (at, kind) in events {
            if *at >= pos && *at < pos + n {
                block_events.push(ProcessEvent {
                    offset: (*at - pos) as u32,
                    kind: *kind,
                });
            }
        }
        let bps = 2.0 / f64::from(SR);
        let transport = if playing {
            TransportInfo {
                playing: true,
                sample_time: pos as u64,
                position: pos as f64 * bps,
                seconds: pos as f64 / f64::from(SR),
                beats_per_sample: bps,
                ..TransportInfo::STOPPED
            }
        } else {
            TransportInfo::STOPPED
        };
        out_events.clear();
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &block_events,
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &[],
            outputs: &mut [],
        };
        assert_no_alloc(|| {
            d.process(&mut ctx, &mut audio);
        });
        let evs = out_events.as_slice();
        assert!(
            evs.windows(2).all(|w| w[0].offset <= w[1].offset),
            "output sorted"
        );
        for e in evs {
            assert!((e.offset as usize) < n, "offset inside the block");
            assert!(
                !matches!(e.kind, EventKind::Param { .. }),
                "params never forwarded"
            );
            result.push((pos + e.offset as usize, e.kind));
        }
        pos += n;
    }
    result
}

fn render(
    d: &mut dyn Device,
    frames: usize,
    events: &[(usize, EventKind)],
) -> Vec<(usize, EventKind)> {
    render_with(d, frames, 256, true, events)
}

/// `(frame, key)` of the note-ons.
fn ons(out: &[(usize, EventKind)]) -> Vec<(usize, u8)> {
    out.iter()
        .filter_map(|(f, k)| match k {
            EventKind::NoteOn { key, .. } => Some((*f, *key)),
            _ => None,
        })
        .collect()
}

/// Every note-on is ended exactly once (note-off or choke) at or after it, by id; returns
/// `id -> (on frame, end frame)`.
fn assert_balanced(out: &[(usize, EventKind)]) -> BTreeMap<u32, (usize, usize)> {
    let mut open: BTreeMap<u32, (usize, u8)> = BTreeMap::new();
    let mut done = BTreeMap::new();
    for (f, k) in out {
        match *k {
            EventKind::NoteOn { note_id, key, .. } => {
                assert!(
                    open.insert(note_id, (*f, key)).is_none(),
                    "id {note_id:#x} started twice"
                );
            }
            EventKind::NoteOff { note_id, key, .. } | EventKind::NoteChoke { note_id, key, .. } => {
                let (at, k0) = open
                    .remove(&note_id)
                    .unwrap_or_else(|| panic!("end of unknown id {note_id:#x} at {f}"));
                assert_eq!(k0, key, "note-off key = its note-on key");
                assert!(*f >= at);
                done.insert(note_id, (at, *f));
            }
            // The instrument releases input notes passed through; generated ones must get
            // their own note-offs.
            EventKind::AllNotesOff => open.retain(|id, _| id & GENERATED != 0),
            _ => {}
        }
    }
    assert!(open.is_empty(), "stuck notes: {open:?}");
    done
}

// ─── Arpeggiator ────────────────────────────────────────────────────────────────────────

fn held_chord(release_at: usize) -> Vec<(usize, EventKind)> {
    vec![
        (0, on(1, 60, 0.8)),
        (0, on(2, 64, 0.7)),
        (0, on(3, 67, 0.6)),
        (release_at, off(1, 60)),
        (release_at, off(2, 64)),
        (release_at, off(3, 67)),
    ]
}

#[test]
fn arpeggiator_up_plays_sixteenths_on_the_grid() {
    let mut d = device(BuiltinDeviceType::Arpeggiator);
    let out = render(&mut *d, 8 * SIXTEENTH, &held_chord(4 * SIXTEENTH));
    assert_eq!(
        ons(&out),
        vec![
            (0, 60),
            (SIXTEENTH, 64),
            (2 * SIXTEENTH, 67),
            (3 * SIXTEENTH, 60)
        ]
    );
    // Gate 75 %, generated ids, the input velocity.
    let notes = assert_balanced(&out);
    for (id, (a, b)) in &notes {
        assert_ne!(id & GENERATED, 0);
        assert_eq!(b - a, SIXTEENTH * 3 / 4);
    }
    assert!(out.iter().any(|(_, k)| matches!(
        k,
        EventKind::NoteOn { key: 64, velocity, .. } if (*velocity - 0.7).abs() < 1e-6
    )));
}

#[test]
fn arpeggiator_styles_and_octaves() {
    let keys = |style: f64, octaves: f64, steps: usize| {
        let mut d = with(
            BuiltinDeviceType::Arpeggiator,
            &[(arp::STYLE, style), (arp::OCTAVES, octaves)],
        );
        let out = render(
            &mut *d,
            steps * SIXTEENTH,
            &held_chord(steps * SIXTEENTH - 1),
        );
        assert_balanced(&out);
        ons(&out).into_iter().map(|(_, k)| k).collect::<Vec<_>>()
    };
    assert_eq!(keys(1.0, 1.0, 4), [67, 64, 60, 67]); // Down
    assert_eq!(keys(2.0, 1.0, 6), [60, 64, 67, 64, 60, 64]); // Up-Down
    assert_eq!(keys(3.0, 1.0, 4), [67, 64, 60, 64]); // Down-Up
    assert_eq!(keys(0.0, 2.0, 7), [60, 64, 67, 72, 76, 79, 60]); // 2 octaves
    assert_eq!(keys(4.0, 2.0, 6), [60, 79, 64, 76, 67, 72]); // Converge
    assert_eq!(keys(8.0, 1.0, 2), [60, 64, 67, 60, 64, 67]); // Chord: all each step
    // Random: only held keys, deterministic.
    let r = keys(7.0, 1.0, 16);
    assert_eq!(r.len(), 16);
    assert!(r.iter().all(|k| [60, 64, 67].contains(k)));
    assert_eq!(r, keys(7.0, 1.0, 16));
}

#[test]
fn arpeggiator_as_played_order() {
    let mut d = with(BuiltinDeviceType::Arpeggiator, &[(arp::STYLE, 6.0)]);
    let events = [
        (0, on(1, 67, 0.8)),
        (0, on(2, 60, 0.8)),
        (0, on(3, 64, 0.8)),
        (3 * SIXTEENTH, off(1, 67)),
        (3 * SIXTEENTH, off(2, 60)),
        (3 * SIXTEENTH, off(3, 64)),
    ];
    let out = render(&mut *d, 4 * SIXTEENTH, &events);
    assert_eq!(
        ons(&out).into_iter().map(|(_, k)| k).collect::<Vec<_>>(),
        [67, 60, 64]
    );
}

#[test]
fn arpeggiator_swing_delays_odd_steps() {
    // 1/8 at 100 % swing: odd eighths a third of a step late (triplet shuffle).
    let eighth = BEAT / 2;
    let mut d = with(
        BuiltinDeviceType::Arpeggiator,
        &[(arp::RATE, 6.0), (arp::SWING, 100.0)],
    );
    let out = render(&mut *d, 4 * eighth, &held_chord(4 * eighth - 1));
    let frames: Vec<usize> = ons(&out).into_iter().map(|(f, _)| f).collect();
    assert_eq!(
        frames,
        [0, eighth + eighth / 3, 2 * eighth, 3 * eighth + eighth / 3]
    );
}

#[test]
fn arpeggiator_rate_and_gate_follow_params() {
    // 1/4 steps, 200 % gate: legato overlap of one step.
    let mut d = with(
        BuiltinDeviceType::Arpeggiator,
        &[(arp::RATE, 8.0), (arp::GATE, 200.0)],
    );
    let out = render(&mut *d, 5 * BEAT, &held_chord(2 * BEAT));
    assert_eq!(ons(&out), [(0, 60), (BEAT, 64)]);
    for (_, (a, b)) in assert_balanced(&out) {
        assert_eq!(b - a, 2 * BEAT);
    }
}

#[test]
fn arpeggiator_stopped_transport_starts_on_the_key_sample_accurately() {
    let mut d = device(BuiltinDeviceType::Arpeggiator);
    let events = [(1234, on(1, 60, 0.8)), (1234 + 3 * SIXTEENTH, off(1, 60))];
    let out = render_with(&mut *d, 5 * SIXTEENTH, 256, false, &events);
    assert_eq!(
        ons(&out),
        [
            (1234, 60),
            (1234 + SIXTEENTH, 60),
            (1234 + 2 * SIXTEENTH, 60)
        ]
    );
    assert_balanced(&out);
}

#[test]
fn arpeggiator_hold_latches_until_a_new_chord() {
    let mut d = with(BuiltinDeviceType::Arpeggiator, &[(arp::HOLD, 1.0)]);
    let events = [
        (0, on(1, 60, 0.8)),
        (0, on(2, 64, 0.8)),
        (100, off(1, 60)),
        (100, off(2, 64)),
        // A new chord after all keys are up replaces the latched one.
        (4 * SIXTEENTH, on(3, 70, 0.8)),
        (4 * SIXTEENTH + 100, off(3, 70)),
    ];
    let out = render(&mut *d, 6 * SIXTEENTH, &events);
    assert_eq!(
        ons(&out).into_iter().map(|(_, k)| k).collect::<Vec<_>>(),
        [60, 64, 60, 64, 70, 70]
    );
    // Hold off drops the latch: the arpeggio stops, its last note still ends.
    let events = [(
        0,
        EventKind::Param {
            param: arp::HOLD,
            value: 0.0,
        },
    )];
    let tail = render(&mut *d, 4 * SIXTEENTH, &events);
    assert!(ons(&tail).is_empty());
    let mut all = out;
    all.extend(tail.into_iter().map(|(f, k)| (f + 6 * SIXTEENTH, k)));
    assert_balanced(&all);
}

#[test]
fn arpeggiator_retrigger_on_note_restarts_the_pattern() {
    let mut d = with(BuiltinDeviceType::Arpeggiator, &[(arp::RETRIGGER, 1.0)]);
    let events = [
        (0, on(1, 60, 0.8)),
        (0, on(2, 64, 0.8)),
        (0, on(3, 67, 0.8)),
        (2 * SIXTEENTH - 10, on(4, 62, 0.8)),
        (4 * SIXTEENTH - 1, off(1, 60)),
        (4 * SIXTEENTH - 1, off(2, 64)),
        (4 * SIXTEENTH - 1, off(3, 67)),
        (4 * SIXTEENTH - 1, off(4, 62)),
    ];
    let out = render(&mut *d, 5 * SIXTEENTH, &events);
    assert_eq!(
        ons(&out).into_iter().map(|(_, k)| k).collect::<Vec<_>>(),
        [60, 64, 60, 62]
    );
}

#[test]
fn arpeggiator_fixed_velocity() {
    let mut d = with(
        BuiltinDeviceType::Arpeggiator,
        &[(arp::VELOCITY_MODE, 1.0), (arp::FIXED_VELOCITY, 127.0)],
    );
    let out = render(&mut *d, 2 * SIXTEENTH, &held_chord(SIXTEENTH + 5));
    assert!(out.iter().all(|(_, k)| match k {
        EventKind::NoteOn { velocity, .. } => *velocity == 1.0,
        _ => true,
    }));
}

#[test]
fn arpeggiator_is_block_size_independent() {
    let events = held_chord(7 * SIXTEENTH + 17);
    let params = [(arp::STYLE, 2.0), (arp::SWING, 37.0), (arp::RATE, 3.0)];
    let mut a = with(BuiltinDeviceType::Arpeggiator, &params);
    let mut b = with(BuiltinDeviceType::Arpeggiator, &params);
    let ra = render_with(&mut *a, 10 * SIXTEENTH, 64, true, &events);
    let rb = render_with(&mut *b, 10 * SIXTEENTH, 1000, true, &events);
    assert_eq!(ra, rb);
    assert_balanced(&ra);
}

/// The stuck-note test: whatever stops the arpeggio (release, `AllNotesOff` from a
/// transport stop/locate, `reset` on bypass/removal/stop), every generated note ends.
#[test]
fn arpeggiator_never_leaves_stuck_notes() {
    let gate = [(arp::GATE, 200.0), (arp::RATE, 8.0)];
    // AllNotesOff mid-note: generated notes end there and it is forwarded.
    let mut d = with(BuiltinDeviceType::Arpeggiator, &gate);
    let events = [
        (0, on(1, 60, 0.8)),
        (0, on(2, 64, 0.8)),
        (BEAT + 500, EventKind::AllNotesOff),
    ];
    let out = render(&mut *d, 4 * BEAT, &events);
    assert!(out.contains(&(BEAT + 500, EventKind::AllNotesOff)));
    let notes = assert_balanced(&out);
    assert_eq!(notes.len(), 2);
    assert!(notes.values().all(|(_, end)| *end == BEAT + 500));
    assert!(
        ons(&out).iter().all(|(f, _)| *f < BEAT + 500),
        "arpeggio stops"
    );

    // `reset` (stop, bypass flush): note-offs at the start of the next block.
    let mut d = with(BuiltinDeviceType::Arpeggiator, &gate);
    let mut out = render(&mut *d, BEAT / 2, &[(0, on(1, 60, 0.8))]);
    d.reset();
    let tail = render(&mut *d, 256, &[]);
    assert_eq!(tail.len(), 1, "{tail:?}");
    out.extend(tail.into_iter().map(|(f, k)| (f + BEAT / 2, k)));
    assert_balanced(&out);

    // A note-off for a key held before the arpeggiator existed passes through.
    let mut d = device(BuiltinDeviceType::Arpeggiator);
    let out = render(&mut *d, 256, &[(3, off(77, 50))]);
    assert_eq!(out, [(3, off(77, 50))]);
}

// ─── Chord ──────────────────────────────────────────────────────────────────────────────

#[test]
fn chord_adds_shifted_notes() {
    let mut d = with(
        BuiltinDeviceType::Chord,
        &[
            (chord::SHIFT_1, 4.0),
            (chord::SHIFT_2, 7.0),
            (chord::SHIFT_3, 12.0),
            (chord::VELOCITY_2, 50.0),
            (chord::SHIFT_4, 7.0), // duplicate: skipped
        ],
    );
    let out = render(&mut *d, 2000, &[(100, on(1, 60, 0.8)), (1500, off(1, 60))]);
    assert_eq!(ons(&out), [(100, 60), (100, 64), (100, 67), (100, 72)]);
    let v67 = out.iter().find_map(|(_, k)| match k {
        EventKind::NoteOn {
            key: 67, velocity, ..
        } => Some(*velocity),
        _ => None,
    });
    assert!((v67.unwrap() - 0.4).abs() < 1e-6);
    let notes = assert_balanced(&out);
    assert!(notes.keys().all(|id| id & GENERATED != 0));
    assert!(notes.values().all(|(a, b)| (*a, *b) == (100, 1500)));
}

#[test]
fn chord_without_shifts_passes_notes_through() {
    let mut d = device(BuiltinDeviceType::Chord);
    let events = [
        (5, on(9, 60, 0.5)),
        (
            7,
            EventKind::Midi {
                data: [0xB0, 1, 64],
            },
        ),
        (9, off(9, 60)),
    ];
    assert_eq!(render(&mut *d, 256, &events), events);
}

#[test]
fn chord_strum_spreads_low_to_high_and_never_ends_early() {
    let mut d = with(
        BuiltinDeviceType::Chord,
        &[
            (chord::SHIFT_1, 7.0),
            (chord::SHIFT_2, 4.0),
            (chord::STRUM, 100.0),
        ],
    );
    // Released before the strum finishes: late notes still start, then end.
    let out = render(&mut *d, 10_000, &[(0, on(1, 60, 0.8)), (1000, off(1, 60))]);
    assert_eq!(ons(&out), [(0, 60), (2400, 64), (4800, 67)]);
    let notes = assert_balanced(&out);
    let ends: Vec<usize> = notes.values().map(|(_, e)| *e).collect();
    assert!(ends.contains(&1000) && ends.contains(&2400) && ends.contains(&4800));
}

// ─── Scale ──────────────────────────────────────────────────────────────────────────────

fn keys_through(d: &mut dyn Device, keys: &[u8]) -> Vec<u8> {
    let mut events = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        events.push((i * 10, on(i as u32 + 1, *k, 0.5)));
        events.push((i * 10 + 5, off(i as u32 + 1, *k)));
    }
    let out = render(d, keys.len() * 10 + 10, &events);
    assert_balanced(&out);
    ons(&out).into_iter().map(|(_, k)| k).collect()
}

#[test]
fn scale_quantize_uses_the_pushed_scale() {
    let mut d = device(BuiltinDeviceType::ScaleQuantize);
    // Chromatic until the controller pushes the scale.
    assert_eq!(keys_through(&mut *d, &[61, 66]), [61, 66]);
    let rejected = d.set_data(Box::new(MusicalScale {
        root: 0,
        kind: ScaleKind::Major,
    }));
    assert!(
        rejected.unwrap().downcast::<MusicalScale>().is_ok(),
        "old box handed back"
    );
    // Nearest (ties down): C# -> C, F# -> F, and in-scale notes unchanged.
    assert_eq!(keys_through(&mut *d, &[61, 66, 64, 71]), [60, 65, 64, 71]);
    d.set_param(sq::DIRECTION, 1.0); // Up
    assert_eq!(keys_through(&mut *d, &[61, 66]), [62, 67]);
    d.set_param(sq::DIRECTION, 2.0); // Down
    d.set_param(sq::TRANSPOSE, 12.0);
    assert_eq!(keys_through(&mut *d, &[61]), [72]);
    // Other data is refused and handed back.
    assert!(
        d.set_data(Box::new(5u32))
            .unwrap()
            .downcast::<u32>()
            .is_ok()
    );
}

#[test]
fn scale_quantize_custom_scale_and_note_off_keys() {
    let mut d = with(
        BuiltinDeviceType::ScaleQuantize,
        &[(sq::SOURCE, 2.0), (sq::ROOT, 9.0), (sq::KIND, 6.0)], // A minor pentatonic
    );
    assert_eq!(keys_through(&mut *d, &[70, 66, 57]), [69, 67, 57]); // F# is nearer G than E
    // A param change while a note is held: its note-off still matches its note-on key.
    let events = [
        (0, on(1, 70, 0.5)),
        (
            10,
            EventKind::Param {
                param: sq::TRANSPOSE,
                value: 5.0,
            },
        ),
        (20, off(1, 70)),
    ];
    let out = render(&mut *d, 64, &events);
    assert_eq!(out, [(0, on(1, 69, 0.5)), (20, off(1, 69))]);
}

// ─── Note Length ────────────────────────────────────────────────────────────────────────

#[test]
fn note_length_fixed_time_ignores_the_input_note_off() {
    let mut d = with(BuiltinDeviceType::NoteLength, &[(nl::LENGTH, 100.0)]);
    let out = render(&mut *d, 10_000, &[(10, on(1, 60, 0.5)), (20, off(1, 60))]);
    let notes = assert_balanced(&out);
    assert_eq!(notes.values().collect::<Vec<_>>(), [&(10, 10 + 4800)]);
}

#[test]
fn note_length_sync_and_gate() {
    // 1/8 (index 6) at 50 % = a sixteenth.
    let mut d = with(
        BuiltinDeviceType::NoteLength,
        &[(nl::MODE, 1.0), (nl::SYNC_LENGTH, 6.0), (nl::GATE, 50.0)],
    );
    let out = render(
        &mut *d,
        BEAT,
        &[(0, on(1, 60, 0.5)), (BEAT / 2 + 3, off(1, 60))],
    );
    let notes = assert_balanced(&out);
    assert_eq!(notes.values().collect::<Vec<_>>(), [&(0, SIXTEENTH)]);
}

#[test]
fn note_length_note_off_trigger_starts_on_release() {
    let mut d = with(
        BuiltinDeviceType::NoteLength,
        &[(nl::TRIGGER, 1.0), (nl::LENGTH, 10.0)],
    );
    let out = render(&mut *d, 2000, &[(10, on(1, 60, 0.7)), (700, off(1, 60))]);
    assert!(
        matches!(out[0], (700, EventKind::NoteOn { velocity, .. }) if (velocity - 0.7).abs() < 1e-6)
    );
    let notes = assert_balanced(&out);
    assert_eq!(notes.values().collect::<Vec<_>>(), [&(700, 700 + 480)]);
}

// ─── Velocity ───────────────────────────────────────────────────────────────────────────

fn velocities(d: &mut dyn Device, input: &[u8]) -> Vec<u8> {
    let mut events = Vec::new();
    for (i, v) in input.iter().enumerate() {
        events.push((i * 10, on(i as u32 + 1, 60, f32::from(*v) / 127.0)));
        events.push((i * 10 + 5, off(i as u32 + 1, 60)));
    }
    let out = render(d, input.len() * 10 + 10, &events);
    assert_balanced(&out);
    out.iter()
        .filter_map(|(_, k)| match k {
            EventKind::NoteOn { velocity, .. } => Some((velocity * 127.0).round() as u8),
            _ => None,
        })
        .collect()
}

#[test]
fn velocity_default_is_identity_and_ranges_map() {
    let mut d = device(BuiltinDeviceType::Velocity);
    assert_eq!(velocities(&mut *d, &[1, 40, 100, 127]), [1, 40, 100, 127]);
    d.set_param(vel::OUT_LOW, 64.0);
    d.set_param(vel::OUT_HIGH, 100.0);
    assert_eq!(velocities(&mut *d, &[1, 127]), [64, 100]);
    // Drive > 0 lifts soft notes, monotonic.
    let mut d = with(BuiltinDeviceType::Velocity, &[(vel::DRIVE, 60.0)]);
    let v = velocities(&mut *d, &[10, 40, 80, 120]);
    assert!(
        v[0] > 10 && v[1] > 40 && v.windows(2).all(|w| w[0] <= w[1]),
        "{v:?}"
    );
}

#[test]
fn velocity_fixed_and_gate_modes() {
    let mut d = with(
        BuiltinDeviceType::Velocity,
        &[(vel::MODE, 2.0), (vel::OUT_HIGH, 90.0)],
    );
    assert_eq!(velocities(&mut *d, &[5, 127]), [90, 90]);
    // Gate drops notes outside In Low..In High, with their note-offs.
    let mut d = with(
        BuiltinDeviceType::Velocity,
        &[(vel::MODE, 1.0), (vel::IN_LOW, 50.0)],
    );
    assert_eq!(velocities(&mut *d, &[20, 60, 49, 127]), [17, 127]); // 50..127 -> 1..127
}

// ─── Random ─────────────────────────────────────────────────────────────────────────────

fn busy_stream(n: usize) -> Vec<(usize, EventKind)> {
    let mut events = Vec::new();
    for i in 0..n {
        let key = 48 + (i * 7 % 24) as u8;
        events.push((i * 1000, on(i as u32 + 1, key, 0.6)));
        events.push((i * 1000 + 1500, off(i as u32 + 1, key)));
    }
    events.sort_by_key(|(f, _)| *f);
    events
}

#[test]
fn random_is_deterministic_delays_only_and_never_sticks() {
    let params = [
        (rnd::CHANCE, 60.0),
        (rnd::PITCH_RANGE, 7.0),
        (rnd::VELOCITY_RANDOM, 50.0),
        (rnd::TIMING_RANDOM, 20.0),
        (rnd::LENGTH_RANDOM, 50.0),
        (rnd::SEED, 42.0),
    ];
    let events = busy_stream(40);
    let mut a = with(BuiltinDeviceType::Randomizer, &params);
    let mut b = with(BuiltinDeviceType::Randomizer, &params);
    let ra = render(&mut *a, 50_000, &events);
    let rb = render_with(&mut *b, 50_000, 97, true, &events);
    assert_eq!(ra, rb, "renders repeat (block size too)");
    let notes = assert_balanced(&ra);
    assert_eq!(notes.len(), 40);
    let in_on: Vec<usize> = events
        .iter()
        .filter(|(_, k)| matches!(k, EventKind::NoteOn { .. }))
        .map(|(f, _)| *f)
        .collect();
    let out_on: Vec<usize> = ons(&ra).into_iter().map(|(f, _)| f).collect();
    for (i, o) in in_on.iter().zip(&out_on) {
        assert!(o >= i && *o <= i + 960, "delay within 20 ms: {i} -> {o}");
    }
    let shifted = ons(&ra)
        .iter()
        .zip(
            events
                .iter()
                .filter(|(_, k)| matches!(k, EventKind::NoteOn { .. })),
        )
        .filter(|((_, k), (_, e))| matches!(e, EventKind::NoteOn { key, .. } if key != k))
        .count();
    assert!((10..=35).contains(&shifted), "{shifted} of 40 shifted");
    // Another seed: another result.
    let mut c = with(BuiltinDeviceType::Randomizer, &params);
    c.set_param(rnd::SEED, 43.0);
    assert_ne!(render(&mut *c, 50_000, &events), ra);
}

#[test]
fn random_use_scale_keeps_notes_in_the_pushed_scale() {
    let mut d = with(
        BuiltinDeviceType::Randomizer,
        &[
            (rnd::CHANCE, 100.0),
            (rnd::PITCH_RANGE, 5.0),
            (rnd::SCALE_AWARE, 1.0),
        ],
    );
    let _ = d.set_data(Box::new(MusicalScale {
        root: 2,
        kind: ScaleKind::MinorPentatonic,
    }));
    let out = render(&mut *d, 50_000, &busy_stream(40));
    assert_balanced(&out);
    let allowed = [0, 3, 5, 7, 10];
    for (_, k) in ons(&out) {
        assert!(
            allowed.contains(&((i32::from(k) - 2).rem_euclid(12))),
            "{k}"
        );
    }
}

#[test]
fn random_chance_zero_is_transparent() {
    let mut d = device(BuiltinDeviceType::Randomizer);
    let events = busy_stream(5);
    assert_eq!(render(&mut *d, 8000, &events), events);
}

// ─── All six ────────────────────────────────────────────────────────────────────────────

const ALL: [BuiltinDeviceType; 6] = [
    BuiltinDeviceType::Arpeggiator,
    BuiltinDeviceType::Chord,
    BuiltinDeviceType::ScaleQuantize,
    BuiltinDeviceType::NoteLength,
    BuiltinDeviceType::Velocity,
    BuiltinDeviceType::Randomizer,
];

/// Settings that make each device generate and delay notes.
fn busy(ty: BuiltinDeviceType) -> Box<dyn Device> {
    let params: &[(ParamId, f64)] = match ty {
        BuiltinDeviceType::Arpeggiator => {
            &[(arp::STYLE, 8.0), (arp::OCTAVES, 3.0), (arp::GATE, 180.0)]
        }
        BuiltinDeviceType::Chord => &[
            (chord::SHIFT_1, 3.0),
            (chord::SHIFT_2, 7.0),
            (chord::SHIFT_3, -12.0),
            (chord::STRUM, 50.0),
        ],
        BuiltinDeviceType::NoteLength => &[(nl::LENGTH, 400.0)],
        BuiltinDeviceType::Randomizer => &[
            (rnd::CHANCE, 100.0),
            (rnd::PITCH_RANGE, 12.0),
            (rnd::TIMING_RANDOM, 50.0),
            (rnd::LENGTH_RANDOM, 100.0),
        ],
        BuiltinDeviceType::Velocity => &[(vel::RANDOM, 30.0)],
        _ => &[(sq::TRANSPOSE, 3.0)],
    };
    let mut d = with(ty, params);
    let _ = d.set_data(Box::new(MusicalScale {
        root: 0,
        kind: ScaleKind::Major,
    }));
    d
}

#[test]
fn every_midi_effect_forwards_all_notes_off_and_leaves_nothing_stuck() {
    for ty in ALL {
        // Notes still held and queued when AllNotesOff arrives.
        let mut d = busy(ty);
        let mut events = busy_stream(10);
        events.retain(|(f, _)| *f < 9000);
        events.push((9000, EventKind::AllNotesOff));
        let out = render(&mut *d, 40_000, &events);
        assert!(out.contains(&(9000, EventKind::AllNotesOff)), "{ty:?}");
        assert!(
            out.iter()
                .all(|(f, k)| *f <= 9000 || !matches!(k, EventKind::NoteOn { .. })),
            "{ty:?}: nothing starts after AllNotesOff"
        );
        assert_balanced(&out);

        // `reset` (transport stop, bypass, removal) flushes on the next block.
        let mut d = busy(ty);
        let mut out = render(&mut *d, 9000, &busy_stream(12));
        d.reset();
        let tail = render(&mut *d, 40_000, &[]);
        out.extend(tail.into_iter().map(|(f, k)| (f + 9000, k)));
        // Input notes that pass through keep their ids: a flush only owns generated notes,
        // so close the pass-through ones like the engine's own AllNotesOff would.
        let passthrough: Vec<(usize, EventKind)> = {
            let mut open = BTreeMap::new();
            for (_, k) in &out {
                match *k {
                    EventKind::NoteOn { note_id, key, .. } => {
                        open.insert(note_id, key);
                    }
                    EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                        open.remove(&note_id);
                    }
                    _ => {}
                }
            }
            assert!(
                open.keys().all(|id| id & GENERATED == 0),
                "{ty:?}: generated notes left after reset: {open:?}"
            );
            open.into_iter()
                .map(|(id, key)| (usize::MAX, off(id, key)))
                .collect()
        };
        out.extend(passthrough);
        assert_balanced(&out);
    }
}

#[test]
fn every_midi_effect_passes_other_midi_and_drops_params() {
    for ty in ALL {
        let mut d = device(ty);
        let events = [
            (
                3,
                EventKind::Param {
                    param: ParamId(0),
                    value: 0.0,
                },
            ),
            (
                4,
                EventKind::Midi {
                    data: [0xE0, 0, 64],
                },
            ),
        ];
        assert_eq!(
            render(&mut *d, 64, &events),
            [(
                4,
                EventKind::Midi {
                    data: [0xE0, 0, 64]
                }
            )],
            "{ty:?}"
        );
        let desc = d.descriptor();
        assert_eq!(d.channels(), (0, 0));
        assert!(desc.layout.is_some() && desc.midi_input, "{ty:?}");
        assert_eq!(d.latency(), 0, "{ty:?}: musical delays are not latency");
    }
}

#[test]
fn every_midi_effect_survives_extreme_params_and_floods() {
    for ty in ALL {
        let mut d = device(ty);
        for p in d.descriptor().params {
            d.set_param(p.id, p.max);
        }
        d.set_param(ParamId(0), f64::NAN);
        // 256 events per block, many overlapping notes, re-used ids.
        let mut events = Vec::new();
        for i in 0..2000usize {
            let id = (i % 300) as u32;
            let key = (i * 13 % 128) as u8;
            events.push((i * 5, on(id, key, 1.0)));
            events.push((i * 5 + 3, off(id, key)));
        }
        let out = render_with(&mut *d, 12_000, 1024, true, &events);
        assert!(!out.is_empty(), "{ty:?}");
    }
}

/// CPU cost per instance (printed; `cargo test --release -- --nocapture cpu_cost`).
#[test]
fn cpu_cost_per_instance() {
    for ty in ALL {
        let mut d = busy(ty);
        let events = busy_stream(200);
        let start = std::time::Instant::now();
        let frames = 200 * 1000 + 48_000;
        let out = render_with(&mut *d, frames, 256, true, &events);
        let secs = start.elapsed().as_secs_f64();
        let audio = frames as f64 / f64::from(SR);
        println!(
            "{ty:?}: {:.4} % of one core ({} events out, test harness included)",
            100.0 * secs / audio,
            out.len()
        );
    }
}
