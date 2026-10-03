//! Presets (v0.2, `presets`; CONTRACTS.md §12.5): factory + user presets of built-ins and
//! plugins, through the controller with an in-memory user library.

mod common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::presets::{
    PresetCommand, PresetEvent, PresetInfo, PresetRef, PresetSource,
};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, Event, ReplyValue, ServerMessage};

const USER: &str = "user";
const PLUGIN: &str = "com.test.Synth";

fn harness() -> Harness {
    let library = MemoryLibrary::new().with_user_root(USER);
    let mut h = Harness::with(FakeBridge::default(), library, Default::default());
    h.create_project("Presets");
    h
}

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
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

fn p(c: PresetCommand) -> Command {
    Command::Preset(c)
}

fn list(h: &mut Harness, device: Option<PresetDevice>, text: Option<&str>) -> Vec<PresetInfo> {
    match h.ok(p(PresetCommand::List {
        device,
        text: text.map(str::to_string),
    })) {
        ReplyValue::Presets { presets } => presets,
        other => panic!("{other:?}"),
    }
}

fn builtin(ty: BuiltinDeviceType) -> Option<PresetDevice> {
    Some(PresetDevice::Builtin { device: ty })
}

fn save(h: &mut Harness, device: DeviceId, name: &str, overwrite: bool) -> Vec<ServerMessage> {
    h.send(p(PresetCommand::Save {
        device,
        name: name.into(),
        meta: PresetMeta {
            tags: vec!["Pad".into(), "warm".into(), "pad".into()],
            author: Some("Me".into()),
            description: None,
        },
        overwrite,
    }))
}

fn saved(out: &[ServerMessage]) -> PresetInfo {
    match ok(out) {
        ReplyValue::Preset { preset } => preset,
        other => panic!("{other:?}"),
    }
}

fn changed(out: &[ServerMessage]) -> bool {
    events(out).iter().any(|e| {
        matches!(
            e,
            Event::Preset {
                event: PresetEvent::Changed
            }
        )
    })
}

fn param(h: &Harness, device: DeviceId, id: u32) -> Option<f64> {
    h.project().devices[&device]
        .params
        .get(&ParamId(id))
        .copied()
}

fn set_param(h: &mut Harness, device: DeviceId, id: u32, value: f64) {
    h.ok(Command::Device(DeviceCommand::SetParam {
        device,
        param: ParamId(id),
        value,
    }));
}

fn factory(id: &str) -> PresetRef {
    PresetRef {
        source: PresetSource::Factory,
        id: id.into(),
    }
}

#[test]
fn lists_factory_presets_sorted_and_filtered() {
    let mut h = harness();
    let all = list(&mut h, builtin(BuiltinDeviceType::Synth), None);
    let names: Vec<_> = all.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["Glass Keys", "Pluck", "Soft Pad", "Square Lead", "Sub Bass"]
    );
    assert!(all.iter().all(|p| p.preset.source == PresetSource::Factory
        && p.preset.id.starts_with("synth/")
        && p.meta.author.as_deref() == Some("Ethereal")));
    // Text matches name, tags and author (case-insensitive).
    let bass = list(&mut h, builtin(BuiltinDeviceType::Synth), Some("BASS"));
    assert_eq!(bass.len(), 1);
    let pads = list(&mut h, None, Some("pad"));
    assert!(pads.iter().any(|p| p.preset.id == "synth/soft-pad"));
    assert!(pads.iter().any(|p| p.preset.id == "sampler/soft-swell"));
    // Every factory preset of every type, and none for plugins.
    let everything = list(&mut h, None, None);
    let total: usize = BuiltinDeviceType::ALL
        .into_iter()
        .map(|t| ether_devices::factory_presets(t).len())
        .sum();
    assert_eq!(everything.len(), total);
    let plugin = PresetDevice::Plugin {
        format: PluginFormat::Clap,
        plugin_id: PLUGIN.into(),
        name: "Synth".into(),
        vendor: "Test".into(),
    };
    assert!(list(&mut h, Some(plugin), None).is_empty());
}

