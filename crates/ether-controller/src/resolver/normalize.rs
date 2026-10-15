//! Plugin name and vendor normalization, and fuzzy name similarity (pure, unit-tested).
//!
//! - [`family`]: the name without version numbers, architecture, format and channel
//!   suffixes ("Kontakt 7" → "kontakt", "Serum (x64)" → "serum", "Pro-Q 3 (VST3)" → "pro q").
//! - [`full`]: the same, keeping version numbers ("Pro-Q 3" → "pro q 3").
//! - [`vendor`]: lowercase, punctuation and company suffixes removed, aliases folded
//!   ("Native Instruments GmbH" = "NI" → "native instruments").
//! - [`similarity`]: Jaro-Winkler on the compact (alphanumeric only) forms.

/// Words dropped from names anywhere (architecture, format, channel layout, wrappers).
const NOISE: &[&str] = &[
    "x64",
    "x86",
    "64bit",
    "32bit",
    "64",
    "32",
    "bit",
    "vst",
    "vst2",
    "vst3",
    "vsti",
    "vst3i",
    "au",
    "aax",
    "clap",
    "mono",
    "stereo",
    "arm64",
    "universal",
    "x86_64",
];

/// Company suffixes dropped from vendor names.
const COMPANY: &[&str] = &[
    "gmbh",
    "inc",
    "llc",
    "ltd",
    "limited",
    "corp",
    "corporation",
    "co",
    "sa",
    "sas",
    "ag",
    "bv",
    "ab",
    "oy",
    "srl",
    "pty",
    "plc",
    "kg",
    "software",
    "technologies",
    "technology",
    "media",
    "labs",
    "audio",
    "dsp",
    "records",
    "music",
    "the",
];

/// Vendor aliases (normalized form → canonical).
const VENDOR_ALIASES: &[(&str, &str)] = &[
    ("ni", "native instruments"),
    ("native instrument", "native instruments"),
    ("xfer", "xfer"),
    ("u he", "u he"),
    ("uhe", "u he"),
    ("ua", "universal"),
    ("uadx", "universal"),
    ("universal audio", "universal"),
    ("plugin alliance", "brainworx"),
    ("bx", "brainworx"),
    ("fabfilter instruments", "fabfilter"),
    ("valhalladsp", "valhalla"),
    ("izotope", "izotope"),
    ("waves", "waves"),
    ("steinberg media", "steinberg"),
    ("spectrasonics", "spectrasonics"),
    ("arturia", "arturia"),
    ("output", "output"),
    ("tal", "tal"),
    ("togu", "tal"),
    ("cockos", "cockos"),
];

/// Lowercase words of `s` (letters and digits; everything else separates). A digit run
/// glued to letters stays one word ("v2", "mk2", "x64").
pub fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// A version-like word: `7`, `12`, `v2`, `2.1` (split already), `mk2`, `mkii`, `ii`, `iii`,
/// `iv`, a year-less number of at most two digits. Longer numbers (`1176`, `160`) are model
/// numbers and stay.
fn is_version(w: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    (digits(w) && w.len() <= 2)
        || (w.starts_with('v') && digits(&w[1..]))
        || (w.starts_with("mk") && (digits(&w[2..]) || matches!(&w[2..], "i" | "ii" | "iii")))
        || matches!(w, "ii" | "iii" | "iv")
}

fn is_noise(w: &str) -> bool {
    NOISE.contains(&w)
}

/// The name without version, architecture, format and channel words.
pub fn family(name: &str) -> String {
    let ws = words(name);
    let kept: Vec<&str> = ws
        .iter()
        .map(String::as_str)
        .filter(|w| !is_version(w) && !is_noise(w))
        .collect();
    if kept.is_empty() {
        // "64" or "VST": keep what there is rather than matching everything.
        return ws.join(" ");
    }
    kept.join(" ")
}

/// The name without architecture, format and channel words (versions kept).
pub fn full(name: &str) -> String {
    let ws = words(name);
    let kept: Vec<&str> = ws
        .iter()
        .map(String::as_str)
        .filter(|w| !is_noise(w))
        .collect();
    if kept.is_empty() {
        return ws.join(" ");
    }
    kept.join(" ")
}

/// Letters and digits only (spacing and punctuation differ between formats:
/// "Valhalla VintageVerb" / "ValhallaVintageVerb").
pub fn compact(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Normalized vendor ("" when unknown).
pub fn vendor(v: &str) -> String {
    let ws = words(v);
    let kept: Vec<&str> = ws
        .iter()
        .map(String::as_str)
        .filter(|w| !COMPANY.contains(w))
        .collect();
    let joined = if kept.is_empty() {
        ws.join(" ")
    } else {
        kept.join(" ")
    };
    for (alias, canon) in VENDOR_ALIASES {
        if joined == *alias || compact(&joined) == compact(alias) {
            return (*canon).to_string();
        }
    }
    joined
}

/// How two vendors compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VendorMatch {
    Same,
    /// One side is unknown.
    Unknown,
    Different,
}

