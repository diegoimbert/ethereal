//! Targeted tests of document rules, History and patches.

use ether_model::*;

struct Fx {
    p: Project,
    ids: IdGen,
    now: u64,
}

impl Fx {
    fn new() -> Self {
        let mut ids = IdGen::new(1);
        let p = Project::new(&mut ids, 1_000);
        Self { p, ids, now: 1_000 }
    }
    fn id<I: Id>(&mut self) -> I {
        self.now += 1;
        self.ids.next(self.now)
    }
    fn track(&mut self, kind: TrackKind, parent: Option<TrackId>) -> Track {
        Track {
            id: self.id(),
            kind,
            name: "t".into(),
            color: Color(0),
            order: OrderKey::between(None, None),
            parent,
            mixer: TrackMixer::default(),
            input: TrackInput::None,
            output: TrackOutput::Default,
            monitor: MonitorMode::Auto,
        }
    }
    fn add_track(&mut self, kind: TrackKind, parent: Option<TrackId>) -> TrackId {
        let t = self.track(kind, parent);
        let id = t.id;
        self.p
            .apply(&Op::Insert {
                entity: Entity::Track(t),
            })
            .unwrap();
        id
    }
    fn midi_clip(&mut self, track: TrackId, location: ClipLocation) -> Clip {
        Clip {
            id: self.id(),
            track,
            location,
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
            launch: LaunchSettings::default(),
            content: ClipContent::Midi,
        }
    }
}

fn upd_track(id: TrackId, change: TrackChange) -> Op {
    Op::Update {
        update: EntityUpdate::Track { id, change },
    }
}

#[test]
fn new_project_is_valid_and_deterministic() {
    let a = Project::new(&mut IdGen::new(5), 42);
    let b = Project::new(&mut IdGen::new(5), 42);
    assert_eq!(a, b);
    a.validate().unwrap();
    assert_eq!(a.master_track().kind, TrackKind::Master);
    assert_eq!(a.scenes_ordered().len(), NEW_PROJECT_SCENES);
    let names: Vec<_> = a.scenes_ordered().iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["1", "2", "3", "4"]);
    assert_eq!(a.tempo_map().bpm_at(Beats::ZERO), 120.0);
    assert_eq!(a.id.0.get_version_num(), 7);
}

#[test]
fn structural_errors() {
    let mut f = Fx::new();
    let master = f.p.master_track().id;
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Track(master)
        }),
        Err(ModelError::Invariant(_))
    ));
    let t = f.track(TrackKind::Master, None);
    assert!(matches!(
        f.p.apply(&Op::Insert {
            entity: Entity::Track(t)
        }),
        Err(ModelError::Invariant(_))
    ));
    let midi = f.add_track(TrackKind::Midi, None);
    let t = f.p.tracks[&midi].clone();
    assert!(matches!(
        f.p.apply(&Op::Insert {
            entity: Entity::Track(t)
        }),
        Err(ModelError::AlreadyExists(_))
    ));
    let missing: NoteId = f.id();
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Note(missing)
        }),
        Err(ModelError::NotFound(_))
    ));
}

#[test]
fn remove_requires_no_dependents() {
    let mut f = Fx::new();
    let midi = f.add_track(TrackKind::Midi, None);
    let clip = f.midi_clip(midi, ClipLocation::Arrangement { start: Beats(0.0) });
    let clip_id = clip.id;
    f.p.apply(&Op::Insert {
        entity: Entity::Clip(clip),
    })
    .unwrap();
    assert_eq!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Track(midi)
        }),
        Err(ModelError::HasChildren(EntityKey::Track(midi)))
    );
    // Children first, then the parent: fine.
    f.p.apply_all(&[
        Op::Remove {
            key: EntityKey::Clip(clip_id),
        },
        Op::Remove {
            key: EntityKey::Track(midi),
        },
    ])
    .unwrap();
    f.p.validate().unwrap();
}

#[test]
fn session_slots_are_unique_and_derived() {
    let mut f = Fx::new();
    let midi = f.add_track(TrackKind::Midi, None);
    let scene = f.p.scenes_ordered()[0].id;
    let a = f.midi_clip(midi, ClipLocation::Session { scene });
    let a_id = a.id;
    f.p.apply(&Op::Insert {
        entity: Entity::Clip(a),
    })
    .unwrap();
    assert_eq!(f.p.clip_slot(midi, scene).clip, Some(a_id));
    let b = f.midi_clip(midi, ClipLocation::Session { scene });
    assert!(matches!(
        f.p.apply(&Op::Insert {
            entity: Entity::Clip(b)
        }),
        Err(ModelError::Invariant(_))
    ));
    // A scene holding a clip can't be removed.
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Scene(scene)
        }),
        Err(ModelError::HasChildren(_))
    ));
}

