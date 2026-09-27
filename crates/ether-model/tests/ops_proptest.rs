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
    /// A uniformly random `f64` in `[lo, hi)` using all 53 mantissa bits (so values need
    /// every digit to survive a JSON round-trip).
    fn float(&mut self, lo: f64, hi: f64) -> f64 {
        let unit = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        lo + unit * (hi - lo)
    }
    /// Mostly-valid beat values, sometimes invalid. Half of them are full-precision floats.
    fn beats(&mut self) -> Beats {
        let v = [0.0, 0.25, 1.0, 1.5, 3.75, 4.0, 8.0, 16.0, 33.3];
        if self.chance(5) {
            Beats(-1.0)
        } else if self.chance(50) {
            Beats(self.float(0.0, 64.0))
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
                scale: Default::default(),
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
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::EqualPower,
                reversed: self.r.chance(20),
            })
        } else {
            ClipContent::Midi
        };
        let start = self.r.beats();
        let length = self.r.beats();
        Some(Op::Insert {
            entity: Entity::Clip(Clip {
                id: self.id(),
                track,
                start,
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
                        velocity: self.r.float(0.0, 1.0) as f32,
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
                            slices: SliceSettings::default(),
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
                        params: [(ParamId(1), self.r.float(-1e6, 1e6)), (ParamId(7), 440.0)].into(),
                        sidechain: None,
                        pad: None,
                    }),
                }
            }
            6 | 7 => {
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
                            value: if self.r.chance(5) {
                                2.0
                            } else {
                                self.r.float(0.0, 1.0)
                            },
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
                            bpm: [90.0, self.r.float(20.0, 999.0), 0.0][self.r.below(3)],
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
                    0 => TrackChange::Volume(Decibels(self.r.float(-70.0, 6.0) as f32)),
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
                    1 => ClipChange::Start(self.r.beats()),
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
                        value: self.r.chance(50).then_some(self.r.float(-1e9, 1e9)),
                    },
                    1 => DeviceChange::Enabled(false),
                    2 => DeviceChange::Kind(DeviceKind::Builtin {
                        device: BuiltinDevice::Sampler {
                            sample: None,
                            slices: SliceSettings::default(),
                        },
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
                    TempoPointChange::Bpm(self.r.float(20.0, 999.0))
                };
                EntityUpdate::TempoPoint { id, change }
            }
            7 => {
                let id = self.r.pick(&self.keys(&self.p.devices))?;
                EntityUpdate::Device {
                    id,
                    change: DeviceChange::Order(self.order()),
                }
            }
            8 => {
                let id = self.r.pick(&self.keys(&self.p.automation_points))?;
                EntityUpdate::AutomationPoint {
                    id,
                    change: AutomationPointChange::Value(self.r.float(0.0, 1.0)),
                }
            }
            _ => {
                let change = match self.r.below(3) {
                    0 => SettingsChange::Name("Song".into()),
                    1 => SettingsChange::LoopRegion(BeatRange {
                        start: self.r.beats(),
                        end: self.r.beats(),
                    }),
                    _ => SettingsChange::Metronome(true),
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
    let mut max = [0usize; 11];
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

/// Project + History under test, with the expected undo/redo stacks as snapshots.
struct Sim {
    p: Project,
    mirror: Project,
    history: History,
    /// Project state before each undoable step (bottom = oldest).
    undo_snaps: Vec<Project>,
    /// Project state after each undone step (top = next redo).
    redo_snaps: Vec<Project>,
}

impl Sim {
    fn check_applied(&mut self, applied: &[Op], what: &str) -> Result<(), TestCaseError> {
        self.p.validate().unwrap();
        self.mirror
            .apply_patch_changes(&changes_for(&self.p, applied));
        prop_assert_eq!(
            &self.mirror,
            &self.p,
            "patch mirror diverged after {}",
            what
        );
        Ok(())
    }

    /// Undo one step; `false` if there was nothing to undo.
    fn undo(&mut self) -> Result<bool, TestCaseError> {
        let current = self.p.clone();
        let applied = self.history.undo(&mut self.p).unwrap();
        match (self.undo_snaps.pop(), applied) {
            (None, None) => Ok(false),
            (Some(expected), Some(applied)) => {
                prop_assert_eq!(&self.p, &expected, "undo did not restore the snapshot");
                self.check_applied(&applied, "undo")?;
                self.redo_snaps.push(current);
                Ok(true)
            }
            (e, a) => Err(TestCaseError::fail(format!(
                "undo availability mismatch: expected {}, got {}",
                e.is_some(),
                a.is_some()
            ))),
        }
    }

    /// Redo one step; `false` if there was nothing to redo.
    fn redo(&mut self) -> Result<bool, TestCaseError> {
        let current = self.p.clone();
        let applied = self.history.redo(&mut self.p).unwrap();
        match (self.redo_snaps.pop(), applied) {
            (None, None) => Ok(false),
            (Some(expected), Some(applied)) => {
                prop_assert_eq!(&self.p, &expected, "redo did not restore the snapshot");
                self.check_applied(&applied, "redo")?;
                self.undo_snaps.push(current);
                Ok(true)
            }
            (e, a) => Err(TestCaseError::fail(format!(
                "redo availability mismatch: expected {}, got {}",
                e.is_some(),
                a.is_some()
            ))),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Random interleaving of commits (with gestures), undos and redos, checked against a
    /// model of the expected undo/redo stacks (as project snapshots).
    #[test]
    fn random_ops_undo_redo_and_patches(
        seed in any::<u64>(),
        steps in prop::collection::vec((any::<u64>(), any::<u8>()), 1..60),
    ) {
        let mut ids = IdGen::new(seed);
        let mut now = 1_700_000_000_000u64;
        let initial = Project::new(&mut ids, now);
        initial.validate().unwrap();

        let mut s = Sim {
            p: initial.clone(),
            mirror: initial.clone(),
            history: History::new(0),
            undo_snaps: Vec::new(),
            redo_snaps: Vec::new(),
        };
        let mut open: Option<GestureId> = None;

        for (tx_seed, g) in steps {
            match g % 8 {
                6 => {
                    s.undo()?;
                    open = None;
                }
                7 => {
                    s.redo()?;
                    open = None;
                }
                _ => {
                    let tx = make_tx(&s.p, &mut ids, &mut now, tx_seed);
                    let gesture = gesture_of(g / 8);
                    let before = s.p.clone();
                    let empty = tx.ops.is_empty();
                    match s.history.commit(&mut s.p, tx, gesture) {
                        Err(_) => prop_assert_eq!(&s.p, &before, "failed commit changed the project"),
                        Ok(applied) => {
                            s.p.validate().unwrap();
                            if !empty {
                                let merge = gesture.is_some() && gesture == open && !s.undo_snaps.is_empty();
                                if !merge {
                                    s.undo_snaps.push(before);
                                }
                                s.redo_snaps.clear();
                                open = gesture;
                            }
                            s.mirror.apply_patch_changes(&changes_for(&s.p, &applied));
                            prop_assert_eq!(&s.mirror, &s.p, "patch mirror diverged after commit");
                        }
                    }
                }
            }
            prop_assert_eq!(s.history.state().can_undo, !s.undo_snaps.is_empty());
            prop_assert_eq!(s.history.state().can_redo, !s.redo_snaps.is_empty());
            if g % 64 == 63
                && let Some(o) = open.take()
            {
                s.history.end_gesture(o);
            }
        }

        // Round-trip through the file format.
        let json = file::save(&s.p, "0.1.0").unwrap();
        prop_assert_eq!(&file::load(&json).unwrap(), &s.p);

        // Undo everything (back to the initial project), then redo everything.
        while s.undo()? {}
        prop_assert_eq!(&s.p, &initial);
        while s.redo()? {}
        prop_assert!(s.history.redo(&mut s.p).unwrap().is_none());
    }

    /// `.ether` save/load preserves arbitrary finite floats bit-for-bit (f64 and f32 fields).
    #[test]
    fn file_roundtrip_preserves_arbitrary_floats(
        seed in any::<u64>(),
        beats in prop::collection::vec(0.0f64..1e7, 4),
        bpm in 1e-3f64..999.0,
        unit in 0.0f64..=1.0,
        param in prop::num::f64::NORMAL | prop::num::f64::SUBNORMAL | prop::num::f64::ZERO,
        db in prop::num::f32::NORMAL | prop::num::f32::SUBNORMAL | prop::num::f32::ZERO,
        pan in -1.0f32..=1.0,
    ) {
        let mut ids = IdGen::new(seed);
        let now = 1_700_000_000_000u64;
        let mut p = Project::new(&mut ids, now);
        let track: TrackId = ids.next(now);
        let clip: ClipId = ids.next(now);
        let note: NoteId = ids.next(now);
        let device: DeviceId = ids.next(now);
        let lane: AutomationLaneId = ids.next(now);
        let point: AutomationPointId = ids.next(now);
        let tempo: TempoPointId = ids.next(now);
        let ops = vec![
            Op::Insert { entity: Entity::Track(Track {
                id: track, kind: TrackKind::Midi, name: "m".into(), color: Color(1),
                order: OrderKey::between(None, None), parent: None,
                mixer: TrackMixer { volume: Decibels(db), pan: Pan(pan), mute: false, solo: false },
                input: TrackInput::None, output: TrackOutput::Default, monitor: MonitorMode::Auto, scale: Default::default(),
            })},
            Op::Insert { entity: Entity::Clip(Clip {
                id: clip, track, start: Beats(beats[0]),
                name: String::new(), color: None, muted: false, length: Beats(beats[1] + 1e-3),
                offset: Beats(beats[2]),
                looping: ClipLoop { enabled: true, start: Beats(beats[3]), end: Beats(beats[3] * 2.0 + 1.0) },
                content: ClipContent::Midi,
            })},
            Op::Insert { entity: Entity::Note(Note {
                id: note, clip, pitch: 60, velocity: unit as f32, release_velocity: pan.abs(),
                start: Beats(beats[1]), duration: Beats(beats[2] + 1e-3), muted: false,
            })},
            Op::Insert { entity: Entity::Device(Device {
                id: device, track, order: OrderKey::between(None, None), name: "d".into(),
                enabled: true, kind: DeviceKind::Builtin { device: BuiltinDevice::Synth },
                params: [(ParamId(1), param)].into(), sidechain: None, pad: None,
            })},
            Op::Insert { entity: Entity::AutomationLane(AutomationLane {
                id: lane, owner: AutomationOwner::Track { track },
                target: AutomationTarget::DeviceParam { device, param: ParamId(1) }, enabled: true,
            })},
            Op::Insert { entity: Entity::AutomationPoint(AutomationPoint {
                id: point, lane, time: Beats(beats[0]), value: unit,
                curve: CurveShape::Curve { tension: pan },
            })},
            Op::Insert { entity: Entity::TempoPoint(TempoPoint {
                id: tempo, time: Beats(beats[3]), bpm, curve: TempoCurve::Linear,
            })},
        ];
        p.apply_all(&ops).unwrap();
        let back = file::load(&file::save(&p, "0.1.0").unwrap()).unwrap();
        prop_assert_eq!(back.devices[&device].params[&ParamId(1)].to_bits(), param.to_bits());
        prop_assert_eq!(back.tracks[&track].mixer.volume.0.to_bits(), db.to_bits());
        prop_assert_eq!(&back, &p);
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