pub fn vendor_match(a: &str, b: &str) -> VendorMatch {
    let (a, b) = (vendor(a), vendor(b));
    if a.is_empty() || b.is_empty() {
        VendorMatch::Unknown
    } else if a == b || compact(&a) == compact(&b) {
        VendorMatch::Same
    } else {
        VendorMatch::Different
    }
}

/// Jaro-Winkler similarity (0..=1) of the compact forms of two normalized names.
pub fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = compact(a).chars().collect();
    let b: Vec<char> = compact(b).chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut a_hit = vec![false; a.len()];
    let mut b_hit = vec![false; b.len()];
    let mut matches = 0usize;
    for (i, ca) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        for j in lo..hi {
            if !b_hit[j] && b[j] == *ca {
                a_hit[i] = true;
                b_hit[j] = true;
                matches += 1;
                break;
            }
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let mut transpositions = 0usize;
    let mut j = 0usize;
    for (i, ca) in a.iter().enumerate() {
        if !a_hit[i] {
            continue;
        }
        while !b_hit[j] {
            j += 1;
        }
        if *ca != b[j] {
            transpositions += 1;
        }
        j += 1;
    }
    let m = matches as f64;
    let jaro =
        (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions as f64 / 2.0) / m) / 3.0;
    let prefix = a.iter().zip(&b).take(4).take_while(|(x, y)| x == y).count() as f64;
    jaro + prefix * 0.1 * (1.0 - jaro)
}

/// Normalized parameter name for by-name mapping ("Filter Cutoff" = "filter_cutoff").
pub fn param(name: &str) -> String {
    let c = compact(&name.to_lowercase());
    // Common synonyms across devices and DAWs.
    match c.as_str() {
        "drywet" | "wet" | "drywetmix" | "mixdrywet" | "mix" => "mix".to_owned(),
        "outputgain" | "output" | "outgain" | "gainout" => "output".to_owned(),
        "inputgain" | "input" | "ingain" | "gainin" => "input".to_owned(),
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_drop_versions_and_suffixes() {
        assert_eq!(family("Kontakt 7"), "kontakt");
        assert_eq!(family("Kontakt"), "kontakt");
        assert_eq!(family("Serum (x64)"), "serum");
        assert_eq!(family("Serum 2"), "serum");
        assert_eq!(family("Pro-Q 3 (VST3)"), "pro q");
        assert_eq!(family("FabFilter Pro-Q 4"), "fabfilter pro q");
        assert_eq!(family("Massive X"), "massive x");
        assert_eq!(family("Diva(x64) (Stereo)"), "diva");
        assert_eq!(family("Ozone 9 Equalizer"), "ozone equalizer");
        assert_eq!(family("Neve 1073 MkII"), "neve 1073");
        assert_eq!(family("Omnisphere v2"), "omnisphere");
        // Model numbers stay.
        assert_eq!(family("UAD 1176 Rev E"), "uad 1176 rev e");
        // Nothing but noise: kept as is.
        assert_eq!(family("64"), "64");
    }

    #[test]
    fn full_names_keep_versions() {
        assert_eq!(full("Kontakt 7 (VST3)"), "kontakt 7");
        assert_eq!(full("Serum x64"), "serum");
    }

    #[test]
    fn vendors_fold_suffixes_and_aliases() {
        assert_eq!(vendor("Native Instruments GmbH"), "native instruments");
        assert_eq!(vendor("NI"), "native instruments");
        assert_eq!(vendor("Xfer Records"), "xfer");
        assert_eq!(vendor("u-he"), "u he");
        assert_eq!(vendor("FabFilter"), "fabfilter");
        assert_eq!(vendor("Valhalla DSP, LLC"), "valhalla");
        assert_eq!(vendor("ValhallaDSP"), "valhalla");
        assert_eq!(vendor_match("Native Instruments", "NI"), VendorMatch::Same);
        assert_eq!(vendor_match("", "NI"), VendorMatch::Unknown);
        assert_eq!(vendor_match("Xfer Records", "u-he"), VendorMatch::Different);
    }

    #[test]
    fn similarity_is_jaro_winkler() {
        assert_eq!(similarity("serum", "serum"), 1.0);
        assert!(similarity("valhalla vintage verb", "valhallavintageverb") > 0.99);
        assert!(similarity("vintageverb", "vintage verb") > 0.99);
        assert!(similarity("ozone equalizer", "ozone eq") > 0.85);
        assert!(similarity("serum", "diva") < 0.6);
        assert_eq!(similarity("", "x"), 0.0);
        // Symmetric.
        let (a, b) = (similarity("pro q", "pro l"), similarity("pro l", "pro q"));
        assert!((a - b).abs() < 1e-12);
    }
}
