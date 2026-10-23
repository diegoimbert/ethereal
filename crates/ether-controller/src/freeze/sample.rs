//! Sampling plan of `FreezeCommand::ConvertToSampler`: which keys and velocities are rendered,
//! the media id of each render, and the multisampler zones that play them.
//!
//! Keys: the sampled keys are `first`, `first + step`, ... up to `last` (the last key is
//! included when the step does not land on it). Each sampled key owns the range up to the
//! midpoint with its neighbours (0..=127 at the edges), so every key plays the closest sample
//! repitched (`SampleZone::root_key`).
//!
//! Velocities: `velocity_steps` equal bands of 1..=127, each rendered at the centre of its
//! band. One step renders at full velocity (127) and keeps the multisampler's velocity
//! sensitivity on; several layers switch it off because each sample already has the level of
//! its band.

use ether_core::protocol::model::*;
use ether_model::derive_id;

use crate::tx::{CmdResult, invalid};

/// Most velocity layers `ConvertToSampler` accepts.
pub const MAX_VELOCITY_STEPS: u8 = 16;

/// One rendered note: its key and velocity, and the media it is written to.
#[derive(Clone, Debug)]
pub struct SamplePoint {
    pub key: u8,
    /// 0..=127 (the velocity the note is rendered at).
    pub velocity: u8,
    pub media: MediaId,
}

/// The samples of a conversion and the zones that play them, in media-id order.
#[derive(Clone, Debug)]
pub struct SamplePlan {
    pub points: Vec<SamplePoint>,
    pub zones: Vec<SampleZone>,
}

/// The sampled keys, ascending: `first`, `first + step`, ... and `last` when it is not hit.
pub fn sampled_keys(first: u8, last: u8, step: u8) -> Vec<u8> {
    let mut keys: Vec<u8> = (first..=last).step_by(step.max(1) as usize).collect();
    if keys.last() != Some(&last) {
        keys.push(last);
    }
    keys
}

/// The velocity bands of `steps` layers over 1..=127, `(lo, hi, rendered velocity)`.
pub fn velocity_bands(steps: u8) -> Vec<(u8, u8, u8)> {
    if steps <= 1 {
        return vec![(1, 127, 127)];
    }
    let n = u32::from(steps);
    (0..n)
        .map(|i| {
            let lo = ((i * 127) / n + 1) as u8;
            let hi = (((i + 1) * 127) / n) as u8;
            let velocity = ((u16::from(lo) + u16::from(hi)) / 2) as u8;
            (lo, hi, velocity.max(1))
        })
        .collect()
}

/// Build the sampling plan. `seed_media` names the renders (`derive_id(seed_media, i)`).
pub fn plan(
    first_key: u8,
    last_key: u8,
    key_step: u8,
    velocity_steps: u8,
    seed_media: MediaId,
) -> CmdResult<SamplePlan> {
    if first_key > 127 || last_key > 127 {
        return Err(invalid("keys must be 0..=127"));
    }
    if first_key > last_key {
        return Err(invalid("the first key must not be above the last key"));
    }
    if key_step == 0 {
        return Err(invalid("the key step must be at least 1"));
    }
    if !(1..=MAX_VELOCITY_STEPS).contains(&velocity_steps) {
        return Err(invalid(format!(
            "velocity steps must be 1..={MAX_VELOCITY_STEPS}"
        )));
    }
    let keys = sampled_keys(first_key, last_key, key_step);
    let bands = velocity_bands(velocity_steps);
    let count = keys.len().saturating_mul(bands.len());
    if count > MAX_ZONES {
        return Err(invalid(format!(
            "this would render {count} samples: at most {MAX_ZONES} zones"
        )));
    }
    let mut points = Vec::with_capacity(count);
    let mut zones = Vec::with_capacity(count);
    for (ki, &key) in keys.iter().enumerate() {
        let lo = if ki == 0 {
            0
        } else {
            (u16::from(keys[ki - 1]) + u16::from(key)) / 2 + 1
        } as u8;
        let hi = if ki + 1 == keys.len() {
            127
        } else {
            ((u16::from(key) + u16::from(keys[ki + 1])) / 2) as u8
        };
        for (vi, &(vlo, vhi, velocity)) in bands.iter().enumerate() {
            let index = (ki * bands.len() + vi) as u32;
            let media: MediaId = derive_id(seed_media, index);
            let zone = SampleZone {
                media: Some(media),
                root_key: key,
                tune_cents: 0.0,
                keys: Zone { lo, hi },
                velocities: Zone { lo: vlo, hi: vhi },
                ..SampleZone::default()
            };
            zones.push(zone);
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
    fn keys_step_and_include_the_last() {
        assert_eq!(sampled_keys(60, 64, 2), vec![60, 62, 64]);
        assert_eq!(sampled_keys(60, 65, 2), vec![60, 62, 64, 65]);
        assert_eq!(sampled_keys(60, 64, 1), vec![60, 61, 62, 63, 64]);
        assert_eq!(sampled_keys(60, 64, 10), vec![60, 64]);
        assert_eq!(sampled_keys(60, 60, 4), vec![60]);
    }

    #[test]
    fn velocity_bands_cover_everything_once() {
        for steps in 1..=MAX_VELOCITY_STEPS {
            let bands = velocity_bands(steps);
            assert_eq!(bands[0].0, 1);
            assert_eq!(bands.last().unwrap().1, 127);
            for pair in bands.windows(2) {
                assert_eq!(pair[0].1 + 1, pair[1].0, "{steps}: {bands:?}");
            }
            assert!(bands.iter().all(|(lo, hi, v)| lo <= v && v <= hi));
        }
        assert_eq!(velocity_bands(1), vec![(1, 127, 127)]);
    }

    #[test]
    fn zones_cover_every_key_and_velocity() {
        let p = plan(60, 64, 2, 3, MediaId::NIL).unwrap();
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
    }

    #[test]
    fn rejects_bad_ranges_and_too_many_samples() {
        let m = MediaId::NIL;
        assert!(plan(64, 60, 1, 2, m).is_err());
        assert!(plan(60, 64, 0, 2, m).is_err());
        assert!(plan(60, 64, 1, 0, m).is_err());
        assert!(plan(60, 64, 1, MAX_VELOCITY_STEPS + 1, m).is_err());
        assert!(plan(0, 127, 1, 5, m).is_err());
    }
}
