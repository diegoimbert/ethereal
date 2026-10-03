//! Templates (v0.3, `templates`; CONTRACTS.md §13.10): track templates saved from the
//! document and inserted elsewhere (one undo step, derived ids), project templates and the
//! default "New project", library management, through the controller with an in-memory
//! user library.

mod common;

use std::collections::BTreeSet;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_controller::store::{Library, ProjectStore};
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::template::load_template;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::protocol::templates::{TemplateCommand, TemplateEvent, TemplateInfo, TemplateKind};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, Event, ReplyValue, ServerMessage};

const USER: &str = "user";

fn harness_with(library: MemoryLibrary) -> Harness {
    let mut h = Harness::with(FakeBridge::default(), library, Default::default());
    h.create_project("Templates");
    h
}

fn harness() -> Harness {
    harness_with(MemoryLibrary::new().with_user_root(USER))
}

fn t(c: TemplateCommand) -> Command {
    Command::Template(c)
}

fn track(h: &mut Harness, kind: TrackKind, parent: Option<TrackId>) -> TrackId {
    let id = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent,
        before: None,
    }));
    id
}

fn insert(h: &mut Harness, track: TrackId, ty: BuiltinDeviceType) -> DeviceId {
    let id = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(ty),
        },
        before: None,
    }));
    id
}

fn send(h: &mut Harness, from: TrackId, to: TrackId) -> SendId {
    let id = h.id();
    h.ok(Command::Mixer(MixerCommand::CreateSend {
        id,
        from,
        to,
        level: Decibels(-6.0),
        pre_fader: false,
    }));
    id
}

fn lane(h: &mut Harness, track: TrackId, target: AutomationTarget) -> AutomationLaneId {
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Track { track },
        target,
    }));
    let a: AutomationPointId = h.id();
    let b: AutomationPointId = h.id();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane,
        points: vec![
            PointSpec {
                id: a,
                time: Beats(0.0),
                value: 0.2,
                curve: CurveShape::Linear,
            },
            PointSpec {
                id: b,
                time: Beats(4.0),
                value: 0.8,
                curve: CurveShape::Linear,
            },
        ],
    }));
    lane
}

fn save_tracks(h: &mut Harness, tracks: Vec<TrackId>, name: &str) -> TemplateInfo {
    match h.ok(t(TemplateCommand::SaveTracks {
        tracks,
        name: name.into(),
        meta: PresetMeta::default(),
        overwrite: false,
    })) {
        ReplyValue::Template { template } => template,
        other => panic!("{other:?}"),
    }
}

fn list(h: &mut Harness, kind: Option<TemplateKind>) -> Vec<TemplateInfo> {
    match h.ok(t(TemplateCommand::List { kind })) {
        ReplyValue::Templates { templates } => templates,
        other => panic!("{other:?}"),
    }
}

fn insert_template(
    h: &mut Harness,
    template: &str,
    parent: Option<TrackId>,
    before: Option<TrackId>,
) -> TrackId {
    let seed: TrackId = h.id();
    h.ok(t(TemplateCommand::Insert {
        template: template.into(),
        seed,
        parent,
        before,
    }));
    seed
}

fn changed(out: &[ServerMessage]) -> bool {
    events(out).iter().any(|e| {
        matches!(
            e,
            Event::Template {
                event: TemplateEvent::Changed
            }
        )
    })
}

/// A "rich" source: a group with an audio child (EQ with a modulated param, an instrument
/// rack with a chain device, automation of the EQ and of a send), a return inside the
/// selection, and a return outside it.
struct Source {
    group: TrackId,
    child: TrackId,
    ret_in: TrackId,
    ret_out: TrackId,
}

