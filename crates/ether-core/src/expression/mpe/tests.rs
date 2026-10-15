use super::*;
use ether_protocol::model::MpeZone;

fn lower(n: u8) -> MpeSettings {
    MpeSettings {
        member_channels: n,
        ..MpeSettings::default()
    }
}

fn on(note_id: u32, channel: u8, key: u8) -> EventKind {
    EventKind::NoteOn {
        note_id,
        channel,
        key,
        velocity: 1.0,
    }
}

fn off(note_id: u32, channel: u8, key: u8) -> EventKind {
    EventKind::NoteOff {
        note_id,
        channel,
        key,
        velocity: 0.0,
    }
}

fn expr(note_id: u32, key: u8, expression: NoteExpressionKind, value: f32) -> EventKind {
    EventKind::NoteExpression {
        note_id,
        channel: 0,
        key,
        expression,
        value,
    }
}

fn kinds(buf: &EventBuffer) -> Vec<EventKind> {
    buf.as_slice().iter().map(|e| e.kind).collect()
}

#[test]
fn zone_layout() {
    let l = lower(15);
    assert_eq!(master_channel(&l), 0);
    assert_eq!(members(&l), 1..=15);
    assert!(!is_member(&l, 0));
    let u = MpeSettings {
        zone: MpeZone::Upper,
        member_channels: 4,
        ..MpeSettings::default()
    };
    assert_eq!(master_channel(&u), 15);
    assert_eq!(members(&u), 11..=14);
    assert!(is_member(&u, 11) && !is_member(&u, 10) && !is_member(&u, 15));
}

#[test]
fn bend_semitone_conversions_round_trip() {
    assert_eq!(bend_semitones(8192, 48.0), 0.0);
    assert_eq!(bend_semitones(16383, 48.0), 48.0);
    assert_eq!(bend_semitones(1, 48.0), -48.0);
    assert_eq!(bend_semitones(0, 48.0), -48.0);
    assert_eq!(semitones_bend(0.0, 48.0), BEND_CENTRE);
    assert_eq!(semitones_bend(48.0, 48.0), 16383);
    assert_eq!(semitones_bend(100.0, 48.0), 16383);
    for s in [-47.5f32, -12.0, -0.25, 0.5, 7.0, 24.0] {
        let back = bend_semitones(semitones_bend(s, 48.0), 48.0);
        assert!((back - s).abs() <= 48.0 / 8191.0, "{s} {back}");
    }
}

#[test]
fn config_messages_are_the_mpe_configuration() {
    let m = MpeSettings {
        zone: MpeZone::Lower,
        member_channels: 7,
        note_pitch_range: 24.5,
        master_pitch_range: 2.0,
    };
    let c = config_messages(Some(&m), &m);
    // MCM on the master channel (channel 1): RPN 6 = 7 members.
    assert_eq!(&c[..3], &[[0xB0, 101, 0], [0xB0, 100, 6], [0xB0, 6, 7]]);
    // Master pitch-bend sensitivity, then the members' (channel 2) with cents.
    assert_eq!(c[5], [0xB0, 6, 2]);
    assert_eq!(&c[9..11], &[[0xB1, 6, 24], [0xB1, 38, 50]]);
    // A receiver learns all of it.
    let mut out = MpeOut::default();
    assert!(!out.active());
    for d in c {
        out.observe(d);
    }
    assert!(out.active());
    assert_eq!(out.note_range(), 24.5);
    // Off: an empty zone.
    for d in config_messages(None, &m) {
        out.observe(d);
    }
    assert!(!out.active());
}

