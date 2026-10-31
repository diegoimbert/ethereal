//! Structural edits on a big group: delete, duplicate and their undo, timed through the real
//! controller with the fake bridge. A harness for before/after numbers:
//!
//! ```text
//! ETHER_SCALE_TRACKS=250,500,1000 cargo test --release -p ether-controller --test it scale_perf:: -- --ignored --nocapture
//! ```
//!
//! Each child track of the group has a synth and three effects, an LFO mapped to three of the
//! synth's params, a send to a return, an automated device param and a volume lane, a
//! MIDI mapping and a clip with 16 notes.

use crate::common;

use std::time::{Duration, Instant};

use common::*;
use ether_controller::store::ProjectStore;
use ether_core::protocol::Command;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;

fn sizes() -> Vec<usize> {
    std::env::var("ETHER_SCALE_TRACKS")
        .ok()
        .map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![64])
}

fn track(id: TrackId, kind: TrackKind, order: OrderKey, parent: Option<TrackId>) -> Track {
    Track {
        vca: Default::default(),
        freeze: None,
        id,
        kind,
        name: "T".into(),
        color: Color(0x123456),
        order,
        parent,
        mixer: TrackMixer::default(),
        input: TrackInput::None,
        output: TrackOutput::Default,
        monitor: MonitorMode::Auto,
        scale: Default::default(),
        mpe: None,
    }
}

fn device(id: DeviceId, track: TrackId, order: OrderKey, kind: BuiltinDevice) -> Device {
    Device {
        sidechain_tap: None,
        midi_sidechain: None,
        chain: None,
        id,
        track,
        order,
        name: "D".into(),
        enabled: true,
        kind: DeviceKind::Builtin { device: kind },
        params: Default::default(),
        sidechain: None,
        pad: None,
    }
}

/// A group of `n` MIDI tracks (see the module docs) and a return track. Returns the project
/// and the group's id.
fn synthetic(ids: &mut IdGen, n: usize) -> (Project, TrackId) {
    let now = 1_700_000_000_000u64;
    let mut p = Project::new(ids, now);
    let mut ops = Vec::new();
    let mut ins = |e: Entity| ops.push(Op::Insert { entity: e });
    let top = OrderKey::n_between(None, None, 2);
    let (group, ret): (TrackId, TrackId) = (ids.next(now), ids.next(now));
    ins(Entity::Track(track(
        group,
        TrackKind::Group,
        top[0].clone(),
        None,
    )));
    ins(Entity::Track(track(
        ret,
        TrackKind::Return,
        top[1].clone(),
        None,
    )));
    let orders = OrderKey::n_between(None, None, n);
    let dev_orders = OrderKey::n_between(None, None, 4);
    for (i, order) in orders.into_iter().enumerate() {
        let t: TrackId = ids.next(now);
        ins(Entity::Track(track(t, TrackKind::Midi, order, Some(group))));
        let synth: DeviceId = ids.next(now);
        ins(Entity::Device(device(
            synth,
            t,
            dev_orders[0].clone(),
            BuiltinDevice::Synth,
        )));
        let lfo: ModulatorId = ids.next(now);
        ins(Entity::Modulator(Modulator {
            id: lfo,
            device: synth,
            order: OrderKey::between(None, None),
            name: "LFO".into(),
            kind: ModulatorKind::Lfo,
            params: Default::default(),
            sidechain: None,
        }));
        let mut fx = Vec::new();
        for (i, kind) in [
            BuiltinDevice::Eq,
            BuiltinDevice::Compressor,
            BuiltinDevice::Delay,
        ]
        .into_iter()
        .enumerate()
        {
            let d: DeviceId = ids.next(now);
            fx.push(d);
            ins(Entity::Device(device(
                d,
                t,
                dev_orders[i + 1].clone(),
                kind,
            )));
            ins(Entity::ModMapping(ModMapping {
                id: ids.next(now),
                source: ModSource::Modulator { modulator: lfo },
                device: synth,
                param: ParamId(i as u32),
                depth: 0.5,
            }));
        }
        ins(Entity::Send(TrackSend {
            id: ids.next(now),
            from: t,
            to: ret,
            level: Decibels(-12.0),
            pre_fader: false,
        }));
        for target in [
            AutomationTarget::TrackVolume { track: t },
            AutomationTarget::DeviceParam {
                device: fx[0],
                param: ParamId(0),
            },
        ] {
            let lane: AutomationLaneId = ids.next(now);
            ins(Entity::AutomationLane(AutomationLane {
                id: lane,
                owner: AutomationOwner::Track { track: t },
                target,
                enabled: true,
            }));
            for k in 0..8 {
                ins(Entity::AutomationPoint(AutomationPoint {
                    id: ids.next(now),
                    lane,
                    time: Beats(k as f64),
                    value: 0.5,
                    curve: CurveShape::Linear,
                }));
            }
        }
        ins(Entity::MidiMapping(MidiMapping {
            id: ids.next(now),
            source: MidiSource {
                port: Some(format!("port {i}")),
                channel: None,
                control: MidiControl::Cc { number: 7 },
            },
            target: MidiMapTarget::TrackMute { track: t },
            min: 0.0,
            max: 1.0,
            mode: MidiMapMode::Absolute,
        }));
        let clip: ClipId = ids.next(now);
        ins(Entity::Clip(Clip {
            lane: None,
            id: clip,
            track: t,
            start: Beats::ZERO,
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
        }));
        for k in 0..16 {
            ins(Entity::Note(Note {
                id: ids.next(now),
                clip,
                pitch: 60,
                velocity: 0.8,
                release_velocity: 0.5,
                start: Beats(k as f64 / 4.0),
                duration: Beats(0.25),
                muted: false,
            }));
        }
    }
    p.apply_all(&ops).unwrap();
    (p, group)
}

