//! Graph compile and delete cost on large synthetic projects, driven
//! through the real controller with the fake bridge.
//!
//! Default runs use one small size (16k notes) and only check deterministic properties
//! (graph-neutral edits don't republish). The timings are printed; the full table of the
//! audit comes from a release run with more sizes:
//!
//! ```text
//! ETHER_PERF_NOTES=16,64,256 cargo test --release -p ether-controller --test it compile_perf:: -- --nocapture
//! ```
//!
//! (`ETHER_PERF_NOTES` = notes per clip; 32 tracks × 32 clips, so 16 → 16k notes, 64 → 65k,
//! 256 → 262k.)

use crate::common;

use std::time::{Duration, Instant};

use common::*;
use ether_controller::compile_graph;
use ether_controller::store::ProjectStore;
use ether_core::protocol::Command;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteEdit};
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;

const TRACKS: usize = 32;
const CLIPS: usize = 32;

/// 32 MIDI tracks × 32 clips × `notes` notes, a synth on each track, a return track fed by
/// every track, volume automation (64 points) on every track and a clip envelope (8 points)
/// on every fourth clip.
fn synthetic(ids: &mut IdGen, notes: usize) -> Project {
    let now = 1_700_000_000_000u64;
    let mut p = Project::new(ids, now);
    let mut ops = Vec::new();
    let orders = OrderKey::n_between(None, None, TRACKS + 1);
    let track = |id, kind, order: &OrderKey| Track {
        vca: Default::default(),
        freeze: None,
        id,
        kind,
        name: "T".into(),
        color: Color(0x123456),
        order: order.clone(),
        parent: None,
        mixer: TrackMixer::default(),
        input: TrackInput::None,
        output: TrackOutput::Default,
        monitor: MonitorMode::Auto,
        scale: Default::default(),
        mpe: None,
    };
    let ret: TrackId = ids.next(now);
    ops.push(Op::Insert {
        entity: Entity::Track(track(ret, TrackKind::Return, &orders[TRACKS])),
    });
    for (ti, order) in orders.iter().take(TRACKS).enumerate() {
        let t: TrackId = ids.next(now);
        ops.push(Op::Insert {
            entity: Entity::Track(track(t, TrackKind::Midi, order)),
        });
        ops.push(Op::Insert {
            entity: Entity::Device(Device {
                sidechain_tap: None,
                midi_sidechain: None,
                chain: None,
                id: ids.next(now),
                track: t,
                order: OrderKey::between(None, None),
                name: "Synth".into(),
                enabled: true,
                kind: DeviceKind::Builtin {
                    device: BuiltinDevice::Synth,
                },
                params: Default::default(),
                sidechain: None,
                pad: None,
            }),
        });
        ops.push(Op::Insert {
            entity: Entity::Send(TrackSend {
                id: ids.next(now),
                from: t,
                to: ret,
                level: Decibels(-12.0),
                pre_fader: false,
            }),
        });
        let lane: AutomationLaneId = ids.next(now);
        ops.push(Op::Insert {
            entity: Entity::AutomationLane(AutomationLane {
                id: lane,
                owner: AutomationOwner::Track { track: t },
                target: AutomationTarget::TrackVolume { track: t },
                enabled: true,
            }),
        });
        for k in 0..64 {
            ops.push(Op::Insert {
                entity: Entity::AutomationPoint(AutomationPoint {
                    id: ids.next(now),
                    lane,
                    time: Beats(k as f64 * 2.0),
                    value: (k % 8) as f64 / 8.0,
                    curve: CurveShape::Linear,
                }),
            });
        }
        for ci in 0..CLIPS {
            let clip: ClipId = ids.next(now);
            ops.push(Op::Insert {
                entity: Entity::Clip(Clip {
                    lane: None,
                    id: clip,
                    track: t,
                    start: Beats(ci as f64 * 4.0),
                    name: String::new(),
                    color: None,
                    muted: false,
                    length: Beats(4.0),
                    offset: Beats::ZERO,
                    looping: ClipLoop {
                        enabled: false,
                        start: Beats::ZERO,
                        end: Beats(4.0),
                    },
                    content: ClipContent::Midi,
                }),
            });
            for n in 0..notes {
                ops.push(Op::Insert {
                    entity: Entity::Note(Note {
                        id: ids.next(now),
                        clip,
                        pitch: 36 + ((n * 7 + ti) % 48) as u8,
                        velocity: 0.8,
                        release_velocity: 0.5,
                        start: Beats((n % 64) as f64 / 16.0),
                        duration: Beats(0.25),
                        muted: false,
                    }),
                });
            }
            if ci % 4 == 0 {
                let env: AutomationLaneId = ids.next(now);
                ops.push(Op::Insert {
                    entity: Entity::AutomationLane(AutomationLane {
                        id: env,
                        owner: AutomationOwner::Clip { clip },
                        target: AutomationTarget::TrackPan { track: t },
                        enabled: true,
                    }),
                });
                for k in 0..8 {
                    ops.push(Op::Insert {
                        entity: Entity::AutomationPoint(AutomationPoint {
                            id: ids.next(now),
                            lane: env,
                            time: Beats(k as f64 * 0.5),
                            value: 0.5,
                            curve: CurveShape::Linear,
                        }),
                    });
                }
            }
        }
    }
    p.apply_all(&ops).unwrap();
    p
}

