//! Every command variant has a human label. The variants aren't listed by hand: the test
//! asks serde for each domain's and each command's variant names ("unknown variant …,
//! expected one of …") and fills in their fields from serde's errors, so a new command is
//! checked without touching this file.

use serde_json::{Value, json};

use super::label_of;
use ether_core::protocol::Command;

const PROBE: &str = "__probe__";
/// A valid ULID (every id type) that is also a fine plain string.
const ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
/// Project ids are UUIDs.
const UUID: &str = "00000000-0000-4000-8000-000000000001";

/// The backticked names in a serde "expected one of `A`, `B`" message.
fn variants(msg: &str) -> Vec<String> {
    let Some((_, list)) = msg.split_once("expected") else {
        return Vec::new();
    };
    list.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

fn backticked(msg: &str) -> Option<String> {
    msg.split('`').nth(1).map(str::to_owned)
}

fn get<'a>(root: &'a mut Value, path: &[String]) -> &'a mut Value {
    path.iter().fold(root, |v, k| match v {
        Value::Array(a) => &mut a[k.parse::<usize>().unwrap()],
        v => &mut v[k.as_str()],
    })
}

/// Candidate values for "expected `exp`", best first.
fn candidates(exp: &str) -> Vec<Value> {
    let e = exp.to_lowercase();
    let mut out = Vec::new();
    if let Some(n) = e
        .split("length ")
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse::<usize>().ok())
    {
        out.push(Value::Array(vec![json!(0); n]));
    }
    if e.contains("internally tagged") {
        out.push(json!({ "type": PROBE }));
    }
    if e.contains("variant") || e.contains("enum") || e.contains("single key") {
        out.push(json!(PROBE));
    }
    if e.contains("struct") || e.contains("map") {
        out.push(json!({}));
    }
    if e.contains("seq") || e.contains("array") || e.contains("tuple") || e.contains("vec") {
        out.push(json!([]));
    }
    if e.contains("bool") {
        out.push(json!(false));
    }
    if [
        "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "usize", "f32", "f64", "number",
        "integer", "float",
    ]
    .iter()
    .any(|t| e.contains(t))
    {
        out.push(json!(1));
    }
    if e.contains("str") || e.contains("ulid") {
        out.push(json!(ULID));
    }
    out.extend([
        json!(null),
        json!(ULID),
        json!(UUID),
        json!(""),
        json!("1"),
        json!(1),
        json!(false),
        json!([]),
        json!({}),
        json!(0.5),
    ]);
    out
}

/// Fill `root` (a command with `domain` and `command.type` set) until it deserializes.
fn complete(mut root: Value) -> Result<Command, String> {
    // Where the last change was made: the next error is about it (serde checks the present
    // fields of a struct before reporting its missing ones).
    let mut focus: Vec<String> = vec!["command".into()];
    let mut tries: std::collections::HashMap<Vec<String>, usize> = Default::default();
    for _ in 0..400 {
        let msg = match serde_json::from_value::<Command>(root.clone()) {
            Ok(c) => return Ok(c),
            Err(e) => e.to_string(),
        };
        if msg.starts_with("unknown variant") {
            let first = variants(&msg).into_iter().next().ok_or(msg.clone())?;
            let v = get(&mut root, &focus);
            match v {
                Value::Object(o) => {
                    o.insert("type".into(), json!(first));
                }
                v => *v = json!(first),
            }
        } else if msg.starts_with("missing field") {
            let field = backticked(&msg).ok_or(msg.clone())?;
            // The field belongs to the focused struct or one of its parents: the first whose
            // insertion changes the error.
            loop {
                let target = get(&mut root, &focus);
                if let Value::Object(o) = target
                    && !o.contains_key(&field)
                {
                    o.insert(field.clone(), Value::Null);
                    let now = serde_json::from_value::<Command>(root.clone())
                        .err()
                        .map(|e| e.to_string());
                    if now.as_deref() != Some(msg.as_str()) {
                        focus.push(field.clone());
                        break;
                    }
                    if let Value::Object(o) = get(&mut root, &focus) {
                        o.remove(&field);
                    }
                }
                if focus.pop().is_none() {
                    return Err(msg);
                }
            }
        } else {
            // A wrong value (serde's "invalid type …, expected …", or a custom parse error):
            // try the next candidate.
            let exp = msg.split_once("expected").map_or("", |(_, e)| e);
            let n = tries.entry(focus.clone()).or_default();
            let all = candidates(exp);
            let guess = all.get(*n).cloned().ok_or(msg.clone())?;
            *n += 1;
            match get(&mut root, &focus) {
                // An element of a filled array is wrong: fix them all.
                Value::Array(a) if !a.is_empty() && !msg.contains("length") => {
                    a.iter_mut().for_each(|x| *x = guess.clone())
                }
                v => *v = guess,
            }
            // Struct variants of externally tagged enums: `{ "Name": {} }`.
            if msg.contains("unit variant") && exp.contains("variant") {
                let v = get(&mut root, &focus);
                if let Value::String(name) = v.clone() {
                    *v = json!({ name.as_str(): {} });
                    focus.push(name);
                }
            }
        }
    }
    Err(format!("gave up on {root}"))
}

/// Debug-ish text: Rust identifiers (`SetVolume`, `set_volume`, `Foo::Bar`), `{:?}` output.
fn looks_debug(label: &str) -> bool {
    label.contains("::")
        || label.contains('{')
        || label.contains('}')
        || label.contains('_')
        || label.contains('"')
        || label.contains(ULID)
        || label.split_whitespace().any(|w| {
            let chars: Vec<char> = w.chars().collect();
            // camelCase / PascalCase: an uppercase letter after a lowercase one.
            chars
                .windows(2)
                .any(|p| p[0].is_lowercase() && p[1].is_uppercase())
        })
}

