//! A small JSON Schema subset validator (the schemas of [`super::tools`] only use this
//! subset) plus typed accessors over a validated input object.
//!
//! Supported keywords: `type` (a name or a list of names; `integer` = a number without a
//! fraction), `properties`, `required`, `additionalProperties: false`, `items`, `enum`,
//! `minimum`, `maximum`, `minItems`, `maxItems`, `minLength`. Anything else is ignored
//! (descriptions, defaults).

use std::str::FromStr;

use serde_json::{Map, Value};

/// A tool input error, shown to the model as an `is_error` result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InputError(pub String);

impl InputError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

fn type_matches(ty: &str, v: &Value) -> bool {
    match ty {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "number" => v.as_f64().is_some_and(f64::is_finite),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => true,
    }
}

fn describe(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Validate `v` against `schema`; `path` names `v` in errors (`input.notes[3].pitch`).
pub(crate) fn validate(schema: &Value, v: &Value, path: &str) -> Result<(), InputError> {
    let fail = |m: String| Err(InputError(format!("{path}: {m}")));
    if let Some(ty) = schema.get("type") {
        let ok = match ty {
            Value::String(t) => type_matches(t, v),
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).any(|t| type_matches(t, v)),
            _ => true,
        };
        if !ok {
            let want = match ty {
                Value::String(t) => t.clone(),
                Value::Array(ts) => ts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" or "),
                _ => String::new(),
            };
            return fail(format!("expected {want}, got {}", describe(v)));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array)
        && !options.contains(v)
    {
        let list: Vec<String> = options.iter().map(Value::to_string).collect();
        return fail(format!("must be one of {}", list.join(", ")));
    }
    if let Some(x) = v.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64)
            && x < min
        {
            return fail(format!("must be >= {min}, got {x}"));
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64)
            && x > max
        {
            return fail(format!("must be <= {max}, got {x}"));
        }
    }
    if let (Some(s), Some(min)) = (v.as_str(), schema.get("minLength").and_then(Value::as_u64))
        && (s.chars().count() as u64) < min
    {
        return fail(format!("must have at least {min} characters"));
    }
    if let Some(items) = v.as_array() {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64)
            && (items.len() as u64) < min
        {
            return fail(format!("needs at least {min} items"));
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64)
            && (items.len() as u64) > max
        {
            return fail(format!(
                "at most {max} items per call (got {}); split the work into several calls",
                items.len()
            ));
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                validate(item_schema, item, &format!("{path}[{i}]"))?;
            }
        }
    }
    if let Some(obj) = v.as_object() {
        let props = schema.get("properties").and_then(Value::as_object);
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for r in required.iter().filter_map(Value::as_str) {
                if obj.get(r).is_none_or(Value::is_null) {
                    return fail(format!("missing required property `{r}`"));
                }
            }
        }
        let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
        for (k, value) in obj {
            match props.and_then(|p| p.get(k)) {
                Some(s) => {
                    // `null` = omitted for optional properties (models often send it).
                    if !value.is_null() {
                        validate(s, value, &format!("{path}.{k}"))?;
                    }
                }
                None if closed => {
                    let known: Vec<&str> = props
                        .map(|p| p.keys().map(String::as_str).collect())
                        .unwrap_or_default();
                    return fail(format!(
                        "unknown property `{k}` (allowed: {})",
                        if known.is_empty() {
                            "none".to_string()
                        } else {
                            known.join(", ")
                        }
                    ));
                }
                None => {}
            }
        }
    }
    Ok(())
}

/// Typed access to a validated input object (`null` reads as absent).
#[derive(Clone, Copy)]
pub(crate) struct Args<'a>(pub &'a Map<String, Value>);

impl<'a> Args<'a> {
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn str(&self, key: &str) -> Option<&'a str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn req_str(&self, key: &str) -> Result<&'a str, InputError> {
        self.str(key)
            .ok_or_else(|| InputError(format!("input.{key}: missing required string")))
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Value::as_f64)
    }

    pub fn req_f64(&self, key: &str) -> Result<f64, InputError> {
        self.f64(key)
            .ok_or_else(|| InputError(format!("input.{key}: missing required number")))
    }

    pub fn u64(&self, key: &str) -> Option<u64> {
        self.f64(key).map(|f| f.max(0.0) as u64)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Value::as_bool)
    }

    pub fn array(&self, key: &str) -> Option<&'a Vec<Value>> {
        self.get(key).and_then(Value::as_array)
    }

    /// An entity id property (`what` names it in errors: "track").
    pub fn id<I: FromStr>(&self, key: &str, what: &str) -> Result<Option<I>, InputError> {
        match self.str(key) {
            None => Ok(None),
            Some(s) => parse_id(s, what).map(Some),
        }
    }

    pub fn req_id<I: FromStr>(&self, key: &str, what: &str) -> Result<I, InputError> {
        parse_id(self.req_str(key)?, what)
    }

    /// An array of entity ids.
    pub fn ids<I: FromStr>(&self, key: &str, what: &str) -> Result<Vec<I>, InputError> {
        self.array(key)
            .map(|a| {
                a.iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| InputError(format!("input.{key}: expected {what} ids")))
                            .and_then(|s| parse_id(s, what))
                    })
                    .collect()
            })
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

pub(crate) fn parse_id<I: FromStr>(s: &str, what: &str) -> Result<I, InputError> {
    s.trim().parse().map_err(|_| {
        InputError(format!(
            "`{s}` is not a valid {what} id (ids are 26-character ULIDs as returned by \
             get_project_overview / get_track)"
        ))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn validates_the_subset() {
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "integer", "minimum": 0, "maximum": 127 },
                "b": { "type": "array", "items": { "type": "string", "enum": ["x", "y"] }, "maxItems": 2 },
                "c": { "type": ["number", "string"] }
            },
            "required": ["a"],
            "additionalProperties": false
        });
        assert!(validate(&schema, &json!({ "a": 5, "b": ["x"], "c": "lbl" }), "input").is_ok());
        assert!(validate(&schema, &json!({ "a": 5, "c": null }), "input").is_ok());
        let e = |v: Value| validate(&schema, &v, "input").unwrap_err().0;
        assert!(e(json!({})).contains("missing required property `a`"));
        assert!(e(json!({ "a": 1.5 })).contains("expected integer"));
        assert!(e(json!({ "a": 200 })).contains("<= 127"));
        assert!(e(json!({ "a": 1, "z": 1 })).contains("unknown property `z`"));
        assert!(e(json!({ "a": 1, "b": ["q"] })).contains("input.b[0]"));
        assert!(e(json!({ "a": 1, "b": ["x", "x", "y"] })).contains("at most 2"));
        assert!(e(json!({ "a": 1, "c": true })).contains("number or string"));
        assert!(e(json!([1])).contains("expected object"));
    }
}