fn best_of<T>(n: usize, mut f: impl FnMut() -> T) -> Duration {
    (0..n)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed()
        })
        .min()
        .unwrap()
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1e3)
}

/// Store `p` and open it in the harness (opening publishes the first graph).
fn open(h: &mut Harness, p: &Project) {
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let json = file::save(p, "0.3.0").unwrap();
    h.ctl.store.create(p.id).unwrap();
    h.ctl.store.save(p.id, &json).unwrap();
    h.ok(Command::Project(ProjectCommand::Open { id: p.id }));
    h.tick();
}

fn sizes() -> Vec<usize> {
    std::env::var("ETHER_PERF_NOTES")
        .ok()
        .map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![16])
}

#[test]
fn compile_and_delete_timings() {
    for per_clip in sizes() {
        let mut h = Harness::with_project();
        let mut ids = IdGen::new(7);
        let p = synthetic(&mut ids, per_clip);
        let total = p.notes.len();
        let compile = best_of(3, || compile_graph(&p, &|_| None, 1));
        open(&mut h, &p);

        // A note drag step: the edit itself, then the tick that publishes it.
        let note = *h.project().notes.keys().nth(total / 2).unwrap();
        let mut edit = Duration::MAX;
        let mut publish = Duration::MAX;
        for i in 0..3 {
            let t = Instant::now();
            h.ok(Command::Note(NoteCommand::Edit {
                edits: vec![NoteEdit {
                    id: note,
                    pitch: None,
                    velocity: None,
                    start: Some(Beats(0.5 + i as f64 * 0.25)),
                    duration: None,
                    muted: None,
                }],
            }));
            edit = edit.min(t.elapsed());
            h.advance(16);
            let before = h.ctl.bridge.publishes();
            let t = Instant::now();
            h.tick();
            publish = publish.min(t.elapsed());
            assert_eq!(h.ctl.bridge.publishes(), before + 1);
        }

        // Delete one track (32 clips) and undo it.
        let track = h
            .project()
            .tracks
            .values()
            .find(|t| t.kind == TrackKind::Midi)
            .unwrap()
            .id;
        let t = Instant::now();
        h.ok(Command::Track(TrackCommand::Delete { id: track }));
        let delete = t.elapsed();
        let t = Instant::now();
        h.ok(Command::Edit(EditCommand::Undo));
        let undo = t.elapsed();
        assert!(h.project().tracks.contains_key(&track));
        println!(
            "{total} notes: compile_graph {}, note edit {}, publish tick {}, track delete {}, undo {}",
            ms(compile),
            ms(edit),
            ms(publish),
            ms(delete),
            ms(undo),
        );
    }
}

