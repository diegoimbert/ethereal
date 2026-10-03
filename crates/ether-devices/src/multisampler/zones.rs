//! Zone set of a multisampler: the document's zones (`ether_model::multisampler`) with their
//! resolved sources, built off the audio thread, and the per-note zone selection.
//!
//! # Selection (per note-on), refining `ether_model::multisampler`
//! 1. **Candidates**: zones whose key range contains the key and whose velocity range
//!    contains `round(velocity · 127)` (clamped to 1..=127), in zone order. At most
//!    [`MAX_LAYERS`] candidates per note (later zones are ignored).
//! 2. **Round robin**: candidates sharing a non-zero group alternate. The group's size is
//!    the number of *candidates* in that group for this note (so a group spread over several
//!    keys always plays one zone per note); the device keeps one counter per group, plays
//!    the candidate at `counter % size` (Cycle) or a random one that differs from the
//!    previous pick (Random), and advances the counter once per note-on that hit the group.
//! 3. **Velocity crossfade**: where two playing zones' velocity ranges overlap *partially*
//!    (one starts and ends above the other), the overlap is an equal-power crossfade: the
//!    lower zone fades out and the upper one fades in across the shared velocities. Zones
//!    with identical or nested ranges are plain layers (both at full gain).
//!
//! Memory: zones are capped at [`MAX_ZONES`]; sources are the host's shared decoded media
//! (`Arc`, one per media however many zones use it), resolved here on the controller thread.

use std::sync::Arc;

use ether_core::AudioSource;
use ether_core::protocol::model::{MediaId, SampleZone};

pub use ether_core::protocol::model::MAX_ZONES;

/// Maximum zones sounding for one note (layers after round robin).
pub const MAX_LAYERS: usize = 16;
/// Round-robin groups (`SampleZone::round_robin` is a `u8`; 0 = none).
pub const RR_GROUPS: usize = 256;

/// A zone with its playback data precomputed for the audio thread.
#[derive(Clone, Debug)]
pub struct ZoneRt {
    pub zone: SampleZone,
    /// Index into [`ZoneSet::sources`] (`None` = empty zone or unresolved media: silent).
    pub source: Option<usize>,
}

/// The zones a live multisampler plays (swapped in with `Node::set_data`).
#[derive(Clone, Default)]
pub struct ZoneSet {
    pub zones: Vec<ZoneRt>,
    /// Distinct media of the zones with their sources (voices refer to them by index and
    /// are remapped by media id when the set is swapped).
    pub sources: Vec<(MediaId, Arc<dyn AudioSource>)>,
}

impl std::fmt::Debug for ZoneSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoneSet")
            .field("zones", &self.zones.len())
            .field("sources", &self.sources.len())
            .finish()
    }
}

impl ZoneSet {
    /// Non-RT. Resolve `zones` (at most [`MAX_ZONES`]) with `resolve`.
    pub fn new(
        zones: &[SampleZone],
        mut resolve: impl FnMut(MediaId) -> Option<Arc<dyn AudioSource>>,
    ) -> Self {
        let mut set = ZoneSet {
            zones: Vec::with_capacity(zones.len().min(MAX_ZONES)),
            sources: Vec::new(),
        };
        for z in zones.iter().take(MAX_ZONES) {
            let source = z.media.and_then(|m| {
                if let Some(i) = set.sources.iter().position(|(id, _)| *id == m) {
                    return Some(i);
                }
                let src = resolve(m)?;
                set.sources.push((m, src));
                Some(set.sources.len() - 1)
            });
            set.zones.push(ZoneRt {
                zone: z.clone(),
                source,
            });
        }
        set
    }

    /// Index of `media` among the sources.
    pub fn source_of(&self, media: MediaId) -> Option<usize> {
        self.sources.iter().position(|(id, _)| *id == media)
    }

