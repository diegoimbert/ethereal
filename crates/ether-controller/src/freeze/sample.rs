//! Sampling plan of `FreezeCommand::ConvertToSampler`: which keys and velocities are rendered,
//! the media id of each render, and the multisampler zones that play them.
//!
//! Keys and velocities are lists (sorted, duplicates dropped). Each sampled key owns the
//! range up to the midpoint with its neighbours (0..=127 at the edges), so every key plays
//! the closest sample repitched (`SampleZone::root_key`); each rendered velocity likewise
//! owns the velocities nearest to it (1..=127 at the edges). One velocity keeps the
//! multisampler's velocity sensitivity on; several layers switch it off because each sample
//! already has the level of its velocity.

use ether_core::protocol::model::*;
use ether_model::derive_id;

use crate::tx::{CmdResult, invalid};

/// Most velocity layers `ConvertToSampler` accepts.
pub const MAX_VELOCITIES: usize = 16;

/// One rendered note: its key and velocity, and the media it is written to.
#[derive(Clone, Debug)]
pub struct SamplePoint {
    pub key: u8,
    /// 1..=127 (the velocity the note is rendered at).
    pub velocity: u8,
    pub media: MediaId,
}

/// The samples of a conversion and the zones that play them, in media-id order.
#[derive(Clone, Debug)]
pub struct SamplePlan {
    pub points: Vec<SamplePoint>,
    pub zones: Vec<SampleZone>,
}

/// The ranges `(lo, hi)` of `min..=127` nearest to each of `values` (sorted, distinct): up
/// to the midpoint with each neighbour, a tie going to the lower value.
fn nearest(values: &[u8], min: u8) -> Vec<(u8, u8)> {
    (0..values.len())
        .map(|i| {
            let lo = if i == 0 {
                min
            } else {
                ((u16::from(values[i - 1]) + u16::from(values[i])) / 2 + 1) as u8
            };
            let hi = if i + 1 == values.len() {
                127
            } else {
                ((u16::from(values[i]) + u16::from(values[i + 1])) / 2) as u8
            };
            (lo, hi)
        })
        .collect()
}

/// Build the sampling plan. `seed_media` names the renders (`derive_id(seed_media, i)`).
pub fn plan(keys: &[u8], velocities: &[u8], seed_media: MediaId) -> CmdResult<SamplePlan> {
    let sorted = |v: &[u8]| {
        let mut v = v.to_vec();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (keys, velocities) = (sorted(keys), sorted(velocities));
    if keys.is_empty() {
        return Err(invalid("pick at least one key to sample"));
    }
    if keys.iter().any(|&k| k > 127) {
        return Err(invalid("keys must be 0..=127"));
    }
    if velocities.is_empty() || velocities.len() > MAX_VELOCITIES {
        return Err(invalid(format!("render 1 to {MAX_VELOCITIES} velocities")));
    }
    if velocities.iter().any(|&v| !(1..=127).contains(&v)) {
        return Err(invalid("velocities must be 1..=127"));
    }
    let count = keys.len().saturating_mul(velocities.len());
    if count > MAX_ZONES {
        return Err(invalid(format!(
            "this would render {count} samples: at most {MAX_ZONES} zones"
        )));
    }
    let key_ranges = nearest(&keys, 0);
    let velocity_ranges = nearest(&velocities, 1);
    let mut points = Vec::with_capacity(count);
    let mut zones = Vec::with_capacity(count);
    for (&key, &(lo, hi)) in keys.iter().zip(&key_ranges) {
        for (&velocity, &(vlo, vhi)) in velocities.iter().zip(&velocity_ranges) {
            let media: MediaId = derive_id(seed_media, points.len() as u32);
            zones.push(SampleZone {
                media: Some(media),
                root_key: key,
                tune_cents: 0.0,
                keys: Zone { lo, hi },
                velocities: Zone { lo: vlo, hi: vhi },
                ..SampleZone::default()
            });
            points.push(SamplePoint {
                key,
                velocity,
                media,
            });
        }
    }
    Ok(SamplePlan { points, zones })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_ranges_split_at_midpoints() {
        assert_eq!(
            nearest(&[60, 62, 64], 0),
            vec![(0, 61), (62, 63), (64, 127)]
        );
        assert_eq!(nearest(&[60], 0), vec![(0, 127)]);
        assert_eq!(
            nearest(&[42, 85, 127], 1),
            vec![(1, 63), (64, 106), (107, 127)]
        );
        assert_eq!(nearest(&[100], 1), vec![(1, 127)]);
    }

    #[test]
    fn zones_cover_every_key_and_velocity() {
        // Unsorted, with duplicates: sorted and deduplicated.
        let p = plan(&[64, 60, 62, 60], &[127, 42, 85], MediaId::NIL).unwrap();
        assert_eq!(p.points.len(), 3 * 3);
        for key in 0..=127u8 {
            let z: Vec<&SampleZone> = p.zones.iter().filter(|z| z.keys.contains(key)).collect();
            assert_eq!(z.len(), 3, "key {key} has {z:?}");
        }
        for vel in 1..=127u8 {
            let n = p
                .zones
                .iter()
                .filter(|z| (z.velocities.lo..=z.velocities.hi).contains(&vel))
                .count();
            assert_eq!(n, 3, "velocity {vel} has {n} zones");
        }
        // Nearest sample: key 61 uses root 60 (tie goes to the lower one).
        assert_eq!(p.zones[0].keys, Zone { lo: 0, hi: 61 });
        assert_eq!(p.zones[0].root_key, 60);
        assert_eq!(p.zones[3].root_key, 62);
        // Rendered at the listed velocities, the loudest at 127.
        let v: Vec<u8> = p.points[..3].iter().map(|p| p.velocity).collect();
        assert_eq!(v, vec![42, 85, 127]);
    }

    #[test]
    fn custom_keys_need_not_be_evenly_spaced() {
        let p = plan(&[36, 40, 60], &[100], MediaId::NIL).unwrap();
        let keys: Vec<(u8, Zone)> = p.zones.iter().map(|z| (z.root_key, z.keys)).collect();
        assert_eq!(
            keys,
            vec![
                (36, Zone { lo: 0, hi: 38 }),
                (40, Zone { lo: 39, hi: 50 }),
                (60, Zone { lo: 51, hi: 127 })
            ]
        );
        assert_eq!(p.zones[0].velocities, Zone { lo: 1, hi: 127 });
    }

    #[test]
    fn rejects_bad_lists_and_too_many_samples() {
        let m = MediaId::NIL;
        assert!(plan(&[], &[127], m).is_err());
        assert!(plan(&[128], &[127], m).is_err());
        assert!(plan(&[60], &[], m).is_err());
        assert!(plan(&[60], &[0], m).is_err());
        assert!(plan(&[60], &[128], m).is_err());
        let many: Vec<u8> = (1..=17).collect();
        assert!(plan(&[60], &many, m).is_err());
        let all: Vec<u8> = (0..=127).collect();
        assert!(plan(&all, &[20, 40, 60, 80, 127], m).is_err());
    }
}
