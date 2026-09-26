//! Automation curve evaluation (RT-safe, allocation-free).
//!
//! Points are `(time beats, normalized value, curve)` sorted by time; the curve of a point
//! shapes the segment towards the next point. Before the first point the first value
//! holds; after the last point the last value holds.

use ether_protocol::devices::{ParamScale, scale_to_plain};
use ether_protocol::model::CurveShape;

use crate::graph::ParamMapping;

/// Shape a 0..1 segment fraction for `CurveShape::Curve { tension }`: `x^(4^tension)`, so
/// tension 0 is linear, +1 is `x⁴` (slow start) and −1 is `x^¼` (fast start). The UI
/// mirrors this formula when drawing curves.
#[inline]
pub fn curve_fraction(x: f64, tension: f32) -> f64 {
    let t = (tension as f64).clamp(-1.0, 1.0);
    x.clamp(0.0, 1.0).powf(4f64.powf(t))
}

/// Normalized value of a lane at `time` (beats). Returns `None` when there are no points.
pub fn evaluate(points: &[(f64, f64, CurveShape)], time: f64) -> Option<f64> {
    let first = points.first()?;
    if time <= first.0 {
        return Some(first.1);
    }
    // Index of the first point strictly after `time`.
    let i = points.partition_point(|p| p.0 <= time);
    if i >= points.len() {
        return Some(points[points.len() - 1].1);
    }
    let (t0, v0, curve) = points[i - 1];
    let (t1, v1, _) = points[i];
    let span = t1 - t0;
    if span <= 0.0 {
        return Some(v1);
    }
    let x = (time - t0) / span;
    let f = match curve {
        CurveShape::Linear => x,
        CurveShape::Step => 0.0,
        CurveShape::Curve { tension } => curve_fraction(x, tension),
    };
    Some(v0 + (v1 - v0) * f)
}

/// Normalized → plain value through a [`ParamMapping`] (snapped for stepped params).
pub fn to_plain(mapping: &ParamMapping, normalized: f64) -> f64 {
    let v = scale_to_plain(mapping.scale, mapping.min, mapping.max, normalized);
    match mapping.steps {
        Some(steps) if steps > 1 => {
            let step = (mapping.max - mapping.min) / (steps - 1) as f64;
            mapping.min + ((v - mapping.min) / step).round() * step
        }
        _ => v,
    }
}

/// Plain value of a mixer gain target (track volume, send level) as *linear gain*.
///
/// Convention for `ResolvedTarget::TrackVolume` / `Send` mappings: a `Fader` (or any dB)
/// scale yields decibels (plain value ≤ `min` = silence); a `Linear`/`Power` scale with
/// `min >= 0` and `max <= 16` is taken as linear gain directly.
pub fn gain_from_plain(mapping: &ParamMapping, plain: f64) -> f32 {
    let is_db = matches!(mapping.scale, ParamScale::Fader) || mapping.min < 0.0;
    if is_db {
        // The bottom of a fader law is -inf dB.
        if plain <= mapping.min {
            0.0
        } else {
            10f64.powf(plain / 20.0) as f32
        }
    } else {
        plain.max(0.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn evaluates_segments() {
        let pts = [
            (0.0, 0.0, CurveShape::Linear),
            (4.0, 1.0, CurveShape::Step),
            (8.0, 0.5, CurveShape::Curve { tension: 1.0 }),
            (10.0, 1.0, CurveShape::Linear),
        ];
        assert_eq!(evaluate(&[], 1.0), None);
        assert!(close(evaluate(&pts, -1.0).unwrap(), 0.0));
        assert!(close(evaluate(&pts, 2.0).unwrap(), 0.5));
        assert!(close(evaluate(&pts, 4.0).unwrap(), 1.0));
        assert!(close(evaluate(&pts, 7.9).unwrap(), 1.0));
        assert!(close(evaluate(&pts, 8.0).unwrap(), 0.5));
        assert!(close(evaluate(&pts, 9.0).unwrap(), 0.5 + 0.5 * 0.5f64.powi(4)));
        assert!(close(evaluate(&pts, 12.0).unwrap(), 1.0));
    }

    #[test]
    fn fader_gain() {
        let m = ParamMapping {
            min: -70.0,
            max: 6.0,
            scale: ParamScale::Fader,
            steps: None,
        };
        assert_eq!(gain_from_plain(&m, -70.0), 0.0);
        assert!((gain_from_plain(&m, 0.0) - 1.0).abs() < 1e-6);
        assert!((gain_from_plain(&m, to_plain(&m, 1.0)) - 10f32.powf(0.3)).abs() < 1e-4);
        let lin = ParamMapping {
            min: 0.0,
            max: 2.0,
            scale: ParamScale::Linear,
            steps: None,
        };
        assert_eq!(gain_from_plain(&lin, 0.5), 0.5);
    }

    #[test]
    fn stepped_mapping() {
        let m = ParamMapping {
            min: 0.0,
            max: 3.0,
            scale: ParamScale::Linear,
            steps: Some(4),
        };
        assert!(close(to_plain(&m, 0.4), 1.0));
        assert!(close(to_plain(&m, 0.6), 2.0));
    }
}
