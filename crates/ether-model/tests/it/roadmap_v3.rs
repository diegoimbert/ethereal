//! v0.2 model contracts: take lanes and comp regions, rack chains,
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
            sidechain_tap: None,
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
    let rack = f.device(t, BuiltinDevice::InstrumentRack { macros: None });
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
    // A chain device, on the rack's track.
    let mut inner = f.device(t, BuiltinDevice::PolySynth);
    inner.chain = Some(chain.id);
    f.insert(Entity::Device(inner.clone())).unwrap();
    assert!(f.p.devices_of(t).iter().all(|d| d.id != inner.id));
    assert_eq!(f.p.chain_devices_of(chain.id).len(), 1);
    // Racks nest; a drum rack on an instrument rack's chain too (see
    // `drum_racks_nest_in_instrument_rack_chains`).
    let mut nested = f.device(t, BuiltinDevice::AudioEffectRack { macros: None });
    nested.chain = Some(chain.id);
    f.insert(Entity::Device(nested.clone())).unwrap();
    assert_eq!(f.p.enclosing_racks(nested.id), Some(vec![rack.id]));
    // Down to MAX_RACK_DEPTH levels: the track-chain rack is level 1.
    let mut parent = nested.id;
    for _ in 2..MAX_RACK_DEPTH {
        let c = RackChain {
            id: f.id(),
            rack: parent,
            ..chain.clone()
        };
        f.insert(Entity::RackChain(c.clone())).unwrap();
        let mut r = f.device(t, BuiltinDevice::AudioEffectRack { macros: None });
        r.chain = Some(c.id);
        f.insert(Entity::Device(r.clone())).unwrap();
        parent = r.id;
    }
    assert_eq!(
        f.p.enclosing_racks(parent).unwrap().len(),
        MAX_RACK_DEPTH - 1
    );
    let last = RackChain {
        id: f.id(),
        rack: parent,
        ..chain.clone()
    };
    f.insert(Entity::RackChain(last.clone())).unwrap();
    let mut too_deep = f.device(t, BuiltinDevice::AudioEffectRack { macros: None });
    too_deep.chain = Some(last.id);
    assert!(matches!(
        f.insert(Entity::Device(too_deep)),
        Err(ModelError::Invariant(_))
    ));
    // A drum rack is an instrument: never on an audio effect rack's chain.
    let mut drums = f.device(t, BuiltinDevice::DrumRack { macros: None });
    drums.chain = Some(c_of(&f, parent));
    assert!(f.insert(Entity::Device(drums)).is_err());
    // A non-rack device fits at the deepest level.
    let mut leaf = f.device(t, BuiltinDevice::Chorus);
    leaf.chain = Some(last.id);
    f.insert(Entity::Device(leaf.clone())).unwrap();
    assert!(f.p.is_inside(leaf.id, rack.id));
    // A rack can't move into its own chains.
    assert!(
        f.update(EntityUpdate::Device {
            id: nested.id,
            change: DeviceChange::Chain(Some(last.id)),
        })
        .is_err()
    );
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
    let rack = f.device(t, BuiltinDevice::AudioEffectRack { macros: None });
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
            up_to: None,
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
    // Group tracks freeze too (the group's render; its children are bypassed while frozen).
    let group = f.track(TrackKind::Group);
    f.update(EntityUpdate::Track {
        id: group,
        change: TrackChange::Freeze(Some(TrackFreeze {
            media: own,
            start: Seconds(0.0),
            up_to: None,
        })),
    })
    .unwrap();
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
fn full_fixture_loads_with_project_media() {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/v1_full.ether"
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
    assert_eq!(v["format_version"], file::CURRENT_VERSION);
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

fn c_of(f: &Fx, rack: DeviceId) -> RackChainId {
    f.p.chains_of(rack)[0].id
}

#[test]
fn drum_racks_nest_in_instrument_rack_chains() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let chain_of = |f: &mut Fx, rack: DeviceId| {
        let c = RackChain {
            id: f.id(),
            rack,
            order: OrderKey::between(None, None),
            name: "Chain".into(),
            color: None,
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
            solo: false,
            keys: Zone::FULL,
            velocities: Zone::FULL,
            select: Zone::FULL,
        };
        f.insert(Entity::RackChain(c.clone())).unwrap();
        c.id
    };
    let pad_of = |f: &mut Fx, rack: DeviceId, note: u8| {
        let id: DrumPadId = f.id();
        f.insert(Entity::DrumPad(DrumPad {
            id,
            rack,
            note,
            name: "Kick".into(),
            color: None,
            choke_group: None,
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
        }))
        .unwrap();
        id
    };
    let inst = f.device(t, BuiltinDevice::InstrumentRack { macros: None });
    f.insert(Entity::Device(inst.clone())).unwrap();
    let ic = chain_of(&mut f, inst.id);
    // On an instrument rack's chain (level 2) with pads.
    let mut drums = f.device(t, BuiltinDevice::DrumRack { macros: None });
    drums.chain = Some(ic);
    f.insert(Entity::Device(drums.clone())).unwrap();
    let pad = pad_of(&mut f, drums.id, 36);
    let mut kick = f.device(t, BuiltinDevice::PolySynth);
    kick.pad = Some(pad);
    f.insert(Entity::Device(kick.clone())).unwrap();
    assert_eq!(
        f.p.device_path(kick.id),
        Some(vec![inst.id, drums.id, kick.id])
    );
    assert!(f.p.is_under_drum_pad(kick.id));
    assert!(f.p.has_nested_drum_rack(inst.id));
    assert_eq!(f.p.nested_rack_height(inst.id), 1);
    // A chain rack on its pad (level 3), whose chains never take a drum rack.
    let mut pad_rack = f.device(t, BuiltinDevice::InstrumentRack { macros: None });
    pad_rack.pad = Some(pad);
    f.insert(Entity::Device(pad_rack.clone())).unwrap();
    let pc = chain_of(&mut f, pad_rack.id);
    let mut inner_drums = f.device(t, BuiltinDevice::DrumRack { macros: None });
    inner_drums.chain = Some(pc);
    assert!(matches!(
        f.insert(Entity::Device(inner_drums.clone())),
        Err(ModelError::Invariant(_))
    ));
    // Nor directly on the pad.
    inner_drums.chain = None;
    inner_drums.pad = Some(pad);
    assert!(f.insert(Entity::Device(inner_drums)).is_err());
    // A track-chain drum rack can't move into the pad rack's chain either.
    let top = f.device(t, BuiltinDevice::DrumRack { macros: None });
    f.insert(Entity::Device(top.clone())).unwrap();
    assert!(
        f.update(EntityUpdate::Device {
            id: top.id,
            change: DeviceChange::Chain(Some(pc)),
        })
        .is_err()
    );
    // The instrument rack (holding the drum rack) can't move onto a pad of another drum
    // rack, nor into the pad rack's chain (a cycle).
    let top_pad = pad_of(&mut f, top.id, 36);
    assert!(
        f.update(EntityUpdate::Device {
            id: inst.id,
            change: DeviceChange::Pad(Some(top_pad)),
        })
        .is_err()
    );
    assert!(
        f.update(EntityUpdate::Device {
            id: inst.id,
            change: DeviceChange::Chain(Some(pc)),
        })
        .is_err()
    );
    // Depth: every rack counts, the drum rack included (inst 1, drums 2, pad rack 3, ...).
    let mut parent_chain = pc;
    for _ in 4..=MAX_RACK_DEPTH {
        let mut r = f.device(t, BuiltinDevice::InstrumentRack { macros: None });
        r.chain = Some(parent_chain);
        f.insert(Entity::Device(r.clone())).unwrap();
        parent_chain = chain_of(&mut f, r.id);
    }
    let mut too_deep = f.device(t, BuiltinDevice::AudioEffectRack { macros: None });
    too_deep.chain = Some(parent_chain);
    assert!(f.insert(Entity::Device(too_deep)).is_err());
    f.p.validate().unwrap();
}

