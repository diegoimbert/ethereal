//! Reference resolution: what "the drum loop", "the reverb on the
//! drums" or "the chorus" mean in the open project.
//!
//! - Names are compared as **tokens**: lowercased, accents folded, punctuation and filler
//!   words dropped ("the", "le", "track", ...), simple plurals stemmed ("drums" = "drum"),
//!   and EN/FR aliases mapped to one key ("batterie" = "drum", "réverb" = "reverb").
//! - A clip also answers to its track's name ("the drum loop" = the clip on track "Drums"):
//!   clip-describing words ("loop", "pattern", "boucle") are optional then.
//! - Ties are broken by the UI selection, then by what the agent touched recently; copies
//!   with the same name on one track resolve to the first (selected) one. Anything still
//!   tied is ambiguous: the candidates are returned so the model can ask.
//!
//! Used by `find`, and by every tool argument that takes an id (`track_id`, `clip_id`,
//! `device_id`, ...): a value that is not an id is resolved here first.

use ether_core::protocol::model::*;
use serde_json::{Map, Value, json};

use super::read::{device_type, kind_name, r3};

/// Below this a match is not a match.
pub(super) const MIN_SCORE: f64 = 0.55;
/// Candidates within this of the best one are tied.
const TIE: f64 = 0.08;
/// Selected entities win ties (and a little more).
const SELECTED_BONUS: f64 = 0.06;
/// Entities the agent touched in the last turns.
const RECENT_BONUS: f64 = 0.03;

/// Words that carry no identity.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "my", "our", "your", "this", "that", "these", "those", "its", "on", "of",
    "in", "for", "le", "la", "les", "l", "un", "une", "des", "du", "de", "d", "mon", "ma", "mes",
    "ce", "cet", "cette", "ces", "sur", "track", "piste", "channel", "clip", "device", "plugin",
    "son", "sa", "ses", "au", "aux", "et", "and",
];

/// Words that describe a clip rather than name it: optional when a clip is found through
/// its track's name.
const CLIP_WORDS: &[&str] = &[
    "loop", "pattern", "part", "riff", "groove", "sample", "phrase", "region", "take", "line",
];

/// Alias groups: every word maps to the first one (after plural stemming).
const ALIASES: &[&[&str]] = &[
    &[
        "drum",
        "batterie",
        "beat",
        "kit",
        "drumkit",
        "perc",
        "percussion",
        "batteri",
    ],
    &["bass", "basse", "sub", "bassline"],
    &[
        "vocal", "vox", "voice", "voix", "voc", "chant", "singer", "lyric",
    ],
    &["guitar", "guitare", "gtr", "gt"],
    &["key", "piano", "clavier", "keyboard", "rhode", "ep"],
    &[
        "synth",
        "synthe",
        "synthesizer",
        "synthetiseur",
        "synthesiseur",
    ],
    &["pad", "nappe"],
    &["kick", "bd", "kik"],
    &["snare", "sd", "caisse"],
    &["hat", "hihat", "hh", "charley", "charleston"],
    &["string", "corde"],
    &["chorus", "refrain"],
    &["verse", "couplet"],
    &["bridge", "pont"],
    &["reverb", "reverbe", "reverberation", "verb", "hall", "room"],
    &["delay", "delai", "echo"],
    &["compressor", "compresseur", "comp", "compression"],
    &[
        "eq",
        "equalizer",
        "equaliser",
        "equaliseur",
        "egaliseur",
        "egalizer",
    ],
    &["loop", "boucle"],
    &["melody", "melodie"],
    &["chord", "accord"],
    &["arp", "arpeggio", "arpege", "arpeggiator", "arpegiateur"],
    &["filter", "filtre"],
    &[
        "distortion",
        "disto",
        "saturation",
        "saturator",
        "drive",
        "overdrive",
    ],
    &["sampler", "echantillonneur"],
    &["lead", "solo"],
];

fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'œ' => 'o',
        'æ' => 'a',
        c => c,
    }
}

fn stem(w: &str) -> &str {
    if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
        &w[..w.len() - 1]
    } else {
        w
    }
}

fn canonical(w: &str) -> String {
    let w = stem(w);
    for group in ALIASES {
        if group.contains(&w) {
            return group[0].to_string();
        }
    }
    w.to_string()
}

/// Lowercased, accent-folded words of `s` ("Drum Loop (2)" → ["drum", "loop", "2"]).
pub(super) fn words(s: &str) -> Vec<String> {
    let folded: String = s.to_lowercase().chars().map(fold).collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_digit = None;
    for c in folded.chars() {
        let digit = c.is_ascii_digit();
        // "Drums2" → "drums", "2"; "hi-hat" → "hi", "hat".
        if (!c.is_alphanumeric() || prev_digit.is_some_and(|p| p != digit)) && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if c.is_alphanumeric() {
            cur.push(c);
            prev_digit = Some(digit);
        } else {
            prev_digit = None;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Identity tokens of a name: [`words`] without filler, stemmed, aliased.
pub(super) fn tokens(s: &str) -> Vec<String> {
    let w = words(s);
    // "hi hat" → "hihat".
    let mut joined = Vec::with_capacity(w.len());
    let mut i = 0;
    while i < w.len() {
        if w[i] == "hi" && w.get(i + 1).is_some_and(|n| stem(n) == "hat") {
            joined.push("hihat".to_string());
            i += 2;
            continue;
        }
        joined.push(w[i].clone());
        i += 1;
    }
    let mut out: Vec<String> = joined
        .iter()
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .map(|w| canonical(w))
        .collect();
    out.dedup();
    out
}

fn token_match(q: &str, t: &str) -> f64 {
    if q == t {
        1.0
    } else if q.len() >= 3 && t.len() >= 3 && (t.starts_with(q) || q.starts_with(t)) {
        0.8
    } else {
        0.0
    }
}

/// Score of query tokens against target tokens: `precision_weight` of the query found in
/// the target, the rest how much of the target the query covers. Exact = 1.
pub(super) fn score_tokens(q: &[String], t: &[String], precision_weight: f64) -> f64 {
    if q.is_empty() || t.is_empty() {
        return 0.0;
    }
    let found: f64 = q
        .iter()
        .map(|q| t.iter().map(|t| token_match(q, t)).fold(0.0_f64, f64::max))
        .sum();
    let covered: f64 = t
        .iter()
        .map(|t| q.iter().map(|q| token_match(q, t)).fold(0.0_f64, f64::max))
        .sum();
    let precision = found / q.len() as f64;
    let recall = covered / t.len() as f64;
    precision_weight * precision + (1.0 - precision_weight) * recall
}

/// Score of a query against a display name (exact text = 1).
pub(super) fn score_name(query: &str, q: &[String], name: &str) -> f64 {
    let a: String = words(query).join(" ");
    let b: String = words(name).join(" ");
    if !a.is_empty() && a == b {
        return 1.0;
    }
    let s = score_tokens(q, &tokens(name), 0.75);
    if s == 0.0 && !a.is_empty() && b.contains(&a) {
        return 0.6;
    }
    s
}

/// What a match is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    Track,
    Clip,
    Device,
    Marker,
    Section,
    Plugin,
    DeviceType,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Track => "track",
            Kind::Clip => "clip",
            Kind::Device => "device",
            Kind::Marker => "marker",
            Kind::Section => "section",
            Kind::Plugin => "plugin",
            Kind::DeviceType => "device_type",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "track" => Kind::Track,
            "clip" => Kind::Clip,
            "device" => Kind::Device,
            "marker" => Kind::Marker,
            "section" => Kind::Section,
            "plugin" => Kind::Plugin,
            "device_type" => Kind::DeviceType,
            _ => return None,
        })
    }
}

