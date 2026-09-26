//! Property tests: random op sequences (valid and invalid) against the document model.
//!
//! - a failed commit leaves the project unchanged; a successful one leaves it valid;
//! - patches from applied ops reproduce the project on a mirror (commit, undo and redo);
//! - undoing every step restores each earlier snapshot exactly, down to the initial project;
//!   redoing everything restores the final project;
//! - gesture merging groups commits into one undo step;
//! - `.ether` save/load round-trips.

use ether_model::*;
use proptest::prelude::*;

/// splitmix64: a tiny deterministic RNG driven by proptest-chosen seeds.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
    fn pick<T: Clone>(&mut self, items: &[T]) -> Option<T> {
        (!items.is_empty()).then(|| items[self.below(items.len())].clone())
    }
    /// Mostly-valid beat values, sometimes invalid.
    fn beats(&mut self) -> Beats {
        let v = [0.0, 0.25, 1.0, 1.5, 3.75, 4.0, 8.0, 16.0, 33.3];
        if self.chance(5) {
            Beats(-1.0)
        } else {
            Beats(v[self.below(v.len())])
        }
    }
}

struct Gen<'a> {
    p: &'a Project,
    ids: &'a mut IdGen,
    now: u64,
    r: Rng,
}

impl Gen<'_> {
    fn id<I: Id>(&mut self) -> I {
        self.now += 1;
        self.ids.next(self.now)
    }

    fn keys<K: Copy, V>(&self, m: &std::collections::BTreeMap<K, V>) -> Vec<K> {
        m.keys().copied().collect()
    }

    fn tracks_of(&self, kinds: &[TrackKind]) -> Vec<TrackId> {
        self.p
            .tracks
            .values()
            .filter(|t| kinds.contains(&t.kind))
            .map(|t| t.id)
            .collect()
    }

    fn order(&mut self) -> OrderKey {
        let keys = OrderKey::n_between(None, None, 8);
        if self.r.chance(3) {
            OrderKey("!bad".into())
        } else {
            keys[self.r.below(keys.len())].clone()
        }
    }

    fn some_track(&mut self) -> Option<TrackId> {
        let all = self.keys(&self.p.tracks);
        self.r.pick(&all)
    }

    fn new_track(&mut self) -> Op {
        let kinds = [
            TrackKind::Audio,
            TrackKind::Midi,
            TrackKind::Group,
            TrackKind::Return,
            TrackKind::Master,
        ];
        let n = if self.r.chance(3) { 5 } else { 4 };
        let kind = kinds[self.r.below(n)];
        let groups = self.tracks_of(&[TrackKind::Group]);
        let parent = if self.r.chance(40) {
            self.r.pick(&groups)
        } else {
            None
        };
        let order = self.order();
        let output = match self.r.below(4) {
            0 => TrackOutput::None,
            1 => match self.some_track() {
                Some(track) => TrackOutput::Track { track },
                None => TrackOutput::Default,
            },
            _ => TrackOutput::Default,
        };
        Op::Insert {
            entity: Entity::Track(Track {
                id: self.id(),
                kind,
                name: format!("T{}", self.r.below(100)),
                color: Color(0x123456),
                order,
                parent,
                mixer: TrackMixer::default(),
                input: TrackInput::None,
                output,
                monitor: MonitorMode::Auto,
            }),
        }
    }

    fn new_clip(&mut self) -> Option<Op> {
        let track = self
            .r
            .pick(&self.tracks_of(&[TrackKind::Audio, TrackKind::Midi]))?;
        let kind = self.p.tracks[&track].kind;
        let content = if kind == TrackKind::Audio || self.r.chance(5) {
            let media = self.r.pick(&self.keys(&self.p.media))?;
            ClipContent::Audio(AudioContent {
                media,
                gain: Decibels(-3.0),
                transpose: 0.0,
                fade_in: Beats::ZERO,
                fade_out: Beats::ZERO,
                warp: WarpSettings::default(),
            })
        } else {
            ClipContent::Midi
        };
        let location = if self.r.chance(50) {
            ClipLocation::Arrangement {
                start: self.r.beats(),
            }
        } else {
            ClipLocation::Session {
                scene: self.r.pick(&self.keys(&self.p.scenes))?,
            }
        };
        let length = self.r.beats();
        Some(Op::Insert {
            entity: Entity::Clip(Clip {
                id: self.id(),
                track,
                location,
                name: String::new(),
                color: None,
                muted: false,
                length,
                offset: Beats::ZERO,
                looping: ClipLoop {
                    enabled: true,
                    start: Beats::ZERO,
                    end: Beats(4.0),
                },
                launch: LaunchSettings::default(),
                content,
            }),
        })
    }

    fn new_entity(&mut self) -> Option<Op> {
        Some(match self.r.below(11) {
            0 | 1 => self.new_track(),
            2 | 3 => return self.new_clip(),
            4 => {
                let clip = self.r.pick(&self.keys(&self.p.clips))?;
                let pitch = if self.r.chance(5) {
                    200
                } else {
                    self.r.below(128) as u8
                };
                Op::Insert {
                    entity: Entity::Note(Note {
                        id: self.id(),
                        clip,
                        pitch,
                        velocity: 0.8,
                        release_velocity: 0.5,
                        start: self.r.beats(),
                        duration: Beats(0.5),
                        muted: false,
                    }),
                }
            }
            5 => {
                let track = self.some_track()?;
                let order = self.order();
                let kind = match self.r.below(3) {
                    0 => DeviceKind::Builtin {
                        device: BuiltinDevice::Synth,
                    },
                    1 => DeviceKind::Builtin {
                        device: BuiltinDevice::Sampler {
                            sample: self.r.pick(&self.keys(&self.p.media)),
                        },
                    },
                    _ => DeviceKind::Plugin {
                        plugin: PluginInstance {
                            format: PluginFormat::Clap,
                            plugin_id: "com.example.p".into(),
                            name: "P".into(),
                            vendor: "E".into(),
                            version: "1".into(),
                            sandboxed: false,
                            state: Some(Base64Bytes(vec![1, 2, 3])),
                        },
                    },
                };
                Op::Insert {
                    entity: Entity::Device(Device {
                        id: self.id(),
                        track,
                        order,
                        name: "D".into(),
                        enabled: true,
                        kind,
                        params: [(ParamId(1), 0.5), (ParamId(7), 440.0)].into(),
                    }),
                }
            }
            6 => {
                let from = self.some_track()?;
                let to = self.some_track()?;
                Op::Insert {
                    entity: Entity::Send(TrackSend {
                        id: self.id(),
                        from,
                        to,
                        level: Decibels(-6.0),
                        pre_fader: false,
                    }),
                }
            }
            7 => {
                let order = self.order();
                Op::Insert {
                    entity: Entity::Scene(Scene {
                        id: self.id(),
                        name: "S".into(),
                        color: None,
                        order,
                        tempo: self.r.chance(30).then_some(128.0),
                        time_signature: None,
                    }),
                }
            }
            8 => {
                if self.r.chance(50) || self.p.automation_lanes.is_empty() {
                    let track = self.some_track()?;
                    let owner = match self.r.pick(&self.keys(&self.p.clips)) {
                        Some(clip) if self.r.chance(30) => AutomationOwner::Clip { clip },
                        _ => AutomationOwner::Track { track },
                    };
                    let target = match self.r.below(3) {
                        0 => AutomationTarget::TrackVolume { track },
                        1 => AutomationTarget::SendLevel {
                            send: self.r.pick(&self.keys(&self.p.sends))?,
                        },
                        _ => AutomationTarget::DeviceParam {
                            device: self.r.pick(&self.keys(&self.p.devices))?,
                            param: ParamId(1),
                        },
                    };
                    Op::Insert {
                        entity: Entity::AutomationLane(AutomationLane {
                            id: self.id(),
                            owner,
                            target,
                            enabled: true,
                        }),
                    }
                } else {
                    let lane = self.r.pick(&self.keys(&self.p.automation_lanes))?;
                    Op::Insert {
                        entity: Entity::AutomationPoint(AutomationPoint {
                            id: self.id(),
                            lane,
                            time: self.r.beats(),
                            value: if self.r.chance(5) { 2.0 } else { 0.25 },
                            curve: CurveShape::Curve { tension: 0.5 },
                        }),
                    }
                }
            }
            9 => {
                if self.r.chance(50) {
                    Op::Insert {
                        entity: Entity::TempoPoint(TempoPoint {
                            id: self.id(),
                            time: self.r.beats(),
                            bpm: [90.0, 140.0, 0.0][self.r.below(3)],
                            curve: TempoCurve::Linear,
                        }),
                    }
                } else {
                    Op::Insert {
                        entity: Entity::TimeSignature(TimeSignaturePoint {
                            id: self.id(),
                            time: self.r.beats(),
                            signature: TimeSignature {
                                numerator: 3,
                                denominator: [4, 8, 3][self.r.below(3)],
                            },
                        }),
                    }
                }
            }
            _ => {
                if self.r.chance(50) || self.p.clips.is_empty() {
                    let id: MediaId = self.id();
                    let file = if self.r.chance(5) {
                        "../escape.wav".to_string()
                    } else {
                        format!("media/{id}.wav")
                    };
                    Op::Insert {
                        entity: Entity::Media(MediaRef {
                            id,
                            name: "kick.wav".into(),
                            file,
                            sample_rate: 48_000,
                            channels: 2,
                            frames: 48_000,
                            hash: None,
                        }),
                    }
                } else {
                    let clip = self.r.pick(&self.keys(&self.p.clips))?;
                    Op::Insert {
                        entity: Entity::WarpMarker(WarpMarker {
                            id: self.id(),
                            clip,
                            beat: self.r.beats(),
                            source: Seconds(0.5),
                        }),
                    }
                }
            }
        })
    }

    fn update(&mut self) -> Option<Op> {
        let update = match self.r.below(10) {
            0 | 1 => {
                let id = self.some_track()?;
                let change = match self.r.below(8) {
                    0 => TrackChange::Volume(Decibels(-12.0)),
                    1 => TrackChange::Mute(self.r.chance(50)),
                    2 => TrackChange::Name("renamed".into()),
                    3 => TrackChange::Parent(self.r.pick(&self.tracks_of(&[TrackKind::Group]))),
                    4 => TrackChange::Output(match self.some_track() {
                        Some(track) => TrackOutput::Track { track },
                        None => TrackOutput::Default,
                    }),
                    5 => TrackChange::Order(self.order()),
                    6 => TrackChange::Pan(Pan(if self.r.chance(10) { 3.0 } else { -0.5 })),
                    _ => TrackChange::Input(match self.some_track() {
                        Some(track) => TrackInput::Track { track },
                        None => TrackInput::None,
                    }),
                };
                EntityUpdate::Track { id, change }
            }
            2 | 3 => {
                let id = self.r.pick(&self.keys(&self.p.clips))?;
                let change = match self.r.below(7) {
                    0 => ClipChange::Length(self.r.beats()),
                    1 => ClipChange::Location(if self.r.chance(50) {
                        ClipLocation::Arrangement {
                            start: self.r.beats(),
                        }
                    } else {
                        ClipLocation::Session {
                            scene: self.r.pick(&self.keys(&self.p.scenes))?,
                        }
                    }),
                    2 => ClipChange::Gain(Decibels(2.0)),
                    3 => ClipChange::Track(self.some_track()?),
                    4 => ClipChange::Warp(WarpSettings {
                        enabled: true,
                        mode: WarpMode::Repitch,
                        source_bpm: Some(100.0),
                    }),
                    5 => ClipChange::Color(Some(Color(0xff0000))),
                    _ => ClipChange::Loop(ClipLoop {
                        enabled: false,
                        start: self.r.beats(),
                        end: self.r.beats(),
                    }),
                };
                EntityUpdate::Clip { id, change }
            }
            4 => {
                let id = self.r.pick(&self.keys(&self.p.notes))?;
                let change = match self.r.below(3) {
                    0 => NoteChange::Pitch(self.r.below(140) as u8),
                    1 => NoteChange::Start(self.r.beats()),
                    _ => NoteChange::Duration(self.r.beats()),
                };
                EntityUpdate::Note { id, change }
            }
            5 => {
                let id = self.r.pick(&self.keys(&self.p.devices))?;
                let change = match self.r.below(4) {
                    0 => DeviceChange::Param {
                        param: ParamId(self.r.below(3) as u32 * 7),
                        value: self.r.chance(50).then_some(0.75),
                    },
                    1 => DeviceChange::Enabled(false),
                    2 => DeviceChange::Kind(DeviceKind::Builtin {
                        device: BuiltinDevice::Sampler { sample: None },
                    }),
                    _ => DeviceChange::Track(self.some_track()?),
                };
                EntityUpdate::Device { id, change }
            }
            6 => {
                let id = self.r.pick(&self.keys(&self.p.tempo_points))?;
                let change = if self.r.chance(50) {
                    TempoPointChange::Time(self.r.beats())
                } else {
                    TempoPointChange::Bpm(100.0)
                };
                EntityUpdate::TempoPoint { id, change }
            }
            7 => {
                let id = self.r.pick(&self.keys(&self.p.scenes))?;
                EntityUpdate::Scene {
                    id,
                    change: SceneChange::Order(self.order()),
                }
            }
            8 => {
                let id = self.r.pick(&self.keys(&self.p.automation_points))?;
                EntityUpdate::AutomationPoint {
                    id,
                    change: AutomationPointChange::Value(0.9),
                }
            }
            _ => {
                let change = match self.r.below(4) {
                    0 => SettingsChange::Name("Song".into()),
                    1 => SettingsChange::LoopRegion(BeatRange {
                        start: self.r.beats(),
                        end: self.r.beats(),
                    }),
                    2 => SettingsChange::Metronome(true),
                    _ => SettingsChange::LaunchQuantization(Quantization::Beats {
                        beats: self.r.beats(),
                    }),
                };
                return Some(Op::Settings { change });
            }
        };
        Some(Op::Update { update })
    }

    fn remove(&mut self) -> Option<Op> {
        let keys: Vec<EntityKey> = self.p.entities().iter().map(Entity::key).collect();
        // Prefer leaves so removals succeed often: pick from the end (children last).
        let key = if self.r.chance(70) {
            keys[keys.len() - 1 - self.r.below(keys.len().min(5))]
        } else {
            self.r.pick(&keys)?
        };
        Some(Op::Remove { key })
    }

    fn op(&mut self) -> Option<Op> {
        match self.r.below(10) {
            0..=4 => self.new_entity(),
            5..=7 => self.update(),
            _ => self.remove(),
        }
    }
}

