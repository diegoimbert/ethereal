//! v0.4 sidechains at any depth: devices inside drum pads, rack chains and nested racks take
//! a sidechain, tapped at a point of a track or of one of its chains (`SidechainTap`), and
//! the graph stays acyclic (same-track rules, cross-track routing edges).

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
    fn set(&mut self, id: DeviceId, change: DeviceChange) -> Result<Op, ModelError> {
        self.p.apply(&Op::Update {
            update: EntityUpdate::Device { id, change },
        })
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
    /// Inserts a device at `order` ("a" < "b" < ...) on `track`, in `chain` / `pad`.
    fn device(
        &mut self,
        track: TrackId,
        device: BuiltinDevice,
        order: &str,
        chain: Option<RackChainId>,
        pad: Option<DrumPadId>,
    ) -> DeviceId {
        let id = self.id();
        self.insert(Entity::Device(Device {
            id,
            track,
            order: OrderKey(format!("a{order}")),
            name: "d".into(),
            enabled: true,
            kind: DeviceKind::Builtin { device },
            params: Default::default(),
            sidechain: None,
            sidechain_tap: None,
            midi_sidechain: None,
            pad,
            chain,
        }))
        .unwrap();
        id
    }
    fn chain(&mut self, rack: DeviceId, order: &str) -> RackChainId {
        let id = self.id();
        self.insert(Entity::RackChain(RackChain {
            id,
            rack,
            order: OrderKey(format!("a{order}")),
            name: "Chain".into(),
            color: None,
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
            solo: false,
            keys: Zone::FULL,
            velocities: Zone::FULL,
            select: Zone::FULL,
        }))
        .unwrap();
        id
    }
    fn pad(&mut self, rack: DeviceId, note: u8) -> DrumPadId {
        let id = self.id();
        self.insert(Entity::DrumPad(DrumPad {
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
    }
    /// Sets `device`'s sidechain the way the controller does (source cleared first, so
    /// every intermediate state is valid).
    fn key(
        &mut self,
        device: DeviceId,
        source: TrackId,
        tap: SidechainTap,
    ) -> Result<(), ModelError> {
        self.set(device, DeviceChange::Sidechain(None))?;
        self.set(device, DeviceChange::SidechainTap(Some(tap)))?;
        self.set(device, DeviceChange::Sidechain(Some(source)))?;
        Ok(())
    }
}

fn tap(point: InputTap, chain: Option<SidechainChain>) -> SidechainTap {
    SidechainTap { point, chain }
}

fn invariant(r: Result<(), ModelError>) -> bool {
    matches!(r, Err(ModelError::Invariant(_)))
}

/// Track "kick" (drum rack, pad with a sampler) and track "bass" with a compressor two racks
/// deep: instrument rack → chain → audio effect rack → chain → compressor.
struct Set {
    f: Fx,
    kick: TrackId,
    bass: TrackId,
    kick_pad: DrumPadId,
    outer: DeviceId,
    outer_chain: RackChainId,
    inner_chain: RackChainId,
    comp: DeviceId,
}

fn set() -> Set {
    let mut f = Fx::new();
    let kick = f.track(TrackKind::Midi);
    let bass = f.track(TrackKind::Midi);
    let drums = f.device(
        kick,
        BuiltinDevice::DrumRack { macros: None },
        "m",
        None,
        None,
    );
    let kick_pad = f.pad(drums, 36);
    f.device(kick, BuiltinDevice::Synth, "m", None, Some(kick_pad));
    let outer = f.device(
        bass,
        BuiltinDevice::InstrumentRack { macros: None },
        "m",
        None,
        None,
    );
    let outer_chain = f.chain(outer, "m");
    f.device(bass, BuiltinDevice::PolySynth, "a", Some(outer_chain), None);
    let inner = f.device(
        bass,
        BuiltinDevice::AudioEffectRack { macros: None },
        "m",
        Some(outer_chain),
        None,
    );
    let inner_chain = f.chain(inner, "m");
    let comp = f.device(
        bass,
        BuiltinDevice::Compressor,
        "m",
        Some(inner_chain),
        None,
    );
    Set {
        f,
        kick,
        bass,
        kick_pad,
        outer,
        outer_chain,
        inner_chain,
        comp,
    }
}

#[test]
fn a_nested_device_keys_from_any_point_of_another_track() {
    let mut s = set();
    let (comp, kick) = (s.comp, s.kick);
    for point in [InputTap::PreFx, InputTap::PostFx, InputTap::PostFader] {
        s.f.key(comp, kick, tap(point, None)).unwrap();
    }
    // ... and from a pad of its drum rack.
    let pad = Some(SidechainChain::Pad { pad: s.kick_pad });
    for point in [InputTap::PreFx, InputTap::PostFx, InputTap::PostFader] {
        s.f.key(comp, kick, tap(point, pad)).unwrap();
    }
    // The pad device itself takes a sidechain too.
    let pad_comp =
        s.f.device(kick, BuiltinDevice::Compressor, "z", None, Some(s.kick_pad));
    s.f.key(pad_comp, s.bass, tap(InputTap::PostFx, None))
        .unwrap_err();
    // (bass is keyed from kick: kick → bass, so bass → kick would loop.)
    let other = s.f.track(TrackKind::Audio);
    s.f.key(pad_comp, other, tap(InputTap::PostFader, None))
        .unwrap();
    s.p_validates();
}

impl Set {
    fn p_validates(&self) {
        let saved = file::save(&self.f.p, "0.4.0").unwrap();
        assert_eq!(file::load(&saved).unwrap(), self.f.p);
    }
}

#[test]
fn a_chain_tap_must_be_on_the_source_track() {
    let mut s = set();
    let other = s.f.track(TrackKind::Audio);
    let pad = Some(SidechainChain::Pad { pad: s.kick_pad });
    assert!(invariant(s.f.key(s.comp, other, tap(InputTap::PreFx, pad))));
    // A dangling chain.
    let ghost: RackChainId = s.f.id();
    assert!(matches!(
        s.f.key(
            s.comp,
            s.kick,
            tap(InputTap::PreFx, Some(SidechainChain::Rack { chain: ghost }))
        ),
        Err(ModelError::DanglingReference { .. })
    ));
}

#[test]
fn own_track_only_before_the_device() {
    let mut s = set();
    let (comp, bass) = (s.comp, s.bass);
    // The track's post-FX / post-fader signal comes after every device of it.
    assert!(invariant(s.f.key(
        comp,
        bass,
        tap(InputTap::PostFader, None)
    )));
    assert!(invariant(s.f.key(comp, bass, tap(InputTap::PostFx, None))));
    // Its pre-FX signal comes first.
    s.f.key(comp, bass, tap(InputTap::PreFx, None)).unwrap();
    // A chain of a rack that holds the device: the chain's output needs the device.
    for chain in [s.outer_chain, s.inner_chain] {
        let c = Some(SidechainChain::Rack { chain });
        assert!(invariant(s.f.key(comp, bass, tap(InputTap::PostFx, c))));
    }
    // A rack placed before the outer rack on the track chain: fine.
    let before = s.f.device(
        bass,
        BuiltinDevice::InstrumentRack { macros: None },
        "c",
        None,
        None,
    );
    let ghost = s.f.chain(before, "m");
    let c = Some(SidechainChain::Rack { chain: ghost });
    s.f.key(comp, bass, tap(InputTap::PreFx, c)).unwrap();
    // Moving that rack after the outer one breaks the order: rejected.
    assert!(matches!(
        s.f.set(before, DeviceChange::Order(OrderKey("az".into()))),
        Err(ModelError::Invariant(_))
    ));
    // A rack after the device's rack: rejected.
    let after = s.f.device(
        bass,
        BuiltinDevice::AudioEffectRack { macros: None },
        "y",
        None,
        None,
    );
    let late = s.f.chain(after, "m");
    let c = Some(SidechainChain::Rack { chain: late });
    assert!(invariant(s.f.key(comp, bass, tap(InputTap::PostFx, c))));
    // Another chain of the rack holding it: that chain runs first, keyed before the rack's
    // alignment (pre- or post-FX; not post-fader).
    let sibling = s.f.chain(s.outer, "z");
    let c = Some(SidechainChain::Rack { chain: sibling });
    s.f.key(comp, bass, tap(InputTap::PostFx, c)).unwrap();
    assert_eq!(
        s.f.p.chain_order(s.outer).unwrap(),
        vec![sibling, s.outer_chain]
    );
    assert!(invariant(s.f.key(comp, bass, tap(InputTap::PostFader, c))));
    // A device of that chain keying the comp's chain back: a loop, rejected.
    let back =
        s.f.device(bass, BuiltinDevice::Compressor, "m", Some(sibling), None);
    let own = Some(SidechainChain::Rack {
        chain: s.outer_chain,
    });
    // (The refused key above left the comp unkeyed: `key` isn't atomic.)
    s.f.key(comp, bass, tap(InputTap::PostFx, c)).unwrap();
    assert!(invariant(s.f.key(back, bass, tap(InputTap::PostFx, own))));
    // ...or the device's own chain of the rack (it needs the device).
    assert!(invariant(s.f.key(comp, bass, tap(InputTap::PreFx, own))));
    // A rack earlier in the same (outer) chain: fine.
    let early = s.f.device(
        bass,
        BuiltinDevice::InstrumentRack { macros: None },
        "b",
        Some(s.outer_chain),
        None,
    );
    let early_chain = s.f.chain(early, "m");
    let c = Some(SidechainChain::Rack { chain: early_chain });
    s.f.key(comp, bass, tap(InputTap::PostFader, c)).unwrap();
    s.p_validates();
}

#[test]
fn chain_taps_are_routing_edges() {
    let mut s = set();
    // bass's compressor keyed from kick's pad: kick → bass.
    let pad = Some(SidechainChain::Pad { pad: s.kick_pad });
    s.f.key(s.comp, s.kick, tap(InputTap::PostFx, pad)).unwrap();
    // A kick device keyed from a chain of bass closes the loop.
    let kick_comp =
        s.f.device(s.kick, BuiltinDevice::Compressor, "z", None, None);
    let c = Some(SidechainChain::Rack {
        chain: s.inner_chain,
    });
    assert!(invariant(s.f.key(
        kick_comp,
        s.bass,
        tap(InputTap::PreFx, c)
    )));
}

#[test]
fn a_tapped_chain_cannot_be_removed_and_undo_restores_the_tap() {
    let mut s = set();
    let pad = Some(SidechainChain::Pad { pad: s.kick_pad });
    s.f.key(s.comp, s.kick, tap(InputTap::PreFx, pad)).unwrap();
    // The pad (with its devices gone) still has a dependent: the keyed compressor.
    let pad_devices: Vec<DeviceId> =
        s.f.p
            .pad_devices_of(s.kick_pad)
            .iter()
            .map(|d| d.id)
            .collect();
    for d in pad_devices {
        s.f.p
            .apply(&Op::Remove {
                key: EntityKey::Device(d),
            })
            .unwrap();
    }
    assert!(matches!(
        s.f.p.apply(&Op::Remove {
            key: EntityKey::DrumPad(s.kick_pad)
        }),
        Err(ModelError::HasChildren(_))
    ));
    // Changing the tap returns its inverse.
    let inverse =
        s.f.set(
            s.comp,
            DeviceChange::SidechainTap(Some(tap(InputTap::PostFx, pad))),
        )
        .unwrap();
    s.f.p.apply(&inverse).unwrap();
    assert_eq!(
        s.f.p.devices[&s.comp].sidechain_tap,
        Some(tap(InputTap::PreFx, pad))
    );
}

#[test]
fn the_tap_is_omitted_from_json_when_unset() {
    let s = set();
    let v = serde_json::to_value(&s.f.p.devices[&s.comp]).unwrap();
    assert!(v.get("sidechain_tap").is_none());
    let t = tap(
        InputTap::PreFx,
        Some(SidechainChain::Rack {
            chain: s.inner_chain,
        }),
    );
    let v = serde_json::to_value(t).unwrap();
    assert_eq!(v["point"], "PreFx");
    assert_eq!(v["chain"]["type"], "Rack");
    assert_eq!(serde_json::from_value::<SidechainTap>(v).unwrap(), t);
}
