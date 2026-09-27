//! Takes and comping (v0.2, `comping`, CONTRACTS.md §12.2): lane CRUD, swipe comping
//! (`SetComp` trims / splits / removes regions), comp playback (pieces trimmed to regions,
//! equal-power boundary crossfades, MIDI cut exactly), flatten (derived ids, plays like the
//! comp), one undo step per command.

mod common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::graph::{ClipContentDesc, ClipDesc};
use ether_core::protocol::clips::{ClipCommand, ClipMove};
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::takes::TakeCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

const SR: u32 = 44_100;
/// 2.5 s = 5 beats at 120 BPM.
const FRAMES: usize = 110_250;
/// Half of the default 5 ms crossfade at 120 BPM, in beats.
const HALF_XF: f64 = 0.005;

struct Fx {
    h: Harness,
    track: TrackId,
    media: MediaId,
}

fn fx(kind: TrackKind) -> Fx {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    lib.add_file("lib", "take.wav", wav(SR, &[sine(SR, 440.0, FRAMES, 0.5)]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Takes");
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "take.wav".into(),
        },
    }));
    let track = new_track(&mut h, kind);
    Fx { h, track, media }
}

fn new_track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn take(c: TakeCommand) -> Command {
    Command::Take(c)
}

impl Fx {
    fn lane(&mut self, name: Option<&str>) -> TakeLaneId {
        let id: TakeLaneId = self.h.id();
        self.h.ok(take(TakeCommand::CreateLane {
            id,
            track: self.track,
            name: name.map(str::to_owned),
            before: None,
        }));
        id
    }

    /// A take clip on `lane` at `[start, start + length)` playing content from `offset`.
    fn take_clip(&mut self, lane: TakeLaneId, start: f64, length: f64, offset: f64) -> ClipId {
        let id: ClipId = self.h.id();
        let kind = self.h.project().tracks[&self.track].kind;
        if kind == TrackKind::Audio {
            self.h.ok(Command::Clip(ClipCommand::CreateAudio {
                id,
                track: self.track,
                start: Beats(start),
                media: self.media,
            }));
        } else {
            self.h.ok(Command::Clip(ClipCommand::CreateMidi {
                id,
                track: self.track,
                start: Beats(start),
                length: Beats(length),
                name: None,
            }));
        }
        self.h.ok(take(TakeCommand::MoveToLane {
            clips: vec![id],
            lane: Some(lane),
        }));
        self.h.ok(Command::Clip(ClipCommand::SetBounds {
            id,
            start: Beats(start),
            length: Beats(length),
            offset: Beats(offset),
        }));
        id
    }

    fn comp(&mut self, lane: TakeLaneId, start: f64, end: f64) -> (CompRegionId, CompRegionId) {
        let (id, split_id): (CompRegionId, CompRegionId) = (self.h.id(), self.h.id());
        self.h.ok(take(TakeCommand::SetComp {
            id,
            split_id,
            track: self.track,
            lane,
            start: Beats(start),
            end: Beats(end),
        }));
        (id, split_id)
    }

    /// `(lane, start, end)` of the track's regions, by start.
    fn regions(&self) -> Vec<(TakeLaneId, f64, f64)> {
        self.h
            .project()
            .comp_of(self.track)
            .iter()
            .map(|r| (r.lane, r.start.0, r.end.0))
            .collect()
    }

    fn graph_clips(&mut self) -> Vec<ClipDesc> {
        self.h.tick();
        let g = self.h.ctl.bridge.last_graph();
        g.tracks
            .iter()
            .find(|t| t.id == self.track)
            .unwrap()
            .clips
            .clone()
    }

