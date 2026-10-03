//! Section edits (owner report, `section-edit` node): copying, pasting and duplicating a time
//! selection keeps the section's EXACT length, gaps included, and copies only the selected
//! span of the clips it crosses (never the whole clip).

mod common;

use common::*;
use ether_core::protocol::Command;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::time_edit::{TimeEditCommand, TimeSelection};
use ether_core::protocol::tracks::TrackCommand;

fn midi_track(h: &mut Harness) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

/// A MIDI clip at `start` of `length` with a short note at each of `notes` (content beats).
fn clip_with_notes(h: &mut Harness, track: TrackId, start: f64, length: f64, notes: &[f64]) -> ClipId {
    let id: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id,
        track,
        start: Beats(start),
        length: Beats(length),
        name: None,
    }));
    let specs = notes
        .iter()
        .map(|&s| NoteSpec {
            id: h.id(),
            pitch: 60,
            velocity: 0.8,
            start: Beats(s),
            duration: Beats(0.5),
        })
        .collect();
    h.ok(Command::Note(NoteCommand::Add { clip: id, notes: specs }));
    id
}

fn sel(start: f64, end: f64, tracks: Vec<TrackId>) -> TimeSelection {
    TimeSelection {
        start: Beats(start),
        end: Beats(end),
        tracks,
        global: false,
    }
}

fn te(h: &mut Harness, c: TimeEditCommand) {
    ok(&h.send(Command::TimeEdit(c)));
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

/// Song positions of the notes that play on `track` (a note plays if it starts inside its
/// clip's window `[offset, offset + length)`; the clips here don't loop), sorted.
fn audible(h: &Harness, track: TrackId) -> Vec<f64> {
    let p = h.project();
    let mut out = Vec::new();
    for c in p.clips.values().filter(|c| c.track == track) {
        assert!(!c.looping.enabled);
        for n in p.notes_of(c.id) {
            let rel = n.start.0 - c.offset.0;
            if rel >= -1e-9 && rel < c.length.0 - 1e-9 {
                out.push(c.start.0 + rel);
            }
        }
    }
    out.sort_by(f64::total_cmp);
    out
}

fn assert_beats(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{got:?} != {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-6, "{got:?} != {want:?}");
    }
}

/// The end of the last clip on `track`.
fn track_end(h: &Harness, track: TrackId) -> f64 {
    h.project()
        .clips
        .values()
        .filter(|c| c.track == track)
        .map(|c| c.start.0 + c.length.0)
        .fold(0.0, f64::max)
}

/// The owner's case: notes at beats 0 and 3 in a 4-beat selection, duplicated 3 times
/// (each time the selection moves onto the copy): the section tiles with its gaps.
#[test]
fn duplicate_section_tiles_its_exact_length_with_gaps() {
    let mut h = Harness::with_project();
    let t = midi_track(&mut h);
    clip_with_notes(&mut h, t, 0.0, 4.0, &[0.0, 3.0]);
    let mut start = 0.0;
    for _ in 0..3 {
        let seed: ClipId = h.id();
        te(
            &mut h,
            TimeEditCommand::DuplicateTime {
                selection: sel(start, start + 4.0, vec![t]),
                seed,
            },
        );
        start += 4.0;
    }
    assert_beats(&audible(&h, t), &[0.0, 3.0, 4.0, 7.0, 8.0, 11.0, 12.0, 15.0]);
    assert!((track_end(&h, t) - 16.0).abs() < 1e-6);
    // Each duplicate is one undo step.
    undo(&mut h);
    assert_beats(&audible(&h, t), &[0.0, 3.0, 4.0, 7.0, 8.0, 11.0]);
}