#[test]
fn audio_fields_on_midi_clip_fail_and_leave_project_unchanged() {
    let mut f = Fx::new();
    let midi = f.add_track(TrackKind::Midi, None);
    let clip = f.midi_clip(midi, ClipLocation::Arrangement { start: Beats(0.0) });
    let id = clip.id;
    f.p.apply(&Op::Insert {
        entity: Entity::Clip(clip),
    })
    .unwrap();
    let before = f.p.clone();
    let err = f.p.apply(&Op::Update {
        update: EntityUpdate::Clip {
            id,
            change: ClipChange::Gain(Decibels(1.0)),
        },
    });
    assert!(matches!(err, Err(ModelError::InvalidValue(_))));
    assert_eq!(f.p, before);
}

#[test]
fn routing_and_grouping_cycles_are_rejected() {
    let mut f = Fx::new();
    let g1 = f.add_track(TrackKind::Group, None);
    let g2 = f.add_track(TrackKind::Group, Some(g1));
    // g1 into its own child group: grouping cycle.
    assert!(
        f.p.apply(&upd_track(g1, TrackChange::Parent(Some(g2))))
            .is_err()
    );
    // g1 outputs to g2 while g2 (Default) feeds g1: routing cycle.
    assert!(matches!(
        f.p.apply(&upd_track(
            g1,
            TrackChange::Output(TrackOutput::Track { track: g2 })
        )),
        Err(ModelError::Invariant(_))
    ));
    // Return → return sends that loop.
    let r1 = f.add_track(TrackKind::Return, None);
    let r2 = f.add_track(TrackKind::Return, None);
    let s1: SendId = f.id();
    let s2: SendId = f.id();
    let send = |id, from, to| Op::Insert {
        entity: Entity::Send(TrackSend {
            id,
            from,
            to,
            level: Decibels(0.0),
            pre_fader: false,
        }),
    };
    f.p.apply(&send(s1, r1, r2)).unwrap();
    assert!(f.p.apply(&send(s2, r2, r1)).is_err());
    // Sends must target returns.
    assert!(f.p.apply(&send(s2, r1, g1)).is_err());
    f.p.validate().unwrap();
    // Group children ordering.
    let a = f.add_track(TrackKind::Audio, Some(g1));
    assert_eq!(
        f.p.child_tracks(g1)
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
            .len(),
        2
    );
    assert!(f.p.child_tracks(g1).iter().any(|t| t.id == a));
}

#[test]
fn tempo_and_signature_at_zero_are_required() {
    let mut f = Fx::new();
    let tp = *f.p.tempo_points.keys().next().unwrap();
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::TempoPoint(tp)
        }),
        Err(ModelError::Invariant(_))
    ));
    assert!(
        f.p.apply(&Op::Update {
            update: EntityUpdate::TempoPoint {
                id: tp,
                change: TempoPointChange::Time(Beats(4.0)),
            },
        })
        .is_err()
    );
    let ts = *f.p.time_signatures.keys().next().unwrap();
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::TimeSignature(ts)
        })
        .is_err()
    );
}

#[test]
fn history_gestures_merge_and_labels() {
    let mut f = Fx::new();
    let t = f.add_track(TrackKind::Audio, None);
    let initial = f.p.clone();
    let mut h = History::new(0);
    assert_eq!(h.state(), HistoryState::default());
    let vol = |db| Transaction {
        label: "Volume".into(),
        ops: vec![upd_track(t, TrackChange::Volume(Decibels(db)))],
    };
    let g = Some(GestureId(1));
    for db in [-1.0, -2.0, -3.0] {
        h.commit(&mut f.p, vol(db), g).unwrap();
    }
    assert_eq!(f.p.tracks[&t].mixer.volume, Decibels(-3.0));
    h.end_gesture(GestureId(1));
    h.commit(&mut f.p, vol(-4.0), g).unwrap(); // new step: gesture was ended
    let st = h.state();
    assert!(st.can_undo && !st.can_redo);
    assert_eq!(st.undo_label.as_deref(), Some("Volume"));

    h.undo(&mut f.p).unwrap().unwrap();
    assert_eq!(f.p.tracks[&t].mixer.volume, Decibels(-3.0));
    let undone = h.undo(&mut f.p).unwrap().unwrap();
    assert_eq!(f.p, initial);
    assert_eq!(undone.len(), 3);
    assert!(h.undo(&mut f.p).unwrap().is_none());
    assert_eq!(h.state().redo_label.as_deref(), Some("Volume"));

    h.redo(&mut f.p).unwrap().unwrap();
    assert_eq!(f.p.tracks[&t].mixer.volume, Decibels(-3.0));
    // A new commit clears redo.
    h.commit(&mut f.p, vol(-9.0), None).unwrap();
    assert!(!h.state().can_redo);
    h.clear();
    assert_eq!(h.state(), HistoryState::default());
}

