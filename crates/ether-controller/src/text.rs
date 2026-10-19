//! The English text of catalog messages (docs/LOCALIZATION.md). The engine sends a
//! [`TextKey`] (message id + arguments) for the UI to show in the user's language, and the
//! English text for agents, logs and older clients. The English comes from the UI's own
//! catalog (`undo.*`, `notice.*`, `error.*` in `ui/src/i18n/locales/en.json`, compiled in by build.rs), formatted by
//! the small ICU subset those messages use (`ether_model::text::format`).

use std::collections::HashMap;
use std::sync::OnceLock;

use ether_core::protocol::model::text::{TextArg, TextKey};

/// `(id, English message)` of every engine message (`undo.*`, `notice.*`, `error.*`).
static EN: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/en_undo.rs"));

fn messages() -> &'static HashMap<&'static str, &'static str> {
    static M: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    M.get_or_init(|| EN.iter().copied().collect())
}

/// English text → id of the messages without arguments.
fn plain() -> &'static HashMap<&'static str, &'static str> {
    static P: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    P.get_or_init(|| {
        EN.iter()
            .filter(|(id, m)| id.starts_with("undo.") && !m.contains(['{', '\'']))
            .map(|&(id, m)| (m, id))
            .collect()
    })
}

/// Catalog message `id` (add its arguments with `.arg(name, value)`). Write `id` as a full
/// literal: the UI's unused-key check and `tests::ids_exist` read the sources.
pub fn msg(id: &str) -> TextKey {
    TextKey::new(id)
}

/// A user-facing message (a notification, an error's text): its English, and its catalog
/// message when it has one (plain text from deeper layers has none).
#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub text: String,
    pub key: Option<TextKey>,
}

impl std::fmt::Display for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl From<String> for Msg {
    fn from(text: String) -> Self {
        Self { text, key: None }
    }
}

impl From<&str> for Msg {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl From<TextKey> for Msg {
    fn from(key: TextKey) -> Self {
        Self {
            text: english(&key),
            key: Some(key),
        }
    }
}

impl From<Msg> for TextArg {
    /// As an argument of another message: the nested message, else its text.
    fn from(m: Msg) -> Self {
        match m.key {
            Some(k) => TextArg::Key(Box::new(k)),
            None => TextArg::Str(m.text),
        }
    }
}

/// An error's text as an argument of another message ("autosave failed: {error}"): its
/// message when it has one, else its English.
pub fn error_arg(e: &ether_core::protocol::CommandError) -> TextArg {
    match &e.key {
        Some(k) => TextArg::Key(Box::new(k.clone())),
        None => TextArg::Str(e.message.clone()),
    }
}

/// A built-in device's display text as the engine names it ("Cutoff", "Macro 3", "Operator
/// B": a descriptor's) as an argument: its catalog message (`engine.cutoff`, `engine.macro_n`
/// with `{n}`; the keys `ui/src/i18n/tooling/engineText.ts` gives), so the UI shows it in the
/// user's language, else the text in lowercase.
pub fn engine_arg(text: &str) -> TextArg {
    let key = engine_key(text);
    if exists(&key.id) {
        TextArg::Key(Box::new(key))
    } else {
        TextArg::Str(text.trim().to_lowercase())
    }
}

/// The `engine.*` message of a built-in's text (whether the catalog has it or not).
fn engine_key(text: &str) -> TextKey {
    fn slug(s: &str) -> String {
        let mut out = String::new();
        for c in s.to_lowercase().replace('&', " and ").chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c);
            } else if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
        }
        out.trim_end_matches('_').to_owned()
    }
    // "Step 12", "Operator B" (not "Invert L": a side).
    let numbered = text.rsplit_once(' ').filter(|(base, n)| {
        base.chars().any(char::is_alphabetic)
            && (n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty()
                || n.len() == 1 && ('A'..='H').contains(&n.chars().next().unwrap_or(' ')))
    });
    match numbered {
        Some((base, n)) => {
            let key = TextKey::new(format!("engine.{}_n", slug(base)));
            match n.parse::<i64>() {
                Ok(i) => key.arg("n", i),
                Err(_) => key.arg("n", n),
            }
        }
        None => TextKey::new(format!("engine.{}", slug(text))),
    }
}

/// Per language (the UI's catalogs): the default names (`name.*`) and the built-in devices'
/// text (`engine.*`).
static NAMES: &[(&str, &[(&str, &str)])] = include!(concat!(env!("OUT_DIR"), "/names.rs"));

fn names_table(code: &str) -> Option<&'static HashMap<&'static str, &'static str>> {
    static T: OnceLock<HashMap<&'static str, HashMap<&'static str, &'static str>>> =
        OnceLock::new();
    T.get_or_init(|| {
        NAMES
            .iter()
            .map(|(code, entries)| (*code, entries.iter().copied().collect()))
            .collect()
    })
    .get(code)
}

