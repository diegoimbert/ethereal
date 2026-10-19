//! Localizable text (docs/LOCALIZATION.md): a message id of the UI's catalogs
//! (`ui/src/i18n/locales/<lang>.json`) with its arguments. The engine sends these next to
//! its English text (undo labels: [`crate::op::Transaction::key`]); the UI shows the
//! message in the user's language, and agents, logs and older clients read the English.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A message id (`undo.clip.move`) and its arguments (`{ count: 3 }`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TextKey {
    pub id: String,
    #[serde(default)]
    pub args: BTreeMap<String, TextArg>,
}

/// One argument: a number (plurals), a name (user content, shown as is), or another
/// message (formatted first, in the same language).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum TextArg {
    Int(#[ts(type = "number")] i64),
    Str(String),
    Key(Box<TextKey>),
}

impl TextKey {
    /// A message without arguments.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            args: BTreeMap::new(),
        }
    }

    /// Adds argument `name`.
    pub fn arg(mut self, name: &str, value: impl Into<TextArg>) -> Self {
        self.args.insert(name.to_owned(), value.into());
        self
    }
}

impl From<i64> for TextArg {
    fn from(n: i64) -> Self {
        TextArg::Int(n)
    }
}

impl From<usize> for TextArg {
    fn from(n: usize) -> Self {
        TextArg::Int(i64::try_from(n).unwrap_or(i64::MAX))
    }
}

macro_rules! int_arg {
    ($($t:ty),*) => {$(
        impl From<$t> for TextArg {
            fn from(n: $t) -> Self {
                TextArg::Int(i64::from(n))
            }
        }
    )*};
}
int_arg!(u8, u16, u32, i32);

impl From<u64> for TextArg {
    fn from(n: u64) -> Self {
        TextArg::Int(i64::try_from(n).unwrap_or(i64::MAX))
    }
}

impl From<String> for TextArg {
    fn from(s: String) -> Self {
        TextArg::Str(s)
    }
}

impl From<&str> for TextArg {
    fn from(s: &str) -> Self {
        TextArg::Str(s.to_owned())
    }
}

impl From<TextKey> for TextArg {
    fn from(k: TextKey) -> Self {
        TextArg::Key(Box::new(k))
    }
}

/// Formats catalog `message` with `args`: the small ICU subset the engine's messages use,
/// `{name}`, `{n, plural, one {…} other {# …}}` (and `=N`), `{x, select, a {…} other {…}}`.
/// `nested` gives the text of an argument that is itself a message.
pub fn format(
    message: &str,
    args: &BTreeMap<String, TextArg>,
    nested: &dyn Fn(&TextKey) -> String,
) -> String {
    format_in(message, args, None, nested)
}

fn arg_text(arg: &TextArg, nested: &dyn Fn(&TextKey) -> String) -> String {
    match arg {
        TextArg::Int(n) => n.to_string(),
        TextArg::Str(s) => s.clone(),
        TextArg::Key(k) => nested(k),
    }
}

/// The index just past the `}` closing the `{` at `open`.
fn close_of(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// `sel {text} sel {text}` → `[(sel, text)]`.
fn options(s: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim();
        let Some(end) = close_of(rest, open) else {
            break;
        };
        out.push((selector, &rest[open + 1..end - 1]));
        rest = rest[end..].trim_start();
    }
    out
}

fn format_in(
    message: &str,
    args: &BTreeMap<String, TextArg>,
    pound: Option<i64>,
    nested: &dyn Fn(&TextKey) -> String,
) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < message.len() {
        let c = message[i..].chars().next().expect("in bounds");
        match c {
            '{' => {
                let Some(end) = close_of(message, i) else {
                    out.push_str(&message[i..]);
                    break;
                };
                let inner = &message[i + 1..end - 1];
                let mut parts = inner.splitn(3, ',');
                let name = parts.next().unwrap_or("").trim();
                let kind = parts.next().map(str::trim);
                let arg = args.get(name);
                match (kind, parts.next()) {
                    (Some("plural"), Some(opts)) => {
                        let n = match arg {
                            Some(TextArg::Int(n)) => *n,
                            _ => 0,
                        };
                        let opts = options(opts);
                        let exact = format!("={n}");
                        let pick = opts
                            .iter()
                            .find(|(s, _)| *s == exact)
                            .or_else(|| {
                                opts.iter()
                                    .find(|(s, _)| *s == if n == 1 { "one" } else { "other" })
                            })
                            .or_else(|| opts.iter().find(|(s, _)| *s == "other"));
                        if let Some((_, text)) = pick {
                            out.push_str(&format_in(text, args, Some(n), nested));
                        }
                    }
                    (Some("select"), Some(opts)) => {
                        let value = arg.map(|a| arg_text(a, nested)).unwrap_or_default();
                        let opts = options(opts);
                        let pick = opts
                            .iter()
                            .find(|(s, _)| *s == value)
                            .or_else(|| opts.iter().find(|(s, _)| *s == "other"));
                        if let Some((_, text)) = pick {
                            out.push_str(&format_in(text, args, pound, nested));
                        }
                    }
                    _ => out.push_str(
                        &arg.map(|a| arg_text(a, nested))
                            .unwrap_or_else(|| format!("{{{name}}}")),
                    ),
                }
                i = end;
            }
            '#' if pound.is_some() => {
                out.push_str(&pound.expect("checked").to_string());
                i += 1;
            }
            c => {
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_compactly() {
        let k = TextKey::new("undo.clip.move").arg("count", 3usize);
        assert_eq!(
            serde_json::to_string(&k).unwrap(),
            r#"{"id":"undo.clip.move","args":{"count":3}}"#
        );
        let nested = TextKey::new("undo.ai_edit").arg("edit", TextKey::new("undo.track.delete"));
        let json = serde_json::to_string(&nested).unwrap();
        assert_eq!(
            json,
            r#"{"id":"undo.ai_edit","args":{"edit":{"id":"undo.track.delete","args":{}}}}"#
        );
        assert_eq!(serde_json::from_str::<TextKey>(&json).unwrap(), nested);
        let named = TextKey::new("undo.device.remove").arg("device", "reverb");
        assert_eq!(
            serde_json::from_str::<TextKey>(&serde_json::to_string(&named).unwrap()).unwrap(),
            named
        );
    }
}