    fn undo(&mut self) {
        self.h.ok(Command::Edit(EditCommand::Undo));
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn fades(d: &ClipDesc) -> (f64, f64, FadeCurve, FadeCurve) {
    match &d.content {
        ClipContentDesc::Audio {
            fade_in,
            fade_out,
            fade_in_curve,
            fade_out_curve,
            ..
        } => (*fade_in, *fade_out, *fade_in_curve, *fade_out_curve),
        ClipContentDesc::Midi { .. } => panic!("midi"),
    }
}

#[test]
fn lanes_are_named_ordered_renamed_and_removed_one_undo_step_each() {
    let mut f = fx(TrackKind::Audio);
    let a = f.lane(None);
    let b = f.lane(None);
    let c = f.lane(Some("Best"));
    let names = |f: &Fx| -> Vec<String> {
        f.h.project()
            .lanes_of(f.track)
            .iter()
            .map(|l| l.name.clone())
            .collect()
    };
    assert_eq!(names(&f), ["Take 1", "Take 2", "Best"]);
    // Idempotent create.
    f.h.ok(take(TakeCommand::CreateLane {
        id: a,
        track: f.track,
        name: Some("Other".into()),
        before: None,
    }));
    assert_eq!(f.h.project().take_lanes.len(), 3);
    f.h.ok(take(TakeCommand::MoveLane {
        id: c,
        before: Some(a),
    }));
    assert_eq!(names(&f), ["Best", "Take 1", "Take 2"]);
    f.h.ok(take(TakeCommand::RenameLane {
        id: b,
        name: " Keeper ".into(),
    }));
    assert_eq!(names(&f), ["Best", "Take 1", "Keeper"]);
    let out = f.h.send(take(TakeCommand::RenameLane {
        id: b,
        name: "  ".into(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    f.h.ok(take(TakeCommand::SetLaneColor {
        id: b,
        color: Some(Color(0x336699)),
    }));
    assert_eq!(f.h.project().take_lanes[&b].color, Some(Color(0x336699)));
    f.undo();
    assert_eq!(f.h.project().take_lanes[&b].color, None);
    f.undo();
    assert_eq!(names(&f), ["Best", "Take 1", "Take 2"]);
    f.undo();
    assert_eq!(names(&f), ["Take 1", "Take 2", "Best"]);

    // Removing a lane takes its clips and the regions selecting it along.
    let clip = f.take_clip(a, 0.0, 4.0, 0.0);
    f.comp(a, 0.0, 4.0);
    f.comp(b, 4.0, 8.0);
    f.h.ok(take(TakeCommand::RemoveLane { id: a }));
    assert!(!f.h.project().clips.contains_key(&clip));
    assert_eq!(f.regions(), vec![(b, 4.0, 8.0)]);
    f.undo();
    assert!(f.h.project().clips.contains_key(&clip));
    assert_eq!(f.regions(), vec![(a, 0.0, 4.0), (b, 4.0, 8.0)]);

    // Only audio/MIDI tracks have lanes.
    let group = new_track(&mut f.h, TrackKind::Group);
    let id: TakeLaneId = f.h.id();
    let out = f.h.send(take(TakeCommand::CreateLane {
        id,
        track: group,
        name: None,
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn swipes_trim_split_and_replace_regions() {
    let mut f = fx(TrackKind::Audio);
    let a = f.lane(None);
    let b = f.lane(None);
    f.comp(a, 0.0, 8.0);
    // A swipe inside a region splits it; the right part gets split_id.
    let (inner, split) = f.comp(b, 2.0, 4.0);
    assert_eq!(
        f.regions(),
        vec![(a, 0.0, 2.0), (b, 2.0, 4.0), (a, 4.0, 8.0)]
    );
    let p = f.h.project();
    assert!(p.comp_regions.contains_key(&inner) && p.comp_regions.contains_key(&split));
    assert_eq!(p.comp_regions[&inner].crossfade, DEFAULT_COMP_CROSSFADE);
    // Same-lane neighbours are not merged.
    f.comp(a, 2.0, 3.0);
    assert_eq!(
        f.regions(),
        vec![(a, 0.0, 2.0), (a, 2.0, 3.0), (b, 3.0, 4.0), (a, 4.0, 8.0)]
    );
    // One undo step per swipe, restoring exactly.
    f.undo();
    assert_eq!(
        f.regions(),
        vec![(a, 0.0, 2.0), (b, 2.0, 4.0), (a, 4.0, 8.0)]
    );
    // Covering swipes remove regions, partial ones trim them.
    f.comp(b, 1.0, 6.0);
    assert_eq!(
        f.regions(),
        vec![(a, 0.0, 1.0), (b, 1.0, 6.0), (a, 6.0, 8.0)]
    );
    // ClearComp leaves a hole.
    let split_id: CompRegionId = f.h.id();
    f.h.ok(take(TakeCommand::ClearComp {
        track: f.track,
        start: Beats(2.0),
        end: Beats(3.0),
        split_id,
    }));
    assert_eq!(
        f.regions(),
        vec![(a, 0.0, 1.0), (b, 1.0, 2.0), (b, 3.0, 6.0), (a, 6.0, 8.0)]
    );

    // Errors: a lane of another track, empty ranges.
    let other = new_track(&mut f.h, TrackKind::Audio);
    let (id, split_id): (CompRegionId, CompRegionId) = (f.h.id(), f.h.id());
    let out = f.h.send(take(TakeCommand::SetComp {
        id,
        split_id,
        track: other,
        lane: a,
        start: Beats(0.0),
        end: Beats(1.0),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = f.h.send(take(TakeCommand::SetComp {
        id,
        split_id,
        track: f.track,
        lane: a,
        start: Beats(3.0),
        end: Beats(3.0),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn comp_plays_region_pieces_with_equal_power_crossfades() {
    let mut f = fx(TrackKind::Audio);
    let a = f.lane(None);
    let b = f.lane(None);
    let ca = f.take_clip(a, 0.0, 4.0, 0.0);
    let cb = f.take_clip(b, 0.0, 4.0, 0.0);
    // Take clips alone are silent.
    assert!(f.graph_clips().is_empty());
    let (ra, _) = f.comp(a, 0.0, 2.0);
    f.comp(b, 2.0, 4.0);
    let clips = f.graph_clips();
    assert_eq!(clips.len(), 2, "{clips:#?}");
    assert!(
        clips.iter().all(|c| c.id != ca && c.id != cb),
        "pieces have their own ids"
    );
    let (x, y) = (&clips[0], &clips[1]);
    // A: [0, 2 + half) with an outer fade-in and the crossfade out.
    assert!(close(x.start, 0.0) && close(x.length, 2.0 + HALF_XF) && close(x.offset, 0.0));
    let (fi, fo, ci, co) = fades(x);
    assert!(close(fi, HALF_XF) && close(fo, 2.0 * HALF_XF), "{fi} {fo}");
    assert_eq!((ci, co), (FadeCurve::EqualPower, FadeCurve::EqualPower));
    // B: [2 - half, 4), content from 2 - half.
    assert!(close(y.start, 2.0 - HALF_XF) && close(y.length, 2.0 + HALF_XF));
    assert!(close(y.offset, 2.0 - HALF_XF));
    let (fi, fo, ..) = fades(y);
    assert!(close(fi, 2.0 * HALF_XF) && close(fo, HALF_XF), "{fi} {fo}");

    // A longer crossfade (seconds) on the later region widens the overlap.
    let second: CompRegionId = f.h.project().comp_of(f.track)[1].id;
    f.h.ok(take(TakeCommand::SetCrossfade {
        region: second,
        crossfade: Seconds(0.1),
    }));
    let clips = f.graph_clips();
    assert!(close(clips[0].start + clips[0].length, 2.1), "{clips:#?}");
    assert!(close(clips[1].start, 1.9));
    let out = f.h.send(take(TakeCommand::SetCrossfade {
        region: ra,
        crossfade: Seconds(0.6),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    // Main-lane clips still play next to the comp.
    let main: ClipId = f.h.id();
    f.h.ok(Command::Clip(ClipCommand::CreateAudio {
        id: main,
        track: f.track,
        start: Beats(8.0),
        media: f.media,
    }));
    let clips = f.graph_clips();
    assert_eq!(clips.len(), 3);
    assert_eq!(clips[2].id, main);
    // Main-lane clips don't trim take clips (and vice versa).
    f.h.ok(Command::Clip(ClipCommand::Move {
        moves: vec![ClipMove {
            id: main,
            track: f.track,
            start: Beats(0.0),
        }],
    }));
    assert!(close(f.h.project().clips[&ca].length.0, 4.0));
}

#[test]
fn midi_comp_cuts_exactly_and_flattens_with_derived_ids() {
    let mut f = fx(TrackKind::Midi);
    let a = f.lane(None);
    let b = f.lane(None);
    let ca = f.take_clip(a, 0.0, 4.0, 0.0);
    let cb = f.take_clip(b, 0.0, 4.0, 0.0);
    for (clip, pitch) in [(ca, 60u8), (cb, 72u8)] {
        let notes = (0..4)
            .map(|i| NoteSpec {
                id: f.h.id(),
                pitch,
                velocity: 0.8,
                start: Beats(f64::from(i)),
                duration: Beats(0.5),
            })
            .collect();
        f.h.ok(Command::Note(NoteCommand::Add { clip, notes }));
    }
    f.comp(a, 0.0, 2.0);
    f.comp(b, 2.0, 4.0);
    let clips = f.graph_clips();
    assert_eq!(clips.len(), 2);
    assert!(close(clips[0].start, 0.0) && close(clips[0].length, 2.0));
    assert!(close(clips[1].start, 2.0) && close(clips[1].offset, 2.0));
    let before: Vec<(f64, f64, f64)> = clips
        .iter()
        .map(|c| (c.start, c.length, c.offset))
        .collect();

    let (seed, seed_notes): (ClipId, ClipId) = (f.h.id(), f.h.id());
    f.h.ok(take(TakeCommand::Flatten {
        track: f.track,
        seed,
        seed_notes,
        keep_lanes: true,
    }));
    let p = f.h.project();
    let main = p.arrangement_clips_of(f.track);
    let ids: Vec<ClipId> = main.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![derive_id(seed, 0), derive_id(seed, 1)]);
    // Notes starting in each piece, ids in (clip, start, pitch) order.
    let n0: Vec<(u8, f64, NoteId)> = p
        .notes_of(ids[0])
        .iter()
        .map(|n| (n.pitch, n.start.0, n.id))
        .collect();
    assert_eq!(
        n0,
        vec![
            (60, 0.0, derive_id(seed_notes, 0)),
            (60, 1.0, derive_id(seed_notes, 1))
        ]
    );
    let n1: Vec<(u8, f64)> = p
        .notes_of(ids[1])
        .iter()
        .map(|n| (n.pitch, n.start.0))
        .collect();
    assert_eq!(n1, vec![(72, 2.0), (72, 3.0)]);
    assert!(p.comp_of(f.track).is_empty());
    assert_eq!(p.lanes_of(f.track).len(), 2, "lanes kept");
    // It plays exactly like the comp did.
    let after: Vec<(f64, f64, f64)> = f
        .graph_clips()
        .iter()
        .map(|c| (c.start, c.length, c.offset))
        .collect();
    assert_eq!(before, after);
    // One undo step.
    f.undo();
    let p = f.h.project();
    assert!(p.arrangement_clips_of(f.track).is_empty());
    assert_eq!(p.comp_of(f.track).len(), 2);

    // Without keeping the lanes, they go with their clips.
    f.h.ok(take(TakeCommand::Flatten {
        track: f.track,
        seed,
        seed_notes,
        keep_lanes: false,
    }));
    let p = f.h.project();
    assert!(p.take_lanes.is_empty());
    assert!(!p.clips.contains_key(&ca) && !p.clips.contains_key(&cb));
    assert_eq!(p.arrangement_clips_of(f.track).len(), 2);
}

#[test]
fn audio_flatten_bakes_the_crossfades_into_clip_fades() {
    let mut f = fx(TrackKind::Audio);
    let a = f.lane(None);
    let b = f.lane(None);
    f.take_clip(a, 0.0, 4.0, 0.0);
    f.take_clip(b, 0.0, 4.0, 0.0);
    f.comp(a, 0.0, 2.0);
    f.comp(b, 2.0, 4.0);
    let mut before = f.graph_clips();
    let (seed, seed_notes): (ClipId, ClipId) = (f.h.id(), f.h.id());
    f.h.ok(take(TakeCommand::Flatten {
        track: f.track,
        seed,
        seed_notes,
        keep_lanes: true,
    }));
    let mut after = f.graph_clips();
    for c in before.iter_mut().chain(after.iter_mut()) {
        c.id = ClipId::NIL;
    }
    assert_eq!(before, after);
    // The two main-lane clips overlap as a crossfade (clip-editing's rule).
    let p = f.h.project();
    let main = p.arrangement_clips_of(f.track);
    assert_eq!(main.len(), 2);
    assert!(main[0].start.0 + main[0].length.0 > main[1].start.0);
}

#[test]
fn moving_clips_between_lanes_stays_on_the_track() {
    let mut f = fx(TrackKind::Audio);
    let a = f.lane(None);
    let clip = f.take_clip(a, 0.0, 2.0, 0.0);
    assert_eq!(f.h.project().clips[&clip].lane, Some(a));
    f.h.ok(take(TakeCommand::MoveToLane {
        clips: vec![clip],
        lane: None,
    }));
    assert_eq!(f.h.project().clips[&clip].lane, None);
    f.undo();
    assert_eq!(f.h.project().clips[&clip].lane, Some(a));
    let other = new_track(&mut f.h, TrackKind::Audio);
    let foreign: TakeLaneId = f.h.id();
    f.h.ok(take(TakeCommand::CreateLane {
        id: foreign,
        track: other,
        name: None,
        before: None,
    }));
    let out = f.h.send(take(TakeCommand::MoveToLane {
        clips: vec![clip],
        lane: Some(foreign),
    }));
    assert_ne!(err(&out).code, ErrorCode::Unsupported);
    assert_eq!(f.h.project().clips[&clip].lane, Some(a));
}