/// Names and colours don't recompile the graph: no publish follows them,
/// and the graph compiled after them is the one already published.
#[test]
fn renames_and_colours_do_not_republish() {
    let mut h = Harness::with_project();
    let mut ids = IdGen::new(11);
    let p = synthetic(&mut ids, 2);
    open(&mut h, &p);
    h.advance(1000);
    h.tick();
    let track = *p
        .tracks
        .keys()
        .find(|t| p.tracks[t].kind == TrackKind::Midi)
        .unwrap();
    let clip = *p.clips.keys().next().unwrap();
    let before = h.ctl.bridge.publishes();
    for c in [
        Command::Track(TrackCommand::Rename {
            id: track,
            name: "Lead".into(),
        }),
        Command::Track(TrackCommand::SetColor {
            id: track,
            color: Color(0xff0000),
        }),
        Command::Clip(ClipCommand::Rename {
            id: clip,
            name: "Hook".into(),
        }),
        Command::Clip(ClipCommand::SetColor {
            id: clip,
            color: Some(Color(0x00ff00)),
        }),
    ] {
        h.ok(c);
        h.advance(1000);
        h.tick();
    }
    assert_eq!(h.ctl.bridge.publishes(), before, "a rename republished");
    let mut published = h.ctl.bridge.last_graph().clone();
    let mut now = compile_graph(h.project(), &|_| None, 0);
    // Only the version and node keys differ between the two compiles.
    published.version = 0;
    for t in published.tracks.iter_mut() {
        t.chain.clear();
    }
    for t in now.tracks.iter_mut() {
        t.chain.clear();
    }
    published.vcas.clear();
    now.vcas.clear();
    assert_eq!(published.tracks.len(), now.tracks.len());
    for (a, b) in published.tracks.iter().zip(&now.tracks) {
        assert_eq!(a.clips, b.clips);
        assert_eq!(a.sends, b.sends);
    }
    // Anything else still republishes.
    h.ok(Command::Track(TrackCommand::Rename {
        id: track,
        name: "Lead 2".into(),
    }));
    h.ok(Command::Clip(ClipCommand::SetMuted {
        ids: vec![clip],
        muted: true,
    }));
    h.advance(1000);
    h.tick();
    assert_eq!(h.ctl.bridge.publishes(), before + 1);
}

/// The published graph (compiled through the controller's per-clip note cache) equals a
/// fresh compile after every kind of note change: move, mute, remove, a whole clip's
/// removal, undo and redo.
#[test]
fn cached_note_compile_matches_a_fresh_one() {
    let mut h = Harness::with_project();
    let mut ids = IdGen::new(13);
    let p = synthetic(&mut ids, 4);
    open(&mut h, &p);
    h.advance(1000);
    h.tick();
    let note = |h: &Harness, k: usize| *h.project().notes.keys().nth(k).unwrap();
    let clip = *p.clips.keys().nth(3).unwrap();
    let check = |h: &mut Harness, what: &str| {
        h.advance(1000);
        h.tick();
        let mut published = h.ctl.bridge.last_graph().clone();
        let mut now = compile_graph(h.project(), &|_| None, 0);
        for t in published.tracks.iter_mut().chain(now.tracks.iter_mut()) {
            t.chain.clear();
        }
        assert_eq!(published.tracks.len(), now.tracks.len(), "{what}");
        for (a, b) in published.tracks.iter().zip(&now.tracks) {
            assert_eq!(a.clips, b.clips, "{what}");
        }
    };
    let n = note(&h, 5);
    h.ok(Command::Note(NoteCommand::Edit {
        edits: vec![NoteEdit {
            id: n,
            pitch: Some(70),
            velocity: None,
            start: Some(Beats(0.75)),
            duration: None,
            muted: None,
        }],
    }));
    check(&mut h, "move");
    let n = note(&h, 9);
    h.ok(Command::Note(NoteCommand::Edit {
        edits: vec![NoteEdit {
            id: n,
            pitch: None,
            velocity: None,
            start: None,
            duration: None,
            muted: Some(true),
        }],
    }));
    check(&mut h, "mute");
    let n = note(&h, 20);
    h.ok(Command::Note(NoteCommand::Remove { ids: vec![n] }));
    check(&mut h, "remove");
    h.ok(Command::Clip(ClipCommand::Delete { ids: vec![clip] }));
    check(&mut h, "clip delete");
    h.ok(Command::Edit(EditCommand::Undo));
    check(&mut h, "undo");
    h.ok(Command::Edit(EditCommand::Undo));
    check(&mut h, "undo 2");
    h.ok(Command::Edit(EditCommand::Redo));
    check(&mut h, "redo");
}

