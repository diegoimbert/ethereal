//! v0.2 model contracts (contracts-3): take lanes and comp regions, rack chains,
//! modulators and mappings, freeze, external media, VCAs and input taps, `.ether` v4.

use ether_model::*;

struct Fx {
    p: Project,
    ids: IdGen,
    now: u64,
}

impl Fx {
    fn new() -> Self {
        let mut ids = IdGen::new(5);
        let p = Project::new(&mut ids, 1_000);
        Self { p, ids, now: 1_000 }
    }
    fn id<I: Id>(&mut self) -> I {
        self.now += 1;
        self.ids.next(self.now)
    }
    fn insert(&mut self, e: Entity) -> Result<Op, ModelError> {
        self.p.apply(&Op::Insert { entity: e })
    }
    fn update(&mut self, update: EntityUpdate) -> Result<Op, ModelError> {
        self.p.apply(&Op::Update { update })
    }
    fn track(&mut self, kind: TrackKind) -> TrackId {
        let id = self.id();
        self.insert(Entity::Track(Track {
            id,
            kind,
            name: "t".into(),
            color: Color(0),
            order: OrderKey::between(None, None),
            parent: None,
            mixer: TrackMixer::default(),
            input: TrackInput::None,
            output: TrackOutput::Default,
            monitor: MonitorMode::Auto,
            scale: Default::default(),
            freeze: None,
            vca: None,
            mpe: None,
        }))
        .unwrap();
        id
    }
    fn device(&mut self, track: TrackId, device: BuiltinDevice) -> Device {
        Device {
            id: self.id(),
            track,
            order: OrderKey::between(None, None),
            name: "d".into(),
            enabled: true,
            kind: DeviceKind::Builtin { device },
            params: Default::default(),
            sidechain: None,
            pad: None,
            chain: None,
        }
    }
    fn media(&mut self, location: MediaLocation) -> MediaId {
        let id = self.id();
        self.insert(Entity::Media(MediaRef {
            id,
            name: "a.wav".into(),
            file: format!("media/{id}-a.wav"),
            sample_rate: 48_000,
            channels: 2,
            frames: 48_000,
            hash: Some("abc".into()),
            location,
        }))
        .unwrap();
        id
    }
    fn lane(&mut self, track: TrackId) -> TakeLaneId {
        let id = self.id();
        self.insert(Entity::TakeLane(TakeLane {
            id,
            track,
            order: OrderKey::between(None, None),
            name: "Take 1".into(),
            color: None,
        }))
        .unwrap();
        id
    }
    fn region(
        &mut self,
        track: TrackId,
        lane: TakeLaneId,
        start: f64,
        end: f64,
    ) -> Result<CompRegionId, ModelError> {
        let id = self.id();
        self.insert(Entity::CompRegion(CompRegion {
            id,
            track,
            lane,
            start: Beats(start),
            end: Beats(end),
            crossfade: DEFAULT_COMP_CROSSFADE,
        }))
        .map(|_| id)
    }
}