#[test]
fn history_max_depth_drops_oldest_and_failed_commit_records_nothing() {
    let mut f = Fx::new();
    let t = f.add_track(TrackKind::Audio, None);
    let mut h = History::new(2);
    for i in 0..5 {
        h.commit(
            &mut f.p,
            Transaction {
                label: format!("v{i}"),
                ops: vec![upd_track(t, TrackChange::Volume(Decibels(-(i as f32))))],
            },
            None,
        )
        .unwrap();
    }
    let before = f.p.clone();
    let bad = Transaction {
        label: "bad".into(),
        ops: vec![
            upd_track(t, TrackChange::Mute(true)),
            upd_track(t, TrackChange::Pan(Pan(5.0))),
        ],
    };
    assert!(h.commit(&mut f.p, bad, None).is_err());
    assert_eq!(f.p, before);
    assert_eq!(h.state().undo_label.as_deref(), Some("v4"));
    assert!(h.undo(&mut f.p).unwrap().is_some());
    assert!(h.undo(&mut f.p).unwrap().is_some());
    assert!(h.undo(&mut f.p).unwrap().is_none());
    assert_eq!(f.p.tracks[&t].mixer.volume, Decibels(-2.0));
}

#[test]
fn patches_coalesce_per_entity() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio, None);
    let id = t.id;
    let ops = vec![
        Op::Insert {
            entity: Entity::Track(t),
        },
        upd_track(id, TrackChange::Mute(true)),
        upd_track(id, TrackChange::Name("x".into())),
        Op::Settings {
            change: SettingsChange::Metronome(true),
        },
        Op::Settings {
            change: SettingsChange::CountInBars(1),
        },
    ];
    f.p.apply_all(&ops).unwrap();
    let changes = changes_for(&f.p, &ops);
    assert_eq!(changes.len(), 2);
    match &changes[0] {
        PatchChange::Upsert {
            entity: Entity::Track(t),
        } => assert!(t.mixer.mute && t.name == "x"),
        c => panic!("unexpected {c:?}"),
    }
    assert!(
        matches!(&changes[1], PatchChange::Settings { settings } if settings.metronome && settings.count_in_bars == 1)
    );

    f.p.apply(&Op::Remove {
        key: EntityKey::Track(id),
    })
    .unwrap();
    let rm = changes_for(
        &f.p,
        &[Op::Remove {
            key: EntityKey::Track(id),
        }],
    );
    assert_eq!(
        rm,
        vec![PatchChange::Remove {
            key: EntityKey::Track(id)
        }]
    );

    // Full-state changes rebuild the project on an empty mirror.
    let mut mirror = f.p.clone();
    mirror.tracks.clear();
    mirror.scenes.clear();
    mirror.apply_patch_changes(&full_changes(&f.p));
    assert_eq!(mirror, f.p);

    // Patch JSON shape for the UI.
    let json = serde_json::to_value(&changes[1]).unwrap();
    assert_eq!(json["type"], "Settings");
}

#[test]
fn params_roundtrip_through_json_and_param_reset() {
    let mut f = Fx::new();
    let t = f.add_track(TrackKind::Midi, None);
    let d = Device {
        id: f.id(),
        track: t,
        order: OrderKey::between(None, None),
        name: "Synth".into(),
        enabled: true,
        kind: DeviceKind::Builtin {
            device: BuiltinDevice::Synth,
        },
        params: [(ParamId(3), 440.0)].into(),
    };
    let did = d.id;
    f.p.apply(&Op::Insert {
        entity: Entity::Device(d),
    })
    .unwrap();
    let inv =
        f.p.apply(&Op::Update {
            update: EntityUpdate::Device {
                id: did,
                change: DeviceChange::Param {
                    param: ParamId(3),
                    value: None,
                },
            },
        })
        .unwrap();
    assert!(f.p.devices[&did].params.is_empty());
    f.p.apply(&inv).unwrap();
    assert_eq!(f.p.devices[&did].params[&ParamId(3)], 440.0);
    // Kind changes must keep the device type.
    assert!(
        f.p.apply(&Op::Update {
            update: EntityUpdate::Device {
                id: did,
                change: DeviceChange::Kind(DeviceKind::Builtin {
                    device: BuiltinDevice::Delay,
                }),
            },
        })
        .is_err()
    );
    let json = file::save(&f.p, "0.1.0").unwrap();
    assert_eq!(file::load(&json).unwrap(), f.p);
}
