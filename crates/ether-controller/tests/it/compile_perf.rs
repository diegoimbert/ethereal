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
