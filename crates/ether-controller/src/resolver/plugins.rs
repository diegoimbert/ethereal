//! Deterministic plugin matching (pure, unit-tested): a missing plugin against the
//! installed ones.
//!
//! Rules, best first (each installed plugin gets its best rule; ties prefer the same
//! format, then CLAP > VST3 > AU > VST2, then the higher version):
//!
//! | Rule | When | Confidence |
//! |---|---|---|
//! | `SameId` | another format carrying the same id: a VST3 class id derived from the VST2 unique id (Steinberg's VST2→VST3 convention, `"VST" + id + name`), or an AU subtype equal to the VST2 id (JUCE's plugin code); vendor not different | 0.95 |
//! | `SameName` | same name (versions kept; format/arch suffixes ignored) and vendor, another format | 0.9 |
//! | `SameFamily` | same name without versions and same vendor ("Kontakt 7" → "Kontakt") | 0.85 same format, 0.8 another |
//! | `SameName`/`SameFamily` with an unknown vendor | | −0.1 |
//! | `Similar` | Jaro-Winkler ≥ [`SIMILAR_THRESHOLD`] on the family names, vendor not different | 0.45..0.7 |
//!
//! The state blob is carried ([`ParamPlan::KeepState`]) only when the target can load it:
//! the same format and the same id or family + vendor (another version of the same plugin).
//! Otherwise saved parameter values are mapped by name (when both sides have names) or by
//! index (same plugin family without names), and what is lost is listed.

use std::collections::BTreeMap;

use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::protocol::resolver::{
    MatchRule, ParamMap, ParamName, ParamPlan, PluginCandidate, SavedParam,
};

use super::normalize::{self, VendorMatch};

/// Fuzzy name similarity needed for a `Similar` candidate.
pub const SIMILAR_THRESHOLD: f64 = 0.88;
/// Candidates kept per missing plugin.
pub const MAX_CANDIDATES: usize = 5;
/// Below this, the UI offers AI suggestions (when a provider is configured; mirrored in
/// `ui/src/features/import-resolver`).
#[cfg(test)]
pub const LOW_CONFIDENCE: f32 = 0.7;

/// The missing plugin, as the project saved it.
#[derive(Clone, Debug)]
pub struct Missing<'a> {
    pub format: PluginFormat,
    pub id: &'a str,
    pub name: &'a str,
    pub vendor: &'a str,
    pub has_state: bool,
    pub params: &'a [SavedParam],
}

/// The VST2 unique id (8 uppercase hex digits) a plugin id carries, in any format: VST2
/// itself, a VST3 class id made from it (`565354` = "VST", then the id, then the name), an
/// AU subtype (four printable chars).
pub fn vst2_equivalent(format: PluginFormat, id: &str) -> Option<String> {
    match format {
        PluginFormat::Vst2 => (id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| id.to_ascii_uppercase()),
        PluginFormat::Vst3 => {
            let id = id.to_ascii_uppercase();
            (id.len() == 32 && id.starts_with("565354") && id.is_ascii())
                .then(|| id[6..14].to_string())
        }
        PluginFormat::Au => {
            let sub = id.split(':').nth(1)?;
            (sub.len() == 4 && sub.is_ascii() && !sub.contains('\\'))
                .then(|| sub.bytes().map(|b| format!("{b:02X}")).collect())
        }
        PluginFormat::Clap => None,
    }
}

fn format_rank(f: PluginFormat) -> u8 {
    match f {
        PluginFormat::Clap => 0,
        PluginFormat::Vst3 => 1,
        PluginFormat::Au => 2,
        PluginFormat::Vst2 => 3,
    }
}

pub fn format_label(f: PluginFormat) -> &'static str {
    match f {
        PluginFormat::Clap => "CLAP",
        PluginFormat::Vst3 => "VST3",
        PluginFormat::Au => "AU",
        PluginFormat::Vst2 => "VST2",
    }
}