/// Drum racks have the 8 rack macros (params after their volume and pan) and, like every
/// rack's, they reach the devices inside the rack at any depth: on its pads and in the
/// chains of the racks on them. An outer rack's macros reach into a nested drum rack's pads.
#[test]
fn drum_rack_macros_reach_pad_devices_at_any_depth() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let chain_of = |f: &mut Fx, rack: DeviceId| {
        let c = RackChain {
            id: f.id(),
            rack,
            order: OrderKey::between(None, None),
            name: "Chain".into(),
            color: None,
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
            solo: false,
            keys: Zone::FULL,
            velocities: Zone::FULL,
            select: Zone::FULL,
        };
        f.insert(Entity::RackChain(c.clone())).unwrap();
        c.id
    };
    let pad_of = |f: &mut Fx, rack: DeviceId, note: u8| {
        let id: DrumPadId = f.id();
        f.insert(Entity::DrumPad(DrumPad {
            id,
            rack,
            note,
            name: "Kick".into(),
            color: None,
            choke_group: None,
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
        }))
        .unwrap();
        id
    };
    let drum = BuiltinDeviceType::DrumRack;
    assert_eq!(macro_param(drum, 0), Some(ParamId(DRUM_RACK_FIRST_MACRO)));
    assert_eq!(macro_param(drum, 7), Some(ParamId(9)));
    assert_eq!(macro_param(drum, 8), Some(ParamId(10)));
    assert_eq!(macro_param(drum, 15), Some(ParamId(17)));
    assert_eq!(macro_param(drum, 16), None);
    assert_eq!(macro_index(drum, ParamId(1)), None);
    assert_eq!(macro_index(drum, ParamId(4)), Some(2));
    assert_eq!(macro_index(drum, ParamId(10)), Some(8));
    assert_eq!(macro_index(drum, ParamId(18)), None);
    assert_eq!(
        macro_param(BuiltinDeviceType::InstrumentRack, 3),
        Some(ParamId(3))
    );
    assert_eq!(macro_param(BuiltinDeviceType::Synth, 0), None);

    // inst > chain > drums > pad > [kick, pad rack > chain > deep]
    let inst = f.device(t, BuiltinDevice::InstrumentRack { macros: None });
    f.insert(Entity::Device(inst.clone())).unwrap();
    let ic = chain_of(&mut f, inst.id);
    let mut drums = f.device(t, BuiltinDevice::DrumRack { macros: None });
    drums.chain = Some(ic);
    f.insert(Entity::Device(drums.clone())).unwrap();
    let pad = pad_of(&mut f, drums.id, 36);
    let mut kick = f.device(t, BuiltinDevice::PolySynth);
    kick.pad = Some(pad);
    f.insert(Entity::Device(kick.clone())).unwrap();
    let mut pad_rack = f.device(t, BuiltinDevice::AudioEffectRack { macros: None });
    pad_rack.pad = Some(pad);
    f.insert(Entity::Device(pad_rack.clone())).unwrap();
    let pc = chain_of(&mut f, pad_rack.id);
    let mut deep = f.device(t, BuiltinDevice::Chorus);
    deep.chain = Some(pc);
    f.insert(Entity::Device(deep.clone())).unwrap();
    let other = f.device(t, BuiltinDevice::Reverb);
    f.insert(Entity::Device(other.clone())).unwrap();
    assert_eq!(f.p.enclosing_racks(kick.id), Some(vec![drums.id, inst.id]));
    assert_eq!(
        f.p.enclosing_racks(deep.id),
        Some(vec![pad_rack.id, drums.id, inst.id])
    );
    assert!(f.p.is_inside(deep.id, inst.id));

    let map = |f: &mut Fx, rack: DeviceId, index: u8, device: DeviceId, param: u32| {
        let m = ModMapping {
            id: f.id(),
            source: ModSource::Macro { rack, index },
            device,
            param: ParamId(param),
            depth: 0.5,
        };
        f.insert(Entity::ModMapping(m))
    };
    // The drum rack's macros: its pad devices at any depth, its own volume and pan.
    map(&mut f, drums.id, 0, kick.id, 0).unwrap();
    map(&mut f, drums.id, 7, deep.id, 0).unwrap();
    map(&mut f, drums.id, 1, drums.id, 0).unwrap();
    map(&mut f, drums.id, 1, drums.id, 1).unwrap();
    // Never its own macros, never a device outside it, at most 16 macros.
    assert!(map(&mut f, drums.id, 0, drums.id, 2).is_err());
    assert!(map(&mut f, drums.id, 0, other.id, 0).is_err());
    assert!(map(&mut f, drums.id, 0, inst.id, RACK_SELECTOR_PARAM.0).is_err());
    assert!(map(&mut f, drums.id, 16, kick.id, 1).is_err());
    // The outer rack's macros reach into the nested drum rack's pads (and the pad rack's
    // chains), and the drum rack's own params.
    map(&mut f, inst.id, 0, kick.id, 0).unwrap();
    map(&mut f, inst.id, 0, deep.id, 1).unwrap();
    map(&mut f, inst.id, 0, drums.id, 0).unwrap();
    // A pad rack's macros stay inside it.
    map(&mut f, pad_rack.id, 0, deep.id, 2).unwrap();
    assert!(map(&mut f, pad_rack.id, 0, kick.id, 2).is_err());

    // A modulator on a track-chain drum rack reaches its pad devices, never its macros.
    let top = f.device(t, BuiltinDevice::DrumRack { macros: None });
    f.insert(Entity::Device(top.clone())).unwrap();
    let top_pad = pad_of(&mut f, top.id, 38);
    let mut snare = f.device(t, BuiltinDevice::PolySynth);
    snare.pad = Some(top_pad);
    f.insert(Entity::Device(snare.clone())).unwrap();
    let lfo = Modulator {
        id: f.id(),
        device: top.id,
        order: OrderKey::between(None, None),
        name: "LFO".into(),
        kind: ModulatorKind::Lfo,
        params: Default::default(),
        sidechain: None,
    };
    f.insert(Entity::Modulator(lfo.clone())).unwrap();
    let from_lfo = |f: &mut Fx, device: DeviceId, param: u32| {
        let m = ModMapping {
            id: f.id(),
            source: ModSource::Modulator { modulator: lfo.id },
            device,
            param: ParamId(param),
            depth: 0.5,
        };
        f.insert(Entity::ModMapping(m))
    };
    from_lfo(&mut f, snare.id, 0).unwrap();
    from_lfo(&mut f, top.id, 1).unwrap();
    assert!(from_lfo(&mut f, top.id, DRUM_RACK_FIRST_MACRO).is_err());
    assert!(from_lfo(&mut f, kick.id, 0).is_err());
    // A pad device moving to another drum rack's pad would take the mapping out of scope.
    assert!(
        f.update(EntityUpdate::Device {
            id: snare.id,
            change: DeviceChange::Pad(Some(pad)),
        })
        .is_err()
    );
    f.p.validate().unwrap();
}