#[test]
fn take_lanes_and_comp_regions() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let other = f.track(TrackKind::Midi);
    let l1 = f.lane(t);
    let l2 = f.lane(t);
    // Lanes only on audio/MIDI tracks.
    let group = f.track(TrackKind::Group);
    let bad = TakeLane {
        id: f.id(),
        track: group,
        order: OrderKey::between(None, None),
        name: "x".into(),
        color: None,
    };
    assert!(matches!(
        f.insert(Entity::TakeLane(bad)),
        Err(ModelError::Invariant(_))
    ));
    // Take clips are ordinary clips on a lane of their track.
    let clip = Clip {
        id: f.id(),
        track: t,
        start: Beats(0.0),
        name: "take".into(),
        color: None,
        muted: false,
        length: Beats(8.0),
        offset: Beats(0.0),
        looping: ClipLoop {
            enabled: false,
            start: Beats(0.0),
            end: Beats(8.0),
        },
        content: ClipContent::Midi,
        lane: Some(l1),
    };
    f.insert(Entity::Clip(clip.clone())).unwrap();
    assert!(
        f.p.arrangement_clips_of(t).is_empty(),
        "lane clips are not main-lane clips"
    );
    assert_eq!(f.p.lane_clips_of(l1).len(), 1);
    let wrong = Clip {
        id: f.id(),
        track: other,
        ..clip.clone()
    };
    assert!(matches!(
        f.insert(Entity::Clip(wrong)),
        Err(ModelError::Invariant(_))
    ));
    // Regions: touching is fine, overlapping is not.
    let a = f.region(t, l1, 0.0, 4.0).unwrap();
    f.region(t, l2, 4.0, 8.0).unwrap();
    assert!(matches!(
        f.region(t, l2, 3.0, 5.0),
        Err(ModelError::Invariant(_))
    ));
    assert!(f.region(t, l2, 5.0, 5.0).is_err(), "empty range");
    assert!(
        f.region(other, l1, 8.0, 9.0).is_err(),
        "lane of another track"
    );
    // A range update keeps the no-overlap rule.
    assert!(
        f.update(EntityUpdate::CompRegion {
            id: a,
            change: CompRegionChange::Range(BeatRange {
                start: Beats(0.0),
                end: Beats(6.0),
            }),
        })
        .is_err()
    );
    // Crossfade bounds.
    assert!(
        f.update(EntityUpdate::CompRegion {
            id: a,
            change: CompRegionChange::Crossfade(Seconds(2.0)),
        })
        .is_err()
    );
    // A lane with clips or regions cannot be removed; the track neither.
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::TakeLane(l1)
        }),
        Err(ModelError::HasChildren(_))
    ));
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Track(t)
        })
        .is_err()
    );
    assert_eq!(f.p.comp_of(t).len(), 2);
    f.p.validate().unwrap();
}

#[test]
fn rack_chains_and_nesting() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let rack = f.device(t, BuiltinDevice::InstrumentRack);
    f.insert(Entity::Device(rack.clone())).unwrap();
    let chain = RackChain {
        id: f.id(),
        rack: rack.id,
        order: OrderKey::between(None, None),
        name: "Chain".into(),
        color: None,
        volume: Decibels::UNITY,
        pan: Pan(0.0),
        mute: false,
        solo: false,
        keys: Zone::FULL,
        velocities: Zone { lo: 1, hi: 127 },
        select: Zone::FULL,
    };
    f.insert(Entity::RackChain(chain.clone())).unwrap();
    // Chains belong to racks.
    let synth = f.device(t, BuiltinDevice::PolySynth);
    f.insert(Entity::Device(synth.clone())).unwrap();
    let bad = RackChain {
        id: f.id(),
        rack: synth.id,
        ..chain.clone()
    };
    assert!(f.insert(Entity::RackChain(bad)).is_err());
    let bad_zone = RackChain {
        id: f.id(),
        keys: Zone { lo: 60, hi: 10 },
        ..chain.clone()
    };
    assert!(f.insert(Entity::RackChain(bad_zone)).is_err());
    // A chain device, on the rack's track; no racks inside chains.
    let mut inner = f.device(t, BuiltinDevice::PolySynth);
    inner.chain = Some(chain.id);
    f.insert(Entity::Device(inner.clone())).unwrap();
    assert!(f.p.devices_of(t).iter().all(|d| d.id != inner.id));
    assert_eq!(f.p.chain_devices_of(chain.id).len(), 1);
    let mut nested = f.device(t, BuiltinDevice::AudioEffectRack);
    nested.chain = Some(chain.id);
    assert!(f.insert(Entity::Device(nested)).is_err());
    // The rack can't be removed while it has chains; the chain while it has devices.
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Device(rack.id)
        })
        .is_err()
    );
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::RackChain(chain.id)
        })
        .is_err()
    );
    f.p.validate().unwrap();
}