/// The language new tracks, devices, chains and markers are named in (the UI's, set with
/// `Settings::SetLanguage`; English until then). Process-wide: one engine, one user.
static NAMES_LANGUAGE: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// A catalog code the names tables have, else English.
fn known_language(code: &str) -> &str {
    if names_table(code).is_some() {
        code
    } else {
        "en"
    }
}

/// Sets the default names' language (a catalog code, "fr"; an unknown one means English).
pub fn set_names_language(code: &str) {
    if let Ok(mut l) = NAMES_LANGUAGE.write() {
        known_language(code).clone_into(&mut l);
    }
}

fn names_language() -> String {
    NAMES_LANGUAGE
        .read()
        .map(|l| {
            if l.is_empty() {
                "en".to_owned()
            } else {
                l.clone()
            }
        })
        .unwrap_or_else(|_| "en".to_owned())
}

/// A default name: `key` (a `name.*` message) in the names language, else in English.
pub fn default_name(key: &TextKey) -> String {
    default_name_in(&names_language(), key)
}

fn default_name_in(lang: &str, key: &TextKey) -> String {
    let message = names_table(lang)
        .and_then(|t| t.get(key.id.as_str()))
        .or_else(|| names_table("en").and_then(|t| t.get(key.id.as_str())));
    match message {
        Some(m) => format(m, &key.args),
        None => key.id.clone(),
    }
}

/// A new built-in device's name: its descriptor's name ("Reverb") in the names language
/// (« réverbération »); in English, lowercase as it always was ("reverb", "eq").
pub fn device_default_name(descriptor_name: &str) -> String {
    device_default_name_in(&names_language(), descriptor_name)
}

fn device_default_name_in(lang: &str, descriptor_name: &str) -> String {
    if lang != "en" {
        let key = engine_key(descriptor_name);
        if let Some(m) = names_table(lang).and_then(|t| t.get(key.id.as_str())) {
            return format(m, &key.args);
        }
    }
    descriptor_name.trim().to_lowercase()
}

/// Is `id` a message of the catalog?
pub fn exists(id: &str) -> bool {
    messages().contains_key(id)
}

/// The English text of `key` (its id when the catalog doesn't have it).
pub fn english(key: &TextKey) -> String {
    match messages().get(key.id.as_str()) {
        Some(m) => format(m, &key.args),
        None => key.id.clone(),
    }
}

/// The message whose English text is exactly `text`, for a message without arguments
/// ("capture midi" → `undo.capture.capture`).
pub fn key_of_english(text: &str) -> Option<TextKey> {
    plain().get(text).map(|id| TextKey::new(*id))
}