fn source(h: &mut Harness) -> Source {
    let group = track(h, TrackKind::Group, None);
    let child = track(h, TrackKind::Midi, Some(group));
    let ret_in = track(h, TrackKind::Return, None);
    let ret_out = track(h, TrackKind::Return, None);
    h.ok(Command::Track(TrackCommand::Rename {
        id: child,
        name: "Lead".into(),
    }));
    let rack = insert(h, child, BuiltinDeviceType::InstrumentRack);
    let chain: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id: chain,
        rack,
        name: None,
        before: None,
    }));
    let synth: DeviceId = h.id();
    h.ok(Command::Rack(RackCommand::InsertDevice {
        id: synth,
        chain,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::PolySynth),
        },
        before: None,
    }));
    let eq = insert(h, child, BuiltinDeviceType::Eq);
    let lfo: ModulatorId = h.id();
    h.ok(Command::Modulation(ModulationCommand::AddModulator {
        id: lfo,
        device: eq,
        kind: ModulatorKind::Lfo,
        name: None,
    }));
    let map: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: map,
        source: ModSource::Modulator { modulator: lfo },
        device: eq,
        param: ParamId(0),
        depth: 0.4,
    }));
    let drums = insert(h, group, BuiltinDeviceType::Compressor);
    let _ = drums;
    let s_in = send(h, child, ret_in);
    let _s_out = send(h, child, ret_out);
    lane(
        h,
        child,
        AutomationTarget::DeviceParam {
            device: eq,
            param: ParamId(0),
        },
    );
    lane(h, child, AutomationTarget::SendLevel { send: s_in });
    // A clip: never part of a template.
    let clip: ClipId = h.id();
    h.ok(Command::Clip(
        ether_core::protocol::clips::ClipCommand::CreateMidi {
            id: clip,
            track: child,
            start: Beats(0.0),
            length: Beats(4.0),
            name: None,
        },
    ));
    Source {
        group,
        child,
        ret_in,
        ret_out,
    }
}

/// Everything the template brought in: entity counts by kind, minus `before`.
fn new_ids(before: &Project, after: &Project) -> BTreeSet<EntityKey> {
    let a: BTreeSet<EntityKey> = before.entities().iter().map(Entity::key).collect();
    after
        .entities()
        .iter()
        .map(Entity::key)
        .filter(|k| !a.contains(k))
        .collect()
}

#[test]
fn save_and_insert_tracks_as_one_undo_step_with_derived_ids() {
    let mut h = harness();
    let s = source(&mut h);
    let out = h.send(t(TemplateCommand::SaveTracks {
        tracks: vec![s.group, s.ret_in],
        name: "Band".into(),
        meta: PresetMeta {
            tags: vec!["Lead".into()],
            author: None,
            description: None,
        },
        overwrite: false,
    }));
    assert!(changed(&out));
    let info = match ok(&out) {
        ReplyValue::Template { template } => template,
        other => panic!("{other:?}"),
    };
    assert_eq!(info.id, "tracks/Band");
    assert_eq!(info.kind, TemplateKind::Tracks);
    assert_eq!(info.meta.tags, ["lead"]);
    assert!(!info.factory && !info.default);
    assert!(
        h.ctl
            .library
            .files(USER)
            .contains(&"Templates/Tracks/Band.ethertemplate".to_string())
    );
    let listed = list(&mut h, Some(TemplateKind::Tracks));
    assert!(
        listed
            .iter()
            .any(|i| i.id == "tracks/Band" && i.modified_ms > 0)
    );

    // Insert into another project.
    h.create_project("Other");
    let before = h.project().clone();
    let seed = insert_template(&mut h, "tracks/Band", None, None);
    let after = h.project().clone();
    let added = new_ids(&before, &after);
    // group, child, return; compressor, rack, chain, synth, eq; modulator; one send (the one
    // to the return outside is dropped); two lanes with two points each; one mapping.
    let count = |f: fn(&EntityKey) -> bool| added.iter().filter(|k| f(k)).count();
    assert_eq!(count(|k| matches!(k, EntityKey::Track(_))), 3);
    assert_eq!(count(|k| matches!(k, EntityKey::Device(_))), 4);
    assert_eq!(count(|k| matches!(k, EntityKey::RackChain(_))), 1);
    assert_eq!(count(|k| matches!(k, EntityKey::Modulator(_))), 1);
    assert_eq!(count(|k| matches!(k, EntityKey::ModMapping(_))), 1);
    assert_eq!(count(|k| matches!(k, EntityKey::Send(_))), 1);
    assert_eq!(count(|k| matches!(k, EntityKey::AutomationLane(_))), 2);
    assert_eq!(count(|k| matches!(k, EntityKey::AutomationPoint(_))), 4);
    assert_eq!(added.len(), 3 + 4 + 1 + 1 + 1 + 1 + 2 + 4);
    assert!(after.clips.is_empty());

    // Ids: entity i of the template is derive_id(seed, i).
    let group: TrackId = derive_id(seed, 0);
    let g = &after.tracks[&group];
    assert_eq!(g.kind, TrackKind::Group);
    assert_eq!(g.parent, None);
    let kids = after.child_tracks(group);
    assert_eq!(kids.len(), 1);
    let child = kids[0].clone();
    assert_eq!(child.name, "Lead");
    assert_eq!(child.id, derive_id::<_, TrackId>(seed, 1));
    // References are remapped to the new entities.
    let send = after.sends.values().next().unwrap();
    assert_eq!(send.from, child.id);
    assert_eq!(after.tracks[&send.to].kind, TrackKind::Return);
    assert!(after.tracks[&send.to].name.contains("Return"));
    for l in after.automation_lanes.values() {
        assert_eq!(l.owner, AutomationOwner::Track { track: child.id });
        match l.target {
            AutomationTarget::DeviceParam { device, .. } => {
                assert_eq!(after.devices[&device].track, child.id)
            }
            AutomationTarget::SendLevel { send: s } => assert_eq!(s, send.id),
            other => panic!("{other:?}"),
        }
    }
    for d in after.devices.values() {
        assert!(after.tracks.contains_key(&d.track));
        if let Some(c) = d.chain {
            assert!(after.rack_chains.contains_key(&c));
        }
    }
    let m = after.mod_mappings.values().next().unwrap();
    assert!(
        matches!(m.source, ModSource::Modulator { modulator } if after.modulators.contains_key(&modulator))
    );
    // Layout: regular tracks before the return, the return before master.
    let order: Vec<TrackKind> = after.tracks_ordered().iter().map(|t| t.kind).collect();
    assert_eq!(
        order,
        [TrackKind::Group, TrackKind::Return, TrackKind::Master]
    );

    // One undo step.
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &before);
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(h.project(), &after);

    // Retried message: no change.
    let out = h.send(t(TemplateCommand::Insert {
        template: "tracks/Band".into(),
        seed,
        parent: None,
        before: None,
    }));
    ok(&out);
    assert!(patches(&out).is_empty());
    assert_eq!(h.project(), &after);
    let _ = (s.child, s.ret_out);
}

