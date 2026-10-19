//! Extension domains (docs/EXTENSIONS.md): commands, events and reply values this crate doesn't
//! declare, for features built outside this repository. On the wire they look like the app's
//! own (`{ "domain": "<name>", "command": { … } }`, `{ "type": "<name>", … }`); a message
//! whose domain or type this crate doesn't declare deserializes to the `Ext` variant, which
//! keeps it as JSON. A message of a declared domain or type always parses as the app's (its
//! errors are the app's). An extension parses the JSON with its own types and answers with
//! its own values; without an extension the engine replies `Unsupported`.
//!
//! The app's TypeScript types leave the `Ext` variants out (`#[ts(skip)]`): an extension
//! generates its own and joins them to the app's.
use std::sync::OnceLock;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::message::{Command, Event, ReplyValue};

/// A command of an extension domain: `{ "domain": "…", "command": … }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtCommand {
    pub domain: String,
    #[serde(default)]
    pub command: Value,
}

impl ExtCommand {
    /// The command as its extension's own type (an adjacently tagged enum over `domain` /
    /// `command`, like [`Command`]).
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_value(serde_json::json!({ "domain": self.domain, "command": self.command }))
    }

    /// An extension's command (an enum tagged like [`Command`]).
    pub fn of<T: Serialize>(command: &T) -> Result<Self, serde_json::Error> {
        serde_json::from_value(serde_json::to_value(command)?)
    }
}

/// An event or a reply value of an extension: a JSON object with its `type`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExtValue(pub Value);

impl ExtValue {
    /// An extension's event or reply value (an enum tagged by `type`, like [`Event`]).
    pub fn of<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        serde_json::to_value(value).map(Self)
    }

    /// The `type` tag.
    pub fn kind(&self) -> Option<&str> {
        self.0.get("type").and_then(Value::as_str)
    }

    /// The value as its extension's own type.
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        T::deserialize(&self.0)
    }
}

/// A tag value no enum declares: its parse error lists the declared ones.
const PROBE: &str = "\u{0}ext-probe";

/// The variant names serde lists in an "unknown variant `x`, expected one of `a`, `b`" error.
fn declared(error: &str) -> Vec<String> {
    let Some((_, list)) = error.split_once("expected") else {
        return Vec::new();
    };
    list.split('`').skip(1).step_by(2).map(str::to_owned).collect()
}

/// The domains [`Command`] declares (its `domain` tags).
pub fn command_domains() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let probe = serde_json::json!({ "domain": PROBE, "command": null });
        declared(&Command::deserialize(probe).expect_err("a probe domain").to_string())
    })
}

/// The `type`s [`Event`] declares.
pub fn event_types() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let probe = serde_json::json!({ "type": PROBE });
        declared(&Event::deserialize(probe).expect_err("a probe type").to_string())
    })
}

/// The `type`s [`ReplyValue`] declares.
pub fn reply_types() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let probe = serde_json::json!({ "type": PROBE });
        declared(&ReplyValue::deserialize(probe).expect_err("a probe type").to_string())
    })
}

/// `true` when `value`'s `tag` names something `declared` doesn't list (an extension's).
fn is_ext(value: &Value, tag: &str, declared: &[String]) -> bool {
    value
        .get(tag)
        .and_then(Value::as_str)
        .is_some_and(|t| !declared.iter().any(|d| d == t))
}

impl Serialize for Command {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Command::Ext(c) => c.serialize(s),
            c => Command::serialize(c, s),
        }
    }
}

impl<'de> Deserialize<'de> for Command {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        if is_ext(&value, "domain", command_domains()) {
            return ExtCommand::deserialize(value).map(Command::Ext).map_err(D::Error::custom);
        }
        Command::deserialize(value).map_err(D::Error::custom)
    }
}

impl Serialize for Event {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Event::Ext(v) => v.serialize(s),
            e => Event::serialize(e, s),
        }
    }
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        if is_ext(&value, "type", event_types()) {
            return Ok(Event::Ext(ExtValue(value)));
        }
        Event::deserialize(value).map_err(D::Error::custom)
    }
}

impl Serialize for ReplyValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ReplyValue::Ext(v) => v.serialize(s),
            r => ReplyValue::serialize(r, s),
        }
    }
}

impl<'de> Deserialize<'de> for ReplyValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        if is_ext(&value, "type", reply_types()) {
            return Ok(ReplyValue::Ext(ExtValue(value)));
        }
        ReplyValue::deserialize(value).map_err(D::Error::custom)
    }
}