#[test]
fn modulators_and_mapping_scope() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let synth = f.device(t, BuiltinDevice::PolySynth);
    f.insert(Entity::Device(synth.clone())).unwrap();
    let other = f.device(t, BuiltinDevice::Chorus);
    f.insert(Entity::Device(other.clone())).unwrap();
    let lfo = Modulator {
        sidechain: Default::default(),
        id: f.id(),
        device: synth.id,
        order: OrderKey::between(None, None),
        name: "LFO".into(),
        kind: ModulatorKind::Lfo,
        params: Default::default(),
    };
    f.insert(Entity::Modulator(lfo.clone())).unwrap();
    let map = ModMapping {
        id: f.id(),
        source: ModSource::Modulator { modulator: lfo.id },
        device: synth.id,
        param: ParamId(26),
        depth: 0.5,
    };
    f.insert(Entity::ModMapping(map.clone())).unwrap();
    // Same (source, target) twice.
    let dup = ModMapping {
        id: f.id(),
        ..map.clone()
    };
    assert!(f.insert(Entity::ModMapping(dup)).is_err());
    // Out of scope: another device of the track (the host is not a rack).
    let far = ModMapping {
        id: f.id(),
        device: other.id,
        ..map.clone()
    };
    assert!(matches!(
        f.insert(Entity::ModMapping(far)),
        Err(ModelError::Invariant(_))
    ));
    // Depth bounds.
    let deep = ModMapping {
        id: f.id(),
        param: ParamId(27),
        depth: 1.5,
        ..map.clone()
    };
    assert!(f.insert(Entity::ModMapping(deep)).is_err());
    // Macros target devices inside their rack (or the rack's non-macro params).
    let rack = f.device(t, BuiltinDevice::AudioEffectRack);
    f.insert(Entity::Device(rack.clone())).unwrap();
    let own = ModMapping {
        id: f.id(),
        source: ModSource::Macro {
            rack: rack.id,
            index: 0,
        },
        device: rack.id,
        param: RACK_SELECTOR_PARAM,
        depth: 1.0,
    };
    f.insert(Entity::ModMapping(own.clone())).unwrap();
    let macro_to_macro = ModMapping {
        id: f.id(),
        param: rack_macro_param(1),
        ..own.clone()
    };
    assert!(f.insert(Entity::ModMapping(macro_to_macro)).is_err());
    let bad_index = ModMapping {
        id: f.id(),
        source: ModSource::Macro {
            rack: rack.id,
            index: RACK_MACROS,
        },
        param: ParamId(9),
        ..own
    };
    assert!(f.insert(Entity::ModMapping(bad_index)).is_err());
    // No modulation of modulation: the rack's own modulator can't drive its macros.
    let rack_lfo = Modulator {
        id: f.id(),
        device: rack.id,
        order: OrderKey::between(None, None),
        name: "LFO".into(),
        kind: ModulatorKind::Lfo,
        params: Default::default(),
        sidechain: None,
    };
    f.insert(Entity::Modulator(rack_lfo.clone())).unwrap();
    let onto_macro = ModMapping {
        id: f.id(),
        source: ModSource::Modulator {
            modulator: rack_lfo.id,
        },
        device: rack.id,
        param: rack_macro_param(0),
        depth: 0.5,
    };
    assert!(matches!(
        f.insert(Entity::ModMapping(onto_macro)),
        Err(ModelError::Invariant(_))
    ));
    // The host can't be removed while mapped; the modulator neither.
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Device(synth.id)
        })
        .is_err()
    );
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Modulator(lfo.id)
        })
        .is_err()
    );
    assert_eq!(f.p.mappings_to(synth.id).len(), 1);
    assert_eq!(f.p.modulators_of(synth.id).len(), 1);
    f.p.validate().unwrap();
}