/// Numeric parts of a version string, for ordering ("10.2.1" > "9.9").
fn version_key(v: &str) -> Vec<u32> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// The best rule for one installed plugin, if any: `(rule, confidence, reason)`.
fn rule_for(m: &Missing<'_>, p: &PluginDescriptor) -> Option<(MatchRule, f32, String)> {
    if p.format == m.format && p.id.eq_ignore_ascii_case(m.id) {
        // Installed after all (the caller filters these out); kept for completeness.
        return Some((
            MatchRule::SameId,
            1.0,
            "The same plugin is installed".into(),
        ));
    }
    let vendor = normalize::vendor_match(m.vendor, &p.vendor);
    let other_format = p.format != m.format;
    if other_format
        && vendor != VendorMatch::Different
        && let (Some(a), Some(b)) = (
            vst2_equivalent(m.format, m.id),
            vst2_equivalent(p.format, &p.id),
        )
        && a == b
    {
        return Some((
            MatchRule::SameId,
            0.95,
            format!(
                "Same plugin id ({a}) in {}: {} → {}",
                format_label(p.format),
                format_label(m.format),
                format_label(p.format)
            ),
        ));
    }
    let unknown_penalty = if vendor == VendorMatch::Unknown {
        0.1
    } else {
        0.0
    };
    let (mf, pf) = (normalize::full(m.name), normalize::full(&p.name));
    if vendor != VendorMatch::Different
        && other_format
        && normalize::compact(&mf) == normalize::compact(&pf)
    {
        return Some((
            MatchRule::SameName,
            0.9 - unknown_penalty,
            format!(
                "Same name{} in {}",
                if vendor == VendorMatch::Same {
                    " and vendor"
                } else {
                    ""
                },
                format_label(p.format)
            ),
        ));
    }
    let (ma, pa) = (normalize::family(m.name), normalize::family(&p.name));
    if vendor != VendorMatch::Different && normalize::compact(&ma) == normalize::compact(&pa) {
        let base = if other_format { 0.8 } else { 0.85 };
        let what = if mf == pf {
            format!("another format ({})", format_label(p.format))
        } else {
            format!(
                "another version{}",
                if other_format {
                    format!(" in {}", format_label(p.format))
                } else {
                    String::new()
                }
            )
        };
        return Some((
            MatchRule::SameFamily,
            base - unknown_penalty,
            format!(
                "Same plugin family{}: {} → {} ({what})",
                if vendor == VendorMatch::Same {
                    " and vendor"
                } else {
                    ""
                },
                m.name,
                p.name
            ),
        ));
    }
    if vendor == VendorMatch::Different {
        return None;
    }
    let sim = normalize::similarity(&ma, &pa);
    if sim >= SIMILAR_THRESHOLD {
        let c = 0.45 + (sim - SIMILAR_THRESHOLD) / (1.0 - SIMILAR_THRESHOLD) * 0.25;
        let c = c as f32
            - if vendor == VendorMatch::Unknown {
                0.05
            } else {
                0.0
            };
        return Some((
            MatchRule::Similar,
            c.clamp(0.0, 0.7),
            format!(
                "Similar name ({:.0}% alike): {} → {}",
                sim * 100.0,
                m.name,
                p.name
            ),
        ));
    }
    None
}

/// Whether a target can load the missing plugin's state blob: same format, and the same id
/// or the same family and vendor (another version of the same plugin).
pub fn can_keep_state(
    m: &Missing<'_>,
    format: PluginFormat,
    id: &str,
    name: &str,
    vendor: &str,
) -> bool {
    if !m.has_state || format != m.format {
        return false;
    }
    id.eq_ignore_ascii_case(m.id)
        || (normalize::vendor_match(m.vendor, vendor) == VendorMatch::Same
            && normalize::compact(&normalize::family(m.name))
                == normalize::compact(&normalize::family(name)))
}

