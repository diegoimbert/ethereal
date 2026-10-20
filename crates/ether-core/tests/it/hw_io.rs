//! `HwIoRt` (`external-instrument`, CONTRACTS.md §13.7): return gathering (mono, stereo,
//! missing channels, sub-block offsets), sends added to hardware outputs, and an
//! instrument's events as MIDI bytes on its channel. Engine-level behaviour with the real
//! devices (alignment, measurement, timestamps) is in `ether-devices/tests/external_engine.rs`.

use ether_core::hw_io::{HwIoDesc, HwIoRt, MAX_HW_MIDI_PER_BLOCK};
use ether_core::protocol::model::{ExternalRouting, HwChannels, ParamId};
use ether_core::{EventKind, NodeKey, ProcessEvent};

fn key(i: u32) -> NodeKey {
    NodeKey {
        index: i,
        generation: 1,
    }
}

fn desc(node: u32, send: Option<(u16, u16)>, ret: Option<(u16, u16)>, midi: bool) -> HwIoDesc {
    let ch = |(first, count)| HwChannels { first, count };
    HwIoDesc {
        node: key(node),
        routing: ExternalRouting {
            midi_out: midi.then(|| "port".to_string()),
            midi_channel: 10,
            audio_send: send.map(ch),
            audio_return: ret.map(ch),
        },
    }
}

#[test]
fn returns_read_their_channels_at_the_sub_block_offset() {
    let mut rt = HwIoRt::default();
    rt.prepare(
        &[
            desc(1, None, Some((1, 2)), false),
            desc(2, None, Some((2, 1)), false),
            desc(3, None, Some((3, 2)), false),
            desc(4, None, None, false),
        ],
        64,
    );
    let ins: Vec<Vec<f32>> = (0..4)
        .map(|c| (0..64).map(|i| (c * 100 + i) as f32).collect())
        .collect();
    let refs: Vec<&[f32]> = ins.iter().map(|c| c.as_slice()).collect();
    rt.gather_returns_at(&refs, 10, 16);
    let get = |rt: &mut HwIoRt, n| {
        let [l, r] = rt.entry_mut(key(n)).unwrap().returns(16);
        (l.to_vec(), r.to_vec())
    };
    let (l, r) = get(&mut rt, 1);
    assert_eq!((l[0], r[0], l[15]), (110.0, 210.0, 125.0));
    // Mono feeds both sides.
    let (l, r) = get(&mut rt, 2);
    assert_eq!((l[0], r[0]), (210.0, 210.0));
    // Stereo with its second channel missing: that side is silent.
    let (l, r) = get(&mut rt, 3);
    assert_eq!((l[0], r[0]), (310.0, 0.0));
    let (l, r) = get(&mut rt, 4);
    assert!(l.iter().chain(&r).all(|s| *s == 0.0));
}

#[test]
fn sends_add_to_their_outputs_only_when_written() {
    let mut rt = HwIoRt::default();
    rt.prepare(
        &[
            desc(1, Some((2, 2)), None, false),
            desc(2, Some((1, 1)), None, false),
            desc(3, Some((9, 2)), None, false),
        ],
        32,
    );
    rt.gather_returns(&[], 8);
    {
        let [l, r] = rt.entry_mut(key(1)).unwrap().send_mut(8);
        l.fill(0.5);
        r.fill(-0.5);
    }
    {
        let [l, r] = rt.entry_mut(key(2)).unwrap().send_mut(8);
        l.fill(1.0);
        r.fill(0.0);
    }
    {
        let [l, _] = rt.entry_mut(key(3)).unwrap().send_mut(8);
        l.fill(1.0);
    }
    let mut outs = vec![vec![0.25f32; 16]; 4];
    {
        let mut o: Vec<&mut [f32]> = outs.iter_mut().map(|c| c.as_mut_slice()).collect();
        rt.write_sends_at(&mut o, 4, 8);
    }
    // Added after what was there (master), only in [4, 12).
    assert_eq!(outs[2][3], 0.25);
    assert_eq!(outs[2][4], 0.75);
    assert_eq!(outs[3][11], -0.25);
    assert_eq!(outs[3][12], 0.25);
    // Mono send: the average of both sides.
    assert_eq!(outs[1][5], 0.75);
    assert_eq!(outs[0][5], 0.25);
    // Next sub-block, nothing written (a bypassed device): nothing added.
    rt.gather_returns(&[], 8);
    let mut o2 = vec![vec![0.0f32; 8]; 4];
    {
        let mut o: Vec<&mut [f32]> = o2.iter_mut().map(|c| c.as_mut_slice()).collect();
        rt.write_sends(&mut o, 8);
    }
    assert!(o2.iter().flatten().all(|s| *s == 0.0));
    // `capture_send` copies an input as is.
    rt.capture_send(key(1), &[&[0.1f32; 8]], 8);
    {
        let mut o: Vec<&mut [f32]> = o2.iter_mut().map(|c| c.as_mut_slice()).collect();
        rt.write_sends(&mut o, 8);
    }
    assert_eq!((o2[2][0], o2[3][7]), (0.1, 0.1));
}

#[test]
fn instrument_events_become_midi_on_its_channel() {
    let mut rt = HwIoRt::default();
    rt.prepare(&[desc(5, None, None, true), desc(6, None, None, false)], 32);
    let ev = |offset, kind| ProcessEvent { offset, kind };
    let events = [
        ev(
            0,
            EventKind::NoteOn {
                note_id: 1,
                channel: 0,
                key: 60,
                velocity: 1.0,
            },
        ),
        ev(
            3,
            EventKind::Param {
                param: ParamId(0),
                value: 1.0,
            },
        ),
        ev(
            4,
            EventKind::Midi {
                data: [0xb0, 74, 99],
            },
        ),
        ev(
            5,
            EventKind::NoteOff {
                note_id: 1,
                channel: 0,
                key: 60,
                velocity: 0.0,
            },
        ),
        ev(
            6,
            EventKind::NoteOn {
                note_id: 2,
                channel: 0,
                key: 61,
                velocity: 0.0,
            },
        ),
        ev(7, EventKind::Midi { data: [0xf8, 0, 0] }),
        ev(9, EventKind::AllNotesOff),
    ];
    rt.entry_mut(key(5)).unwrap().push_events(&events, 1000);
    // No MIDI out: nothing staged.
    rt.entry_mut(key(6)).unwrap().push_events(&events, 1000);
    let mut got = Vec::new();
    assert!(!rt.drain_midi(|m| got.push((m.node, m.frame, m.data))));
    let k = key(5);
    assert_eq!(
        got,
        vec![
            (k, 1000, [0x99, 60, 127]),
            (k, 1004, [0xb9, 74, 99]),
            (k, 1005, [0x89, 60, 0]),
            // A zero velocity note-on still sounds (velocity 1).
            (k, 1006, [0x99, 61, 1]),
            (k, 1009, [0xb9, 123, 0]),
        ]
    );
    // Bounded staging: overflow is reported, never allocates.
    let many = vec![ev(0, EventKind::AllNotesOff); MAX_HW_MIDI_PER_BLOCK + 3];
    rt.entry_mut(key(5)).unwrap().push_events(&many, 0);
    let mut n = 0;
    assert!(rt.drain_midi(|_| n += 1));
    assert_eq!(n, MAX_HW_MIDI_PER_BLOCK);
}
