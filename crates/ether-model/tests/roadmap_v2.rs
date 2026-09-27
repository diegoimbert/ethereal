//! Roadmap v2 model contracts (contracts-2): `.ether` v2 → v3 migration on a realistic
//! fixture, and the invariants of the new entities (markers, MIDI mappings, drum pads,
//! sidechains, slices, fade curves).

use ether_model::*;
use serde_json::Value;

const V2: &str = include_str!("fixtures/v2_full.ether");

#[test]
fn v2_fixture_migrates_to_v3_with_neutral_defaults() {
    let raw: Value = serde_json::from_str(V2).unwrap();
    assert_eq!(raw["version"], 2);
    let p = file::load(V2).expect("v2 fixture migrates and validates");

    // Every v2 entity survives unchanged in count.
    for (table, n) in [
        ("tracks", p.tracks.len()),
        ("clips", p.clips.len()),
        ("notes", p.notes.len()),
        ("devices", p.devices.len()),
        ("sends", p.sends.len()),
        ("automation_lanes", p.automation_lanes.len()),
        ("automation_points", p.automation_points.len()),
        ("tempo_points", p.tempo_points.len()),
        ("time_signatures", p.time_signatures.len()),
        ("warp_markers", p.warp_markers.len()),
        ("media", p.media.len()),
    ] {
        assert_eq!(
            raw["project"][table].as_object().unwrap().len(),
            n,
            "{table}"
        );
    }
    assert!(p.markers.is_empty() && p.midi_mappings.is_empty() && p.drum_pads.is_empty());

    // Defaults.
    let s = &p.settings;
    assert_eq!(s.name, "Fixture v2");
    assert!(s.metronome && s.metronome_accent);
    assert_eq!(s.metronome_volume, Decibels(-6.0));
    assert_eq!(s.metronome_sound, MetronomeSound::Classic);
    assert_eq!((s.swing, s.swing_grid), (0.0, Beats(0.25)));
    for d in p.devices.values() {
        assert_eq!((d.sidechain, d.pad), (None, None));
        if let DeviceKind::Builtin {
            device: BuiltinDevice::Sampler { sample, slices },
        } = &d.kind
        {
            assert!(sample.is_some());
            assert_eq!(*slices, SliceSettings::default());
        }
    }
    let audio: Vec<&AudioContent> = p
        .clips
        .values()
        .filter_map(|c| match &c.content {
            ClipContent::Audio(a) => Some(a),
            ClipContent::Midi => None,
        })
        .collect();
    assert_eq!(audio.len(), 1);
    let a = audio[0];
    assert_eq!((a.fade_in, a.fade_out), (Beats(0.25), Beats(0.5)));
    assert_eq!(
        (a.fade_in_curve, a.fade_out_curve, a.reversed),
        (FadeCurve::Linear, FadeCurve::Linear, false)
    );

    // Saved as v3; saving is stable; loading the v3 file gives the same project.
    let saved = file::save(&p, "0.2.0").unwrap();
    let v: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(v["version"], file::CURRENT_VERSION);
    assert_eq!(file::CURRENT_VERSION, 3);
    assert_eq!(file::load(&saved).unwrap(), p);
    assert_eq!(
        file::save(&file::load(&saved).unwrap(), "0.2.0").unwrap(),
        saved
    );
}

// ─── Invariants ─────────────────────────────────────────────────────────────────────────

struct Fx {
    p: Project,
    ids: IdGen,
    now: u64,
}

impl Fx {
    fn new() -> Self {
        let mut ids = IdGen::new(3);
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
        }))
        .unwrap();
        id
    }
    fn device(&mut self, track: TrackId, device: BuiltinDevice, pad: Option<DrumPadId>) -> Device {
        Device {
            id: self.id(),
            track,
            order: OrderKey::between(None, None),
            name: "d".into(),
            enabled: true,
            kind: DeviceKind::Builtin { device },
            params: Default::default(),
            sidechain: None,
            pad,
        }
    }
    fn pad(&mut self, rack: DeviceId, note: u8) -> DrumPad {
        DrumPad {
            id: self.id(),
            rack,
            note,
            name: "Kick".into(),
            color: None,
            choke_group: None,
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
        }
    }
}