/// Map saved parameter values onto a target's parameters: by name when both sides have
/// names, else by index (`same_family`: the same plugin in another version/format, whose
/// parameter ids usually line up), else nothing. Returns the map and what is lost.
pub fn map_params(
    source: &[SavedParam],
    target: Option<&[ParamName]>,
    same_family: bool,
) -> (Vec<ParamMap>, Vec<String>) {
    let label = |p: &SavedParam| {
        p.name
            .clone()
            .unwrap_or_else(|| format!("parameter {}", p.id))
    };
    let named = source.iter().any(|p| p.name.is_some());
    if let Some(target) = target.filter(|t| !t.is_empty() && named) {
        let by_name: BTreeMap<String, u32> = target
            .iter()
            .map(|t| (normalize::param(&t.name), t.id))
            .collect();
        let mut map = Vec::new();
        let mut lost = Vec::new();
        for p in source {
            match p
                .name
                .as_deref()
                .and_then(|n| by_name.get(&normalize::param(n)))
            {
                Some(to) => map.push(ParamMap {
                    from: p.id,
                    to: *to,
                }),
                None => lost.push(label(p)),
            }
        }
        return (map, lost);
    }
    if same_family {
        let ids: Option<std::collections::BTreeSet<u32>> = target
            .filter(|t| !t.is_empty())
            .map(|t| t.iter().map(|t| t.id).collect());
        let mut map = Vec::new();
        let mut lost = Vec::new();
        for p in source {
            if ids.as_ref().is_none_or(|ids| ids.contains(&p.id)) {
                map.push(ParamMap {
                    from: p.id,
                    to: p.id,
                });
            } else {
                lost.push(label(p));
            }
        }
        return (map, lost);
    }
    (Vec::new(), source.iter().map(label).collect())
}

/// The state/parameter plan for replacing `m` with a target, and what it loses.
pub fn plan(
    m: &Missing<'_>,
    format: PluginFormat,
    id: &str,
    name: &str,
    vendor: &str,
    target_params: Option<&[ParamName]>,
) -> (ParamPlan, Vec<String>) {
    if can_keep_state(m, format, id, name, vendor) {
        return (ParamPlan::KeepState, Vec::new());
    }
    let same_family = normalize::vendor_match(m.vendor, vendor) != VendorMatch::Different
        && (normalize::compact(&normalize::family(m.name))
            == normalize::compact(&normalize::family(name))
            || matches!(
                (vst2_equivalent(m.format, m.id), vst2_equivalent(format, id)),
                (Some(a), Some(b)) if a == b
            ));
    let (map, mut lost) = map_params(m.params, target_params, same_family);
    if m.has_state {
        lost.insert(
            0,
            if format != m.format {
                format!(
                    "Plugin state (presets, loaded samples): {} state can't be read by the {} plugin",
                    format_label(m.format),
                    format_label(format)
                )
            } else {
                "Plugin state (presets, loaded samples): another plugin can't read it".into()
            },
        );
    }
    let lost_params = lost.len() - usize::from(m.has_state);
    if lost_params > 8 {
        // Keep the list readable.
        let keep = usize::from(m.has_state) + 6;
        let more = lost.len() - keep;
        lost.truncate(keep);
        lost.push(format!("and {more} more parameters"));
    }
    (ParamPlan::Map { map }, lost)
}

