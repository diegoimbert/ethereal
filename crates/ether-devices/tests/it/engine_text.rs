//! The built-in devices' display text (docs/LOCALIZATION.md "Text from the engine"): device,
//! modulator, parameter, group and choice names and the panels' section titles and captions.
//! The engine keeps them in English (agents, MCP, logs); the UI shows each one through its
//! catalog (`engine.*` keys, `ui/src/i18n/engineText.ts`), in lowercase English or the user's
//! language. This test keeps the list the UI reads, `ui/src/i18n/engine-text.json`, in sync:
//! run `UPDATE_ENGINE_TEXT=1 cargo test -p ether-devices --test engine_text`, then
//! `pnpm i18n gen` (which adds the new `engine.*` keys to `en.json`) and translate them.

use std::collections::BTreeSet;

use ether_core::protocol::devices::ParamInfo;
use ether_core::protocol::layout::DeviceLayout;

/// Words, not a number or a notation ("1/16", "1/8 T", "Q", "L"): at least two letters in a row.
fn is_text(s: &str) -> bool {
    let mut run = 0;
    for c in s.chars() {
        run = if c.is_alphabetic() { run + 1 } else { 0 };
        if run >= 2 {
            return true;
        }
    }
    false
}

fn params(out: &mut BTreeSet<String>, params: &[ParamInfo]) {
    for p in params {
        out.insert(p.name.clone());
        out.extend(p.group.clone());
        out.extend(p.labels.iter().flatten().cloned());
    }
}

fn layout(out: &mut BTreeSet<String>, layout: Option<&DeviceLayout>) {
    for s in layout.map(|l| l.sections.as_slice()).unwrap_or_default() {
        out.extend(s.title.clone());
        out.extend(s.items.iter().filter_map(|i| i.label.clone()));
    }
}

/// Every display string of the built-in devices and modulators, sorted.
fn engine_text() -> Vec<String> {
    let mut out = BTreeSet::new();
    for d in ether_devices::all_descriptors() {
        out.insert(d.name.clone());
        params(&mut out, &d.params);
        layout(&mut out, d.layout.as_ref());
    }
    for m in ether_devices::modulators::all() {
        out.insert(m.name.clone());
        params(&mut out, &m.params);
        layout(&mut out, m.layout.as_ref());
    }
    out.into_iter().filter(|s| is_text(s)).collect()
}

#[test]
fn ui_engine_text_list_is_current() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/i18n/engine-text.json");
    let expected = format!(
        "{}\n",
        serde_json::to_string_pretty(&engine_text()).unwrap()
    );
    if std::env::var_os("UPDATE_ENGINE_TEXT").is_some() {
        std::fs::write(&path, &expected).unwrap();
        return;
    }
    let found = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        found == expected,
        "{} is stale: run `UPDATE_ENGINE_TEXT=1 cargo test -p ether-devices --test engine_text`, \
         then `pnpm i18n gen` and translate the new `engine.*` keys",
        path.display()
    );
}
