//! Project querying (`agent-plugins`): `find`, `list_clips`, and the name-or-id resolution
//! of tool arguments (see [`super::resolve`]).

use ether_core::protocol::model::*;
use serde_json::{Map, Value, json};

use super::resolve::{self, Kind, Match, Resolved, Salience};
use super::schema::Args;
use super::{ToolError, ToolResult};
use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices};

/// Entities remembered as "recently touched by the agent".
pub(super) const RECENT_KEPT: usize = 24;

/// Argument keys that hold one entity reference, and what they refer to.
const REF_KEYS: &[(&str, Kind)] = &[
    ("track_id", Kind::Track),
    ("parent_track_id", Kind::Track),
    ("before_track_id", Kind::Track),
    ("clip_id", Kind::Clip),
    ("device_id", Kind::Device),
];
/// Argument keys that hold a list of references.
const REF_LIST_KEYS: &[(&str, Kind)] = &[("clip_ids", Kind::Clip), ("stem_track_ids", Kind::Track)];

fn is_id(s: &str) -> bool {
    s.trim().parse::<TrackId>().is_ok()
}

fn candidates_json(c: &[Match], max: usize) -> Value {
    json!(c.iter().take(max).map(Match::to_json).collect::<Vec<_>>())
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(super) fn agent_salience(&self) -> Salience {
        let mut sel = Vec::new();
        if let Some(s) = &self.agent.selection {
            sel.extend(s.tracks.iter().map(ToString::to_string));
            sel.extend(s.clips.iter().map(ToString::to_string));
            sel.extend(s.devices.iter().map(ToString::to_string));
            if let Some(c) = s.editing_clip {
                sel.push(c.to_string());
            }
        }
        Salience {
            selected: sel,
            recent: self.agent.recent.clone(),
        }
    }

    /// Remember the ids an edit tool returned (for recency).
    pub(super) fn agent_touch(&mut self, result: &Value) {
        let Some(m) = result.as_object() else {
            return;
        };
        for (k, v) in m {
            if !k.ends_with("_id") {
                continue;
            }
            if let Some(id) = v.as_str().filter(|s| is_id(s)) {
                self.agent.recent.retain(|r| r != id);
                self.agent.recent.insert(0, id.to_string());
            }
        }
        self.agent.recent.truncate(RECENT_KEPT);
    }

    fn resolve_one(
        &self,
        p: &Project,
        key: &str,
        query: &str,
        kind: Kind,
        scope: Option<TrackId>,
        sal: &Salience,
    ) -> Result<(String, Value), ToolError> {
        let scoped = |scope| resolve::candidates(p, query, &[kind], scope, sal);
        let mut c = scoped(scope);
        if c.is_empty() && scope.is_some() {
            c = scoped(None);
        }
        match resolve::pick(c, sal) {
            Resolved::One { m, copies } => {
                let mut note = json!({ "query": query, "id": m.id, "name": m.name });
                if let Some(t) = m.extra.get("track") {
                    note["track"] = t.clone();
                }
                if copies > 0 {
                    note["note"] = json!(format!(
                        "{copies} other clip(s) with this name on this track; used the {}",
                        if sal.selected.contains(&m.id) {
                            "selected one"
                        } else {
                            "first one"
                        }
                    ));
                }
                Ok((m.id, note))
            }
            Resolved::Ambiguous(c) => Err(ToolError::Ambiguous {
                message: format!(
                    "`{key}`: \"{query}\" matches several {}s. Ask the user which one, or pass one of these ids",
                    kind.name()
                ),
                candidates: candidates_json(&c, 8),
            }),
            Resolved::None => Err(ToolError::input(format!(
                "`{key}`: no {} matches \"{query}\" (ids are 26-character ULIDs; names work too). Use find or get_project_overview",
                kind.name()
            ))),
        }
    }

    /// Replace names in reference arguments by ids (in place). Returns what was resolved,
    /// for the result's `resolved` field.
    pub(super) fn resolve_refs(
        &self,
        tool: &str,
        input: &mut Map<String, Value>,
    ) -> Result<Vec<Value>, ToolError> {
        let Some(p) = self.doc.as_ref().map(|d| &d.project) else {
            return Ok(Vec::new());
        };
        let needs = |v: &Value| {
            v.as_str()
                .is_some_and(|s| !s.trim().is_empty() && !is_id(s))
        };
        let any = REF_KEYS
            .iter()
            .any(|(k, _)| input.get(*k).is_some_and(needs))
            || REF_LIST_KEYS.iter().any(|(k, _)| {
                input
                    .get(*k)
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.iter().any(needs))
            });
        if !any {
            return Ok(Vec::new());
        }
        let sal = self.agent_salience();
        let mut resolved = Vec::new();
        // Tracks first: they scope the clips and devices of the same call (except set_clip,
        // where `track_id` is the destination).
        for (key, kind) in REF_KEYS {
            if *kind != Kind::Track {
                continue;
            }
            if let Some(q) = input.get(*key).and_then(Value::as_str).map(str::to_string)
                && !is_id(&q)
                && !q.trim().is_empty()
            {
                let (id, note) = self.resolve_one(p, key, &q, *kind, None, &sal)?;
                input.insert((*key).to_string(), json!(id));
                resolved.push(note);
            }
        }
        let scope = if tool == "set_clip" {
            None
        } else {
            input
                .get("track_id")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<TrackId>().ok())
        };
        for (key, kind) in REF_KEYS {
            if *kind == Kind::Track {
                continue;
            }
            if let Some(q) = input.get(*key).and_then(Value::as_str).map(str::to_string)
                && !is_id(&q)
                && !q.trim().is_empty()
            {
                let (id, note) = self.resolve_one(p, key, &q, *kind, scope, &sal)?;
                input.insert((*key).to_string(), json!(id));
                resolved.push(note);
            }
        }
        for (key, kind) in REF_LIST_KEYS {
            let Some(items) = input.get(*key).and_then(Value::as_array).cloned() else {
                continue;
            };
            let mut ids = Vec::with_capacity(items.len());
            for v in items {
                match v.as_str() {
                    Some(q) if !is_id(q) && !q.trim().is_empty() => {
                        let (id, note) = self.resolve_one(p, key, q, *kind, scope, &sal)?;
                        ids.push(json!(id));
                        resolved.push(note);
                    }
                    _ => ids.push(v),
                }
            }
            input.insert((*key).to_string(), Value::Array(ids));
        }
        let _ = tool;
        Ok(resolved)
    }

    // ─── find ───────────────────────────────────────────────────────────────────────────

    pub(super) fn tool_find(&mut self, a: Args) -> ToolResult {
        let query = a.req_str("query")?.trim().to_string();
        let mut kinds: Vec<Kind> = match a.array("kinds") {
            Some(list) => list
                .iter()
                .filter_map(Value::as_str)
                .filter_map(Kind::parse)
                .collect(),
            None => vec![
                Kind::Track,
                Kind::Clip,
                Kind::Device,
                Kind::Section,
                Kind::Plugin,
                Kind::DeviceType,
            ],
        };
        if kinds.contains(&Kind::Marker) && kinds.contains(&Kind::Section) {
            kinds.retain(|k| *k != Kind::Marker);
        }
        let prefer = a.str("prefer").and_then(Kind::parse);
        let scope: Option<TrackId> = a.id("track_id", "track")?;
        let limit = a.u64("limit").unwrap_or(8).clamp(1, 50) as usize;
        let sal = self.agent_salience();
        let p = self.agent_project()?;
        let mut matches = resolve::candidates(p, &query, &kinds, scope, &sal);
        // Library entries only when asked for, or when they match well.
        let explicit = a.array("kinds").is_some();
        let q = resolve::tokens(&query);
        if kinds.contains(&Kind::DeviceType) {
            for d in ether_devices::offered_descriptors() {
                let ether_core::protocol::devices::DeviceTypeRef::Builtin { device } =
                    d.device_type
                else {
                    continue;
                };
                let key = device.key();
                let s = resolve::score_name(&query, &q, &d.name).max(resolve::score_name(
                    &query,
                    &q,
                    &key.replace('-', " "),
                ));
                if s >= if explicit { resolve::MIN_SCORE } else { 0.8 } {
                    let mut extra = Map::new();
                    extra.insert(
                        "category".into(),
                        json!(super::read::category_name(d.category)),
                    );
                    matches.push(Match {
                        kind: Kind::DeviceType,
                        id: key,
                        name: d.name.clone(),
                        score: s,
                        extra,
                        group: None,
                        order: 0.0,
                        why: "add with add_device type",
                    });
                }
            }
        }
        if kinds.contains(&Kind::Plugin)
            && let Some(cat) = self.bridge.plugin_catalog()
        {
            for pl in &cat.plugins {
                let s = resolve::score_name(&query, &q, &pl.name).max(
                    0.9 * resolve::score_name(&query, &q, &format!("{} {}", pl.vendor, pl.name)),
                );
                if s >= if explicit { resolve::MIN_SCORE } else { 0.8 } {
                    let mut extra = Map::new();
                    extra.insert("vendor".into(), json!(pl.vendor));
                    extra.insert(
                        "format".into(),
                        json!(super::plugins::format_key(pl.format)),
                    );
                    extra.insert(
                        "category".into(),
                        json!(super::read::category_name(pl.category)),
                    );
                    matches.push(Match {
                        kind: Kind::Plugin,
                        id: pl.id.clone(),
                        name: pl.name.clone(),
                        score: s,
                        extra,
                        group: None,
                        order: 0.0,
                        why: "installed plugin: add with add_device plugin_id",
                    });
                }
            }
        }
        if let Some(k) = prefer {
            for m in &mut matches {
                if m.kind == k {
                    m.score += 0.15;
                }
            }
        }
        matches.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(a.kind.cmp(&b.kind))
                .then(a.order.total_cmp(&b.order))
        });
        let total = matches.len();
        let mut v = json!({ "query": query, "total": total });
        // The referent, when there is one (same rules as id arguments).
        let top_kind = matches.first().map(|m| m.kind);
        if let Some(k) = top_kind {
            let same: Vec<Match> = matches.iter().filter(|m| m.kind == k).cloned().collect();
            match resolve::pick(same, &sal) {
                Resolved::One { m, copies } => {
                    let mut b = m.to_json();
                    if copies > 0 {
                        b["copies"] = json!(copies);
                    }
                    v["best"] = b;
                }
                Resolved::Ambiguous(c) => {
                    v["ambiguous"] = json!(true);
                    v["note"] = json!(format!(
                        "{} {}s match about equally: ask the user, or pick the most salient and say which",
                        c.len(),
                        k.name()
                    ));
                }
                Resolved::None => {}
            }
        }
        v["matches"] = candidates_json(&matches, limit);
        if total == 0 {
            v["note"] = json!("nothing matches: try other words, or get_project_overview");
        }
        Ok(v)
    }

    // ─── list_clips ─────────────────────────────────────────────────────────────────────

    pub(super) fn tool_list_clips(&mut self, a: Args) -> ToolResult {
        let track: Option<TrackId> = a.id("track_id", "track")?;
        let name = a.str("name").map(str::trim).filter(|s| !s.is_empty());
        let from = a.f64("from_beats");
        let to = a.f64("to_beats");
        let kind = a.str("kind");
        let offset = a.u64("offset").unwrap_or(0) as usize;
        let limit = a.u64("limit").unwrap_or(100).max(1) as usize;
        let p = self.agent_project()?;
        if let Some(t) = track
            && !p.tracks.contains_key(&t)
        {
            return Err(ToolError::Command(crate::tx::not_found(format!(
                "track {t}"
            ))));
        }
        let q = name.map(resolve::tokens);
        let map = p.tempo_map();
        let mut clips: Vec<(&Clip, f64)> = p
            .clips
            .values()
            .filter(|c| c.lane.is_none())
            .filter(|c| track.is_none_or(|t| c.track == t))
            .filter(|c| match kind {
                Some("midi") => matches!(c.content, ClipContent::Midi),
                Some("audio") => matches!(c.content, ClipContent::Audio(_)),
                _ => true,
            })
            .filter(|c| {
                let end = c.start.0 + c.length.0;
                from.is_none_or(|f| end > f + Beats::EPSILON)
                    && to.is_none_or(|t| c.start.0 < t - Beats::EPSILON)
            })
            .filter_map(|c| match (&q, name) {
                (Some(q), Some(n)) => {
                    let s = resolve::score_name(n, q, &c.name);
                    (s >= resolve::MIN_SCORE).then_some((c, s))
                }
                _ => Some((c, 1.0)),
            })
            .collect();
        let positions: std::collections::HashMap<TrackId, usize> = super::read::tracks_in_order(p)
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id, i))
            .collect();
        let order = |c: &Clip| positions.get(&c.track).copied().unwrap_or(usize::MAX);
        clips.sort_by(|(a, _), (b, _)| {
            a.start
                .0
                .total_cmp(&b.start.0)
                .then(order(a).cmp(&order(b)))
                .then(a.id.cmp(&b.id))
        });
        let items: Vec<Value> = clips
            .iter()
            .skip(offset)
            .take(limit)
            .map(|(c, _)| {
                let mut v = super::read::clip_summary(p, &map, c);
                v["track_id"] = json!(c.track.to_string());
                v["track"] = json!(p.tracks.get(&c.track).map(|t| t.name.clone()));
                v
            })
            .collect();
        let mut v = json!({ "total": clips.len(), "offset": offset, "clips": items });
        if offset + limit < clips.len() {
            v["note"] = json!(format!(
                "more clips: call again with offset {}",
                offset + limit
            ));
        }
        Ok(v)
    }
}
