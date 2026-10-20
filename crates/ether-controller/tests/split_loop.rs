//! Cutting a looping clip after its first loop pass: the part right of the cut gets
//! its offset wrapped into the loop, so it keeps playing what the whole clip played there
//! (split, time-edit split / delete time, and the overlap trims of clip edits). Checked
//! through the real engine: the notes played before and after the edit.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use ether_core::graph::ChainEntry;
use ether_core::offline::OfflineRenderer;
use ether_core::protocol::Command;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::time_edit::{TimeEditCommand, TimeSelection};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::{
    AudioBuffers, EngineConfig, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

const SR: u32 = 48_000;
/// 120 BPM.
const BEAT: u64 = 24_000;

type Log = Arc<Mutex<Vec<(u64, u8)>>>;

/// Logs every note-on (sample, key).
struct Recorder(Log);

impl Node for Recorder {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        _: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            if let EventKind::NoteOn { key, .. } = e.kind {
                self.0
                    .lock()
                    .unwrap()
                    .push((ctx.transport.sample_time + u64::from(e.offset), key));
            }
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

struct Fx {
    h: Harness,
    track: TrackId,
    clip: ClipId,
}

/// A MIDI clip at 0, length 32, looping [0, 16), notes at 0, 4, 8 and 12 (pitches 60..63).
fn fx() -> Fx {
    let mut h = Harness::with_project();
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let clip: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: clip,
        track,
        start: Beats(0.0),
        length: Beats(32.0),
        name: None,
    }));
    let notes = (0..4)
        .map(|i| NoteSpec {
            id: h.id(),
            pitch: 60 + i as u8,
            velocity: 0.8,
            start: Beats(4.0 * i as f64),
            duration: Beats(1.0),
        })
        .collect();
    h.ok(Command::Note(NoteCommand::Add { clip, notes }));
    h.ok(Command::Clip(ClipCommand::SetLoop {
        id: clip,
        looping: ClipLoop {
            enabled: true,
            start: Beats(0.0),
            end: Beats(16.0),
        },
    }));
    Fx { h, track, clip }
}

impl Fx {
    /// The note-ons (beat, key) the track plays over beats 0..40, through the real engine.
    fn played(&mut self) -> Vec<(f64, u8)> {
        self.h.advance(100);
        self.h.tick();
        let mut g = self.h.ctl.bridge.last_graph().clone();
        let log: Log = Arc::default();
        let mut r = OfflineRenderer::new(EngineConfig {
            sample_rate: SR,
            max_block_size: 512,
            ..EngineConfig::default()
        });
        let rec = r
            .handle()
            .add_node(Box::new(Recorder(log.clone())))
            .unwrap();
        for t in &mut g.tracks {
            t.chain.clear();
            if t.id == self.track {
                t.chain = vec![ChainEntry {
                    node: rec,
                    enabled: true,
                    sidechain: None,
                }];
            }
        }
        r.publish(g).unwrap();
        r.start(Beats(0.0)).unwrap();
        let frames = 40 * BEAT as usize;
        let (mut l, mut rr) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        r.render(frames, &mut [&mut l, &mut rr]);
        log.lock()
            .unwrap()
            .iter()
            .map(|&(t, k)| (t as f64 / BEAT as f64, k))
            .collect()
    }

    /// The track's clips as sorted (start, length, offset).
    fn spans(&self) -> Vec<(f64, f64, f64)> {
        let mut v: Vec<_> = self
            .h
            .project()
            .clips
            .values()
            .filter(|c| c.track == self.track)
            .map(|c| (c.start.0, c.length.0, c.offset.0))
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    }
}

/// The unsplit clip: two passes of the loop.
fn whole() -> Vec<(f64, u8)> {
    (0..8)
        .map(|i| (4.0 * i as f64, 60 + (i % 4) as u8))
        .collect()
}

#[test]
fn splitting_a_looping_clip_after_its_first_pass_keeps_what_plays() {
    let mut f = fx();
    assert_eq!(f.played(), whole());
    let right: ClipId = f.h.id();
    f.h.ok(Command::Clip(ClipCommand::Split {
        id: f.clip,
        at: Beats(20.0),
        new_id: right,
    }));
    // The right part starts 4 beats into the loop.
    assert_eq!(f.spans(), vec![(0.0, 20.0, 0.0), (20.0, 12.0, 4.0)]);
    assert_eq!(f.played(), whole());
}

#[test]
fn time_edit_split_and_delete_time_keep_what_plays() {
    let mut f = fx();
    let seed: ClipId = f.h.id();
    f.h.ok(Command::TimeEdit(TimeEditCommand::Split {
        tracks: vec![],
        at: Beats(20.0),
        seed,
    }));
    assert_eq!(f.spans(), vec![(0.0, 20.0, 0.0), (20.0, 12.0, 4.0)]);
    assert_eq!(f.played(), whole());

    // Delete beats 17..19 (from the left part): what played after 19 moves 2 beats earlier.
    let mut f = fx();
    let seed: ClipId = f.h.id();
    f.h.ok(Command::TimeEdit(TimeEditCommand::DeleteTime {
        selection: TimeSelection {
            start: Beats(17.0),
            end: Beats(19.0),
            tracks: vec![],
            global: false,
        },
        seed,
    }));
    let want: Vec<(f64, u8)> = whole()
        .into_iter()
        .filter(|&(b, _)| !(17.0..19.0).contains(&b))
        .map(|(b, k)| (if b >= 19.0 { b - 2.0 } else { b }, k))
        .collect();
    assert_eq!(f.played(), want);
}

#[test]
fn clips_dropped_onto_a_looping_clip_trim_it_without_losing_the_loop() {
    // A 4-beat clip duplicated onto beats 16..20 cuts the looping clip in two; a 20-beat clip
    // duplicated onto 0..20 trims its start. What still shows plays as before.
    for (len, at) in [(4.0, 16.0), (20.0, 0.0)] {
        let mut f = fx();
        let other: ClipId = f.h.id();
        f.h.ok(Command::Clip(ClipCommand::CreateMidi {
            id: other,
            track: f.track,
            start: Beats(40.0),
            length: Beats(len),
            name: None,
        }));
        let copy: ClipId = f.h.id();
        f.h.ok(Command::Clip(ClipCommand::Duplicate {
            id: other,
            new_id: copy,
            start: Some(Beats(at)),
        }));
        let want: Vec<(f64, u8)> = whole()
            .into_iter()
            .filter(|&(b, _)| !(at..at + len).contains(&b))
            .collect();
        assert_eq!(f.played(), want, "a {len}-beat clip at {at}");
    }
}