#[test]
fn replay_with_the_same_seed_mints_the_same_ids() {
    // Two sites (collab replay) inserting the same template with the same seed.
    let library = {
        let mut h = harness();
        let s = source(&mut h);
        save_tracks(&mut h, vec![s.child], "Lead");
        h.ctl.library.clone()
    };
    let mut a = harness_with(library.clone());
    let mut b = harness_with(library);
    let seed: TrackId = a.id();
    let before_a = a.project().clone();
    let before_b = b.project().clone();
    for h in [&mut a, &mut b] {
        h.ok(t(TemplateCommand::Insert {
            template: "tracks/Lead".into(),
            seed,
            parent: None,
            before: None,
        }));
    }
    let ids_a = new_ids(&before_a, a.project());
    let ids_b = new_ids(&before_b, b.project());
    assert!(!ids_a.is_empty());
    assert_eq!(ids_a, ids_b);
}

#[test]
fn insert_under_a_group_before_a_sibling_and_in_a_batch() {
    let mut h = harness();
    let s = source(&mut h);
    save_tracks(&mut h, vec![s.child], "Lead");
    let g = track(&mut h, TrackKind::Group, None);
    let a = track(&mut h, TrackKind::Audio, Some(g));
    let b = track(&mut h, TrackKind::Audio, Some(g));
    let seed = insert_template(&mut h, "tracks/Lead", Some(g), Some(b));
    let new: TrackId = derive_id(seed, 0);
    let kids: Vec<TrackId> = h.project().child_tracks(g).iter().map(|t| t.id).collect();
    assert_eq!(kids, [a, new, b]);
    // The outside send is dropped and the outside sidechain-free devices stay.
    assert!(h.project().sends.values().all(|s| s.from != new));

    // Inside a batch, with another edit: one undo step.
    let before = h.project().clone();
    let seed2: TrackId = h.id();
    let extra: TrackId = h.id();
    h.ok(Command::Edit(EditCommand::Batch {
        label: "Add stuff".into(),
        commands: vec![
            t(TemplateCommand::Insert {
                template: "tracks/Lead".into(),
                seed: seed2,
                parent: None,
                before: None,
            }),
            Command::Track(TrackCommand::Create {
                id: extra,
                kind: TrackKind::Audio,
                name: None,
                color: None,
                parent: None,
                before: None,
            }),
        ],
    }));
    assert!(h.project().tracks.contains_key(&derive_id(seed2, 0)));
    assert!(h.project().tracks.contains_key(&extra));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &before);

    // Bad placement.
    let seed3: TrackId = h.id();
    let out = h.send(t(TemplateCommand::Insert {
        template: "tracks/Lead".into(),
        seed: seed3,
        parent: Some(a),
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(t(TemplateCommand::Insert {
        template: "tracks/Nope".into(),
        seed: seed3,
        parent: None,
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    assert_eq!(h.project(), &before);
    let _ = (s.group, s.ret_in, s.ret_out);
}

#[test]
fn track_template_samples_are_carried_and_imported() {
    let mut library = MemoryLibrary::new().with_user_root(USER);
    library.add_file(
        "lib",
        "kick.wav",
        wav(48_000, &[sine(48_000, 60.0, 4_800, 0.5)]),
    );
    let mut h = harness_with(library);
    let tr = track(&mut h, TrackKind::Midi, None);
    let s = insert(&mut h, tr, BuiltinDeviceType::Sampler);
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "kick.wav".into(),
        },
    }));
    h.ok(Command::Device(DeviceCommand::SetSample {
        device: s,
        media: Some(media),
    }));
    save_tracks(&mut h, vec![tr], "Kick");
    let files = h.ctl.library.files(USER);
    assert!(
        files
            .iter()
            .any(|f| f.starts_with("Samples/") && f.ends_with("-kick.wav")),
        "{files:?}"
    );

    // Same project: matched by hash.
    let seed = insert_template(&mut h, "tracks/Kick", None, None);
    assert_eq!(h.project().media.len(), 1);
    let sampler = |h: &Harness, track: TrackId| {
        h.project()
            .devices_of(track)
            .iter()
            .find_map(|d| match &d.kind {
                DeviceKind::Builtin {
                    device: BuiltinDevice::Sampler { sample, .. },
                } => Some(*sample),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(sampler(&h, derive_id(seed, 0)), Some(media));

    // Another project: imported, one undo step with the insert.
    h.create_project("Other");
    let before = h.project().clone();
    let seed = insert_template(&mut h, "tracks/Kick", None, None);
    assert_eq!(h.project().media.len(), 1);
    let imported = sampler(&h, derive_id(seed, 0)).expect("sample set");
    assert!(h.project().media.contains_key(&imported));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().tracks, before.tracks);
    assert!(h.project().media.is_empty());
}

#[test]
fn library_management() {
    let mut h = harness();
    let a = track(&mut h, TrackKind::Audio, None);
    save_tracks(&mut h, vec![a], "Vox");
    // Names are unique per kind (case-insensitive).
    let out = h.send(t(TemplateCommand::SaveTracks {
        tracks: vec![a],
        name: "vox".into(),
        meta: PresetMeta::default(),
        overwrite: false,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(t(TemplateCommand::SaveTracks {
        tracks: vec![a],
        name: "vox".into(),
        meta: PresetMeta::default(),
        overwrite: true,
    }));
    match ok(&out) {
        ReplyValue::Template { template } => {
            assert_eq!(template.id, "tracks/Vox");
            assert_eq!(template.name, "vox");
        }
        other => panic!("{other:?}"),
    }
    // A project template may share the name.
    h.ok(t(TemplateCommand::SaveProject {
        name: "Vox".into(),
        meta: PresetMeta::default(),
        overwrite: false,
    }));
    // Rename moves the file.
    let out = h.send(t(TemplateCommand::Rename {
        template: "tracks/Vox".into(),
        name: "Lead Vox".into(),
    }));
    assert!(changed(&out));
    match ok(&out) {
        ReplyValue::Template { template } => assert_eq!(template.id, "tracks/Lead Vox"),
        other => panic!("{other:?}"),
    }
    let files = h.ctl.library.files(USER);
    assert!(files.contains(&"Templates/Tracks/Lead Vox.ethertemplate".to_string()));
    assert!(!files.contains(&"Templates/Tracks/Vox.ethertemplate".to_string()));
    // Empty names, factory templates and bad ids are refused.
    let out = h.send(t(TemplateCommand::Rename {
        template: "tracks/Lead Vox".into(),
        name: "  ".into(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(t(TemplateCommand::Delete {
        template: "factory/vocal-chain".into(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(t(TemplateCommand::Delete {
        template: "../x".into(),
    }));
    err(&out);
    // Master can't be saved.
    let master = h.project().master_track().id;
    let out = h.send(t(TemplateCommand::SaveTracks {
        tracks: vec![master],
        name: "M".into(),
        meta: PresetMeta::default(),
        overwrite: false,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    // Listing: factory first, then user, by name; filtered by kind.
    let all = list(&mut h, None);
    let first_user = all.iter().position(|i| !i.factory).unwrap();
    assert!(all[..first_user].iter().all(|i| i.factory));
    assert!(all[first_user..].iter().all(|i| !i.factory));
    let user: Vec<&str> = all[first_user..].iter().map(|i| i.name.as_str()).collect();
    assert_eq!(user, ["Lead Vox", "Vox"]);
    assert!(
        list(&mut h, Some(TemplateKind::Project))
            .iter()
            .all(|i| i.kind == TemplateKind::Project)
    );

    // Delete.
    let out = h.send(t(TemplateCommand::Delete {
        template: "tracks/Lead Vox".into(),
    }));
    assert!(changed(&out));
    assert!(
        !list(&mut h, Some(TemplateKind::Tracks))
            .iter()
            .any(|i| i.id == "tracks/Lead Vox")
    );
}

#[test]
fn factory_templates_insert() {
    let mut h = harness();
    let factory: Vec<TemplateInfo> = list(&mut h, Some(TemplateKind::Tracks))
        .into_iter()
        .filter(|i| i.factory)
        .collect();
    assert!(factory.len() >= 3);
    for f in &factory {
        let before = h.project().tracks.len();
        let seed = insert_template(&mut h, &f.id, None, None);
        assert!(h.project().tracks.len() > before, "{}", f.id);
        assert!(
            !h.project().devices_of(derive_id(seed, 0)).is_empty(),
            "{}",
            f.id
        );
    }
}

#[test]
fn hosts_without_a_user_library_list_factory_templates_only() {
    let mut h = harness_with(MemoryLibrary::new());
    let all = list(&mut h, None);
    assert!(!all.is_empty() && all.iter().all(|i| i.factory));
    let a = track(&mut h, TrackKind::Audio, None);
    let out = h.send(t(TemplateCommand::SaveTracks {
        tracks: vec![a],
        name: "X".into(),
        meta: PresetMeta::default(),
        overwrite: false,
    }));
    assert_eq!(err(&out).code, ErrorCode::Unsupported);
    // "New project" still works (empty project).
    let id = h.project_id();
    h.ok(t(TemplateCommand::NewProject {
        id,
        name: "Song".into(),
        template: None,
    }));
    assert_eq!(h.project().id, id);
}

#[test]
fn new_project_from_a_project_template_and_the_default() {
    let mut library = MemoryLibrary::new().with_user_root(USER);
    library.add_file(
        "lib",
        "pad.wav",
        wav(48_000, &[sine(48_000, 220.0, 4_800, 0.5)]),
    );
    let mut h = harness_with(library);
    // A song skeleton: a MIDI track with a synth, a tempo, an imported sample on an audio clip.
    let keys = track(&mut h, TrackKind::Midi, None);
    insert(&mut h, keys, BuiltinDeviceType::PolySynth);
    let audio = track(&mut h, TrackKind::Audio, None);
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "pad.wav".into(),
        },
    }));
    let _ = audio;
    h.ok(Command::Transport(
        ether_core::protocol::transport::TransportCommand::SetTempo { bpm: 96.0 },
    ));
    let source = h.project().clone();
    let out = h.send(t(TemplateCommand::SaveProject {
        name: "Band".into(),
        meta: PresetMeta::default(),
        overwrite: false,
    }));
    assert!(changed(&out));
    let file = h
        .ctl
        .library
        .files(USER)
        .into_iter()
        .find(|f| f.ends_with("Band.ethertemplate"))
        .unwrap();
    assert_eq!(file, "Templates/Projects/Band.ethertemplate");

    // New project from it: same content, new id and name, media copied in.
    let id = h.project_id();
    let reply = h.ok(t(TemplateCommand::NewProject {
        id,
        name: "Song".into(),
        template: Some("projects/Band".into()),
    }));
    let p = match reply {
        ReplyValue::Project { project } => project,
        other => panic!("{other:?}"),
    };
    assert_eq!(p.id, id);
    assert_eq!(p.settings.name, "Song");
    assert_eq!(p.tracks, source.tracks);
    assert_eq!(p.devices, source.devices);
    assert_eq!(p.tempo_points, source.tempo_points);
    assert_eq!(h.project().id, id);
    let m = &p.media[&media];
    assert_eq!(m.location, MediaLocation::Project);
    assert!(h.ctl.store.read(id, &m.file).is_ok());
    // Stored and listed.
    let listed = match h.ok(Command::Project(ProjectCommand::List)) {
        ReplyValue::Projects { projects } => projects,
        other => panic!("{other:?}"),
    };
    assert!(listed.iter().any(|s| s.id == id && s.name == "Song"));

    // A track template is not a project template.
    let a = track(&mut h, TrackKind::Audio, None);
    save_tracks(&mut h, vec![a], "Vox");
    let id2 = h.project_id();
    let out = h.send(t(TemplateCommand::NewProject {
        id: id2,
        name: "X".into(),
        template: Some("tracks/Vox".into()),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(t(TemplateCommand::SetDefault {
        template: Some("tracks/Vox".into()),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    // No default: an empty project.
    let id3 = h.project_id();
    h.ok(t(TemplateCommand::NewProject {
        id: id3,
        name: "Empty".into(),
        template: None,
    }));
    assert_eq!(h.project().tracks.len(), 1);

    // The default project template.
    let out = h.send(t(TemplateCommand::SetDefault {
        template: Some("projects/Band".into()),
    }));
    assert!(changed(&out));
    assert!(
        list(&mut h, Some(TemplateKind::Project))
            .iter()
            .any(|i| i.id == "projects/Band" && i.default)
    );
    let id4 = h.project_id();
    h.ok(t(TemplateCommand::NewProject {
        id: id4,
        name: "From default".into(),
        template: None,
    }));
    assert_eq!(h.project().tracks, source.tracks);

    // Renaming the default keeps it the default; deleting it clears it.
    h.ok(t(TemplateCommand::Rename {
        template: "projects/Band".into(),
        name: "Band 2".into(),
    }));
    assert!(
        list(&mut h, Some(TemplateKind::Project))
            .iter()
            .any(|i| i.id == "projects/Band 2" && i.default)
    );
    h.ok(t(TemplateCommand::Delete {
        template: "projects/Band 2".into(),
    }));
    let id5 = h.project_id();
    h.ok(t(TemplateCommand::NewProject {
        id: id5,
        name: "After delete".into(),
        template: None,
    }));
    assert_eq!(h.project().tracks.len(), 1);

    // The file on disk is a valid template.
    let bytes = h
        .ctl
        .library
        .clone()
        .files(USER)
        .into_iter()
        .find(|f| f == "Templates/Tracks/Vox.ethertemplate");
    assert!(bytes.is_some());
}

#[test]
fn saved_files_are_valid_template_files() {
    let mut h = harness();
    let s = source(&mut h);
    save_tracks(&mut h, vec![s.group], "Drums");
    let bytes = h
        .ctl
        .library
        .read(USER, "Templates/Tracks/Drums.ethertemplate")
        .unwrap();
    let tpl = load_template(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(tpl.name, "Drums");
    match tpl.body {
        ether_core::protocol::model::template::TemplateBody::Tracks { entities, .. } => {
            assert!(matches!(entities.first(), Some(Entity::Track(t)) if t.id == s.group));
            assert!(!entities.iter().any(|e| matches!(e, Entity::Clip(_))));
        }
        other => panic!("{other:?}"),
    }
}
