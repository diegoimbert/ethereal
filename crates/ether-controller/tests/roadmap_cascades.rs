//! Roadmap v2 cascades and drum-rack structure rules (base-17): deleting a track cuts the
//! sidechains listening to it and removes its MIDI mappings; deleting a rack removes its
//! pads and pad chains; all of it is one undo step that restores the document exactly.
//! Device/track duplication copies pads; moving a rack across tracks and moving a pad
//! device with `Device::Move` are rejected.
//!
//! The document is built with model ops and opened from the store, since the drum-rack and
//! sidechain commands are not implemented yet.

mod common;

use common::*;
use ether_controller::store::ProjectStore;
use ether_core::protocol::devices::DeviceCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};

struct Doc {
    p: Project,
    ids: IdGen,
}

impl Doc {
    fn new() -> Self {
        let mut ids = IdGen::new(5);
        let p = Project::new(&mut ids, T0);
        Self { p, ids }
    }
    fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
    }
    fn insert(&mut self, e: Entity) {
        self.p.apply(&Op::Insert { entity: e }).unwrap();
    }
    fn track(&mut self, kind: TrackKind, order: &str) -> TrackId {
        let id = self.id();
        self.insert(Entity::Track(Track {
            freeze: None,
            vca: Default::default(),
            id,
            kind,
            name: format!("{kind:?}"),
            color: Color(0),
            order: OrderKey(order.into()),
            parent: None,
            mixer: TrackMixer::default(),
            input: TrackInput::None,
            output: TrackOutput::Default,
            monitor: MonitorMode::Auto,
            scale: Default::default(),
        }));
        id
    }
    fn device(
        &mut self,
        track: TrackId,
        device: BuiltinDevice,
        pad: Option<DrumPadId>,
        order: &str,
    ) -> DeviceId {
        let id = self.id();
        self.insert(Entity::Device(Device {
            chain: None,
            id,
            track,
            order: OrderKey(order.into()),
            name: "d".into(),
            enabled: true,
            kind: DeviceKind::Builtin { device },
            params: Default::default(),
            sidechain: None,
            pad,
        }));
        id
    }
    fn pad(&mut self, rack: DeviceId, note: u8) -> DrumPadId {
        let id = self.id();
        self.insert(Entity::DrumPad(DrumPad {
            id,
            rack,
            note,
            name: "pad".into(),
            color: None,
            choke_group: Some(1),
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
        }));
        id
    }
    /// Save to the harness store and open it.
    fn open(self, h: &mut Harness) -> Project {
        let id = self.p.id;
        h.ctl.store.create(id).unwrap();
        h.ctl
            .store
            .save(id, &file::save(&self.p, "test").unwrap())
            .unwrap();
        h.ok(Command::Project(ProjectCommand::Open { id }));
        assert_eq!(h.project(), &self.p);
        self.p
    }
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

#[test]
fn deleting_a_track_cuts_sidechains_and_mappings_and_undoes() {
    let mut d = Doc::new();
    let kick = d.track(TrackKind::Audio, "a1");
    let bass = d.track(TrackKind::Audio, "a2");
    let comp = d.device(bass, BuiltinDevice::Compressor, None, "a0");
    d.p.apply(&Op::Update {
        update: EntityUpdate::Device {
            id: comp,
            change: DeviceChange::Sidechain(Some(kick)),
        },
    })
    .unwrap();
    let mapping: MidiMappingId = d.id();
    d.insert(Entity::MidiMapping(MidiMapping {
        id: mapping,
        source: MidiSource {
            port: None,
            channel: None,
            control: MidiControl::Cc { number: 7 },
        },
        target: MidiMapTarget::Param {
            target: AutomationTarget::TrackVolume { track: kick },
        },
        min: 0.0,
        max: 1.0,
        mode: MidiMapMode::Absolute,
    }));
    let mute_map: MidiMappingId = d.id();
    d.insert(Entity::MidiMapping(MidiMapping {
        id: mute_map,
        source: MidiSource {
            port: None,
            channel: None,
            control: MidiControl::Note { key: 36 },
        },
        target: MidiMapTarget::TrackMute { track: kick },
        min: 0.0,
        max: 1.0,
        mode: MidiMapMode::Toggle,
    }));
    let mut h = Harness::new();
    let before = d.open(&mut h);

    h.ok(Command::Track(TrackCommand::Delete { id: kick }));
    let p = h.project();
    assert!(!p.tracks.contains_key(&kick));
    assert_eq!(p.devices[&comp].sidechain, None);
    assert!(p.midi_mappings.is_empty());
    p.validate().unwrap();

    undo(&mut h);
    assert_eq!(h.project(), &before);
}