/// One candidate.
#[derive(Clone, Debug)]
pub(super) struct Match {
    pub kind: Kind,
    /// Entity id (plugins: `format:id`; device types: the key).
    pub id: String,
    pub name: String,
    pub score: f64,
    /// Kind-specific fields (track, position, type, ...).
    pub extra: Map<String, Value>,
    /// Same-name group key (clips: track + name) for "copies of one clip".
    pub group: Option<String>,
    /// Sort key among a group (clips: start).
    pub order: f64,
    pub why: &'static str,
}

impl Match {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("kind".into(), json!(self.kind.name()));
        m.insert("id".into(), json!(self.id));
        m.insert("name".into(), json!(self.name));
        for (k, v) in &self.extra {
            m.insert(k.clone(), v.clone());
        }
        m.insert("score".into(), json!((self.score * 100.0).round() / 100.0));
        if !self.why.is_empty() {
            m.insert("why".into(), json!(self.why));
        }
        Value::Object(m)
    }
}

/// What the selection/recency says (ids as strings).
#[derive(Default)]
pub(super) struct Salience {
    pub selected: Vec<String>,
    pub recent: Vec<String>,
}

impl Salience {
    fn bonus(&self, id: &str) -> f64 {
        if self.selected.iter().any(|s| s == id) {
            SELECTED_BONUS
        } else if self.recent.iter().any(|s| s == id) {
            RECENT_BONUS
        } else {
            0.0
        }
    }
}

/// Sections: each marker starts one, until the next marker (the last until the song end).
pub(super) fn sections(p: &Project) -> Vec<(&Marker, f64)> {
    let markers = p.markers_sorted();
    let end = song_end(p);
    markers
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let next = markers
                .get(i + 1)
                .map_or(end.max(m.position.0), |n| n.position.0);
            (*m, next)
        })
        .collect()
}

/// End of the last arrangement clip (beats).
pub(super) fn song_end(p: &Project) -> f64 {
    p.clips
        .values()
        .filter(|c| c.lane.is_none())
        .map(|c| c.start.0 + c.length.0)
        .fold(0.0, f64::max)
}