fn open(h: &mut Harness, p: &Project) {
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let json = file::save(p, "0.3.0").unwrap();
    h.ctl.store.create(p.id).unwrap();
    h.ctl.store.save(p.id, &json).unwrap();
    h.ok(Command::Project(ProjectCommand::Open { id: p.id }));
    h.tick();
}

fn timed(h: &mut Harness, c: Command) -> Duration {
    let t = Instant::now();
    h.ok(c);
    t.elapsed()
}

fn ms(d: Duration) -> String {
    format!("{:.0} ms", d.as_secs_f64() * 1e3)
}

#[test]
#[ignore = "perf harness: run in release with --ignored --nocapture"]
fn group_delete_and_duplicate_timings() {
    for n in sizes() {
        let mut h = Harness::with_project();
        let mut ids = IdGen::new(7);
        let (p, group) = synthetic(&mut ids, n);
        let entities = p.tracks.len() + p.devices.len() + p.notes.len();
        open(&mut h, &p);
        let tracks = h.project().tracks.len();

        let delete = timed(&mut h, Command::Track(TrackCommand::Delete { id: group }));
        assert_eq!(h.project().tracks.len(), tracks - n - 1);
        let undo_delete = timed(&mut h, Command::Edit(EditCommand::Undo));
        assert_eq!(h.project().tracks.len(), tracks);

        let copy: TrackId = h.id();
        let duplicate = timed(
            &mut h,
            Command::Track(TrackCommand::Duplicate {
                id: group,
                new_id: copy,
            }),
        );
        assert_eq!(h.project().tracks.len(), tracks + n + 1);
        let undo_duplicate = timed(&mut h, Command::Edit(EditCommand::Undo));
        assert_eq!(h.project().tracks.len(), tracks);
        println!(
            "{n} tracks ({entities} entities): delete {}, undo {}, duplicate {}, undo {}",
            ms(delete),
            ms(undo_delete),
            ms(duplicate),
            ms(undo_duplicate),
        );
    }
}