#[test]
fn markers_validate_and_patch() {
    let mut f = Fx::new();
    let m = Marker {
        id: f.id(),
        position: Beats(8.0),
        name: "Chorus".into(),
        color: Some(Color(0xff0000)),
    };
    f.insert(Entity::Marker(m.clone())).unwrap();
    let bad = Marker {
        id: f.id(),
        position: Beats(-1.0),
        ..m.clone()
    };
    assert!(f.insert(Entity::Marker(bad)).is_err());
    let inv =
        f.p.apply(&Op::Update {
            update: EntityUpdate::Marker {
                id: m.id,
                change: MarkerChange::Position(Beats(4.0)),
            },
        })
        .unwrap();
    assert_eq!(f.p.markers[&m.id].position, Beats(4.0));
    f.p.apply(&inv).unwrap();
    assert_eq!(f.p.markers[&m.id], m);
    assert_eq!(f.p.markers_sorted().len(), 1);
    f.p.validate().unwrap();
}

#[test]
fn drum_pads_and_pad_chains() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let rack = f.device(t, BuiltinDevice::DrumRack, None);
    f.insert(Entity::Device(rack.clone())).unwrap();
    let pad = f.pad(rack.id, 36);
    f.insert(Entity::DrumPad(pad.clone())).unwrap();
    // Same note twice in one rack is rejected.
    let dup = f.pad(rack.id, 36);
    assert!(matches!(
        f.insert(Entity::DrumPad(dup)),
        Err(ModelError::Invariant(_))
    ));
    // Bad choke group.
    let mut g = f.pad(rack.id, 38);
    g.choke_group = Some(0);
    assert!(f.insert(Entity::DrumPad(g)).is_err());
    // A pad only lives on a drum rack.
    let synth = f.device(t, BuiltinDevice::Synth, None);
    f.insert(Entity::Device(synth.clone())).unwrap();
    let on_synth = f.pad(synth.id, 40);
    assert!(f.insert(Entity::DrumPad(on_synth)).is_err());

    // Pad chain device: on the rack's track, not in the track chain.
    let sampler = f.device(
        t,
        BuiltinDevice::new(BuiltinDeviceType::Sampler),
        Some(pad.id),
    );
    f.insert(Entity::Device(sampler.clone())).unwrap();
    assert_eq!(f.p.pad_devices_of(pad.id).len(), 1);
    assert!(f.p.devices_of(t).iter().all(|d| d.id != sampler.id));
    assert_eq!(f.p.pads_of(rack.id).len(), 1);
    // No nested racks; no pad device on another track.
    let nested = f.device(t, BuiltinDevice::DrumRack, Some(pad.id));
    assert!(f.insert(Entity::Device(nested)).is_err());
    let other = f.track(TrackKind::Midi);
    let elsewhere = f.device(other, BuiltinDevice::Synth, Some(pad.id));
    assert!(f.insert(Entity::Device(elsewhere)).is_err());

    // Removal order: pad device, then pad, then rack.
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::DrumPad(pad.id)
        }),
        Err(ModelError::HasChildren(_))
    ));
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Device(rack.id)
        }),
        Err(ModelError::HasChildren(_))
    ));
    f.p.validate().unwrap();
    // Entities are listed parents-first (so full-state patches apply in order).
    let order: Vec<EntityKey> = f.p.entities().iter().map(Entity::key).collect();
    let pos = |k| order.iter().position(|x| *x == k).unwrap();
    assert!(pos(EntityKey::Device(rack.id)) < pos(EntityKey::DrumPad(pad.id)));
    assert!(pos(EntityKey::DrumPad(pad.id)) < pos(EntityKey::Device(sampler.id)));
}

#[test]
fn sidechain_edges_are_routing_edges() {
    let mut f = Fx::new();
    let a = f.track(TrackKind::Audio);
    let b = f.track(TrackKind::Audio);
    let mut comp = f.device(b, BuiltinDevice::Compressor, None);
    comp.sidechain = Some(a);
    f.insert(Entity::Device(comp.clone())).unwrap();
    // Own track is rejected.
    let mut own = f.device(a, BuiltinDevice::Compressor, None);
    own.sidechain = Some(a);
    assert!(f.insert(Entity::Device(own)).is_err());
    // a → b (sidechain) plus b → a (sidechain) is a cycle.
    let mut back = f.device(a, BuiltinDevice::Compressor, None);
    back.sidechain = Some(b);
    assert!(matches!(
        f.insert(Entity::Device(back)),
        Err(ModelError::Invariant(_))
    ));
    // The source track can't be removed while referenced.
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Track(a)
        }),
        Err(ModelError::HasChildren(_))
    ));
    f.p.apply(&Op::Update {
        update: EntityUpdate::Device {
            id: comp.id,
            change: DeviceChange::Sidechain(None),
        },
    })
    .unwrap();
    f.p.apply(&Op::Remove {
        key: EntityKey::Track(a),
    })
    .unwrap();
}

