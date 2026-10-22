//! Settings "sandboxed by default" (#231): the host's list picks the flag of a NEW plugin
//! instance only (an insert without a flag, a replacement by another plugin); the flag then
//! belongs to the instance: copies and undo keep it, an explicit flag wins, and editing the
//! list never changes an existing instance.

use crate::common;

use std::collections::BTreeMap;

use common::*;
use ether_core::protocol::Command;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::resolver::{ReplacementPlugin, ResolveCommand};
use ether_core::protocol::tracks::TrackCommand;

const VERB: &str = "com.test.Verb";
const DELAY: &str = "com.test.Delay";

fn harness() -> (Harness, TrackId) {
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([
            (
                VERB.to_string(),
                plugin_descriptor("Verb", DeviceCategory::AudioEffect),
            ),
            (
                DELAY.to_string(),
                plugin_descriptor("Delay", DeviceCategory::AudioEffect),
            ),
        ])),
        // Listed by another case of its id: ids compare case-insensitively.
        sandbox_defaults: vec![(PluginFormat::Clap, "COM.TEST.VERB".into())],
        ..Default::default()
    };
    let mut h = Harness::with(bridge, Default::default(), Default::default());
    h.create_project("Sandbox defaults");
    let t = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    (h, t)
}

fn insert(
    h: &mut Harness,
    t: TrackId,
    id: &str,
    format: PluginFormat,
    sandboxed: Option<bool>,
) -> DeviceId {
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: id.into(),
            sandboxed,
            format: Some(format),
            name: None,
        },
        before: None,
    }));
    d
}

fn sandboxed(h: &Harness, d: DeviceId) -> bool {
    match &h.project().devices[&d].kind {
        DeviceKind::Plugin { plugin } => plugin.sandboxed,
        k => panic!("{k:?}"),
    }
}

#[test]
fn a_new_instance_of_a_listed_plugin_starts_sandboxed() {
    let (mut h, t) = harness();
    let verb = insert(&mut h, t, VERB, PluginFormat::Clap, None);
    let delay = insert(&mut h, t, DELAY, PluginFormat::Clap, None);
    // The same id in another format is another plugin.
    let vst3 = insert(&mut h, t, VERB, PluginFormat::Vst3, None);
    // An explicit flag wins over the list.
    let forced = insert(&mut h, t, VERB, PluginFormat::Clap, Some(false));
    assert!(sandboxed(&h, verb));
    assert!(!sandboxed(&h, delay));
    assert!(!sandboxed(&h, vst3));
    assert!(!sandboxed(&h, forced));
}

#[test]
fn the_flag_is_the_instances_copies_and_undo_keep_it_and_list_edits_never_touch_it() {
    let (mut h, t) = harness();
    let verb = insert(&mut h, t, VERB, PluginFormat::Clap, None);
    // Switched back in process by hand: a copy keeps that, not the list's default.
    h.ok(Command::Plugin(
        ether_core::protocol::plugins::PluginCommand::SetSandboxed {
            device: verb,
            sandboxed: false,
        },
    ));
    let copy: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Duplicate {
        id: verb,
        new_id: copy,
        track: None,
    }));
    assert!(!sandboxed(&h, copy));

    // A sandboxed instance of an unlisted plugin: its copy is sandboxed too.
    let delay = insert(&mut h, t, DELAY, PluginFormat::Clap, Some(true));
    let delay_copy: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Duplicate {
        id: delay,
        new_id: delay_copy,
        track: None,
    }));
    assert!(sandboxed(&h, delay_copy));

    // Emptying the list changes no instance; undo / redo bring back the flag they had.
    let listed = insert(&mut h, t, VERB, PluginFormat::Clap, None);
    h.ctl.bridge.sandbox_defaults.clear();
    assert!(sandboxed(&h, listed));
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(!h.project().devices.contains_key(&listed));
    h.ok(Command::Edit(EditCommand::Redo));
    assert!(sandboxed(&h, listed));
    assert!(!sandboxed(&h, verb));
}

#[test]
fn a_replacement_by_another_plugin_is_a_new_instance_the_same_plugin_keeps_its_flag() {
    let (mut h, t) = harness();
    let to = |id: &str, name: &str| ReplacementPlugin {
        format: PluginFormat::Clap,
        plugin_id: id.into(),
        name: name.into(),
        vendor: String::new(),
        version: String::new(),
    };
    let replace = |h: &mut Harness, device: DeviceId, plugin: ReplacementPlugin| {
        h.ok(Command::Resolve(ResolveCommand::Replace {
            device,
            plugin: Some(plugin),
            builtin: None,
            retain: false,
            preview: false,
        }));
    };
    // Delay (in process) -> Verb (listed): sandboxed.
    let d = insert(&mut h, t, DELAY, PluginFormat::Clap, None);
    replace(&mut h, d, to(VERB, "Verb"));
    assert!(sandboxed(&h, d));
    // Verb switched in process, replaced by Verb again: keeps its own flag.
    h.ok(Command::Plugin(
        ether_core::protocol::plugins::PluginCommand::SetSandboxed {
            device: d,
            sandboxed: false,
        },
    ));
    replace(&mut h, d, to(VERB, "Verb"));
    assert!(!sandboxed(&h, d));
    // A sandboxed Verb -> Delay (unlisted): Delay's default, in process.
    let v = insert(&mut h, t, VERB, PluginFormat::Clap, None);
    replace(&mut h, v, to(DELAY, "Delay"));
    assert!(!sandboxed(&h, v));
}

#[test]
fn an_imported_project_gets_the_list_on_its_listed_plugins_only() {
    let (mut h, t) = harness();
    let verb = insert(&mut h, t, VERB, PluginFormat::Clap, Some(false));
    let delay = insert(&mut h, t, DELAY, PluginFormat::Clap, Some(true));
    let mut imported = h.project().clone();
    h.ctl.apply_sandbox_defaults(&mut imported);
    let flag = |p: &Project, d: DeviceId| match &p.devices[&d].kind {
        DeviceKind::Plugin { plugin } => plugin.sandboxed,
        k => panic!("{k:?}"),
    };
    assert!(flag(&imported, verb));
    // Never turned off.
    assert!(flag(&imported, delay));
}