/// Ranked candidates for `m` among `installed` (at most [`MAX_CANDIDATES`]).
/// `params`: known parameter lists of installed plugins, by `(format, id)`.
pub fn candidates(
    m: &Missing<'_>,
    installed: &[PluginDescriptor],
    params: &BTreeMap<(PluginFormat, String), Vec<ParamName>>,
) -> Vec<PluginCandidate> {
    let mut found: Vec<(PluginCandidate, u8, Vec<u32>)> = Vec::new();
    for p in installed {
        if p.format == m.format && p.id.eq_ignore_ascii_case(m.id) {
            continue;
        }
        let Some((rule, confidence, reason)) = rule_for(m, p) else {
            continue;
        };
        let target = params.get(&(p.format, p.id.clone())).map(Vec::as_slice);
        let (plan, lost) = plan(m, p.format, &p.id, &p.name, &p.vendor, target);
        found.push((
            PluginCandidate {
                format: p.format,
                id: p.id.clone(),
                name: p.name.clone(),
                vendor: p.vendor.clone(),
                version: p.version.clone(),
                rule,
                confidence,
                reason,
                plan,
                lost,
            },
            if p.format == m.format {
                0
            } else {
                1 + format_rank(p.format)
            },
            version_key(&p.version),
        ));
    }
    found.sort_by(|a, b| {
        b.0.confidence
            .total_cmp(&a.0.confidence)
            .then(a.1.cmp(&b.1))
            .then(b.2.cmp(&a.2))
            .then(a.0.name.cmp(&b.0.name))
    });
    // Two plugins may share a (format, id) (VST2 uniqueID collisions): the name tells them apart.
    found.dedup_by(|a, b| a.0.format == b.0.format && a.0.id == b.0.id && a.0.name == b.0.name);
    found.truncate(MAX_CANDIDATES);
    found.into_iter().map(|(c, _, _)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::devices::DeviceCategory;

    fn desc(
        format: PluginFormat,
        id: &str,
        name: &str,
        vendor: &str,
        version: &str,
    ) -> PluginDescriptor {
        PluginDescriptor {
            format,
            id: id.into(),
            name: name.into(),
            vendor: vendor.into(),
            version: version.into(),
            description: String::new(),
            features: vec![],
            category: DeviceCategory::Instrument,
            path: String::new(),
            sidechain_inputs: 0,
        }
    }

    fn missing<'a>(
        format: PluginFormat,
        id: &'a str,
        name: &'a str,
        vendor: &'a str,
        params: &'a [SavedParam],
    ) -> Missing<'a> {
        Missing {
            format,
            id,
            name,
            vendor,
            has_state: true,
            params,
        }
    }

    fn sp(id: u32, name: Option<&str>) -> SavedParam {
        SavedParam {
            id,
            value: 0.5,
            name: name.map(str::to_string),
        }
    }

    const K7: &str = "5653544B6F6E74616B74370000000000";

    #[test]
    fn kontakt_7_becomes_kontakt_keeping_the_state() {
        let installed = [
            desc(
                PluginFormat::Vst3,
                "AAAA0000BBBB1111CCCC2222DDDD3333",
                "Kontakt 8",
                "Native Instruments",
                "8.1.0",
            ),
            desc(
                PluginFormat::Vst3,
                "11112222333344445555666677778888",
                "Serum",
                "Xfer Records",
                "1.3",
            ),
        ];
        let m = missing(
            PluginFormat::Vst3,
            "0123456789ABCDEF0123456789ABCDEF",
            "Kontakt 7",
            "Native Instruments GmbH",
            &[],
        );
        let c = candidates(&m, &installed, &BTreeMap::new());
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "Kontakt 8");
        assert_eq!(c[0].rule, MatchRule::SameFamily);
        assert!((c[0].confidence - 0.85).abs() < 1e-6);
        assert_eq!(c[0].plan, ParamPlan::KeepState);
        assert!(c[0].lost.is_empty());
        assert!(
            c[0].reason.contains("Kontakt 7 → Kontakt 8"),
            "{}",
            c[0].reason
        );

        // "Kontakt" with no version, CLAP and VST3 installed: the same format wins the tie.
        let installed = [
            desc(
                PluginFormat::Clap,
                "com.native-instruments.kontakt",
                "Kontakt",
                "Native Instruments",
                "8.0",
            ),
            desc(
                PluginFormat::Vst3,
                "AAAA0000BBBB1111CCCC2222DDDD3333",
                "Kontakt",
                "NI",
                "8.0",
            ),
        ];
        let c = candidates(&m, &installed, &BTreeMap::new());
        assert_eq!(c[0].format, PluginFormat::Vst3);
        assert_eq!(c[0].plan, ParamPlan::KeepState);
        assert_eq!(c[1].format, PluginFormat::Clap);
        assert!(matches!(c[1].plan, ParamPlan::Map { .. }));
        assert!(
            c[1].lost[0].contains("VST3 state can't be read by the CLAP plugin"),
            "{:?}",
            c[1].lost
        );
    }

    #[test]
    fn same_id_across_formats() {
        // VST2 'Kont' (4B6F6E74) → its VST3 class id ("VST" + id + "kontakt7\0").
        assert_eq!(
            vst2_equivalent(PluginFormat::Vst3, K7).as_deref(),
            Some("4B6F6E74")
        );
        assert_eq!(
            vst2_equivalent(PluginFormat::Au, "aumu:Kont:NInS").as_deref(),
            Some("4B6F6E74")
        );
        assert_eq!(
            vst2_equivalent(PluginFormat::Vst2, "4b6f6e74").as_deref(),
            Some("4B6F6E74")
        );
        assert_eq!(
            vst2_equivalent(PluginFormat::Vst3, "0123456789ABCDEF0123456789ABCDEF"),
            None
        );
        assert_eq!(vst2_equivalent(PluginFormat::Clap, "x"), None);

        let m = missing(
            PluginFormat::Vst2,
            "4B6F6E74",
            "Some Old Name",
            "Native Instruments",
            &[],
        );
        let installed = [
            desc(
                PluginFormat::Vst3,
                K7,
                "Totally Renamed",
                "Native Instruments",
                "7.0",
            ),
            desc(
                PluginFormat::Au,
                "aumu:Kont:NInS",
                "Kontakt AU",
                "Native Instruments",
                "7.0",
            ),
        ];
        let c = candidates(&m, &installed, &BTreeMap::new());
        assert_eq!(c.len(), 2);
        assert!(
            c.iter()
                .all(|c| c.rule == MatchRule::SameId && (c.confidence - 0.95).abs() < 1e-6)
        );
        // VST3 ranks before AU at equal confidence.
        assert_eq!(c[0].format, PluginFormat::Vst3);
        // Another format: the state is not carried, parameters map by index.
        assert!(matches!(c[0].plan, ParamPlan::Map { .. }));
        // A different vendor with the same 4-char code is not the same plugin.
        let other = [desc(
            PluginFormat::Au,
            "aufx:Kont:Acme",
            "Konto",
            "Acme",
            "1",
        )];
        assert!(candidates(&m, &other, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn vst2_to_vst3_by_name_and_vendor() {
        let params = [sp(0, None), sp(1, None)];
        let m = missing(
            PluginFormat::Vst2,
            "58667358",
            "Serum (x64)",
            "Xfer Records",
            &params,
        );
        let installed = [
            desc(
                PluginFormat::Vst3,
                "56535458667358736572756D00000000",
                "Serum",
                "Xfer Records",
                "1.36",
            ),
            desc(
                PluginFormat::Vst3,
                "00000000000000000000000000000001",
                "Serum FX",
                "Xfer Records",
                "1.36",
            ),
        ];
        let c = candidates(&m, &installed, &BTreeMap::new());
        // Its VST3 id carries the VST2 id: SameId beats the name.
        assert_eq!(c[0].name, "Serum");
        assert_eq!(c[0].rule, MatchRule::SameId);
        // Same family without names: by index.
        assert_eq!(
            c[0].plan,
            ParamPlan::Map {
                map: vec![ParamMap { from: 0, to: 0 }, ParamMap { from: 1, to: 1 }]
            }
        );
        // A VST3 build with an unrelated id: by name.
        let installed = [desc(
            PluginFormat::Vst3,
            "ABCDEF00ABCDEF00ABCDEF00ABCDEF00",
            "Serum",
            "Xfer",
            "1.36",
        )];
        let c = candidates(&m, &installed, &BTreeMap::new());
        assert_eq!(c[0].rule, MatchRule::SameName);
        assert!((c[0].confidence - 0.9).abs() < 1e-6);
        assert_eq!(c[0].reason, "Same name and vendor in VST3");
    }

    #[test]
    fn params_map_by_name() {
        let source = [
            sp(10, Some("Filter Cutoff")),
            sp(11, Some("Resonance")),
            sp(12, Some("Drive")),
        ];
        let target = [
            ParamName {
                id: 3,
                name: "filter_cutoff".into(),
            },
            ParamName {
                id: 4,
                name: "RESONANCE".into(),
            },
            ParamName {
                id: 5,
                name: "Mix".into(),
            },
        ];
        let (map, lost) = map_params(&source, Some(&target), false);
        assert_eq!(
            map,
            vec![ParamMap { from: 10, to: 3 }, ParamMap { from: 11, to: 4 }]
        );
        assert_eq!(lost, vec!["Drive".to_string()]);

        // Through `candidates`, with the target's names known.
        let m = missing(
            PluginFormat::Vst3,
            "0123456789ABCDEF0123456789ABCDEF",
            "Pro-Q 3",
            "FabFilter",
            &source,
        );
        let installed = [desc(
            PluginFormat::Clap,
            "com.fabfilter.pro-q.4",
            "Pro-Q 4",
            "FabFilter",
            "4.0",
        )];
        let known = BTreeMap::from([(
            (PluginFormat::Clap, "com.fabfilter.pro-q.4".to_string()),
            target.to_vec(),
        )]);
        let c = candidates(&m, &installed, &known);
        assert_eq!(c[0].rule, MatchRule::SameFamily);
        assert_eq!(
            c[0].plan,
            ParamPlan::Map {
                map: vec![ParamMap { from: 10, to: 3 }, ParamMap { from: 11, to: 4 }]
            }
        );
        assert_eq!(c[0].lost.len(), 2, "{:?}", c[0].lost);
        assert_eq!(c[0].lost[1], "Drive");

        // Unrelated plugin without names: everything is lost.
        let (map, lost) = map_params(&[sp(1, None)], None, false);
        assert!(map.is_empty());
        assert_eq!(lost, vec!["parameter 1".to_string()]);
        // Same family, the target's ids known: ids it lacks are lost.
        let (map, lost) = map_params(
            &[sp(1, None), sp(9, None)],
            Some(&[ParamName {
                id: 1,
                name: "A".into(),
            }]),
            true,
        );
        assert_eq!(map, vec![ParamMap { from: 1, to: 1 }]);
        assert_eq!(lost, vec!["parameter 9".to_string()]);
    }

    #[test]
    fn fuzzy_names_with_a_threshold() {
        let m = missing(
            PluginFormat::Vst3,
            "0123456789ABCDEF0123456789ABCDEF",
            "ValhallaVintageVerb",
            "",
            &[],
        );
        let installed = [
            desc(
                PluginFormat::Clap,
                "com.valhalla.vv",
                "Valhalla Vintage Verb",
                "Valhalla DSP",
                "3",
            ),
            desc(
                PluginFormat::Clap,
                "com.valhalla.delay",
                "ValhallaDelay",
                "Valhalla DSP",
                "3",
            ),
            desc(
                PluginFormat::Clap,
                "org.surge",
                "Surge XT",
                "Surge Synth Team",
                "1",
            ),
        ];
        let c = candidates(&m, &installed, &BTreeMap::new());
        assert_eq!(c[0].name, "Valhalla Vintage Verb");
        // Unknown vendor: a little less sure.
        assert!((c[0].confidence - 0.8).abs() < 1e-6, "{}", c[0].confidence);
        assert!(c.iter().all(|c| c.name != "Surge XT"));

        let m = missing(
            PluginFormat::Vst3,
            "0123456789ABCDEF0123456789ABCDEF",
            "Vintage Verb Deluxe",
            "Valhalla DSP",
            &[],
        );
        let c = candidates(&m, &installed[..1], &BTreeMap::new());
        assert!(c.is_empty() || c[0].rule == MatchRule::Similar);
        let m = missing(
            PluginFormat::Vst3,
            "0123456789ABCDEF0123456789ABCDEF",
            "Valhala VintageVerb",
            "Valhalla DSP",
            &[],
        );
        let c = candidates(&m, &installed[..1], &BTreeMap::new());
        assert_eq!(c[0].rule, MatchRule::Similar);
        assert!(c[0].confidence < LOW_CONFIDENCE);
    }

    #[test]
    fn different_vendors_never_match_by_family() {
        let m = missing(
            PluginFormat::Vst3,
            "0123456789ABCDEF0123456789ABCDEF",
            "Compressor",
            "Acme",
            &[],
        );
        let installed = [desc(
            PluginFormat::Vst3,
            "1123456789ABCDEF0123456789ABCDEF",
            "Compressor",
            "Other Co",
            "1",
        )];
        assert!(candidates(&m, &installed, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn ties_prefer_newer_versions_and_installed_ones_are_skipped() {
        let m = missing(
            PluginFormat::Vst3,
            "0123456789ABCDEF0123456789ABCDEF",
            "Kontakt 6",
            "NI",
            &[],
        );
        let installed = [
            desc(
                PluginFormat::Vst3,
                "0123456789ABCDEF0123456789ABCDEF",
                "Kontakt 6",
                "NI",
                "6",
            ),
            desc(
                PluginFormat::Vst3,
                "A0000000000000000000000000000000",
                "Kontakt 7",
                "NI",
                "7.10.2",
            ),
            desc(
                PluginFormat::Vst3,
                "B0000000000000000000000000000000",
                "Kontakt 8",
                "NI",
                "8.2",
            ),
        ];
        let c = candidates(&m, &installed, &BTreeMap::new());
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].name, "Kontakt 8");
        assert_eq!(c[1].name, "Kontakt 7");
    }
}
