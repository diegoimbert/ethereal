//! v0.3 model contracts (contracts-4): clip expression lanes, note expressions, track MPE,
//! the v0.3 devices' kind data, rack presets, templates, `.ether` v5.

use ether_model::*;

struct Fx {
    p: Project,
    ids: IdGen,
    now: u64,
}

impl Fx {
    fn new() -> Self {
        let mut ids = IdGen::new(9);
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
    fn clip(&mut self, track: TrackId, content: ClipContent) -> ClipId {
        let id = self.id();
        self.insert(Entity::Clip(Clip {
            id,
            track,
            start: Beats(0.0),
            name: "c".into(),
            color: None,
            muted: false,
            length: Beats(4.0),
            offset: Beats(0.0),
            looping: ClipLoop {
                enabled: false,
                start: Beats(0.0),
                end: Beats(4.0),
            },
            content,
            lane: None,
        }))
        .unwrap();
        id
    }
    fn note(&mut self, clip: ClipId) -> NoteId {
        let id = self.id();
        self.insert(Entity::Note(Note {
            id,
            clip,
            pitch: 60,
            velocity: 0.8,
            release_velocity: 0.5,
            start: Beats(0.0),
            duration: Beats(1.0),
            muted: false,
        }))
        .unwrap();
        id
    }
    fn device(&mut self, track: TrackId, device: BuiltinDevice) -> Result<DeviceId, ModelError> {
        let id = self.id();
        self.insert(Entity::Device(Device {
            id,
            track,
            order: OrderKey::between(None, None),
            name: "d".into(),
            enabled: true,
            kind: DeviceKind::Builtin { device },
            params: Default::default(),
            sidechain: None,
            pad: None,
            chain: None,
        }))
        .map(|_| id)
    }
}

fn pt(time: f64, value: f32) -> ExpressionPoint {
    ExpressionPoint {
        time: Beats(time),
        value,
        curve: CurveShape::Linear,
    }
}

#[test]
fn expression_lanes_live_in_midi_clips() {
    let mut f = Fx::new();
    let midi = f.track(TrackKind::Midi);
    let clip = f.clip(midi, ClipContent::Midi);
    let lane = |f: &mut Fx, clip, kind, points| ExpressionLane {
        id: f.id(),
        clip,
        kind,
        points,
    };
    let bend = lane(
        &mut f,
        clip,
        ExpressionKind::PitchBend,
        vec![pt(0.0, -1.0), pt(1.0, 1.0)],
    );
    f.insert(Entity::ExpressionLane(bend.clone())).unwrap();
    assert_eq!(f.p.expression_lanes_of(clip).len(), 1);
    // One lane per (clip, kind).
    let dup = lane(&mut f, clip, ExpressionKind::PitchBend, vec![]);
    assert!(matches!(
        f.insert(Entity::ExpressionLane(dup)),
        Err(ModelError::Invariant(_))
    ));
    // Values in the kind's range; CC 0..=119; sorted points.
    let cc = lane(
        &mut f,
        clip,
        ExpressionKind::Cc { controller: 1 },
        vec![pt(0.0, -0.5)],
    );
    assert!(f.insert(Entity::ExpressionLane(cc)).is_err());
    let mode = lane(&mut f, clip, ExpressionKind::Cc { controller: 120 }, vec![]);
    assert!(f.insert(Entity::ExpressionLane(mode)).is_err());
    let unsorted = lane(
        &mut f,
        clip,
        ExpressionKind::ChannelPressure,
        vec![pt(1.0, 0.5), pt(0.5, 0.5)],
    );
    assert!(f.insert(Entity::ExpressionLane(unsorted)).is_err());
    // Not on audio clips.
    let audio_track = f.track(TrackKind::Audio);
    let media: MediaId = f.id();
    f.insert(Entity::Media(MediaRef {
        id: media,
        name: "a.wav".into(),
        file: format!("media/{media}-a.wav"),
        sample_rate: 48_000,
        channels: 2,
        frames: 48_000,
        hash: None,
        location: MediaLocation::Project,
    }))
    .unwrap();
    let audio = f.clip(
        audio_track,
        ClipContent::Audio(AudioContent {
            media,
            gain: Decibels(0.0),
            transpose: 0.0,
            fade_in: Beats(0.0),
            fade_out: Beats(0.0),
            warp: WarpSettings::default(),
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
            reversed: false,
        }),
    );
    let on_audio = lane(&mut f, audio, ExpressionKind::PitchBend, vec![]);
    assert!(matches!(
        f.insert(Entity::ExpressionLane(on_audio)),
        Err(ModelError::Invariant(_))
    ));
    // The curve is one value; updates are validated and invertible.
    let inv = f
        .update(EntityUpdate::ExpressionLane {
            id: bend.id,
            change: ExpressionLaneChange::Points(vec![pt(0.0, 0.0)]),
        })
        .unwrap();
    assert!(
        f.update(EntityUpdate::ExpressionLane {
            id: bend.id,
            change: ExpressionLaneChange::Points(vec![pt(0.0, 2.0)]),
        })
        .is_err()
    );
    f.p.apply(&inv).unwrap();
    assert_eq!(f.p.expression_lanes[&bend.id], bend);
    // A clip with lanes can't be removed before them (the controller cascades).
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Clip(clip)
        }),
        Err(ModelError::HasChildren(_))
    ));
    f.p.validate().unwrap();
}