/// Every project candidate of `kinds` for `query`, scored (unsorted, below-threshold
/// dropped). `track` scopes clips and devices.
pub(super) fn candidates(
    p: &Project,
    query: &str,
    kinds: &[Kind],
    track: Option<TrackId>,
    sal: &Salience,
) -> Vec<Match> {
    let q = tokens(query);
    let q_clip: Vec<String> = q
        .iter()
        .filter(|w| !CLIP_WORDS.contains(&w.as_str()))
        .cloned()
        .collect();
    let mut out = Vec::new();
    let want = |k: Kind| kinds.contains(&k);
    let track_name = |id: TrackId| p.tracks.get(&id).map(|t| t.name.as_str()).unwrap_or("");
    let map = p.tempo_map();
    if want(Kind::Track) {
        for t in p.tracks.values() {
            if t.kind == TrackKind::Master && !q.iter().any(|w| w == "master") {
                continue;
            }
            let mut s = score_name(query, &q, &t.name);
            if s < MIN_SCORE {
                continue;
            }
            let id = t.id.to_string();
            s += sal.bonus(&id);
            let mut extra = Map::new();
            extra.insert("track_kind".into(), json!(kind_name(t.kind)));
            out.push(Match {
                kind: Kind::Track,
                id,
                name: t.name.clone(),
                score: s,
                extra,
                group: None,
                order: 0.0,
                why: "",
            });
        }
    }
    if want(Kind::Clip) {
        for c in p.clips.values() {
            if c.lane.is_some() || track.is_some_and(|t| t != c.track) {
                continue;
            }
            let tname = track_name(c.track);
            let by_name = score_name(query, &q, &c.name);
            // Clip and track words together ("loop 1" on "Drums" = "drum loop").
            let mut both = tokens(&c.name);
            both.extend(tokens(tname));
            let by_both = 0.95 * score_tokens(&q, &both, 0.85);
            // The track's name, clip words optional ("the drum loop" = the clip on Drums).
            let by_track = if q_clip.is_empty() {
                0.0
            } else {
                0.9 * score_tokens(&q_clip, &tokens(tname), 0.75)
            };
            let (mut s, why) = if by_name >= by_both && by_name >= by_track {
                (by_name, "")
            } else if by_both >= by_track {
                (by_both, "clip + track name")
            } else {
                (by_track, "track name")
            };
            if s < MIN_SCORE {
                continue;
            }
            let id = c.id.to_string();
            s += sal.bonus(&id);
            let end = c.start.0 + c.length.0;
            let mut extra = Map::new();
            extra.insert("track_id".into(), json!(c.track.to_string()));
            extra.insert("track".into(), json!(tname));
            extra.insert("start_beats".into(), json!(r3(c.start.0)));
            extra.insert("end_beats".into(), json!(r3(end)));
            extra.insert(
                "bars".into(),
                json!(super::read::bars(&map, c.start.0, end)),
            );
            out.push(Match {
                kind: Kind::Clip,
                id,
                name: c.name.clone(),
                score: s,
                extra,
                group: Some(format!("{}/{}", c.track, words(&c.name).join(" "))),
                order: c.start.0,
                why,
            });
        }
    }
    if want(Kind::Device) {
        for d in p.devices.values() {
            if track.is_some_and(|t| t != d.track) {
                continue;
            }
            let tname = track_name(d.track);
            let ty = device_type(d);
            let type_words = ty.split(':').next_back().unwrap_or("").replace('-', " ");
            let by_name =
                score_name(query, &q, &d.name).max(0.95 * score_name(query, &q, &type_words));
            let mut both = tokens(&d.name);
            both.extend(tokens(&type_words));
            both.extend(tokens(tname));
            let by_both = 0.95 * score_tokens(&q, &both, 0.9);
            let (mut s, why) = if by_name >= by_both {
                (by_name, "")
            } else {
                (by_both, "device + track name")
            };
            if s < MIN_SCORE {
                continue;
            }
            let id = d.id.to_string();
            s += sal.bonus(&id);
            let mut extra = Map::new();
            extra.insert("type".into(), json!(ty));
            extra.insert("track_id".into(), json!(d.track.to_string()));
            extra.insert("track".into(), json!(tname));
            if d.chain.is_some() || d.pad.is_some() {
                extra.insert("nested".into(), json!(true));
            }
            out.push(Match {
                kind: Kind::Device,
                id,
                name: d.name.clone(),
                score: s,
                extra,
                group: None,
                order: 0.0,
                why,
            });
        }
    }
    if want(Kind::Marker) || want(Kind::Section) {
        for (m, end) in sections(p) {
            let s = score_name(query, &q, &m.name);
            if s < MIN_SCORE {
                continue;
            }
            let mut extra = Map::new();
            extra.insert("start_beats".into(), json!(r3(m.position.0)));
            if want(Kind::Section) {
                extra.insert("end_beats".into(), json!(r3(end)));
                extra.insert(
                    "bars".into(),
                    json!(super::read::bars(
                        &map,
                        m.position.0,
                        end.max(m.position.0 + 1e-3)
                    )),
                );
            }
            out.push(Match {
                kind: if want(Kind::Section) {
                    Kind::Section
                } else {
                    Kind::Marker
                },
                id: m.id.to_string(),
                name: m.name.clone(),
                score: s,
                extra,
                group: None,
                order: m.position.0,
                why: "",
            });
        }
    }
    out
}

/// The outcome of resolving one reference.
pub(super) enum Resolved {
    One {
        m: Match,
        /// Other clips with the same name on the same track (copies).
        copies: usize,
    },
    Ambiguous(Vec<Match>),
    None,
}

