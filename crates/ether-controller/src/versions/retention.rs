//! Which autosave versions to keep (CONTRACTS.md §13.11): a pure, deterministic thinning
//! of the project's unnamed autosave versions into at most `MAX_AUTOSAVE_VERSIONS` (30):
//!
//! 1. **Recent**: the newest version, then walking back, every version at least
//!    `VERSION_INTERVAL_MS` (1 min) older than the last one picked, up to
//!    `RECENT_AUTOSAVE_VERSIONS` (5).
//! 2. **Hourly**: of the others, the first (oldest) version of each clock hour (UTC hours of
//!    the epoch) for the `HOURLY_AUTOSAVE_VERSIONS` (5) newest hours that have one.
//! 3. **Spread**: of the others, the oldest version in each of `SPREAD_AUTOSAVE_VERSIONS`
//!    (20) geometric age buckets: boundaries `SPREAD_MIN_AGE_MS · rⁱ` for i = 1..19 with
//!    `r = (SPREAD_MAX_AGE_MS / SPREAD_MIN_AGE_MS)^(1/20)` ≈ 1.44 (6 h to 1 year: 8.6 h,
//!    12.4 h, 17.9 h, 1.1 d, 1.5 d, 2.2 d, ... 254 d); the first bucket is open towards 0,
//!    the last towards infinity.
//! 4. **Fill**: while fewer than 30 are kept and others remain (a young project, or gaps in
//!    the history leave buckets empty), keep the one farthest from every kept version in
//!    log-age (`ln(age + 1 min)`), the older one on a tie, so exactly
//!    `min(30, count)` are kept.
//!
//! Hourly and spread tiers keep the *oldest* of their bin: the bins of an hour never change,
//! and a version that is the oldest of its age bucket stays the oldest as it ages into the
//! next one, so tier picks survive from one pruning to the next (with the newest, every
//! new version would replace the previous pick and history would never get old). It also
//! means the project's oldest autosave version is never pruned.
//!
//! A version dated after `now` (the clock went backwards) counts as written at `now`,
//! older than one actually written at `now`: the version just written is always the newest
//! and is never pruned.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::versions::{
    HOURLY_AUTOSAVE_VERSIONS, MAX_AUTOSAVE_VERSIONS, RECENT_AUTOSAVE_VERSIONS,
    SPREAD_AUTOSAVE_VERSIONS, SPREAD_MAX_AGE_MS, SPREAD_MIN_AGE_MS, VERSION_INTERVAL_MS, VersionId,
    VersionInfo,
};

const HOUR_MS: u64 = 3_600_000;

/// One candidate: its effective time (`min(created, now)`) and identity.
#[derive(Clone, Copy)]
struct Stamp<'a> {
    t: u64,
    created: u64,
    id: &'a str,
}

/// Newest first; among equal effective times, the one really written then first.
fn newest_first(a: &Stamp, b: &Stamp) -> Ordering {
    b.t.cmp(&a.t)
        .then(a.created.cmp(&b.created))
        .then(b.id.cmp(a.id))
}

/// The oldest of two (for the "oldest of its bin" picks).
fn older<'a>(a: Stamp<'a>, b: Stamp<'a>) -> Stamp<'a> {
    if newest_first(&a, &b) == Ordering::Greater {
        a
    } else {
        b
    }
}

/// Upper age boundary of spread bucket `i` (0..19): `SPREAD_MIN_AGE_MS · r^(i+1)`.
fn spread_boundary(i: usize) -> u64 {
    let r = (SPREAD_MAX_AGE_MS as f64 / SPREAD_MIN_AGE_MS as f64)
        .powf(1.0 / SPREAD_AUTOSAVE_VERSIONS as f64);
    (SPREAD_MIN_AGE_MS as f64 * r.powi(i as i32 + 1)) as u64
}

/// Spread bucket (0..`SPREAD_AUTOSAVE_VERSIONS`) of a version `age` ms old.
fn spread_bucket(age: u64) -> usize {
    (0..SPREAD_AUTOSAVE_VERSIONS - 1)
        .take_while(|&i| age >= spread_boundary(i))
        .count()
}

fn log_age(now: u64, s: &Stamp) -> f64 {
    ((now - s.t + VERSION_INTERVAL_MS) as f64).ln()
}