/// Build a transaction of 1..=3 ops from a seed against the current project. Later ops in
/// the same transaction are generated against the state after the earlier ones.
fn make_tx(p: &Project, ids: &mut IdGen, now: &mut u64, seed: u64) -> Transaction {
    let mut r = Rng(seed);
    let n = 1 + r.below(3);
    let mut scratch = p.clone();
    let mut ops = Vec::new();
    for _ in 0..n {
        let mut g = Gen {
            p: &scratch,
            ids,
            now: *now,
            r: Rng(r.next()),
        };
        let op = g.op();
        *now = g.now;
        if let Some(op) = op {
            let _ = scratch.apply(&op);
            ops.push(op);
        }
    }
    Transaction {
        label: format!("tx{seed}"),
        ops,
    }
}

fn gesture_of(code: u8) -> Option<GestureId> {
    match code % 4 {
        0 | 1 => None,
        g => Some(GestureId(u32::from(g))),
    }
}

/// Guard against a generator that only produces failing ops: over a long run, every table
/// gets populated and both successes and failures happen.
#[test]
fn generator_covers_every_table() {
    let mut ids = IdGen::new(99);
    let mut now = 1_700_000_000_000u64;
    let mut p = Project::new(&mut ids, now);
    let (mut ok, mut err) = (0, 0);
    let mut max = [0usize; 12];
    for s in 0..3000u64 {
        let tx = make_tx(
            &p,
            &mut ids,
            &mut now,
            s.wrapping_mul(0x2545_F491_4F6C_DD1D),
        );
        match p.apply_all(&tx.ops) {
            Ok(_) => ok += 1,
            Err(_) => err += 1,
        }
        let sizes = [
            p.tracks.len(),
            p.clips.len(),
            p.notes.len(),
            p.devices.len(),
            p.sends.len(),
            p.scenes.len(),
            p.automation_lanes.len(),
            p.automation_points.len(),
            p.tempo_points.len(),
            p.time_signatures.len(),
            p.warp_markers.len(),
            p.media.len(),
        ];
        for (m, s) in max.iter_mut().zip(sizes) {
            *m = (*m).max(s);
        }
    }
    p.validate().unwrap();
    assert!(ok > 500 && err > 100, "ok={ok} err={err}");
    assert!(
        max.iter().all(|&m| m >= 2),
        "table sizes never grew: {max:?}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_ops_undo_redo_and_patches(
        seed in any::<u64>(),
        steps in prop::collection::vec((any::<u64>(), any::<u8>()), 1..60),
    ) {
        let mut ids = IdGen::new(seed);
        let mut now = 1_700_000_000_000u64;
        let initial = Project::new(&mut ids, now);
        initial.validate().unwrap();

        let mut p = initial.clone();
        let mut mirror = initial.clone();
        let mut history = History::new(0);
        // Snapshot at the start of each undo step.
        let mut snapshots: Vec<Project> = Vec::new();
        let mut open: Option<GestureId> = None;
        let mut applied_any = 0;

        for (tx_seed, g) in steps {
            let tx = make_tx(&p, &mut ids, &mut now, tx_seed);
            let gesture = gesture_of(g);
            let before = p.clone();
            let empty = tx.ops.is_empty();
            match history.commit(&mut p, tx, gesture) {
                Err(_) => prop_assert_eq!(&p, &before, "failed commit changed the project"),
                Ok(applied) => {
                    p.validate().unwrap();
                    if !empty {
                        applied_any += 1;
                        let merge = gesture.is_some() && gesture == open && !snapshots.is_empty();
                        if !merge {
                            snapshots.push(before);
                        }
                        open = gesture;
                    }
                    mirror.apply_patch_changes(&changes_for(&p, &applied));
                    prop_assert_eq!(&mirror, &p, "patch mirror diverged after commit");
                }
            }
            if g % 16 == 15 {
                // End the gesture sometimes.
                if let Some(o) = open {
                    history.end_gesture(o);
                    open = None;
                }
            }
        }

        // Round-trip through the file format.
        let json = file::save(&p, "0.1.0").unwrap();
        prop_assert_eq!(&file::load(&json).unwrap(), &p);

        let final_state = p.clone();
        prop_assert_eq!(history.state().can_undo, applied_any > 0);

        // Undo everything, checking each step boundary.
        while let Some(expected) = snapshots.pop() {
            let applied = history.undo(&mut p).unwrap().expect("an undo step");
            prop_assert_eq!(&p, &expected, "undo did not restore the snapshot");
            p.validate().unwrap();
            mirror.apply_patch_changes(&changes_for(&p, &applied));
            prop_assert_eq!(&mirror, &p, "patch mirror diverged after undo");
        }
        prop_assert!(history.undo(&mut p).unwrap().is_none());
        prop_assert_eq!(&p, &initial);

        // Redo everything.
        while let Some(applied) = history.redo(&mut p).unwrap() {
            p.validate().unwrap();
            mirror.apply_patch_changes(&changes_for(&p, &applied));
            prop_assert_eq!(&mirror, &p, "patch mirror diverged after redo");
        }
        prop_assert_eq!(&p, &final_state);
    }

    #[test]
    fn apply_then_inverse_restores_exactly(
        seed in any::<u64>(),
        steps in prop::collection::vec(any::<u64>(), 1..80),
    ) {
        let mut ids = IdGen::new(seed);
        let mut now = 1_700_000_000_000u64;
        let mut p = Project::new(&mut ids, now);
        for s in steps {
            let tx = make_tx(&p, &mut ids, &mut now, s);
            let before = p.clone();
            match p.apply_all(&tx.ops) {
                Ok(inverse) => {
                    let mut undone = p.clone();
                    let redo = undone.apply_all(&inverse).unwrap();
                    prop_assert_eq!(&undone, &before);
                    // The inverse of the inverse is the original transaction.
                    prop_assert_eq!(&redo, &tx.ops);
                }
                Err(_) => prop_assert_eq!(&p, &before),
            }
        }
    }
}