#[test]
fn freeze_media_and_vcas() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio);
    let ext = f.media(MediaLocation::External {
        path: "/Samples/kick.wav".into(),
    });
    let own = f.media(MediaLocation::Project);
    // Freeze renders are project media.
    let freeze = |media| EntityUpdate::Track {
        id: t,
        change: TrackChange::Freeze(Some(TrackFreeze {
            media,
            start: Seconds(0.0),
        })),
    };
    assert!(f.update(freeze(ext)).is_err());
    f.update(freeze(own)).unwrap();
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Media(own)
        })
        .is_err()
    );
    let group = f.track(TrackKind::Group);
    assert!(
        f.update(EntityUpdate::Track {
            id: group,
            change: TrackChange::Freeze(Some(TrackFreeze {
                media: own,
                start: Seconds(0.0),
            })),
        })
        .is_err()
    );
    // VCAs: assignment targets VCA tracks, no cycles, no devices on VCAs.
    let v1 = f.track(TrackKind::Vca);
    let v2 = f.track(TrackKind::Vca);
    let vca = |id, vca| EntityUpdate::Track {
        id,
        change: TrackChange::Vca(Some(vca)),
    };
    f.update(vca(t, v1)).unwrap();
    f.update(vca(v1, v2)).unwrap();
    assert!(f.update(vca(v2, v1)).is_err(), "VCA cycle");
    assert!(f.update(vca(t, group)).is_err(), "not a VCA");
    let d = f.device(v1, BuiltinDevice::Utility);
    assert!(f.insert(Entity::Device(d)).is_err());
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Track(v1)
        })
        .is_err()
    );
    // Input taps are routing edges.
    let a = f.track(TrackKind::Audio);
    f.update(EntityUpdate::Track {
        id: a,
        change: TrackChange::Input(TrackInput::Track {
            track: t,
            tap: InputTap::PreFx,
        }),
    })
    .unwrap();
    assert!(
        f.update(EntityUpdate::Track {
            id: t,
            change: TrackChange::Input(TrackInput::Track {
                track: a,
                tap: InputTap::PostFader,
            }),
        })
        .is_err(),
        "tap cycle"
    );
    f.p.validate().unwrap();
    // Round trip through the file format.
    let json = file::save(&f.p, "0.2.0").unwrap();
    assert_eq!(file::load(&json).unwrap(), f.p);
}

#[test]
fn v2_fixture_loads_at_v4_with_project_media() {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/v2_full.ether"
    ))
    .unwrap();
    let p = file::load(&json).unwrap();
    assert!(
        p.media
            .values()
            .all(|m| m.location == MediaLocation::Project)
    );
    assert!(p.take_lanes.is_empty() && p.rack_chains.is_empty() && p.mod_mappings.is_empty());
    assert!(p.chat.is_empty() && p.pinned_notes.is_empty());
    let saved = file::save(&p, "0.2.0").unwrap();
    let v: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(v["version"], file::CURRENT_VERSION);
    // Absent optional fields stay absent (older-shaped entities are unchanged on disk).
    assert!(
        !saved.contains("\"lane\": null")
            && !saved.contains("\"freeze\"")
            && !saved.contains("\"chain\": null")
    );
    assert_eq!(file::load(&saved).unwrap(), p);
}

#[test]
fn envelope_follower_sidechain_is_a_routing_edge() {
    let mut f = Fx::new();
    let host_track = f.track(TrackKind::Audio);
    let source = f.track(TrackKind::Audio);
    let host = f.device(host_track, BuiltinDevice::AutoFilter);
    f.insert(Entity::Device(host.clone())).unwrap();
    let follower = Modulator {
        id: f.id(),
        device: host.id,
        order: OrderKey::between(None, None),
        name: "Follower".into(),
        kind: ModulatorKind::EnvelopeFollower,
        params: Default::default(),
        sidechain: Some(source),
    };
    f.insert(Entity::Modulator(follower.clone())).unwrap();
    // Only followers; not the host's own track.
    let lfo = Modulator {
        id: f.id(),
        kind: ModulatorKind::Lfo,
        ..follower.clone()
    };
    assert!(f.insert(Entity::Modulator(lfo)).is_err());
    let own = Modulator {
        id: f.id(),
        sidechain: Some(host_track),
        ..follower.clone()
    };
    assert!(f.insert(Entity::Modulator(own)).is_err());
    // source → host is an edge: the reverse sidechain would close a cycle.
    let mut back = f.device(source, BuiltinDevice::Gate);
    back.sidechain = Some(host_track);
    assert!(matches!(
        f.insert(Entity::Device(back)),
        Err(ModelError::Invariant(_))
    ));
    // The source track can't be removed while followed.
    assert!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Track(source)
        })
        .is_err()
    );
    f.p.validate().unwrap();
}