#[test]
fn partial_freeze_freezes_the_chain_up_to_the_freeze_point() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio);
    let own = f.media(MediaLocation::Project);
    // Chain order = insertion order (same order keys, ids increase).
    let eq = f.device(t, BuiltinDevice::Eq);
    f.insert(Entity::Device(eq.clone())).unwrap();
    let rack = f.device(t, BuiltinDevice::AudioEffectRack { macros: None });
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
    let nested = Device {
        chain: Some(chain.id),
        ..f.device(t, BuiltinDevice::Utility)
    };
    f.insert(Entity::Device(nested.clone())).unwrap();
    let reverb = f.device(t, BuiltinDevice::Reverb);
    f.insert(Entity::Device(reverb.clone())).unwrap();
    assert_eq!(f.p.chain_root(nested.id).map(|d| d.id), Some(rack.id));
    assert_eq!(f.p.frozen_chain_len(t), None);
    assert!(f.p.frozen_devices().is_empty());

    let freeze = |f: &mut Fx, up_to| {
        f.update(EntityUpdate::Track {
            id: t,
            change: TrackChange::Freeze(Some(TrackFreeze {
                media: own,
                start: Seconds(0.0),
                up_to,
            })),
        })
        .unwrap();
        f.p.frozen_devices()
    };
    let set = |ids: &[DeviceId]| {
        ids.iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(freeze(&mut f, Some(eq.id)), set(&[eq.id]));
    assert_eq!(f.p.frozen_chain_len(t), Some(1));
    assert!(!f.p.device_frozen(nested.id));
    // A rack freezes with what is nested in it.
    assert_eq!(
        freeze(&mut f, Some(rack.id)),
        set(&[eq.id, rack.id, nested.id])
    );
    assert!(f.p.device_frozen(nested.id));
    assert!(!f.p.device_frozen(reverb.id));
    // No freeze point, or one that is not on the chain: the whole chain.
    let all = set(&[eq.id, rack.id, nested.id, reverb.id]);
    assert_eq!(freeze(&mut f, None), all);
    assert_eq!(freeze(&mut f, Some(nested.id)), all);
    f.p.validate().unwrap();
}

