//! Parent → children indexes: kept in sync through apply, undo and patch
//! mirroring, used by the dependent checks, and never trusted once the tables were edited
//! behind the model's back.

use ether_model::*;

const NOW: u64 = 1_700_000_000_000;

struct Doc {
    p: Project,
    ids: IdGen,
    track: TrackId,
    clip: ClipId,
    note: NoteId,
}

fn doc() -> Doc {
    let mut ids = IdGen::new(5);
    let mut p = Project::new(&mut ids, NOW);
    let track: TrackId = ids.next(NOW);
    let clip: ClipId = ids.next(NOW);
    let note: NoteId = ids.next(NOW);
    let lane: AutomationLaneId = ids.next(NOW);
    let ops = vec![
        Op::Insert {
            entity: Entity::Track(Track {
                vca: Default::default(),
                freeze: None,
                id: track,
                kind: TrackKind::Midi,
                name: "m".into(),
                color: Color(1),
                order: OrderKey::between(None, None),
                parent: None,
                mixer: TrackMixer::default(),
                input: TrackInput::None,
                output: TrackOutput::Default,
                monitor: MonitorMode::Auto,
                scale: Default::default(),
                mpe: None,
            }),
        },
        Op::Insert {
            entity: Entity::Clip(Clip {
                lane: None,
                id: clip,
                track,
                start: Beats(0.0),
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
        },
        Op::Insert {
            entity: Entity::Note(Note {
                id: note,
                clip,
                pitch: 60,
                velocity: 0.8,
                release_velocity: 0.5,
                start: Beats(1.0),
                duration: Beats(0.5),
                muted: false,
            }),
        },
        Op::Insert {
            entity: Entity::AutomationLane(AutomationLane {
                id: lane,
                owner: AutomationOwner::Clip { clip },
                target: AutomationTarget::TrackVolume { track },
                enabled: true,
            }),
        },
        Op::Insert {
            entity: Entity::AutomationPoint(AutomationPoint {
                id: ids.next(NOW),
                lane,
                time: Beats(0.0),
                value: 0.5,
                curve: CurveShape::Linear,
            }),
        },
        Op::Insert {
            entity: Entity::ExpressionLane(ExpressionLane {
                id: ids.next(NOW),
                clip,
                kind: ExpressionKind::PitchBend,
                points: vec![],
            }),
        },
        Op::Insert {
            entity: Entity::NoteExpression(NoteExpression {
                id: ids.next(NOW),
                note,
                kind: NoteExpressionKind::Pressure,
                points: vec![],
            }),
        },
    ];
    p.apply_all(&ops).unwrap();
    Doc {
        p,
        ids,
        track,
        clip,
        note,
    }
}

#[test]
fn index_follows_apply_undo_and_patches() {
    let Doc {
        mut p,
        mut ids,
        track,
        clip,
        note,
    } = doc();
    assert!(p.index_is_consistent());
    let mut mirror = p.clone();
    assert!(mirror.index_is_consistent());

    // Children block removals, through the index.
    assert!(matches!(
        p.apply(&Op::Remove {
            key: EntityKey::Clip(clip)
        }),
        Err(ModelError::HasChildren(_))
    ));
    assert!(matches!(
        p.apply(&Op::Remove {
            key: EntityKey::Note(note)
        }),
        Err(ModelError::HasChildren(_))
    ));
    assert!(matches!(
        p.apply(&Op::Remove {
            key: EntityKey::Track(track)
        }),
        Err(ModelError::HasChildren(_))
    ));

    // Move the clip to another track (an update that changes the parent), then undo.
    let other: TrackId = ids.next(NOW);
    let mut t = p.tracks[&track].clone();
    t.id = other;
    p.apply(&Op::Insert {
        entity: Entity::Track(t),
    })
    .unwrap();
    let ops = [Op::Update {
        update: EntityUpdate::Clip {
            id: clip,
            change: ClipChange::Track(other),
        },
    }];
    let undo = p.apply_all(&ops).unwrap();
    assert!(p.index_is_consistent());
    assert_eq!(p.clip_ids_of(other), vec![clip]);
    assert!(p.clip_ids_of(track).is_empty());
    mirror.apply_patch_changes(&full_changes(&p));
    assert_eq!(mirror, p);
    assert!(mirror.index_is_consistent());
    p.apply_all(&undo).unwrap();
    assert!(p.index_is_consistent());
    assert_eq!(p.clip_ids_of(track), vec![clip]);

    // Removing children bottom-up empties the index.
    let mut removals = Vec::new();
    for e in p.note_expression_ids_of(note) {
        removals.push(Op::Remove {
            key: EntityKey::NoteExpression(e),
        });
    }
    removals.push(Op::Remove {
        key: EntityKey::Note(note),
    });
    for l in p.expression_lane_ids_of(clip) {
        removals.push(Op::Remove {
            key: EntityKey::ExpressionLane(l),
        });
    }
    for l in p.lane_ids_owned_by(AutomationOwner::Clip { clip }) {
        for pt in p.point_ids_of(l) {
            removals.push(Op::Remove {
                key: EntityKey::AutomationPoint(pt),
            });
        }
        removals.push(Op::Remove {
            key: EntityKey::AutomationLane(l),
        });
    }
    removals.push(Op::Remove {
        key: EntityKey::Clip(clip),
    });
    let restore = p.apply_all(&removals).unwrap();
    assert!(p.index_is_consistent());
    assert!(p.notes.is_empty() && p.automation_points.is_empty());
    p.apply_all(&restore).unwrap();
    assert!(p.index_is_consistent());
    assert_eq!(p.note_ids_of(clip), vec![note]);

    // A save/load round-trip comes back indexed.
    let back = file::load(&file::save(&p, "0.3.0").unwrap()).unwrap();
    assert!(back.index_is_consistent());
}

#[test]
fn direct_table_edits_never_trust_a_stale_index() {
    let Doc {
        mut p,
        mut ids,
        clip,
        note,
        ..
    } = doc();
    // A row inserted behind the model's back: the accessors scan instead.
    let extra: NoteId = ids.next(NOW);
    let mut n = p.notes[&note].clone();
    n.id = extra;
    p.notes.insert(extra, n);
    assert!(!p.index_is_consistent());
    assert_eq!(p.note_ids_of(clip).len(), 2);
    assert_eq!(p.notes_of(clip).len(), 2);
    // `apply` rebuilds it first.
    p.apply(&Op::Remove {
        key: EntityKey::Note(extra),
    })
    .unwrap();
    assert!(p.index_is_consistent());

    // A parent changed in place: invalidate, then the next apply rebuilds.
    let lane = p.lane_ids_owned_by(AutomationOwner::Clip { clip })[0];
    let point = p.point_ids_of(lane)[0];
    let other: AutomationLaneId = ids.next(NOW);
    let mut l = p.automation_lanes[&lane].clone();
    l.id = other;
    p.apply(&Op::Insert {
        entity: Entity::AutomationLane(l),
    })
    .unwrap();
    p.automation_points.get_mut(&point).unwrap().lane = other;
    p.invalidate_index();
    assert!(p.point_ids_of(lane).is_empty());
    assert_eq!(p.point_ids_of(other), vec![point]);
    p.apply(&Op::Remove {
        key: EntityKey::AutomationLane(lane),
    })
    .unwrap();
    assert!(p.index_is_consistent());
}

/// `notes_revision` changes exactly when a note of the clip changes (insert, edit in place,
/// removal), through apply and undo, and never matches across index builds.
#[test]
fn notes_revision_follows_note_changes_only() {
    let Doc {
        mut p,
        mut ids,
        track,
        clip,
        note,
    } = doc();
    let other: ClipId = ids.next(NOW);
    let r0 = p.notes_revision(clip).unwrap();
    let o0 = p.notes_revision(other).unwrap();
    // Not a note: same revision.
    p.apply(&Op::Update {
        update: EntityUpdate::Track {
            id: track,
            change: TrackChange::Name("x".into()),
        },
    })
    .unwrap();
    assert_eq!(p.notes_revision(clip), Some(r0));
    // Edited in place.
    let inv = p
        .apply(&Op::Update {
            update: EntityUpdate::Note {
                id: note,
                change: NoteChange::Pitch(61),
            },
        })
        .unwrap();
    let r1 = p.notes_revision(clip).unwrap();
    assert_ne!(r1, r0);
    // A clone keeps the revisions until it changes on its own.
    let mut copy = p.clone();
    assert_eq!(copy.notes_revision(clip), Some(r1));
    // Undo is a change too (back to the old notes, a new revision).
    p.apply(&inv).unwrap();
    let r2 = p.notes_revision(clip).unwrap();
    assert!(r2 != r1 && r2 != r0);
    copy.apply(&Op::Insert {
        entity: Entity::Note(Note {
            id: ids.next(NOW),
            clip,
            pitch: 64,
            velocity: 0.8,
            release_velocity: 0.5,
            start: Beats(2.0),
            duration: Beats(0.5),
            muted: false,
        }),
    })
    .unwrap();
    assert!(copy.notes_revision(clip) != Some(r1) && copy.notes_revision(clip) != Some(r2));
    // Other clips untouched.
    assert_eq!(p.notes_revision(other), Some(o0));
    // Rows changed behind the model's back: no revision until the index is rebuilt, then
    // none of the old ones.
    p.notes.clear();
    assert_eq!(p.notes_revision(clip), None);
    p.ensure_index();
    let r3 = p.notes_revision(clip).unwrap();
    assert!(r3 != r0 && r3 != r1 && r3 != r2);
}

/// The dependents index ("who references this entity") answers like a scan of the tables,
/// and a reference changed in place is never reported once it no longer holds.
#[test]
fn dependents_index_answers_like_a_scan() {
    let Doc {
        mut p,
        mut ids,
        track,
        clip,
        ..
    } = doc();
    let device: DeviceId = ids.next(NOW);
    let lane: AutomationLaneId = ids.next(NOW);
    p.apply_all(&[
        Op::Insert {
            entity: Entity::Device(Device {
                sidechain_tap: None,
                midi_sidechain: None,
                chain: None,
                id: device,
                track,
                order: OrderKey::between(None, None),
                name: "u".into(),
                enabled: true,
                kind: DeviceKind::Builtin {
                    device: BuiltinDevice::Utility,
                },
                params: Default::default(),
                sidechain: None,
                pad: None,
            }),
        },
        Op::Insert {
            entity: Entity::AutomationLane(AutomationLane {
                id: lane,
                owner: AutomationOwner::Track { track },
                target: AutomationTarget::DeviceEnabled { device },
                enabled: true,
            }),
        },
    ])
    .unwrap();
    assert!(p.index_is_consistent());
    let clip_lane = p.lane_ids_owned_by(AutomationOwner::Clip { clip })[0];
    let mut want = vec![
        EntityKey::Clip(clip),
        EntityKey::Device(device),
        EntityKey::AutomationLane(lane),
        EntityKey::AutomationLane(clip_lane),
    ];
    want.sort();
    assert_eq!(p.referencing(EntityKey::Track(track)), want);
    assert_eq!(
        p.referencing(EntityKey::Device(device)),
        vec![EntityKey::AutomationLane(lane)]
    );
    // Same answers from the scan.
    let mut scanned = p.clone();
    scanned.invalidate_index();
    assert_eq!(scanned.referencing(EntityKey::Track(track)), want);
    // Referenced: not removable; the lane goes first, then the device can.
    assert!(matches!(
        p.apply(&Op::Remove {
            key: EntityKey::Device(device)
        }),
        Err(ModelError::HasChildren(_))
    ));
    p.apply(&Op::Remove {
        key: EntityKey::AutomationLane(lane),
    })
    .unwrap();
    assert!(p.referencing(EntityKey::Device(device)).is_empty());
    assert!(p.index_is_consistent());
    // A reference dropped in place (without `invalidate_index`): never reported.
    p.automation_lanes.get_mut(&clip_lane).unwrap().target = AutomationTarget::ClipGain;
    assert!(
        !p.referencing(EntityKey::Track(track))
            .contains(&EntityKey::AutomationLane(clip_lane))
    );
}