#[test]
fn note_expressions_belong_to_notes() {
    let mut f = Fx::new();
    let midi = f.track(TrackKind::Midi);
    let clip = f.clip(midi, ClipContent::Midi);
    let note = f.note(clip);
    let expr = |f: &mut Fx, note, kind, points| NoteExpression {
        id: f.id(),
        note,
        kind,
        points,
    };
    let pitch = expr(
        &mut f,
        note,
        NoteExpressionKind::Pitch,
        vec![pt(0.0, 0.0), pt(0.5, 12.0)],
    );
    f.insert(Entity::NoteExpression(pitch)).unwrap();
    let timbre = expr(
        &mut f,
        note,
        NoteExpressionKind::Timbre,
        vec![pt(0.0, 0.25)],
    );
    f.insert(Entity::NoteExpression(timbre)).unwrap();
    assert_eq!(f.p.note_expressions_of(note).len(), 2);
    let dup = expr(&mut f, note, NoteExpressionKind::Pitch, vec![]);
    assert!(f.insert(Entity::NoteExpression(dup)).is_err());
    let out_of_range = expr(
        &mut f,
        note,
        NoteExpressionKind::Pressure,
        vec![pt(0.0, 1.5)],
    );
    assert!(f.insert(Entity::NoteExpression(out_of_range)).is_err());
    let missing: NoteId = f.id();
    let dangling = expr(&mut f, missing, NoteExpressionKind::Pressure, vec![]);
    assert!(matches!(
        f.insert(Entity::NoteExpression(dangling)),
        Err(ModelError::DanglingReference { .. })
    ));
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Note(note)
        }),
        Err(ModelError::HasChildren(_))
    ));
    f.p.validate().unwrap();
}

#[test]
fn mpe_is_for_midi_tracks() {
    let mut f = Fx::new();
    let midi = f.track(TrackKind::Midi);
    let audio = f.track(TrackKind::Audio);
    let set = |f: &mut Fx, id, mpe| {
        f.update(EntityUpdate::Track {
            id,
            change: TrackChange::Mpe(mpe),
        })
    };
    set(&mut f, midi, Some(MpeSettings::default())).unwrap();
    assert!(set(&mut f, audio, Some(MpeSettings::default())).is_err());
    let bad = MpeSettings {
        member_channels: 0,
        ..MpeSettings::default()
    };
    assert!(set(&mut f, midi, Some(bad)).is_err());
    // Omitted from JSON when off.
    let json = serde_json::to_value(&f.p.tracks[&audio]).unwrap();
    assert!(json.get("mpe").is_none());
    f.p.validate().unwrap();
}

#[test]
fn v03_devices_kind_data() {
    let mut f = Fx::new();
    let audio = f.track(TrackKind::Audio);
    let midi = f.track(TrackKind::Midi);
    f.device(
        audio,
        BuiltinDevice::new(BuiltinDeviceType::ConvolutionReverb),
    )
    .unwrap();
    f.device(
        audio,
        BuiltinDevice::ConvolutionReverb {
            ir: Some(IrSource::Factory { id: "hall".into() }),
        },
    )
    .unwrap();
    // A media IR must exist (and counts as the device's media).
    let missing: MediaId = f.id();
    let ir = BuiltinDevice::ConvolutionReverb {
        ir: Some(IrSource::Media { media: missing }),
    };
    assert_eq!(ir.media(), vec![missing]);
    assert!(matches!(
        f.device(audio, ir),
        Err(ModelError::DanglingReference { .. })
    ));
    assert!(
        f.device(
            audio,
            BuiltinDevice::ConvolutionReverb {
                ir: Some(IrSource::Factory { id: String::new() }),
            },
        )
        .is_err()
    );
    let routing = ExternalRouting {
        midi_out: Some("synth-port".into()),
        midi_channel: 2,
        audio_send: None,
        audio_return: Some(HwChannels { first: 2, count: 2 }),
    };
    f.device(
        midi,
        BuiltinDevice::ExternalInstrument {
            routing: routing.clone(),
        },
    )
    .unwrap();
    let bad = ExternalRouting {
        midi_channel: 17,
        ..routing.clone()
    };
    assert!(
        f.device(midi, BuiltinDevice::ExternalInstrument { routing: bad })
            .is_err()
    );
    let mono3 = ExternalRouting {
        audio_return: Some(HwChannels { first: 0, count: 3 }),
        ..routing
    };
    assert!(
        f.device(audio, BuiltinDevice::ExternalAudioEffect { routing: mono3 })
            .is_err()
    );
    for t in [
        BuiltinDeviceType::ConvolutionReverb,
        BuiltinDeviceType::ExternalInstrument,
        BuiltinDeviceType::ExternalAudioEffect,
    ] {
        assert!(BuiltinDeviceType::ALL.contains(&t));
        assert_eq!(BuiltinDevice::new(t).device_type(), t);
    }
    assert_eq!(
        BuiltinDeviceType::ExternalAudioEffect.key(),
        "external-audio-effect"
    );
    f.p.validate().unwrap();
}