#[test]
fn track_freeze_without_a_freeze_point_loads_as_the_whole_chain() {
    let media: MediaId = IdGen::new(3).next(1);
    let json = format!(r#"{{"media":"{media}","start":0.0}}"#);
    let f: TrackFreeze = serde_json::from_str(&json).unwrap();
    assert_eq!(f.up_to, None);
    // ...and saves without the field.
    assert!(!serde_json::to_string(&f).unwrap().contains("up_to"));
}

/// Rack macro banks (`RackMacros` on every rack's `BuiltinDevice`): 16 macros, 1..=16 shown
/// (default 8), custom names (empty = "Macro N"). Absent in JSON = the defaults, so documents
/// saved before load unchanged; out-of-range banks are refused.
#[test]
fn rack_macro_banks_default_normalize_and_validate() {
    // Documents and presets without a bank read as the defaults.
    let old: BuiltinDevice = serde_json::from_str(r#"{"type":"InstrumentRack"}"#).unwrap();
    assert_eq!(old, BuiltinDevice::InstrumentRack { macros: None });
    assert_eq!(old.rack_macros(), Some(RackMacros::default()));
    assert_eq!(
        serde_json::to_string(&old).unwrap(),
        r#"{"type":"InstrumentRack"}"#
    );
    assert_eq!(BuiltinDevice::Synth.rack_macros(), None);

    let mut m = RackMacros::default();
    assert_eq!(
        (m.visible, m.display_name(0)),
        (DEFAULT_VISIBLE_MACROS, "Macro 1".to_owned())
    );
    m.set_name(4, "  Grit ");
    m.set_name(RACK_MACROS, "ignored");
    assert_eq!(m.names, vec!["", "", "", "", "Grit"]);
    assert_eq!(m.display_name(4), "Grit");
    m.set_name(4, "");
    assert!(m.is_default());
    // The defaults are stored as none.
    let d = BuiltinDevice::DrumRack { macros: None };
    let named = d.clone().with_rack_macros(RackMacros {
        visible: 99,
        names: vec!["A".into()],
    });
    assert_eq!(
        named.rack_macros(),
        Some(RackMacros {
            visible: 16,
            names: vec!["A".into()]
        })
    );
    assert_eq!(named.clone().with_rack_macros(RackMacros::default()), d);
    let json = serde_json::to_string(&named).unwrap();
    assert_eq!(serde_json::from_str::<BuiltinDevice>(&json).unwrap(), named);

    // The model refuses a bank out of range (a peer's or a hand-edited document).
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    for bad in [
        RackMacros {
            visible: 0,
            names: vec![],
        },
        RackMacros {
            visible: 17,
            names: vec![],
        },
        RackMacros {
            visible: 8,
            names: vec!["a".into(), String::new()],
        },
        RackMacros {
            visible: 8,
            names: vec!["a".into(); 17],
        },
    ] {
        let d = f.device(
            t,
            BuiltinDevice::AudioEffectRack {
                macros: Some(bad.clone()),
            },
        );
        assert!(f.insert(Entity::Device(d)).is_err(), "{bad:?}");
    }
    let ok = f.device(t, named);
    f.insert(Entity::Device(ok)).unwrap();
    f.p.validate().unwrap();
}