/// The autosave versions (of `versions`, all candidates) to keep at `now` (the module
/// docs). Deterministic: the same set whatever the input order.
pub fn retain(versions: &[VersionInfo], now: u64) -> BTreeSet<VersionId> {
    let mut all: Vec<Stamp> = versions
        .iter()
        .map(|v| Stamp {
            t: v.created_ms.min(now),
            created: v.created_ms,
            id: v.id.as_str(),
        })
        .collect();
    all.sort_by(newest_first);
    all.dedup_by(|a, b| a.id == b.id);
    let mut kept: BTreeSet<&str> = BTreeSet::new();

    // 1. Recent.
    let mut last: Option<u64> = None;
    for s in &all {
        if kept.len() == RECENT_AUTOSAVE_VERSIONS {
            break;
        }
        if last.is_none_or(|l| l.saturating_sub(s.t) >= VERSION_INTERVAL_MS) {
            kept.insert(s.id);
            last = Some(s.t);
        }
    }

    // 2. Hourly: the oldest of each clock hour, newest hours first.
    let mut hours: BTreeMap<u64, Stamp> = BTreeMap::new();
    for s in all.iter().filter(|s| !kept.contains(s.id)) {
        hours
            .entry(s.t / HOUR_MS)
            .and_modify(|o| *o = older(*o, *s))
            .or_insert(*s);
    }
    for s in hours.values().rev().take(HOURLY_AUTOSAVE_VERSIONS) {
        kept.insert(s.id);
    }

    // 3. Spread: the oldest of each geometric age bucket.
    let mut buckets: BTreeMap<usize, Stamp> = BTreeMap::new();
    for s in all.iter().filter(|s| !kept.contains(s.id)) {
        buckets
            .entry(spread_bucket(now - s.t))
            .and_modify(|o| *o = older(*o, *s))
            .or_insert(*s);
    }
    for s in buckets.values() {
        kept.insert(s.id);
    }

    // 4. Fill: farthest in log-age from every kept version, older first on a tie.
    let mut kept_logs: Vec<f64> = all
        .iter()
        .filter(|s| kept.contains(s.id))
        .map(|s| log_age(now, s))
        .collect();
    while kept.len() < MAX_AUTOSAVE_VERSIONS.min(all.len()) {
        let mut best: Option<(f64, &Stamp)> = None;
        // Oldest first, so a strictly greater distance is needed to prefer a newer one.
        for s in all.iter().rev().filter(|s| !kept.contains(s.id)) {
            let l = log_age(now, s);
            let d = kept_logs
                .iter()
                .map(|k| (k - l).abs())
                .fold(f64::INFINITY, f64::min);
            if best.is_none_or(|(bd, _)| d > bd) {
                best = Some((d, s));
            }
        }
        let Some((_, s)) = best else { break };
        kept.insert(s.id);
        kept_logs.push(log_age(now, s));
    }

    kept.into_iter().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::versions::VersionKind;

    const MIN: u64 = 60_000;
    const DAY: u64 = 24 * HOUR_MS;
    /// A start on a clock-hour boundary, so the expectations read simply.
    const T0: u64 = 1_750_000_000_000 / HOUR_MS * HOUR_MS;

    fn v(ms: u64) -> VersionInfo {
        VersionInfo {
            id: format!("{ms}-autosave"),
            kind: VersionKind::Autosave,
            name: None,
            created_ms: ms,
            size: 1,
        }
    }

    /// Keeps the versions pruned like the controller does: one written at each of `times`,
    /// then pruned at that time. Returns the creation times kept, newest first.
    #[derive(Default)]
    struct Sim(Vec<VersionInfo>);

    impl Sim {
        fn write(&mut self, now: u64) {
            let mut ms = now;
            while self.0.iter().any(|x| x.created_ms == ms) {
                ms += 1;
            }
            self.0.push(v(ms));
            let keep = retain(&self.0, now);
            let before = self.0.len();
            self.0.retain(|x| keep.contains(&x.id));
            assert!(keep.contains(&format!("{ms}-autosave")), "the newest stays");
            assert_eq!(self.0.len(), before.min(MAX_AUTOSAVE_VERSIONS));
        }

        fn every(&mut self, from: u64, to: u64, step: u64) {
            let mut t = from;
            while t < to {
                self.write(t);
                t += step;
            }
        }

        fn times(&self) -> Vec<u64> {
            let mut t: Vec<u64> = self.0.iter().map(|x| x.created_ms).collect();
            t.sort_unstable_by(|a, b| b.cmp(a));
            t
        }
    }

    #[test]
    fn tiers_are_documented_sizes() {
        assert_eq!(MAX_AUTOSAVE_VERSIONS, 30);
        assert_eq!(spread_bucket(0), 0);
        assert_eq!(spread_bucket(SPREAD_MIN_AGE_MS), 0);
        assert_eq!(spread_bucket(9 * HOUR_MS), 1);
        assert_eq!(spread_bucket(2 * DAY), 5);
        assert_eq!(spread_bucket(10 * 365 * DAY), SPREAD_AUTOSAVE_VERSIONS - 1);
        let last = spread_boundary(SPREAD_AUTOSAVE_VERSIONS - 1);
        assert!(last.abs_diff(SPREAD_MAX_AGE_MS) < MIN, "{last}");
        for i in 1..SPREAD_AUTOSAVE_VERSIONS {
            assert!(spread_boundary(i) > spread_boundary(i - 1));
        }
    }

    #[test]
    fn few_versions_are_all_kept() {
        assert!(retain(&[], T0).is_empty());
        let vs: Vec<_> = (0..30).map(|i| v(T0 + i * 1000)).collect();
        assert_eq!(retain(&vs, T0 + 30_000).len(), 30);
        let vs: Vec<_> = (0..31).map(|i| v(T0 + i * 1000)).collect();
        assert_eq!(retain(&vs, T0 + 31_000).len(), 30);
    }

    #[test]
    fn input_order_does_not_matter() {
        let mut vs: Vec<_> = (0..200u64)
            .map(|i| v(T0 + (i * i * 7919) % (30 * DAY)))
            .collect();
        let now = T0 + 30 * DAY;
        let a = retain(&vs, now);
        vs.reverse();
        assert_eq!(retain(&vs, now), a);
        vs.rotate_left(77);
        assert_eq!(retain(&vs, now), a);
        assert_eq!(a.len(), 30);
    }

    #[test]
    fn steady_editing_for_a_day() {
        let mut sim = Sim::default();
        let end = T0 + DAY;
        sim.every(T0, end, MIN);
        let now = end - MIN;
        let t = sim.times();
        assert_eq!(t.len(), 30);
        // Recent: the last five minutes.
        assert_eq!(
            &t[..5],
            &[now, now - MIN, now - 2 * MIN, now - 3 * MIN, now - 4 * MIN]
        );
        // Hourly: the first version of each of the five newest clock hours.
        for h in 0..5 {
            let hour = now / HOUR_MS * HOUR_MS - h * HOUR_MS;
            assert!(t.contains(&hour), "hour -{h}");
        }
        // The oldest is never pruned; the rest spreads over the day.
        assert_eq!(*t.last().unwrap(), T0);
        let older_than = |age: u64| t.iter().filter(|&&x| now - x >= age).count();
        assert!(older_than(6 * HOUR_MS) >= 3, "{t:?}");
        assert!(older_than(12 * HOUR_MS) >= 2, "{t:?}");
    }

    #[test]
    fn steady_editing_for_weeks_spreads_logarithmically() {
        let mut sim = Sim::default();
        let end = T0 + 21 * DAY;
        // 1 min for the last day, 10 min before (keeps the test fast; same tiers).
        sim.every(T0, end - DAY, 10 * MIN);
        sim.every(end - DAY, end, MIN);
        let now = end - MIN;
        let t = sim.times();
        assert_eq!(t.len(), 30);
        assert_eq!(*t.last().unwrap(), T0, "the oldest stays");
        let ages: Vec<u64> = t.iter().map(|x| now - x).collect();
        // Every decade of age past a few hours holds versions.
        for (lo, hi) in [
            (0, 5 * MIN),
            (HOUR_MS, 5 * HOUR_MS),
            (6 * HOUR_MS, DAY),
            (DAY, 4 * DAY),
            (4 * DAY, 21 * DAY),
        ] {
            let n = ages.iter().filter(|&&a| a >= lo && a < hi).count();
            assert!(n >= 2, "{n} versions aged {lo}..{hi}: {ages:?}");
        }
        // No gap between kept versions older than 6 h is wider than ~2.5x the age.
        for w in ages.windows(2).filter(|w| w[0] >= 6 * HOUR_MS) {
            assert!(w[1] as f64 / w[0] as f64 <= 2.5, "{ages:?}");
        }
    }

    #[test]
    fn tier_picks_survive_later_prunings() {
        let mut sim = Sim::default();
        sim.every(T0, T0 + 3 * DAY, 5 * MIN);
        // The first version of each of the five newest hours stays while it is one of them.
        let now = T0 + 3 * DAY - 5 * MIN;
        let hourly: Vec<u64> = (1..5)
            .map(|h| now / HOUR_MS * HOUR_MS - h * HOUR_MS)
            .collect();
        for step in 1..=11 {
            sim.write(now + step * 5 * MIN);
            for &x in &hourly {
                assert!(sim.times().contains(&x), "{x} after {step}");
            }
        }
    }

    #[test]
    fn bursts_and_gaps() {
        let mut sim = Sim::default();
        let mut t = T0;
        // Short sessions separated by gaps from an hour to a month.
        for gap in [HOUR_MS, 3 * DAY, 2 * HOUR_MS, 30 * DAY, 7 * DAY, 10 * MIN] {
            sim.every(t, t + 20 * MIN, MIN);
            t += 20 * MIN + gap;
        }
        sim.every(t, t + 3 * MIN, MIN);
        let times = sim.times();
        assert_eq!(times.len(), 30);
        assert_eq!(times[0], t + 2 * MIN);
        assert_eq!(*times.last().unwrap(), T0);
        // Every session left something.
        let mut s = T0;
        for gap in [HOUR_MS, 3 * DAY, 2 * HOUR_MS, 30 * DAY, 7 * DAY, 10 * MIN] {
            assert!(
                times.iter().any(|&x| x >= s && x < s + 20 * MIN),
                "session at {s}"
            );
            s += 20 * MIN + gap;
        }
    }

    #[test]
    fn writes_faster_than_the_interval_keep_minute_spacing_in_the_recent_tier() {
        let vs: Vec<_> = (0..120).map(|i| v(T0 + i * 10_000)).collect();
        let now = T0 + 119 * 10_000;
        let keep = retain(&vs, now);
        assert_eq!(keep.len(), 30);
        for i in 0..5 {
            assert!(keep.contains(&format!("{}-autosave", now - i * MIN)));
        }
    }

    #[test]
    fn clock_going_backwards() {
        let mut sim = Sim::default();
        sim.every(T0, T0 + 2 * DAY, 10 * MIN);
        // The clock jumps back a day: everything looks like it is in the future.
        let back = T0 + DAY + 30_000;
        sim.every(back, back + 2 * HOUR_MS, MIN);
        let t = sim.times();
        assert_eq!(t.len(), 30);
        let newest = back + 2 * HOUR_MS - MIN;
        // Recent tier is the versions really written last.
        let recent: Vec<u64> = (0..5).map(|i| newest - i * MIN).collect();
        for x in &recent {
            assert!(t.contains(x), "{x}");
        }
        assert!(t.contains(&T0), "the oldest stays");
    }

    #[test]
    fn an_existing_folder_of_fifty_five_minute_versions_is_pruned_once() {
        let vs: Vec<_> = (0..50).map(|i| v(T0 + i * 5 * MIN)).collect();
        let now = T0 + 50 * 5 * MIN;
        let keep = retain(&vs, now);
        assert_eq!(keep.len(), 30);
        assert!(keep.contains(&format!("{}-autosave", T0 + 49 * 5 * MIN)));
        assert!(keep.contains(&format!("{T0}-autosave")));
        // Pruning again changes nothing.
        let left: Vec<_> = vs.into_iter().filter(|x| keep.contains(&x.id)).collect();
        assert_eq!(retain(&left, now), keep);
    }

    #[test]
    fn random_histories_respect_the_invariants() {
        // Deterministic pseudo-random histories (xorshift).
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rand = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..50 {
            let mut sim = Sim::default();
            let mut t = T0;
            for _ in 0..300 {
                t += match rand(4) {
                    0 => MIN,
                    1 => rand(HOUR_MS),
                    2 => rand(10 * DAY),
                    _ => MIN + rand(10 * MIN),
                };
                // Sometimes the clock goes back.
                let now = if rand(20) == 0 {
                    t - rand(DAY).min(t - T0)
                } else {
                    t
                };
                sim.write(now);
            }
            assert!(sim.0.len() <= MAX_AUTOSAVE_VERSIONS);
            assert_eq!(sim.0.len(), 30);
        }
    }
}