/// Pick the referent among scored candidates (see the module docs).
pub(super) fn pick(mut c: Vec<Match>, sal: &Salience) -> Resolved {
    if c.is_empty() {
        return Resolved::None;
    }
    c.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.order.total_cmp(&b.order))
    });
    let best = c[0].score;
    let tied: Vec<Match> = c.into_iter().filter(|m| m.score >= best - TIE).collect();
    if tied.len() == 1 {
        return Resolved::One {
            m: tied.into_iter().next().expect("one"),
            copies: 0,
        };
    }
    // Copies of one clip (same name, same track): the selected one, else the first.
    let g = tied[0].group.clone();
    if g.is_some() && tied.iter().all(|m| m.group == g) {
        let copies = tied.len() - 1;
        let m = tied
            .iter()
            .find(|m| sal.selected.contains(&m.id))
            .cloned()
            .unwrap_or_else(|| {
                tied.iter()
                    .min_by(|a, b| a.order.total_cmp(&b.order))
                    .cloned()
                    .expect("non-empty")
            });
        return Resolved::One { m, copies };
    }
    // The user's selection decides (one selected candidate, or copies of one).
    let in_one_group = |ms: &[&Match]| {
        ms.first().is_some_and(|f| {
            ms.iter()
                .all(|m| m.id == f.id || (m.group.is_some() && m.group == f.group))
        })
    };
    let selected: Vec<&Match> = tied
        .iter()
        .filter(|m| sal.selected.contains(&m.id))
        .collect();
    if in_one_group(&selected) {
        let mut m = selected[0].clone();
        m.why = "selected";
        return Resolved::One { m, copies: 0 };
    }
    // Then what the agent edited last ("mute the drum loop" ... "now duplicate it").
    let rank = |m: &Match| sal.recent.iter().position(|r| *r == m.id);
    if let Some(best) = tied.iter().filter_map(rank).min() {
        let m = tied.iter().find(|m| rank(m) == Some(best)).expect("ranked");
        let same: Vec<&Match> = tied
            .iter()
            .filter(|x| x.id == m.id || (x.group.is_some() && x.group == m.group))
            .collect();
        let first = same
            .iter()
            .min_by(|a, b| a.order.total_cmp(&b.order))
            .copied()
            .unwrap_or(m);
        let mut pick = if same.len() > 1 {
            first.clone()
        } else {
            m.clone()
        };
        pick.why = "most recently edited";
        return Resolved::One {
            m: pick,
            copies: same.len() - 1,
        };
    }
    // One clear favourite on the name alone.
    if tied[0].score - tied[1].score >= SELECTED_BONUS - 1e-9 {
        return Resolved::One {
            m: tied.into_iter().next().expect("one"),
            copies: 0,
        };
    }
    Resolved::Ambiguous(tied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Vec<String> {
        tokens(s)
    }

    #[test]
    fn tokens_fold_alias_and_stem() {
        assert_eq!(t("The Drums"), ["drum"]);
        assert_eq!(t("la batterie"), ["drum"]);
        assert_eq!(t("Drum Loop (2)"), ["drum", "loop", "2"]);
        assert_eq!(t("la réverb"), ["reverb"]);
        assert_eq!(t("Hi-Hats"), ["hat"]);
        assert_eq!(t("Lead Vox"), ["lead", "vocal"]);
        assert_eq!(t("Synth Pads"), ["synth", "pad"]);
        assert_eq!(t("Bass"), ["bass"]);
        assert_eq!(words("Drums2"), ["drums", "2"]);
    }

    #[test]
    fn scores() {
        let q = t("drum loop");
        assert_eq!(score_name("drum loop", &q, "Drum Loop"), 1.0);
        assert!(score_name("drum loop", &q, "Drum Loop 2") > 0.85);
        assert!(score_name("drum loop", &q, "Bass Loop") < 0.6);
        assert_eq!(score_name("bass", &t("bass"), "Drums"), 0.0);
        assert!(score_tokens(&t("drums"), &t("Drums"), 0.75) == 1.0);
    }
}
