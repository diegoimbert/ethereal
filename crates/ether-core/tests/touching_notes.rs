//! Touching notes: a note ending on the sample the next one starts must reach
//! the instrument as note-off *then* note-on (no false legato / overlap in mono synths, and
//! a same-pitch repeat retriggers), whatever the block size, and across clip boundaries,
//! content loops and the transport loop wrap.

mod common;

use common::*;
use ether_core::graph::ClipDesc;
use ether_core::protocol::model::TrackKind;
use ether_core::{EventKind, RenderGraphDesc, TransportControl, create};

/// Block sizes: dividing the boundaries (on a block edge) and not (mid-block).
const BLOCKS: [usize; 5] = [7, 64, 500, 509, 512];

/// `(time, key, is_on)` of every note event the track's instrument saw, in delivery order.
fn note_events(
    clips: Vec<ClipDesc>,
    looped: Option<(f64, f64)>,
    frames: usize,
    block: usize,
) -> Vec<(u64, u8, bool)> {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = clips;
    let mut d = RenderGraphDesc {
        version: 1,
        tracks: vec![master(), t],
        ..Default::default()
    };
    if let Some((s, e)) = looped {
        d.loop_enabled = true;
        d.loop_start = s;
        d.loop_end = e;
    }
    p.handle.publish(d).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, frames, block);
    let ev = events(&drain(&mut rx));
    // No note-on may precede, on the same sample, the note-off of another note.
    for (i, a) in ev.iter().enumerate() {
        if let EventKind::NoteOn { note_id, .. } = a.1 {
            for b in ev[i + 1..].iter().take_while(|b| b.0 == a.0) {
                if let EventKind::NoteOff { note_id: off, .. } = b.1 {
                    assert_eq!(off, note_id, "block {block}: on before off at {}", a.0);
                }
            }
        }
    }
    ev.iter()
        .filter_map(|(t, k)| match *k {
            EventKind::NoteOn { key, .. } => Some((*t, key, true)),
            EventKind::NoteOff { key, .. } => Some((*t, key, false)),
            _ => None,
        })
        .collect()
}

#[test]
fn touching_notes_in_a_clip_end_before_the_next_starts() {
    // 120 BPM @ 48 kHz: 24000 samples per beat. Different pitch, then the same pitch.
    let clip = || {
        midi_clip(
            cid(1),
            0.0,
            8.0,
            &[(1.0, 0.5, 60), (1.5, 0.5, 64), (2.0, 0.5, 64)],
        )
    };
    for block in BLOCKS {
        assert_eq!(
            note_events(vec![clip()], None, 72_000, block),
            [
                (24_000, 60, true),
                (36_000, 60, false),
                (36_000, 64, true),
                (48_000, 64, false),
                (48_000, 64, true),
                (60_000, 64, false),
            ],
            "block {block}"
        );
    }
}

#[test]
fn short_touching_notes_inside_one_block() {
    // 1/256-beat notes (~94 samples): both edges inside one 512-sample block.
    let d = 1.0 / 256.0;
    let clip = midi_clip(
        cid(1),
        0.0,
        8.0,
        &[(1.0, d, 60), (1.0 + d, d, 62), (1.0 + 2.0 * d, d, 62)],
    );
    // Edges at 24000 + 93.75·k samples: floored.
    let ev = note_events(vec![clip], None, 30_000, 512);
    assert_eq!(
        ev,
        [
            (24_000, 60, true),
            (24_093, 60, false),
            (24_093, 62, true),
            (24_187, 62, false),
            (24_187, 62, true),
            (24_281, 62, false),
        ]
    );
}

#[test]
fn touching_notes_across_adjacent_clips() {
    for key2 in [62, 60] {
        let clips = || {
            vec![
                midi_clip(cid(1), 0.0, 2.0, &[(1.0, 1.0, 60)]),
                midi_clip(cid(2), 2.0, 2.0, &[(0.0, 1.0, key2)]),
            ]
        };
        for block in BLOCKS {
            assert_eq!(
                note_events(clips(), None, 96_000, block),
                [
                    (24_000, 60, true),
                    (48_000, 60, false),
                    (48_000, key2, true),
                    (72_000, key2, false),
                ],
                "block {block}"
            );
        }
    }
}

#[test]
fn looped_clip_content_retriggers_a_full_length_note() {
    let clip = || {
        let mut c = midi_clip(cid(1), 0.0, 3.0, &[(0.0, 1.0, 60)]);
        c.looping = Some((0.0, 1.0));
        c
    };
    for block in BLOCKS {
        assert_eq!(
            note_events(vec![clip()], None, 80_000, block),
            [
                (0, 60, true),
                (24_000, 60, false),
                (24_000, 60, true),
                (48_000, 60, false),
                (48_000, 60, true),
                (72_000, 60, false),
            ],
            "block {block}"
        );
    }
}

#[test]
fn note_ending_at_the_loop_end_releases_before_the_loop_start_note() {
    for key2 in [62, 60] {
        let clip = || midi_clip(cid(1), 0.0, 8.0, &[(0.0, 0.5, key2), (1.0, 1.0, 60)]);
        for block in BLOCKS {
            assert_eq!(
                note_events(vec![clip()], Some((0.0, 2.0)), 55_000, block),
                [
                    (0, key2, true),
                    (12_000, key2, false),
                    (24_000, 60, true),
                    (48_000, 60, false),
                    (48_000, key2, true),
                ],
                "block {block}"
            );
        }
    }
}