/// Lowercase, acronyms included ("midi", "lfo", "ai": no exceptions, docs/LOCALIZATION.md).
fn starts_lowercase(label: &str) -> bool {
    let first = label.split_whitespace().next().unwrap_or("");
    first.chars().all(|c| !c.is_uppercase())
}

fn every_command() -> Vec<Command> {
    let domain_err = serde_json::from_value::<Command>(json!({ "domain": PROBE, "command": null }))
        .unwrap_err()
        .to_string();
    let domains = variants(&domain_err);
    assert!(domains.len() > 40, "domains from {domain_err}");
    let mut out = Vec::new();
    for domain in domains {
        let err = serde_json::from_value::<Command>(
            json!({ "domain": domain, "command": { "type": PROBE } }),
        )
        .unwrap_err()
        .to_string();
        let names = variants(&err);
        assert!(!names.is_empty(), "{domain}: no variants in {err}");
        for name in names {
            let c = complete(json!({ "domain": domain, "command": { "type": name } }))
                .unwrap_or_else(|e| panic!("can't build {domain}::{name}: {e}"));
            out.push(c);
        }
    }
    out
}

#[test]
fn every_command_has_a_human_label() {
    let commands = every_command();
    assert!(
        commands.len() > 300,
        "only {} commands built",
        commands.len()
    );
    for c in &commands {
        let full = label_of(c, None);
        let label = full.text.clone();
        // `Batch` carries its caller's label.
        if matches!(
            c,
            Command::Edit(ether_core::protocol::project::EditCommand::Batch { .. })
        ) {
            continue;
        }
        // Every label is a catalog message (docs/LOCALIZATION.md), its text the catalog's.
        let key = full
            .key
            .as_ref()
            .unwrap_or_else(|| panic!("{c:?}: label {label:?} has no message id"));
        assert!(
            crate::text::exists(&key.id),
            "{c:?}: {} isn't in ui/src/i18n/locales/en.json",
            key.id
        );
        assert_eq!(crate::text::english(key), label, "{c:?}");
        assert!(!label.trim().is_empty(), "{c:?}: empty label");
        assert_eq!(
            label.trim(),
            label,
            "{c:?}: label {label:?} has outer spaces"
        );
        assert!(label.len() <= 40, "{c:?}: label {label:?} is long");
        assert!(
            !looks_debug(&label),
            "{c:?}: label {label:?} looks like debug text"
        );
        assert!(
            starts_lowercase(&label),
            "{c:?}: label {label:?} isn't lowercase"
        );
    }
}

#[test]
fn labels_count_and_name_targets() {
    use ether_core::protocol::clips::{ClipCommand, ClipMove};
    use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
    use ether_core::protocol::mixer::MixerCommand;
    use ether_core::protocol::model::*;

    let mv = |_: usize| ClipMove {
        id: ClipId::NIL,
        track: TrackId::NIL,
        start: Beats::ZERO,
    };
    let moves = |n: usize| {
        Command::Clip(ClipCommand::Move {
            moves: (0..n).map(mv).collect(),
        })
    };
    assert_eq!(label_of(&moves(1), None), "move clip");
    assert_eq!(label_of(&moves(3), None), "move 3 clips");
    assert_eq!(
        label_of(
            &Command::Mixer(MixerCommand::SetVolume {
                track: TrackId::NIL,
                volume: Decibels(-6.0),
            }),
            None
        ),
        "set volume"
    );
    let reverb = BuiltinDevice::new(BuiltinDeviceType::Reverb);
    let insert = Command::Device(DeviceCommand::Insert {
        id: DeviceId::NIL,
        track: TrackId::NIL,
        device: DeviceSpec::Builtin { device: reverb },
        before: None,
    });
    assert_eq!(label_of(&insert, None), "add reverb");
}

#[test]
fn labels_keep_names_as_arguments() {
    use ether_core::protocol::devices::DeviceCommand;
    use ether_core::protocol::model::text::{TextArg, TextKey};
    use ether_core::protocol::model::*;

    // A device the project doesn't name is "device", through the catalog (translated).
    let remove = label_of(
        &Command::Device(DeviceCommand::Remove { id: DeviceId::NIL }),
        None,
    );
    assert_eq!(remove.text, "remove device");
    let key = remove.key.unwrap();
    assert_eq!(key.id, "undo.device.remove");
    assert_eq!(
        key.args.get("device"),
        Some(&TextArg::Key(Box::new(TextKey::new("undo.noun.device"))))
    );
    // Counts are arguments of plural messages.
    let mv = label_of(
        &Command::Clip(ether_core::protocol::clips::ClipCommand::Delete {
            ids: vec![ClipId::NIL, ClipId::NIL],
        }),
        None,
    );
    assert_eq!(mv.text, "delete 2 clips");
    assert_eq!(
        mv.key,
        Some(TextKey::new("undo.clip.delete").arg("count", 2usize))
    );
}

#[test]
fn ui_batch_labels_are_keyed_when_the_catalog_has_them() {
    use super::UndoLabel;
    use ether_core::protocol::model::text::TextKey;
    // A known English label gets its message; free text stays as is.
    assert_eq!(
        UndoLabel::batch("set volume", None).key,
        Some(TextKey::new("undo.mixer.set_volume"))
    );
    assert_eq!(UndoLabel::batch("paste 3 clips", None).key, None);
    // A key the catalog has wins; an unknown one is dropped.
    let k = TextKey::new("undo.clip.move").arg("count", 3usize);
    assert_eq!(UndoLabel::batch("move 3 clips", Some(&k)).key, Some(k));
    assert_eq!(
        UndoLabel::batch("x", Some(&TextKey::new("undo.nope"))).key,
        None
    );
}