/// `message` (ICU, the subset above) with `args`, nested messages in English.
fn format(message: &str, args: &std::collections::BTreeMap<String, TextArg>) -> String {
    ether_core::protocol::model::text::format(message, args, &english)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(message: &str, key: TextKey) -> String {
        format(message, &key.args)
    }

    #[test]
    fn default_names_follow_the_language() {
        // The `_in` forms: other tests run alongside on the process-wide language.
        let group = |lang: &str| default_name_in(lang, &msg("name.track.group").arg("n", 2i64));
        assert_eq!(group("en"), "2 group");
        assert_eq!(device_default_name_in("en", "Reverb"), "reverb");
        assert_eq!(device_default_name_in("en", "EQ"), "eq");
        assert_eq!(group("fr"), "2 groupe");
        assert_eq!(device_default_name_in("fr", "Reverb"), "réverbération");
        assert_eq!(
            default_name_in("fr", &msg("name.rack.chain").arg("n", 3i64)),
            "chaîne 3"
        );
        assert_eq!(known_language("xx"), "en");
        assert_eq!(known_language("fr"), "fr");
        // Every `name.*` message has its French.
        let en = names_table("en").unwrap();
        let fr = names_table("fr").unwrap();
        for id in en.keys().filter(|id| id.starts_with("name.")) {
            assert!(fr.contains_key(id), "{id}");
        }
    }

    #[test]
    fn engine_text_is_keyed_like_the_ui() {
        let k = |t: &str| match engine_arg(t) {
            TextArg::Key(k) => (k.id.clone(), english(&k)),
            other => panic!("{t}: not keyed: {other:?}"),
        };
        assert_eq!(k("Cutoff"), ("engine.cutoff".into(), "cutoff".into()));
        assert_eq!(
            k("Attack Time"),
            ("engine.attack_time".into(), "attack time".into())
        );
        assert_eq!(k("Macro 3"), ("engine.macro_n".into(), "macro 3".into()));
        assert_eq!(
            k("Operator B"),
            ("engine.operator_n".into(), "operator B".into())
        );
        assert_eq!(
            k("Sample & Hold"),
            ("engine.sample_and_hold".into(), "sample & hold".into())
        );
        assert_eq!(
            k("Invert L"),
            ("engine.invert_l".into(), "invert left".into())
        );
        assert_eq!(k("EQ"), ("engine.eq".into(), "eq".into()));
        // Not a built-in's: the lowercase text.
        assert_eq!(engine_arg("Air Width"), TextArg::Str("air width".into()));
    }

    #[test]
    fn every_builtin_text_is_keyed() {
        let mut texts = Vec::new();
        for d in ether_devices::all_descriptors() {
            texts.push(d.name.clone());
            texts.extend(d.params.iter().map(|p| p.name.clone()));
        }
        let missing: Vec<_> = texts
            .iter()
            // Words (not "Q": `tests/engine_text.rs` lists those only).
            .filter(|t| {
                t.as_bytes()
                    .windows(2)
                    .any(|w| w.iter().all(u8::is_ascii_alphabetic))
            })
            .filter(|t| !matches!(engine_arg(t), TextArg::Key(_)))
            .collect();
        assert!(
            missing.is_empty(),
            "not in en.json's engine.* keys (`pnpm i18n gen`): {missing:?}"
        );
    }

    #[test]
    fn formats_the_icu_subset() {
        let k = |n: usize| TextKey::new("x").arg("count", n);
        let plural = "{count, plural, one {move clip} other {move # clips}}";
        assert_eq!(fmt(plural, k(1)), "move clip");
        assert_eq!(fmt(plural, k(3)), "move 3 clips");
        assert_eq!(fmt("{count, plural, =0 {none} other {#}}", k(0)), "none");
        let named = TextKey::new("x").arg("device", "reverb");
        assert_eq!(fmt("remove {device}", named), "remove reverb");
        let select = "add {kind, select, audio {audio track} midi {midi track} other {vca}}";
        assert_eq!(
            fmt(select, TextKey::new("x").arg("kind", "midi")),
            "add midi track"
        );
        assert_eq!(fmt(select, TextKey::new("x").arg("kind", "vca")), "add vca");
        assert_eq!(fmt("{missing} left", TextKey::new("x")), "{missing} left");
    }

    #[test]
    fn english_from_the_catalog_and_back() {
        let k = TextKey::new("undo.clip.move").arg("count", 2usize);
        assert_eq!(english(&k), "move 2 clips");
        assert_eq!(
            key_of_english("set volume"),
            Some(TextKey::new("undo.mixer.set_volume"))
        );
        assert_eq!(key_of_english("not a label"), None);
        // Nested messages are formatted in English too.
        let ai = TextKey::new("undo.agent.edit").arg("edit", TextKey::new("undo.mixer.set_volume"));
        assert_eq!(english(&ai), "ai: set volume");
        assert_eq!(english(&TextKey::new("undo.no_such")), "undo.no_such");
    }

    #[test]
    fn notices_and_errors_read_as_before() {
        let freeze = |sends: usize, sidechains: usize| {
            english(
                &msg("notice.freeze.silenced")
                    .arg("group", "drums")
                    .arg("sends", sends)
                    .arg("sidechains", sidechains),
            )
        };
        assert_eq!(
            freeze(1, 2),
            "freezing \"drums\": 1 send from tracks in the group will be silent; 2 sidechain \
             inputs fed by tracks in the group will hear silence while it is frozen."
        );
        assert_eq!(
            freeze(2, 0),
            "freezing \"drums\": 2 sends from tracks in the group will be silent while it is frozen."
        );
        let autosave = msg("notice.project.autosave_failed").arg(
            "error",
            error_arg(&crate::tx::keyed_err(
                ether_core::protocol::ErrorCode::InvalidState,
                msg("error.project.none_open"),
            )),
        );
        assert_eq!(english(&autosave), "autosave failed: no project is open");
    }

    /// Every `notice.*` / `error.*` id the controller and the hosts name is in the catalog
    /// (a typo would show the id).
    #[test]
    fn ids_exist() {
        let id = regex::Regex::new(r#""((?:notice|error)\.[a-z0-9_.]+)""#).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut stack: Vec<_> = ["src", "../ether-native/src", "../ether-wasm/src"]
            .iter()
            .map(|d| root.join(d))
            .collect();
        let mut seen = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for c in id.captures_iter(&text) {
                        seen += 1;
                        assert!(
                            exists(&c[1]),
                            "{}: unknown message id {}",
                            path.display(),
                            &c[1]
                        );
                    }
                }
            }
        }
        assert!(seen > 50, "found only {seen} ids");
    }
}