#[test]
fn live_member_channels_become_note_expressions() {
    let m = lower(15);
    let mut input = MpeIn::default();
    let mut buf = EventBuffer::with_capacity(64);
    // Initial state before the note-on (held, sent after it).
    input.translate(
        &m,
        0,
        EventKind::Midi {
            data: [0xE2, 0x7F, 0x7F],
        },
        &mut buf,
    );
    input.translate(
        &m,
        0,
        EventKind::Midi {
            data: [0xB2, 74, 127],
        },
        &mut buf,
    );
    assert!(buf.as_slice().is_empty());
    input.translate(&m, 1, on(77, 2, 60), &mut buf);
    // Master channel bend and other controllers pass through.
    input.translate(
        &m,
        2,
        EventKind::Midi {
            data: [0xE0, 0, 0x40],
        },
        &mut buf,
    );
    input.translate(&m, 2, EventKind::Midi { data: [0xB2, 1, 5] }, &mut buf);
    input.translate(
        &m,
        3,
        EventKind::Midi {
            data: [0xD2, 64, 0],
        },
        &mut buf,
    );
    // Another channel without a note: held, nothing sent.
    input.translate(
        &m,
        3,
        EventKind::Midi {
            data: [0xD3, 64, 0],
        },
        &mut buf,
    );
    input.translate(&m, 4, off(77, 2, 60), &mut buf);
    // After the note-off the channel's expression is no longer the note's.
    input.translate(&m, 5, EventKind::Midi { data: [0xD2, 1, 0] }, &mut buf);
    let ne = |expression, value| EventKind::NoteExpression {
        note_id: 77,
        channel: 2,
        key: 60,
        expression,
        value,
    };
    assert_eq!(
        kinds(&buf),
        vec![
            on(77, 2, 60),
            ne(NoteExpressionKind::Pitch, 48.0),
            ne(NoteExpressionKind::Timbre, 1.0),
            EventKind::Midi {
                data: [0xE0, 0, 0x40]
            },
            EventKind::Midi { data: [0xB2, 1, 5] },
            ne(NoteExpressionKind::Pressure, 64.0 / 127.0),
            off(77, 2, 60),
        ]
    );
    let offsets: Vec<u32> = buf.as_slice().iter().map(|e| e.offset).collect();
    assert_eq!(offsets, vec![1, 1, 1, 2, 2, 3, 4]);
}

#[test]
fn mpe_out_gives_each_note_its_member_channel() {
    let m = lower(3);
    let mut out = MpeOut::default();
    let mut got = Vec::new();
    for d in config_messages(Some(&m), &m) {
        out.translate(&EventKind::Midi { data: d }, |e| got.push(e));
    }
    assert_eq!(
        got.len(),
        CONFIG_MESSAGES,
        "the configuration reaches the plugin"
    );
    got.clear();
    for (id, key) in [(1, 60), (2, 64), (3, 67)] {
        out.translate(&on(id, 0, key), |e| got.push(e));
    }
    out.translate(&expr(2, 64, NoteExpressionKind::Pitch, 12.0), |e| {
        got.push(e)
    });
    out.translate(&expr(3, 67, NoteExpressionKind::Pressure, 1.0), |e| {
        got.push(e)
    });
    out.translate(&expr(1, 60, NoteExpressionKind::Timbre, 0.5), |e| {
        got.push(e)
    });
    // Unknown note: dropped.
    out.translate(&expr(9, 61, NoteExpressionKind::Timbre, 0.5), |e| {
        got.push(e)
    });
    out.translate(&off(2, 0, 64), |e| got.push(e));
    let q = semitones_bend(12.0, 48.0);
    assert_eq!(
        got,
        vec![
            on(1, 1, 60),
            on(2, 2, 64),
            on(3, 3, 67),
            EventKind::Midi {
                data: [0xE2, (q & 0x7F) as u8, (q >> 7) as u8]
            },
            EventKind::Midi {
                data: [0xD3, 127, 0]
            },
            EventKind::Midi {
                data: [0xB1, 74, 64]
            },
            off(2, 2, 64),
        ]
    );
    got.clear();
    // The freed (and bent) channel is reused and re-centred first.
    out.translate(&on(4, 0, 50), |e| got.push(e));
    assert_eq!(
        got,
        vec![
            EventKind::Midi {
                data: [0xE2, 0, 0x40]
            },
            EventKind::Midi { data: [0xD2, 0, 0] },
            on(4, 2, 50),
        ]
    );
    // All busy: the least recently used channel is shared.
    got.clear();
    out.translate(&on(5, 0, 51), |e| got.push(e));
    assert_eq!(got, vec![on(5, 1, 51)]);
}

#[test]
fn without_mpe_pressure_is_poly_aftertouch_and_the_rest_is_dropped() {
    let mut out = MpeOut::default();
    let mut got = Vec::new();
    out.translate(&on(1, 0, 60), |e| got.push(e));
    out.translate(&expr(1, 60, NoteExpressionKind::Pressure, 1.0), |e| {
        got.push(e)
    });
    out.translate(&expr(1, 60, NoteExpressionKind::Pitch, 3.0), |e| {
        got.push(e)
    });
    out.translate(&expr(1, 60, NoteExpressionKind::Timbre, 1.0), |e| {
        got.push(e)
    });
    out.translate(&off(1, 0, 60), |e| got.push(e));
    assert_eq!(
        got,
        vec![
            on(1, 0, 60),
            EventKind::Midi {
                data: [0xA0, 60, 127]
            },
            off(1, 0, 60),
        ]
    );
}
