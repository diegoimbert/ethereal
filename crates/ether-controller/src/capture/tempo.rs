//! Tempo inference for a phrase played while stopped (CONTRACTS.md §13.4).
//!
//! The first onset is the first downbeat. Every tempo in [`MIN_BPM`]..=[`MAX_BPM`] (steps of
//! [`STEP_BPM`]) is scored by how well the onsets sit on its metrical grid: an onset on a beat
//! costs nothing, on an eighth / triplet / sixteenth a little more, plus its timing deviation
//! from that grid point. Doubling a tempo never fits worse (every onset moves up a grid
//! level), so a prior pulls towards 120 bpm (`(log2(bpm / 120))²`): of two equally good
//! readings the one nearer 120 wins (ties prefer 120). The best tempo is then refined by a
//! least-squares fit of the onsets to their grid positions.

/// Slowest inferred tempo.
pub const MIN_BPM: f64 = 60.0;
/// Fastest inferred tempo.
pub const MAX_BPM: f64 = 180.0;
/// Search resolution.
const STEP_BPM: f64 = 0.1;
/// Onsets closer than this (seconds) are one onset (a chord).
const CHORD_SECONDS: f64 = 0.04;
/// Timing deviation (seconds) that costs as much as a whole grid level.
const JITTER_SECONDS: f64 = 0.04;
/// At most this many onsets are scored (the start of the phrase sets the tempo).
const MAX_ONSETS: usize = 512;
/// Grid levels in beats with their cost: beat, eighth, triplet eighth, sixteenth.
const LEVELS: [(f64, f64); 4] = [(1.0, 0.0), (0.5, 0.15), (1.0 / 3.0, 0.3), (0.25, 0.35)];
/// Two scores this close are a tie.
const TIE: f64 = 1e-9;
/// Weight of the prior towards 120 bpm (per squared octave).
const PRIOR_WEIGHT: f64 = 0.4;
/// Cost of the share of beats (first to last onset) with no onset on them: a reading that
/// leaves beats empty (e.g. quarter notes read 3/2 faster as off-beats) is less likely.
const EMPTY_BEAT_WEIGHT: f64 = 0.3;
/// An onset this close to a beat (in beats) sounds it.
const ON_BEAT: f64 = 0.125;
/// Most an onset may cost (a stray note does not dominate).
const MAX_ONSET_COST: f64 = 1.0;

/// Onsets (seconds, any order) → inferred tempo, or `None` with fewer than two distinct
/// onsets (nothing to infer from).
pub fn infer_bpm(onsets: &[f64]) -> Option<f64> {
    let onsets = distinct_onsets(onsets);
    if onsets.len() < 2 {
        return None;
    }
    let t0 = onsets[0];
    let rel: Vec<f64> = onsets.iter().map(|t| t - t0).collect();
    let steps = ((MAX_BPM - MIN_BPM) / STEP_BPM).round() as usize;
    let mut best: Option<(f64, f64)> = None;
    for i in 0..=steps {
        let bpm = MIN_BPM + i as f64 * STEP_BPM;
        let cost = score(&rel, bpm);
        let better = match best {
            None => true,
            Some((b, c)) => {
                cost < c - TIE
                    || (cost <= c + TIE && (bpm - 120.0).abs() < (b - 120.0).abs() - STEP_BPM / 2.0)
            }
        };
        if better {
            best = Some((bpm, cost));
        }
    }
    let (bpm, _) = best?;
    Some(refine(&rel, bpm))
}

/// Sorted onsets with chords merged (the first note of a chord counts), at most
/// [`MAX_ONSETS`].
fn distinct_onsets(onsets: &[f64]) -> Vec<f64> {
    let mut sorted: Vec<f64> = onsets.iter().copied().filter(|t| t.is_finite()).collect();
    sorted.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::new();
    for t in sorted {
        if out.last().is_none_or(|last| t - last > CHORD_SECONDS) {
            out.push(t);
            if out.len() == MAX_ONSETS {
                break;
            }
        }
    }
    out
}

/// The grid point (beats) and cost of one onset `t` (seconds from the first) at `bpm`.
fn place(t: f64, bpm: f64) -> (f64, f64) {
    let spb = 60.0 / bpm;
    let x = t / spb;
    let mut best = (x.round(), MAX_ONSET_COST);
    for (grid, level_cost) in LEVELS {
        let at = (x / grid).round() * grid;
        let dev = (x - at).abs() * spb / JITTER_SECONDS;
        let cost = (level_cost + dev * dev).min(MAX_ONSET_COST);
        if cost < best.1 {
            best = (at, cost);
        }
    }
    best
}

fn score(rel: &[f64], bpm: f64) -> f64 {
    let fit = rel.iter().map(|&t| place(t, bpm).1).sum::<f64>() / rel.len() as f64;
    let spb = 60.0 / bpm;
    let beats = (rel.last().copied().unwrap_or(0.0) / spb + ON_BEAT).floor() as usize + 1;
    let mut sounded = vec![false; beats];
    for &t in rel {
        let x = t / spb;
        let k = x.round();
        if (x - k).abs() <= ON_BEAT
            && let Some(s) = sounded.get_mut(k as usize)
        {
            *s = true;
        }
    }
    let empty = sounded.iter().filter(|s| !**s).count() as f64 / beats as f64;
    let octaves = (bpm / 120.0).log2();
    fit + EMPTY_BEAT_WEIGHT * empty + PRIOR_WEIGHT * octaves * octaves
}