    /// RT-safe. The zones a note-on plays, with their crossfade gains, written to `out`;
    /// returns how many. `velocity` is 0..=1. `rr` holds the round-robin state.
    pub fn select(
        &self,
        key: u8,
        velocity: f32,
        rr: &mut RoundRobin,
        out: &mut [Hit; MAX_LAYERS],
    ) -> usize {
        let vel = velocity_127(velocity);
        // 1. Candidates.
        let mut cand = [0u16; MAX_LAYERS];
        let mut n = 0;
        for (i, z) in self.zones.iter().enumerate() {
            let zz = &z.zone;
            if zz.keys.contains(key) && vel_zone(zz.velocities.lo, zz.velocities.hi, vel) {
                cand[n] = i as u16;
                n += 1;
                if n == MAX_LAYERS {
                    break;
                }
            }
        }
        // 2. Round robin: keep one candidate per non-zero group.
        let mut m = 0;
        for c in 0..n {
            let g = self.zones[cand[c] as usize].zone.round_robin;
            if g == 0 {
                out[m] = Hit {
                    zone: cand[c],
                    gain: 1.0,
                };
                m += 1;
                continue;
            }
            // First candidate of its group handles the whole group.
            if cand[..c]
                .iter()
                .any(|&p| self.zones[p as usize].zone.round_robin == g)
            {
                continue;
            }
            let size = cand[c..n]
                .iter()
                .filter(|&&p| self.zones[p as usize].zone.round_robin == g)
                .count();
            let pick = rr.next(g, size);
            let chosen = cand[c..n]
                .iter()
                .filter(|&&p| self.zones[p as usize].zone.round_robin == g)
                .nth(pick)
                .copied()
                .unwrap_or(cand[c]);
            out[m] = Hit {
                zone: chosen,
                gain: 1.0,
            };
            m += 1;
        }
        // 3. Velocity crossfades between partially overlapping layers.
        for a in 0..m {
            let za = &self.zones[out[a].zone as usize].zone.velocities;
            let (alo, ahi) = (za.lo.max(1), za.hi);
            let mut gain = 1.0f32;
            for b in 0..m {
                if a == b {
                    continue;
                }
                let zb = &self.zones[out[b].zone as usize].zone.velocities;
                let (blo, bhi) = (zb.lo.max(1), zb.hi);
                if blo > alo && bhi > ahi && blo <= ahi {
                    // `b` is above `a`: `a` fades out across [blo, ahi].
                    let t = (vel as f32 - blo as f32 + 0.5) / (ahi as f32 - blo as f32 + 1.0);
                    gain *= (t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).cos();
                } else if blo < alo && bhi < ahi && alo <= bhi {
                    // `b` is below `a`: `a` fades in across [alo, bhi].
                    let t = (vel as f32 - alo as f32 + 0.5) / (bhi as f32 - alo as f32 + 1.0);
                    gain *= (t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin();
                }
            }
            out[a].gain = gain;
        }
        m
    }
}

/// `round(velocity · 127)` clamped to 1..=127 (the velocity-zone scale).
pub fn velocity_127(velocity: f32) -> u8 {
    let v = if velocity.is_finite() { velocity } else { 0.0 };
    ((v.clamp(0.0, 1.0) * 127.0).round() as u8).max(1)
}

fn vel_zone(lo: u8, hi: u8, vel: u8) -> bool {
    (lo.max(1)..=hi).contains(&vel)
}

/// One zone sounding for a note.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hit {
    /// Index into [`ZoneSet::zones`].
    pub zone: u16,
    /// Velocity-crossfade gain (1 outside crossfades).
    pub gain: f32,
}

/// Round-robin state: one counter per group (and the last pick, for Random).
#[derive(Clone, Debug)]
pub struct RoundRobin {
    counters: [u32; RR_GROUPS],
    last: [u16; RR_GROUPS],
    /// `true` = random order instead of cycling.
    pub random: bool,
    rng: u32,
}

impl Default for RoundRobin {
    fn default() -> Self {
        Self {
            counters: [0; RR_GROUPS],
            last: [u16::MAX; RR_GROUPS],
            random: false,
            rng: 0x9E37_79B9,
        }
    }
}

impl RoundRobin {
    /// Restart every group at its first zone.
    pub fn reset(&mut self) {
        let random = self.random;
        *self = Self {
            random,
            ..Self::default()
        };
    }

