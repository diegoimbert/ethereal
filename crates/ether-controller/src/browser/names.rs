//! Tempo and key from file names (`"Break 120.wav"`, `"Bass_Am_128bpm.wav"`,
//! `"Pad C#maj.wav"`). Cheap and conservative: a bare number counts as a tempo only next to
//! a musical context (a "loop" in the name or path, an explicit key or a "bpm" token).

/// Explicit tempos (`128bpm`, `128 BPM`) in this range are accepted.
const EXPLICIT_BPM: std::ops::RangeInclusive<f64> = 40.0..=300.0;
/// Bare numbers in this range are accepted with a musical context.
const BARE_BPM: std::ops::RangeInclusive<f64> = 60.0..=200.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NameInfo {
    pub bpm: Option<f64>,
    /// `"A minor"`, `"C# major"`, `"Eb minor"`.
    pub key: Option<String>,
}

/// Parse `stem` (file name without extension); `path` is the item's relative path (for the
/// "loop" context).
pub(crate) fn parse(stem: &str, path: &str) -> NameInfo {
    let tokens: Vec<&str> = stem
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '#'))
        .filter(|t| !t.is_empty())
        .collect();
    let key = find_key(&tokens);
    let mut bpm = None;
    let mut bare = None;
    for (i, t) in tokens.iter().enumerate() {
        let lower = t.to_ascii_lowercase();
        if let Some(n) = lower.strip_suffix("bpm").and_then(number)
            && EXPLICIT_BPM.contains(&n)
        {
            bpm = Some(n);
            break;
        }
        if let Some(n) = number(&lower) {
            let next_is_bpm = tokens
                .get(i + 1)
                .is_some_and(|n| n.eq_ignore_ascii_case("bpm"));
            if next_is_bpm && EXPLICIT_BPM.contains(&n) {
                bpm = Some(n);
                break;
            }
            if BARE_BPM.contains(&n) {
                bare = Some(n);
            }
        }
    }
    if bpm.is_none() && bare.is_some() {
        let context = key.is_some()
            || path.to_ascii_lowercase().contains("loop")
            || tokens.iter().any(|t| t.eq_ignore_ascii_case("bpm"));
        if context {
            bpm = bare;
        }
    }
    NameInfo { bpm, key }
}

/// A plain decimal integer of 2-3 digits.
fn number(s: &str) -> Option<f64> {
    if (2..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

fn find_key(tokens: &[&str]) -> Option<String> {
    for (i, t) in tokens.iter().enumerate() {
        let mut chars = t.chars();
        let Some(letter) = chars.next().filter(|c| ('A'..='G').contains(c)) else {
            continue;
        };
        let rest = chars.as_str();
        let (accidental, suffix) = match rest.chars().next() {
            Some('#') => ("#", &rest[1..]),
            // A flat is a lowercase `b` followed by a mode or the end ("Bb", "Ebm", "Abmaj").
            Some('b') if rest.len() == 1 || mode(&rest[1..]).is_some() => ("b", &rest[1..]),
            _ => ("", rest),
        };
        let mode = if suffix.is_empty() {
            tokens.get(i + 1).and_then(|n| match mode(n) {
                // A lone "m" token is too ambiguous.
                Some(m) if n.len() > 1 => Some(m),
                _ => None,
            })
        } else {
            mode(suffix)
        };
        if let Some(mode) = mode {
            return Some(format!("{letter}{accidental} {mode}"));
        }
    }
    None
}

fn mode(s: &str) -> Option<&'static str> {
    if s == "m" {
        return Some("minor");
    }
    match s.to_ascii_lowercase().as_str() {
        "min" | "minor" => Some("minor"),
        "maj" | "major" => Some("major"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> (Option<f64>, Option<String>) {
        let i = parse(name, name);
        (i.bpm, i.key)
    }

    #[test]
    fn tempos() {
        assert_eq!(p("Bass_128bpm").0, Some(128.0));
        assert_eq!(p("Bass 128 BPM").0, Some(128.0));
        assert_eq!(p("Break 120 loop").0, Some(120.0));
        assert_eq!(parse("Break 120", "Drums/Loops/Break 120.wav").bpm, Some(120.0));
        // No context: a bare number is not a tempo.
        assert_eq!(parse("Snare 100", "Drums/Snare 100.wav").bpm, None);
        assert_eq!(p("Kick 01").0, None);
        assert_eq!(p("808 Loop").0, None);
        assert_eq!(p("Pad Am 90").0, Some(90.0));
    }

    #[test]
    fn keys() {
        assert_eq!(p("Bass_Am_128bpm").1.as_deref(), Some("A minor"));
        assert_eq!(p("Pad C#maj").1.as_deref(), Some("C# major"));
        assert_eq!(p("Lead Ebm").1.as_deref(), Some("Eb minor"));
        assert_eq!(p("Keys F minor").1.as_deref(), Some("F minor"));
        assert_eq!(p("Chord Bb").1, None);
        assert_eq!(p("Take A").1, None);
        assert_eq!(p("Am I here").1.as_deref(), Some("A minor"));
        assert_eq!(p("Dmaj7 stab").1, None);
        assert_eq!(p("Cmin").1.as_deref(), Some("C minor"));
        assert_eq!(p("Gb major").1.as_deref(), Some("Gb major"));
        assert_eq!(p("Big Room").1, None);
        assert_eq!(p("Bass").1, None);
    }
}