#[test]
fn load_is_one_undo_step_and_resets_missing_params() {
    let mut h = harness();
    let t = track(&mut h, TrackKind::Midi);
    let d = insert(&mut h, t, BuiltinDeviceType::Synth);
    // Transpose (1) is not in "Soft Pad": it resets to its default.
    set_param(&mut h, d, 1, 7.0);
    h.tick();
    let live = h.ctl.bridge.param_changes().len();
    let before = h.project().clone();
    let out = h.send(p(PresetCommand::Load {
        device: d,
        preset: factory("synth/soft-pad"),
        seed: None,
    }));
    ok(&out);
    assert_eq!(patches(&out).len(), 1, "one patch");
    assert_eq!(param(&h, d, 2), Some(800.0)); // attack
    assert_eq!(param(&h, d, 6), Some(2500.0)); // cutoff
    assert_eq!(param(&h, d, 1), Some(0.0)); // transpose reset
    assert_eq!(
        patches(&out)[0].history.undo_label.as_deref(),
        Some("Load Preset")
    );
    // The engine got the new values live.
    assert!(h.ctl.bridge.param_changes().len() > live);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().devices, before.devices);

    // Another device type is rejected without changes.
    let fx = track(&mut h, TrackKind::Audio);
    let comp = insert(&mut h, fx, BuiltinDeviceType::Compressor);
    let before = h.project().clone();
    let out = h.send(p(PresetCommand::Load {
        device: comp,
        preset: factory("synth/pluck"),
        seed: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project(), &before);
    let out = h.send(p(PresetCommand::Load {
        device: comp,
        preset: factory("compressor/nope"),
        seed: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
}

#[test]
fn save_rename_set_meta_delete_user_presets() {
    let mut h = harness();
    let t = track(&mut h, TrackKind::Midi);
    let d = insert(&mut h, t, BuiltinDeviceType::Synth);
    set_param(&mut h, d, 6, 1234.0);
    let out = save(&mut h, d, "  My Pad ", false);
    assert!(changed(&out));
    let mine = saved(&out);
    assert_eq!(mine.name, "My Pad");
    assert_eq!(mine.preset.source, PresetSource::User);
    assert_eq!(mine.preset.id, "synth/My Pad.etherpreset");
    assert_eq!(mine.meta.tags, ["pad", "warm"]);
    assert_eq!(
        h.ctl.library.files(USER),
        ["Presets/synth/My Pad.etherpreset"]
    );
    // Saving is not a document edit.
    assert!(patches(&out).is_empty());

    // Listed after the factory ones.
    let l = list(&mut h, builtin(BuiltinDeviceType::Synth), None);
    assert_eq!(l.last().unwrap(), &mine);
    assert_eq!(
        list(
            &mut h,
            builtin(BuiltinDeviceType::Compressor),
            Some("my pad")
        )
        .len(),
        0
    );

    // Same name (any case): only with overwrite, into the same file.
    let out = save(&mut h, d, "my pad", false);
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    set_param(&mut h, d, 6, 999.0);
    let again = saved(&save(&mut h, d, "my pad", true));
    assert_eq!(again.preset.id, mine.preset.id);
    assert_eq!(again.name, "my pad");
    assert_eq!(h.ctl.library.files(USER).len(), 1);

    // Load it back onto a fresh synth.
    let d2 = insert(&mut h, t, BuiltinDeviceType::Synth);
    h.ok(p(PresetCommand::Load {
        device: d2,
        preset: again.preset.clone(),
        seed: None,
    }));
    assert_eq!(param(&h, d2, 6), Some(999.0));

    // Unsafe characters are replaced; a name that maps to a taken file name gets a
    // numbered file.
    let a = saved(&save(&mut h, d, "My/Pad", false));
    assert_eq!(a.preset.id, "synth/My-Pad.etherpreset");
    let b = saved(&save(&mut h, d, "My:Pad", false));
    assert_eq!(b.preset.id, "synth/My-Pad 2.etherpreset");

    // Rename: moves the file, refuses taken names, factory presets are read-only.
    let out = h.send(p(PresetCommand::Rename {
        preset: again.preset.clone(),
        name: "my/pad".into(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(p(PresetCommand::Rename {
        preset: again.preset.clone(),
        name: "Dark Pad".into(),
    }));
    assert!(changed(&out));
    let renamed = saved(&out);
    assert_eq!(renamed.preset.id, "synth/Dark Pad.etherpreset");
    assert!(
        !h.ctl
            .library
            .files(USER)
            .contains(&"Presets/synth/My Pad.etherpreset".to_string())
    );
    let out = h.send(p(PresetCommand::Rename {
        preset: factory("synth/pluck"),
        name: "X".into(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    // SetMeta.
    let out = h.send(p(PresetCommand::SetMeta {
        preset: renamed.preset.clone(),
        meta: PresetMeta {
            tags: vec!["Dark".into()],
            author: None,
            description: Some("moody".into()),
        },
    }));
    let meta = saved(&out).meta;
    assert_eq!(meta.tags, ["dark"]);
    // Only this user preset carries the tag; factory presets (e.g. Poly Synth's) may too.
    let user_dark: Vec<_> = list(&mut h, None, Some("dark"))
        .into_iter()
        .filter(|p| p.preset.source == PresetSource::User)
        .collect();
    assert_eq!(user_dark.len(), 1);

    // Delete.
    let out = h.send(p(PresetCommand::Delete {
        preset: renamed.preset.clone(),
    }));
    ok(&out);
    assert!(changed(&out));
    let out = h.send(p(PresetCommand::Delete {
        preset: renamed.preset,
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    // Paths can't escape the preset folder.
    let out = h.send(p(PresetCommand::Delete {
        preset: PresetRef {
            source: PresetSource::User,
            id: "../x.etherpreset".into(),
        },
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn read_only_library_lists_factory_and_refuses_writes() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let d = insert(&mut h, t, BuiltinDeviceType::Synth);
    assert_eq!(
        list(&mut h, builtin(BuiltinDeviceType::Synth), None).len(),
        5
    );
    let out = save(&mut h, d, "X", false);
    assert_eq!(err(&out).code, ErrorCode::Unsupported);
    // Factory loads still work.
    h.ok(p(PresetCommand::Load {
        device: d,
        preset: factory("synth/pluck"),
        seed: None,
    }));
}

#[test]
fn invalid_user_files_are_skipped() {
    let mut library = MemoryLibrary::new().with_user_root(USER);
    library.add_file(USER, "Presets/synth/broken.etherpreset", b"{".to_vec());
    library.add_file(USER, "Presets/synth/notes.txt", b"x".to_vec());
    let mut h = Harness::with(FakeBridge::default(), library, Default::default());
    h.create_project("P");
    let l = list(&mut h, builtin(BuiltinDeviceType::Synth), None);
    assert!(l.iter().all(|p| p.preset.source == PresetSource::Factory));
}

#[test]
fn sampler_presets_carry_their_sample() {
    let mut library = MemoryLibrary::new().with_user_root(USER);
    library.add_file(
        "lib",
        "kick.wav",
        wav(48_000, &[sine(48_000, 60.0, 4_800, 0.5)]),
    );
    let mut h = Harness::with(FakeBridge::default(), library, Default::default());
    h.create_project("Samples");
    let t = track(&mut h, TrackKind::Midi);
    let s = insert(&mut h, t, BuiltinDeviceType::Sampler);
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
    let preset = saved(&save(&mut h, s, "Kick", false));
    // Project media are copied into the user library.
    let files = h.ctl.library.files(USER);
    assert!(
        files
            .iter()
            .any(|f| f.starts_with("Samples/") && f.ends_with("-kick.wav")),
        "{files:?}"
    );

    // Same project: the sample is matched by hash (no new media).
    let s2 = insert(&mut h, t, BuiltinDeviceType::Sampler);
    h.ok(p(PresetCommand::Load {
        device: s2,
        preset: preset.preset.clone(),
        seed: None,
    }));
    let kind = |h: &Harness, d: DeviceId| match &h.project().devices[&d].kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::Sampler { sample, .. },
        } => *sample,
        other => panic!("{other:?}"),
    };
    assert_eq!(kind(&h, s2), Some(media));
    assert_eq!(h.project().media.len(), 1);

    // Another project: imported from the user library, one undo step with the load.
    h.create_project("Other");
    let t = track(&mut h, TrackKind::Midi);
    let s3 = insert(&mut h, t, BuiltinDeviceType::Sampler);
    let before = h.project().clone();
    h.ok(p(PresetCommand::Load {
        device: s3,
        preset: preset.preset.clone(),
        seed: None,
    }));
    assert_eq!(h.project().media.len(), 1);
    let imported = kind(&h, s3).expect("sample set");
    assert!(h.project().media.contains_key(&imported));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().devices, before.devices);
    assert!(h.project().media.is_empty());
}

#[test]
fn plugin_presets_store_and_restore_the_state_blob() {
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([(
            PLUGIN.to_string(),
            plugin_descriptor("Synth", DeviceCategory::Instrument),
        )])),
        ..Default::default()
    };
    let library = MemoryLibrary::new().with_user_root(USER);
    let mut h = Harness::with(bridge, library, Default::default());
    h.create_project("Plugins");
    let t = track(&mut h, TrackKind::Midi);
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: PLUGIN.into(),
            sandboxed: Some(false),
            format: Some(PluginFormat::Clap),
        },
        before: None,
    }));
    h.tick();
    h.ctl
        .bridge
        .plugin_states
        .insert(d, Base64Bytes(b"state-A".to_vec()));
    let preset = saved(&save(&mut h, d, "Bright", false));
    assert_eq!(
        preset.preset.id,
        "plugins/clap/com.test.Synth/Bright.etherpreset"
    );
    assert!(
        matches!(preset.device, PresetDevice::Plugin { ref plugin_id, .. } if plugin_id == PLUGIN)
    );
    assert_eq!(
        list(&mut h, Some(preset.device.clone()), None),
        vec![preset.clone()]
    );

    // Load: the document gets the blob and the instance is re-created from it.
    h.ctl
        .bridge
        .plugin_states
        .insert(d, Base64Bytes(b"state-B".to_vec()));
    let creates = |h: &Harness| {
        h.ctl
            .bridge
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::CreatePlugin(id, _, state) if *id == d => Some(state.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let n = creates(&h).len();
    h.ok(p(PresetCommand::Load {
        device: d,
        preset: preset.preset.clone(),
        seed: None,
    }));
    h.tick();
    let DeviceKind::Plugin { plugin } = &h.project().devices[&d].kind else {
        panic!()
    };
    assert_eq!(plugin.state, Some(Base64Bytes(b"state-A".to_vec())));
    let after = creates(&h);
    assert_eq!(after.len(), n + 1, "re-instantiated");
    assert_eq!(
        after.last().unwrap(),
        &Some(Base64Bytes(b"state-A".to_vec()))
    );

    // A built-in can't load a plugin preset.
    let s = insert(&mut h, t, BuiltinDeviceType::Synth);
    let out = h.send(p(PresetCommand::Load {
        device: s,
        preset: preset.preset,
        seed: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}