    /// Position (0..size) to play in group `g`; advances the group.
    pub fn next(&mut self, g: u8, size: usize) -> usize {
        let gi = g as usize;
        if size <= 1 {
            self.counters[gi] = self.counters[gi].wrapping_add(1);
            self.last[gi] = 0;
            return 0;
        }
        let pick = if self.random {
            // xorshift32; never repeat the previous pick.
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let mut p = (self.rng as usize) % (size - 1);
            if p >= self.last[gi] as usize && (self.last[gi] as usize) < size {
                p += 1;
            }
            p
        } else {
            self.counters[gi] as usize % size
        };
        self.counters[gi] = self.counters[gi].wrapping_add(1);
        self.last[gi] = pick as u16;
        pick
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::model::Zone;

    fn zone(keys: (u8, u8), vels: (u8, u8), rr: u8) -> SampleZone {
        SampleZone {
            keys: Zone {
                lo: keys.0,
                hi: keys.1,
            },
            velocities: Zone {
                lo: vels.0,
                hi: vels.1,
            },
            round_robin: rr,
            ..SampleZone::default()
        }
    }

    fn set(zones: &[SampleZone]) -> ZoneSet {
        ZoneSet::new(zones, |_| None)
    }

    fn hits(s: &ZoneSet, key: u8, vel: f32, rr: &mut RoundRobin) -> Vec<Hit> {
        let mut out = [Hit::default(); MAX_LAYERS];
        let n = s.select(key, vel, rr, &mut out);
        out[..n].to_vec()
    }

    #[test]
    fn selects_by_key_and_velocity() {
        let s = set(&[
            zone((0, 59), (1, 127), 0),
            zone((60, 127), (1, 63), 0),
            zone((60, 127), (64, 127), 0),
        ]);
        let mut rr = RoundRobin::default();
        let z = |h: Vec<Hit>| h.iter().map(|h| h.zone).collect::<Vec<_>>();
        assert_eq!(z(hits(&s, 40, 0.5, &mut rr)), vec![0]);
        assert_eq!(z(hits(&s, 60, 0.3, &mut rr)), vec![1]);
        assert_eq!(z(hits(&s, 60, 0.9, &mut rr)), vec![2]);
        // Velocity 0 counts as 1.
        assert_eq!(z(hits(&s, 60, 0.0, &mut rr)), vec![1]);
        assert_eq!(velocity_127(0.5), 64);
    }

    #[test]
    fn round_robin_cycles_per_group() {
        let s = set(&[
            zone((0, 127), (1, 127), 1),
            zone((0, 127), (1, 127), 1),
            zone((0, 127), (1, 127), 1),
            zone((0, 127), (1, 127), 0),
        ]);
        let mut rr = RoundRobin::default();
        let order: Vec<Vec<u16>> = (0..4)
            .map(|_| hits(&s, 60, 0.8, &mut rr).iter().map(|h| h.zone).collect())
            .collect();
        assert_eq!(order, vec![vec![0, 3], vec![1, 3], vec![2, 3], vec![0, 3]]);
    }

    #[test]
    fn random_round_robin_never_repeats() {
        let s = set(&[zone((0, 127), (1, 127), 2), zone((0, 127), (1, 127), 2)]);
        let mut rr = RoundRobin {
            random: true,
            ..RoundRobin::default()
        };
        let mut prev = u16::MAX;
        for _ in 0..50 {
            let h = hits(&s, 60, 0.5, &mut rr);
            assert_eq!(h.len(), 1);
            assert_ne!(h[0].zone, prev);
            prev = h[0].zone;
        }
    }

    #[test]
    fn velocity_crossfade_is_equal_power() {
        // Soft 1..=80, loud 60..=127: crossfade across 60..=80.
        let s = set(&[zone((0, 127), (1, 80), 0), zone((0, 127), (60, 127), 0)]);
        let mut rr = RoundRobin::default();
        let at = |v: u8, rr: &mut RoundRobin| hits(&s, 60, v as f32 / 127.0, rr);
        let below = at(40, &mut rr);
        assert_eq!(below.len(), 1);
        assert_eq!(below[0].gain, 1.0);
        let mut prev_soft = 1.0;
        for v in 60..=80 {
            let h = at(v, &mut rr);
            assert_eq!(h.len(), 2);
            let (soft, loud) = (h[0].gain, h[1].gain);
            assert!((soft * soft + loud * loud - 1.0).abs() < 1e-5, "{v}");
            assert!(soft < prev_soft);
            prev_soft = soft;
        }
        // Nested ranges are plain layers.
        let s = set(&[zone((0, 127), (1, 127), 0), zone((0, 127), (40, 90), 0)]);
        let h = hits(&s, 60, 0.5, &mut rr);
        assert_eq!(h.iter().map(|h| h.gain).collect::<Vec<_>>(), vec![1.0, 1.0]);
    }

    #[test]
    fn caps_zones_and_layers() {
        let zones = vec![zone((0, 127), (1, 127), 0); MAX_ZONES + 10];
        let s = set(&zones);
        assert_eq!(s.zones.len(), MAX_ZONES);
        let mut rr = RoundRobin::default();
        assert_eq!(hits(&s, 60, 0.5, &mut rr).len(), MAX_LAYERS);
    }
}
