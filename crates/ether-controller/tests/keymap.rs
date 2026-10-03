//! Keymap storage (v0.3, `keymap`; CONTRACTS.md §13.12): `Keymap::{Get, Set, Reset}` in the
//! user library (`Settings/keymap.json`), validation, `KeymapEvent::Changed`, and the session
//! fallback for hosts without a writable library.

mod common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_controller::store::Library;
use ether_core::protocol::keymap::{
    KeyBinding, Keymap, KeymapCommand, KeymapEvent, KeymapPreset, MAX_CHORDS_PER_ACTION,
    MAX_KEYMAP_OVERRIDES,
};
use ether_core::protocol::{Command, ErrorCode, Event, ReplyValue};

const USER: &str = "user";
const FILE: &str = "Settings/keymap.json";

fn harness() -> Harness {
    Harness::with(
        FakeBridge::default(),
        MemoryLibrary::new().with_user_root(USER),
        Default::default(),
    )
}

fn binding(action: &str, chords: &[&str]) -> KeyBinding {
    KeyBinding {
        action: action.into(),
        chords: chords.iter().map(|c| c.to_string()).collect(),
    }
}

fn sample() -> Keymap {
    Keymap {
        preset: KeymapPreset::AbletonLike,
        overrides: vec![
            binding("edit.duplicate", &["Mod+Shift+D", "Alt+D"]),
            binding("transport.play", &[]),
        ],
    }
}

fn get(h: &mut Harness) -> Keymap {
    match h.ok(Command::Keymap(KeymapCommand::Get)) {
        ReplyValue::Keymap { keymap } => keymap,
        other => panic!("expected Keymap, got {other:?}"),
    }
}

fn set(h: &mut Harness, keymap: Keymap) -> Vec<ether_core::protocol::ServerMessage> {
    h.send(Command::Keymap(KeymapCommand::Set { keymap }))
}

fn changed(out: &[ether_core::protocol::ServerMessage]) -> Vec<Keymap> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Keymap {
                event: KeymapEvent::Changed { keymap },
            } => Some(keymap),
            _ => None,
        })
        .collect()
}

#[test]
fn get_answers_the_default_when_nothing_is_stored() {
    let mut h = harness();
    assert_eq!(get(&mut h), Keymap::default());
    assert_eq!(Keymap::default().preset, KeymapPreset::Ethereal);
}

#[test]
fn set_persists_in_the_user_library_and_emits_changed() {
    let mut h = harness();
    let out = set(&mut h, sample());
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert_eq!(changed(&out), vec![sample()]);
    assert_eq!(get(&mut h), sample());
    let bytes = h.ctl.library.read(USER, FILE).expect("stored file");
    let stored: Keymap = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored, sample());
    // Works without a project and survives opening one (a user setting, not a document one).
    h.create_project("Any");
    assert_eq!(get(&mut h), sample());
}

#[test]
fn keymap_follows_the_user_library_into_a_new_session() {
    let mut h = harness();
    assert_eq!(ok(&set(&mut h, sample())), ReplyValue::Unit);
    let bytes = h.ctl.library.read(USER, FILE).unwrap();
    // A new controller over the same user library (an app restart).
    let mut library = MemoryLibrary::new().with_user_root(USER);
    library.write_file(USER, FILE, &bytes).unwrap();
    let mut next = Harness::with(FakeBridge::default(), library, Default::default());
    assert_eq!(get(&mut next), sample());
}

#[test]
fn reset_removes_the_file_and_emits_the_default() {
    let mut h = harness();
    set(&mut h, sample());
    let out = h.send(Command::Keymap(KeymapCommand::Reset));
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert_eq!(changed(&out), vec![Keymap::default()]);
    assert!(h.ctl.library.read(USER, FILE).is_err());
    assert_eq!(get(&mut h), Keymap::default());
    // Reset with nothing stored is fine.
    assert_eq!(
        h.ok(Command::Keymap(KeymapCommand::Reset)),
        ReplyValue::Unit
    );
}

#[test]
fn hosts_without_a_writable_library_keep_it_for_the_session() {
    let mut h = Harness::new();
    assert_eq!(get(&mut h), Keymap::default());
    let out = set(&mut h, sample());
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert_eq!(changed(&out), vec![sample()]);
    assert_eq!(get(&mut h), sample());
    h.ok(Command::Keymap(KeymapCommand::Reset));
    assert_eq!(get(&mut h), Keymap::default());
}

#[test]
fn invalid_keymaps_are_refused_and_change_nothing() {
    let mut h = harness();
    set(&mut h, sample());
    let too_many_chords: Vec<String> = (0..=MAX_CHORDS_PER_ACTION)
        .map(|i| format!("F{}", i + 1))
        .collect();
    let too_many_overrides: Vec<KeyBinding> = (0..=MAX_KEYMAP_OVERRIDES)
        .map(|i| binding(&format!("a.{i:05}"), &[]))
        .collect();
    let bad = [
        // Chord syntax: lowercase key, modifiers out of order, unknown modifier, two keys.
        vec![binding("edit.copy", &["Mod+c"])],
        vec![binding("edit.copy", &["Shift+Mod+C"])],
        vec![binding("edit.copy", &["Cmd+C"])],
        vec![binding("edit.copy", &["Mod+C+V"])],
        vec![binding("edit.copy", &["Mod+Shift+?"])],
        // Same chord twice for one action.
        vec![binding("edit.copy", &["Mod+C", "Mod+C"])],
        // Not sorted, duplicated, empty action id.
        vec![binding("b", &[]), binding("a", &[])],
        vec![binding("a", &[]), binding("a", &["F1"])],
        vec![binding("", &["F1"])],
        vec![KeyBinding {
            action: "edit.copy".into(),
            chords: too_many_chords,
        }],
        too_many_overrides,
    ];
    for overrides in bad {
        let out = set(
            &mut h,
            Keymap {
                preset: KeymapPreset::Ethereal,
                overrides,
            },
        );
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
        assert!(changed(&out).is_empty());
        assert_eq!(get(&mut h), sample());
    }
}

#[test]
fn limits_are_inclusive() {
    let mut h = harness();
    let chords: Vec<String> = (0..MAX_CHORDS_PER_ACTION)
        .map(|i| format!("Mod+Alt+F{}", i + 1))
        .collect();
    let mut overrides: Vec<KeyBinding> = (0..MAX_KEYMAP_OVERRIDES - 1)
        .map(|i| binding(&format!("a.{i:05}"), &[]))
        .collect();
    overrides.push(KeyBinding {
        action: "z.last".into(),
        chords,
    });
    let keymap = Keymap {
        preset: KeymapPreset::Ethereal,
        overrides,
    };
    assert_eq!(ok(&set(&mut h, keymap.clone())), ReplyValue::Unit);
    assert_eq!(get(&mut h), keymap);
}

#[test]
fn an_unusable_stored_file_reads_as_the_default() {
    let mut h = harness();
    for junk in [
        &b"not json"[..],
        br#"{"preset":"Ethereal","overrides":[{"action":"x","chords":["mod+x"]}]}"#,
        br#"{"preset":"Future","overrides":[]}"#,
    ] {
        h.ctl.library.write_file(USER, FILE, junk).unwrap();
        assert_eq!(get(&mut h), Keymap::default());
    }
    // The next Set overwrites it.
    set(&mut h, sample());
    assert_eq!(get(&mut h), sample());
}