#[test]
fn rack_presets_store_chains() {
    let rack = PresetRack {
        chains: vec![PresetChain {
            name: "Layer".into(),
            color: None,
            volume: Decibels(0.0),
            pan: Pan(0.0),
            mute: false,
            solo: false,
            keys: Zone::default(),
            velocities: Zone::default(),
            select: Zone::default(),
            devices: vec![PresetChainDevice {
                name: "Poly".into(),
                enabled: true,
                device: PresetDevice::Builtin {
                    device: BuiltinDeviceType::PolySynth,
                },
                params: Default::default(),
                kind: None,
                state: None,
            }],
        }],
        modulators: vec![],
        mappings: vec![PresetModMapping {
            source: PresetModSource::Macro { index: 0 },
            target: PresetModTarget::ChainDevice {
                chain: 0,
                device: 0,
            },
            param: ParamId(3),
            depth: 0.5,
        }],
    };
    let preset = Preset {
        name: "Stack".into(),
        device: PresetDevice::Builtin {
            device: BuiltinDeviceType::InstrumentRack,
        },
        meta: PresetMeta::default(),
        params: Default::default(),
        kind: None,
        samples: vec![],
        state: None,
        rack: Some(rack.clone()),
    };
    let json = save_preset(&preset, "0.3.0").unwrap();
    assert!(json.contains("\"version\": 2"));
    assert_eq!(load_preset(&json).unwrap(), preset);
    // Mappings must point inside the preset; only racks carry a structure.
    let mut bad = preset.clone();
    bad.rack.as_mut().unwrap().mappings[0].target = PresetModTarget::ChainDevice {
        chain: 0,
        device: 5,
    };
    assert!(load_preset(&save_preset(&bad, "x").unwrap()).is_err());
    let mut not_rack = preset.clone();
    not_rack.device = PresetDevice::Builtin {
        device: BuiltinDeviceType::Chorus,
    };
    assert!(load_preset(&save_preset(&not_rack, "x").unwrap()).is_err());
    // Version-1 presets still load (no structure).
    let v1 = r#"{"format":"ethereal-preset","version":1,"app_version":"0.2.0",
        "preset":{"name":"A","device":{"type":"Builtin","device":"InstrumentRack"}}}"#;
    assert_eq!(load_preset(v1).unwrap().rack, None);
}

#[test]
fn ether_v5_roundtrip_and_older_files_migrate() {
    let mut f = Fx::new();
    let midi = f.track(TrackKind::Midi);
    let clip = f.clip(midi, ClipContent::Midi);
    let note = f.note(clip);
    let lane = ExpressionLane {
        id: f.id(),
        clip,
        kind: ExpressionKind::Cc { controller: 74 },
        points: vec![pt(0.0, 0.1), pt(2.0, 0.9)],
    };
    f.insert(Entity::ExpressionLane(lane)).unwrap();
    let ne = NoteExpression {
        id: f.id(),
        note,
        kind: NoteExpressionKind::Pressure,
        points: vec![pt(0.0, 0.3)],
    };
    f.insert(Entity::NoteExpression(ne)).unwrap();
    let json = file::save(&f.p, "0.3.0").unwrap();
    assert!(json.contains("\"version\": 5"));
    assert_eq!(file::load(&json).unwrap(), f.p);
    // Entities are listed parents first (notes before their expressions).
    let keys: Vec<EntityKey> = f.p.entities().iter().map(Entity::key).collect();
    let pos = |k: EntityKey| keys.iter().position(|x| *x == k).unwrap();
    assert!(pos(EntityKey::Note(note)) < pos(EntityKey::NoteExpression(ne_id(&f.p))));
    // The v2 fixture migrates through v3, v4 and v5.
    let v2 = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/v2_full.ether"
    ))
    .unwrap();
    let p = file::load(&v2).unwrap();
    assert!(p.expression_lanes.is_empty() && p.note_expressions.is_empty());
    assert!(p.tracks.values().all(|t| t.mpe.is_none()));
}

fn ne_id(p: &Project) -> NoteExpressionId {
    *p.note_expressions.keys().next().unwrap()
}
