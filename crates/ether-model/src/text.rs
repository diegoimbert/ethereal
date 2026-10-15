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