/// Least-squares fit `t = a + spb · k` of the onsets to their grid positions `k` at `bpm`,
/// rounded to 0.01 bpm and kept in range (the coarse `bpm` when the fit is degenerate).
fn refine(rel: &[f64], bpm: f64) -> f64 {
    let ks: Vec<f64> = rel.iter().map(|&t| place(t, bpm).0).collect();
    let n = ks.len() as f64;
    let mk = ks.iter().sum::<f64>() / n;
    let mt = rel.iter().sum::<f64>() / n;
    let (mut cov, mut var) = (0.0, 0.0);
    for (k, t) in ks.iter().zip(rel) {
        cov += (k - mk) * (t - mt);
        var += (k - mk) * (k - mk);
    }
    if var <= 0.0 || cov <= 0.0 {
        return bpm;
    }
    let fitted = 60.0 / (cov / var);
    // The fit only nudges the search result (never another metrical reading).
    let fitted = if (fitted - bpm).abs() <= STEP_BPM {
        fitted
    } else {
        bpm
    };
    ((fitted * 100.0).round() / 100.0).clamp(MIN_BPM, MAX_BPM)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quarters(bpm: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 60.0 / bpm).collect()
    }

    #[test]
    fn steady_quarters_give_their_tempo() {
        for bpm in [85.0, 90.0, 100.0, 110.0, 128.0, 140.0, 160.0, 170.0] {
            let got = infer_bpm(&quarters(bpm, 16)).unwrap();
            assert!((got - bpm).abs() < 0.05, "{bpm} → {got}");
        }
    }

    #[test]
    fn ambiguous_pulse_prefers_120() {
        // 1 s apart: 60, 120 and 180 all fit exactly.
        assert_eq!(infer_bpm(&quarters(60.0, 8)), Some(120.0));
        assert_eq!(infer_bpm(&quarters(120.0, 8)), Some(120.0));
        // A plain pulse reads in the octave nearest 120.
        let got = infer_bpm(&quarters(70.0, 8)).unwrap();
        assert!((got - 140.0).abs() < 0.05, "{got}");
    }

    #[test]
    fn eighth_note_grooves_are_not_read_double_time() {
        // 90 bpm, beats and eighths: 180 fits as well (all beats) but is further from 120.
        let spb = 60.0 / 90.0;
        let beats = [0.0, 0.5, 1.0, 2.0, 2.5, 3.0, 4.0, 4.5, 5.0, 6.0, 6.5, 7.0];
        let onsets: Vec<f64> = beats.iter().map(|b| b * spb).collect();
        let got = infer_bpm(&onsets).unwrap();
        assert!((got - 90.0).abs() < 0.05, "{got}");
    }

    #[test]
    fn eighths_and_rests_keep_the_beat() {
        // 100 bpm: x . x x | x . . x | ... (eighth-note rhythm with rests).
        let spb = 0.6;
        let beats = [0.0, 1.0, 1.5, 2.0, 3.5, 4.0, 5.0, 5.5, 6.0, 7.5, 8.0];
        let onsets: Vec<f64> = beats.iter().map(|b| b * spb).collect();
        let got = infer_bpm(&onsets).unwrap();
        assert!((got - 100.0).abs() < 0.05, "{got}");
    }

    #[test]
    fn human_jitter_is_tolerated() {
        // 96 bpm quarters with ±12 ms deterministic jitter.
        let jitter = [
            0.0, 0.012, -0.008, 0.005, -0.012, 0.009, -0.004, 0.011, -0.01, 0.003,
        ];
        let onsets: Vec<f64> = (0..24)
            .map(|i| i as f64 * 0.625 + jitter[i % jitter.len()])
            .collect();
        let got = infer_bpm(&onsets).unwrap();
        assert!((got - 96.0).abs() < 0.5, "{got}");
    }

    #[test]
    fn chords_count_once_and_one_onset_infers_nothing() {
        assert_eq!(infer_bpm(&[1.0, 1.01, 1.02]), None);
        assert_eq!(infer_bpm(&[]), None);
        let mut chords = quarters(110.0, 8);
        chords.extend(quarters(110.0, 8).iter().map(|t| t + 0.01));
        let got = infer_bpm(&chords).unwrap();
        assert!((got - 110.0).abs() < 0.05, "{got}");
    }

    #[test]
    fn result_stays_in_range() {
        // 40 bpm quarters (1.5 s): read at a faster multiple inside the range.
        let got = infer_bpm(&quarters(40.0, 8)).unwrap();
        assert!((MIN_BPM..=MAX_BPM).contains(&got), "{got}");
        // 240 bpm quarters: half time.
        let got = infer_bpm(&quarters(240.0, 16)).unwrap();
        assert!((got - 120.0).abs() < 0.05, "{got}");
    }
}