/// Command → patch: the controller's handling of a command (the edit and the patch it
/// emits) and the JSON of everything it sends back (what the desktop host puts on the wire
/// to the UI), then the publish tick that follows. Printed, like the timings above.
#[test]
fn command_to_patch_timings() {
    use ether_core::protocol::clips::ClipMove;
    for per_clip in sizes() {
        let mut h = Harness::with_project();
        let mut ids = IdGen::new(17);
        let p = synthetic(&mut ids, per_clip);
        let total = p.notes.len();
        open(&mut h, &p);
        h.advance(1000);
        h.tick();
        let note = *h.project().notes.keys().nth(total / 2).unwrap();
        let clip = h.project().notes[&note].clip;
        let track = h.project().clips[&clip].track;
        let start = h.project().clips[&clip].start;
        let mut rows = Vec::new();
        let mut run = |h: &mut Harness, what: &str, c: Command| {
            let t = Instant::now();
            let out = h.send(c);
            let handle = t.elapsed();
            let t = Instant::now();
            let json: usize = out
                .iter()
                .map(|m| serde_json::to_vec(m).expect("serializes").len())
                .sum();
            let encode = t.elapsed();
            h.advance(16);
            let t = Instant::now();
            h.tick();
            let tick = t.elapsed();
            rows.push(format!(
                "{what}: handle {}, JSON {} ({json} B), next tick {}",
                ms(handle),
                ms(encode),
                ms(tick)
            ));
        };
        run(
            &mut h,
            "first note edit after open",
            Command::Note(NoteCommand::Edit {
                edits: vec![NoteEdit {
                    id: note,
                    pitch: None,
                    velocity: None,
                    start: Some(Beats(0.5)),
                    duration: None,
                    muted: None,
                }],
            }),
        );
        for k in 0..3 {
            run(
                &mut h,
                "note drag step",
                Command::Note(NoteCommand::Edit {
                    edits: vec![NoteEdit {
                        id: note,
                        pitch: None,
                        velocity: None,
                        start: Some(Beats(0.75 + k as f64 * 0.25)),
                        duration: None,
                        muted: None,
                    }],
                }),
            );
        }
        run(
            &mut h,
            "clip move",
            Command::Clip(ClipCommand::Move {
                moves: vec![ClipMove {
                    id: clip,
                    track,
                    start: Beats(start.0 + 1.0),
                }],
            }),
        );
        run(
            &mut h,
            "track delete",
            Command::Track(TrackCommand::Delete { id: track }),
        );
        run(&mut h, "undo", Command::Edit(EditCommand::Undo));
        println!("{total} notes: {}", rows.join("; "));
    }
}

/// Opening a stored project: the file's size, parsing it (`file::load`), and the `Open`
/// command end to end (read, parse, upgrade, index, first compile and publish). Printed.
#[test]
fn open_timings() {
    for per_clip in sizes() {
        let mut h = Harness::with_project();
        let mut ids = IdGen::new(19);
        let p = synthetic(&mut ids, per_clip);
        let total = p.notes.len();
        let json = file::save(&p, "0.3.0").unwrap();
        let parse = best_of(3, || file::load(&json).unwrap());
        h.ctl.store.create(p.id).unwrap();
        h.ctl.store.save(p.id, &json).unwrap();
        let t = Instant::now();
        h.ok(Command::Project(ProjectCommand::Open { id: p.id }));
        let open = t.elapsed();
        let t = Instant::now();
        h.tick();
        let first_tick = t.elapsed();
        println!(
            "{total} notes: file {:.1} MB, parse {}, Open {}, first tick (compile + publish) {}",
            json.len() as f64 / 1e6,
            ms(parse),
            ms(open),
            ms(first_tick)
        );
    }
}

/// `OpenMirrored` loads like `Open` (the same `ProjectLoaded`), but replies `Unit`: the
/// project goes out once.
#[test]
fn open_mirrored_sends_the_project_once() {
    let mut h = Harness::with_project();
    let mut ids = IdGen::new(23);
    let p = synthetic(&mut ids, 2);
    let json = file::save(&p, "0.3.0").unwrap();
    h.ctl.store.create(p.id).unwrap();
    h.ctl.store.save(p.id, &json).unwrap();
    let count = |out: &[ether_core::protocol::ServerMessage]| {
        out.iter()
            .filter(|m| {
                serde_json::to_string(m)
                    .unwrap()
                    .contains(&format!("\"{}\"", p.notes.keys().next().unwrap()))
            })
            .count()
    };
    let open = h.send(Command::Project(ProjectCommand::Open { id: p.id }));
    assert_eq!(count(&open), 2, "Open: ProjectLoaded and the reply");
    let mirrored = h.send(Command::Project(ProjectCommand::OpenMirrored {
        id: p.id,
        safe: false,
    }));
    assert_eq!(count(&mirrored), 1, "OpenMirrored: ProjectLoaded only");
    assert!(matches!(
        ok(&mirrored),
        ether_core::protocol::ReplyValue::Unit
    ));
    assert_eq!(h.project().notes.len(), p.notes.len());
}