#[test]
fn rack_pads_cascade_duplicate_and_move_rules() {
    let mut d = Doc::new();
    let drums = d.track(TrackKind::Midi, "a1");
    let other = d.track(TrackKind::Midi, "a2");
    let rack = d.device(drums, BuiltinDevice::DrumRack, None, "a0");
    let kick = d.pad(rack, 36);
    let snare = d.pad(rack, 38);
    let kick_sampler = d.device(
        drums,
        BuiltinDevice::new(BuiltinDeviceType::Sampler),
        Some(kick),
        "a0",
    );
    let snare_sampler = d.device(
        drums,
        BuiltinDevice::new(BuiltinDeviceType::Sampler),
        Some(snare),
        "a0",
    );
    let snare_delay = d.device(drums, BuiltinDevice::Delay, Some(snare), "a1");
    let mut h = Harness::new();
    let before = d.open(&mut h);

    // Pad devices are not part of the track chain.
    assert_eq!(h.project().devices_of(drums).len(), 1);

    // Moving a pad device with Device::Move, or a rack with pads to another track: rejected.
    let out = h.send(Command::Device(DeviceCommand::Move {
        id: snare_delay,
        track: drums,
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(Command::Device(DeviceCommand::Move {
        id: rack,
        track: other,
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project(), &before);

    // Duplicating a pad device stays in its pad chain.
    let dup: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Duplicate {
        id: snare_delay,
        new_id: dup,
    }));
    assert_eq!(h.project().devices[&dup].pad, Some(snare));
    assert_eq!(h.project().pad_devices_of(snare).len(), 3);
    undo(&mut h);

    // Duplicating the rack copies its pads and their chains.
    let rack2: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Duplicate {
        id: rack,
        new_id: rack2,
    }));
    let p = h.project();
    let pads2 = p.pads_of(rack2);
    assert_eq!(
        pads2.iter().map(|p| p.note).collect::<Vec<_>>(),
        vec![36, 38]
    );
    assert_eq!(pads2[0].choke_group, Some(1));
    assert_eq!(p.pad_devices_of(pads2[1].id).len(), 2);
    assert!(pads2.iter().all(|pad| pad.id != kick && pad.id != snare));
    p.validate().unwrap();
    undo(&mut h);
    assert_eq!(h.project(), &before);

    // Duplicating the track copies the rack with its pads.
    let copy: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Duplicate {
        id: drums,
        new_id: copy,
    }));
    let p = h.project();
    let racks: Vec<&Device> = p.devices_of(copy);
    assert_eq!(racks.len(), 1);
    assert_eq!(p.pads_of(racks[0].id).len(), 2);
    assert_eq!(
        p.devices
            .values()
            .filter(|d| d.track == copy && d.pad.is_some())
            .count(),
        3
    );
    p.validate().unwrap();
    undo(&mut h);
    assert_eq!(h.project(), &before);

    // Removing the rack removes its pads and pad chains, in one undo step.
    h.ok(Command::Device(DeviceCommand::Remove { id: rack }));
    let p = h.project();
    assert!(p.drum_pads.is_empty());
    for id in [rack, kick_sampler, snare_sampler, snare_delay] {
        assert!(!p.devices.contains_key(&id));
    }
    undo(&mut h);
    assert_eq!(h.project(), &before);

    // Deleting the track does the same through the rack.
    h.ok(Command::Track(TrackCommand::Delete { id: drums }));
    assert!(h.project().drum_pads.is_empty() && h.project().devices.is_empty());
    undo(&mut h);
    assert_eq!(h.project(), &before);
}