/// The clip ends at its last note (3.5) but the selection is 4 beats: the gap after the
/// last note is part of the section, so copies never stick to each other.
#[test]
fn duplicate_keeps_the_empty_space_after_the_clip() {
    let mut h = Harness::with_project();
    let t = midi_track(&mut h);
    clip_with_notes(&mut h, t, 0.0, 3.5, &[0.0, 3.0]);
    let mut start = 0.0;
    for _ in 0..3 {
        let seed: ClipId = h.id();
        te(
            &mut h,
            TimeEditCommand::DuplicateTime {
                selection: sel(start, start + 4.0, vec![t]),
                seed,
            },
        );
        start += 4.0;
    }
    assert_beats(&audible(&h, t), &[0.0, 3.0, 4.0, 7.0, 8.0, 11.0, 12.0, 15.0]);
}

/// A section with empty space before its first note (selection 1..5 over notes at 2 and 4).
#[test]
fn duplicate_keeps_the_empty_space_before_the_first_note() {
    let mut h = Harness::with_project();
    let t = midi_track(&mut h);
    clip_with_notes(&mut h, t, 0.0, 5.0, &[2.0, 4.0]);
    for i in 0..2 {
        let s = 1.0 + 4.0 * i as f64;
        let seed: ClipId = h.id();
        te(
            &mut h,
            TimeEditCommand::DuplicateTime {
                selection: sel(s, s + 4.0, vec![t]),
                seed,
            },
        );
    }
    assert_beats(&audible(&h, t), &[2.0, 4.0, 6.0, 8.0, 10.0, 12.0]);
}

/// Copy a section from the middle of a long clip: only the selected span is copied (the
/// clip is cut at the selection edges), and pasting it twice tiles its full length.
#[test]
fn copy_paste_takes_only_the_section_at_its_full_length() {
    let mut h = Harness::with_project();
    let t = midi_track(&mut h);
    clip_with_notes(&mut h, t, 0.0, 8.0, &[0.0, 3.0, 5.0, 7.0]);
    te(
        &mut h,
        TimeEditCommand::Copy {
            selection: sel(2.0, 6.0, vec![t]),
        },
    );
    for at in [8.0, 12.0] {
        let seed: ClipId = h.id();
        te(
            &mut h,
            TimeEditCommand::Paste {
                at: Beats(at),
                tracks: vec![],
                insert: false,
                seed,
            },
        );
    }
    // The pasted sections hold the notes at 3 and 5 only (relative 1 and 3).
    assert_beats(&audible(&h, t), &[0.0, 3.0, 5.0, 7.0, 9.0, 11.0, 13.0, 15.0]);
    assert!((track_end(&h, t) - 16.0).abs() < 1e-6);
    // The source clip is untouched by the copy.
    let src: Vec<_> = h.project().clips.values().filter(|c| c.start.0 == 0.0).collect();
    assert_eq!(src.len(), 1);
    assert!((src[0].length.0 - 8.0).abs() < 1e-6);
    // One paste = one undo step.
    undo(&mut h);
    assert_beats(&audible(&h, t), &[0.0, 3.0, 5.0, 7.0, 9.0, 11.0]);
}

/// A section that ends in empty space past the clips still pastes its whole length (the
/// next paste lands after the gap).
#[test]
fn paste_overwrites_exactly_the_section_length() {
    let mut h = Harness::with_project();
    let t = midi_track(&mut h);
    clip_with_notes(&mut h, t, 0.0, 2.0, &[0.0, 1.0]);
    // Material at 6..10 that a 4-beat paste at 4 overwrites only up to 8.
    clip_with_notes(&mut h, t, 6.0, 4.0, &[0.0, 3.0]);
    te(
        &mut h,
        TimeEditCommand::Copy {
            selection: sel(0.0, 4.0, vec![t]),
        },
    );
    let seed: ClipId = h.id();
    te(
        &mut h,
        TimeEditCommand::Paste {
            at: Beats(4.0),
            tracks: vec![],
            insert: false,
            seed,
        },
    );
    // 0, 1 (source), 4, 5 (paste), 9 (the later clip's note past the pasted range).
    assert_beats(&audible(&h, t), &[0.0, 1.0, 4.0, 5.0, 9.0]);
}