#[test]
fn midi_mappings_validate_sources_and_targets() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio);
    let synth = f.device(t, BuiltinDevice::Synth, None);
    f.insert(Entity::Device(synth.clone())).unwrap();
    let source = MidiSource {
        port: Some("nanoKONTROL".into()),
        channel: Some(0),
        control: MidiControl::Cc { number: 7 },
    };
    let m = MidiMapping {
        id: f.id(),
        source: source.clone(),
        target: MidiMapTarget::Param {
            target: AutomationTarget::DeviceParam {
                device: synth.id,
                param: ParamId(2),
            },
        },
        min: 0.0,
        max: 1.0,
        mode: MidiMapMode::Absolute,
    };
    f.insert(Entity::MidiMapping(m.clone())).unwrap();
    // One mapping per source.
    let dup = MidiMapping {
        id: f.id(),
        target: MidiMapTarget::TrackMute { track: t },
        ..m.clone()
    };
    assert!(f.insert(Entity::MidiMapping(dup)).is_err());
    // Bad channel / range.
    let bad = MidiMapping {
        id: f.id(),
        source: MidiSource {
            channel: Some(16),
            ..source.clone()
        },
        ..m.clone()
    };
    assert!(f.insert(Entity::MidiMapping(bad)).is_err());
    let bad = MidiMapping {
        id: f.id(),
        source: MidiSource {
            port: None,
            ..source.clone()
        },
        max: 1.5,
        ..m.clone()
    };
    assert!(f.insert(Entity::MidiMapping(bad)).is_err());
    // The mapped device and track can't be removed while mapped.
    assert!(matches!(
        f.p.apply(&Op::Remove {
            key: EntityKey::Device(synth.id)
        }),
        Err(ModelError::HasChildren(_))
    ));
    let relative = MidiMapping {
        id: f.id(),
        source: MidiSource {
            port: None,
            channel: None,
            control: MidiControl::Cc { number: 16 },
        },
        target: MidiMapTarget::Transport {
            action: TransportAction::TogglePlay,
        },
        min: 1.0,
        max: 0.0,
        mode: MidiMapMode::Relative {
            encoding: RelativeEncoding::BinaryOffset,
        },
    };
    f.insert(Entity::MidiMapping(relative)).unwrap();
    f.p.validate().unwrap();
}

#[test]
fn slices_fades_and_settings_are_checked() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let mut sampler = f.device(t, BuiltinDevice::new(BuiltinDeviceType::Sampler), None);
    sampler.kind = DeviceKind::Builtin {
        device: BuiltinDevice::Sampler {
            sample: None,
            slices: SliceSettings {
                enabled: true,
                base_note: 36,
                markers: vec![Seconds(0.5), Seconds(0.25)],
            },
        },
    };
    assert!(f.insert(Entity::Device(sampler.clone())).is_err());
    sampler.kind = DeviceKind::Builtin {
        device: BuiltinDevice::Sampler {
            sample: None,
            slices: SliceSettings {
                enabled: true,
                base_note: 36,
                markers: vec![Seconds(0.0), Seconds(0.25)],
            },
        },
    };
    f.insert(Entity::Device(sampler)).unwrap();

    for (change, ok) in [
        (SettingsChange::Swing(0.5), true),
        (SettingsChange::Swing(1.5), false),
        (SettingsChange::SwingGrid(Beats(0.0)), false),
        (SettingsChange::MetronomeVolume(Decibels(-12.0)), true),
        (SettingsChange::MetronomeSound(MetronomeSound::Wood), true),
    ] {
        assert_eq!(f.p.apply(&Op::Settings { change }).is_ok(), ok);
    }
    // Fade curve tension is range-checked on audio clips (see ops_proptest for the rest).
    assert_eq!(FadeCurve::default(), FadeCurve::Linear);
}

#[test]
fn pad_devices_cannot_have_a_sidechain() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let src = f.track(TrackKind::Audio);
    let rack = f.device(t, BuiltinDevice::DrumRack, None);
    f.insert(Entity::Device(rack.clone())).unwrap();
    let pad = f.pad(rack.id, 36);
    f.insert(Entity::DrumPad(pad.clone())).unwrap();
    let mut comp = f.device(t, BuiltinDevice::Compressor, Some(pad.id));
    comp.sidechain = Some(src);
    assert!(matches!(
        f.insert(Entity::Device(comp.clone())),
        Err(ModelError::Invariant(_))
    ));
    comp.sidechain = None;
    f.insert(Entity::Device(comp.clone())).unwrap();
    assert!(
        f.p.apply(&Op::Update {
            update: EntityUpdate::Device {
                id: comp.id,
                change: DeviceChange::Sidechain(Some(src)),
            },
        })
        .is_err()
    );
}
