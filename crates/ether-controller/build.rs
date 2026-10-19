//! The engine's messages from the UI's catalogs (`ui/src/i18n/locales/*.json`, the one
//! source of their text, docs/LOCALIZATION.md), compiled in as tables:
//! - `en_undo.rs`: the English of `undo.*`, `notice.*`, `error.*` and the built-in devices'
//!   text `engine.*`: the controller gives each undo label, notification and error in English
//!   (agents, logs) next to its message id;
//! - `names.rs`: per language, the default names (`name.*`) and the built-in devices' text
//!   (`engine.*`) the engine names new tracks, devices, chains and markers with, in the
//!   language the UI sets (`Settings::SetLanguage`).
//!
//! `ETHER_EXTRA_LOCALES` (a directory of `<code>.json` catalogs): an extension's messages,
//! merged in (its undo labels, notices and errors; docs/EXTENSIONS.md).

use std::collections::BTreeMap;
use std::path::Path;

/// The catalog areas the engine sends.
const ENGINE_PREFIXES: [&str; 5] = ["undo.", "notice.", "error.", "engine.", "name."];
/// The catalog areas of default names.
const NAME_PREFIXES: [&str; 2] = ["name.", "engine."];

/// `<dir>/<code>.json`, merged with the extension's catalog of that language, if any.
fn catalog(dir: &Path, extra: Option<&Path>, code: &str) -> BTreeMap<String, String> {
    let mut messages = read(&dir.join(format!("{code}.json")));
    if let Some(extra) = extra {
        let path = extra.join(format!("{code}.json"));
        if path.exists() {
            messages.extend(read(&path));
        }
    }
    messages
}

fn read(path: &Path) -> BTreeMap<String, String> {
    println!("cargo:rerun-if-changed={}", path.display());
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn table(messages: &BTreeMap<String, String>, prefixes: &[&str]) -> String {
    let mut out = String::from("&[\n");
    for (id, message) in messages
        .iter()
        .filter(|(id, _)| prefixes.iter().any(|p| id.starts_with(p)))
    {
        out.push_str(&format!("    ({id:?}, {message:?}),\n"));
    }
    out.push(']');
    out
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let dir = Path::new(&manifest).join("../../ui/src/i18n/locales");
    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-env-changed=ETHER_EXTRA_LOCALES");
    let extra = std::env::var_os("ETHER_EXTRA_LOCALES").map(std::path::PathBuf::from);
    if let Some(extra) = &extra {
        println!("cargo:rerun-if-changed={}", extra.display());
    }
    let extra = extra.as_deref();
    let en = catalog(&dir, extra, "en");
    std::fs::write(
        Path::new(&out_dir).join("en_undo.rs"),
        format!("{}\n", table(&en, &ENGINE_PREFIXES)),
    )
    .expect("writing en_undo.rs");

    let mut codes: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    codes.sort();
    let mut names = String::from("&[\n");
    for code in codes {
        let messages = catalog(&dir, extra, &code);
        names.push_str(&format!(
            "({code:?}, {}),\n",
            table(&messages, &NAME_PREFIXES)
        ));
    }
    names.push_str("]\n");
    std::fs::write(Path::new(&out_dir).join("names.rs"), names).expect("writing names.rs");
}
