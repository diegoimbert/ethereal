//! Drawn LFO cycles (owned by the `fx-modulation` node): the curve of the Volume LFO
//! (`BuiltinDevice::VolumeLfo { curve }`).
//!
//! A curve is one LFO cycle drawn as breakpoints: `x` is the position in the cycle (0..=1),
//! `y` the value (0..=1; the Volume LFO's linear gain). Each point's [`CurveShape`] shapes
//! the segment to the next point, with the automation law (`x^(4^tension)`, so the UI
//! draws both with the same formula). The cycle wraps: the last point's segment runs to the
//! first point of the next cycle. Points may share an `x` (a vertical edge: the later point
//! wins from that `x` on).
//!
//! The curve lives in the device kind, like the multisampler's zones: an edit is one
//! `DeviceChange::Kind` (one undo step, one collab op), and live nodes take it in place
//! (`EngineBridge::update_builtin` → `Node::set_data`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::CurveShape;

/// Most points in an [`LfoCurve`].
pub const MAX_LFO_POINTS: usize = 64;

/// One breakpoint of an [`LfoCurve`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LfoPoint {
    /// Position in the cycle, 0..=1.
    pub x: f32,
    /// Value, 0..=1.
    pub y: f32,
    /// Shape of the segment from this point to the next one (the last point's: to the
    /// first point of the next cycle).
    pub curve: CurveShape,
}

impl LfoPoint {
    /// A point with a linear segment after it.
    pub const fn linear(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            curve: CurveShape::Linear,
        }
    }

    /// A point with a curved segment after it (`tension` -1..=1, as automation).
    pub const fn curved(x: f32, y: f32, tension: f32) -> Self {
        Self {
            x,
            y,
            curve: CurveShape::Curve { tension },
        }
    }
}

/// One drawn LFO cycle: 1..=[`MAX_LFO_POINTS`] points sorted by `x`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LfoCurve {
    pub points: Vec<LfoPoint>,
}

impl Default for LfoCurve {
    /// The classic sidechain pump: a fast duck at the start of the cycle and a curved
    /// recovery over its first 60 % (the Volume LFO's default, at 1/4).
    fn default() -> Self {
        Self {
            points: vec![
                LfoPoint::linear(0.0, 1.0),
                LfoPoint::curved(0.02, 0.0, -0.45),
                LfoPoint::linear(0.6, 1.0),
            ],
        }
    }
}

/// `x^(4^tension)` on a 0..1 segment fraction (the automation law, see
/// `ether_core::automation::curve_fraction`).
#[inline]
pub fn curve_fraction(x: f64, tension: f32) -> f64 {
    let t = f64::from(tension).clamp(-1.0, 1.0);
    x.clamp(0.0, 1.0).powf(4f64.powf(t))
}

impl LfoCurve {
    /// `Err` with the reason when the curve isn't valid: 1..=64 points, finite, `x` and `y`
    /// in 0..=1, sorted by `x`, tensions in -1..=1.
    pub fn check(&self) -> Result<(), String> {
        let n = self.points.len();
        if n == 0 || n > MAX_LFO_POINTS {
            return Err(format!("an LFO curve has 1..={MAX_LFO_POINTS} points"));
        }
        let mut last = 0.0;
        for p in &self.points {
            if !(p.x.is_finite() && (0.0..=1.0).contains(&p.x)) {
                return Err("LFO curve point x must be within 0..=1".into());
            }
            if !(p.y.is_finite() && (0.0..=1.0).contains(&p.y)) {
                return Err("LFO curve point y must be within 0..=1".into());
            }
            if p.x < last {
                return Err("LFO curve points must be sorted by x".into());
            }
            if let CurveShape::Curve { tension } = p.curve
                && !(tension.is_finite() && (-1.0..=1.0).contains(&tension))
            {
                return Err("LFO curve tension must be within -1..=1".into());
            }
            last = p.x;
        }
        Ok(())
    }

    /// This curve made valid: non-finite values dropped, values clamped, points sorted by
    /// `x` (stable) and cut to [`MAX_LFO_POINTS`]; no point left = a flat line at 1.
    pub fn sanitized(&self) -> Self {
        let mut points: Vec<LfoPoint> = self
            .points
            .iter()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .map(|p| LfoPoint {
                x: p.x.clamp(0.0, 1.0),
                y: p.y.clamp(0.0, 1.0),
                curve: match p.curve {
                    CurveShape::Curve { tension } if tension.is_finite() => CurveShape::Curve {
                        tension: tension.clamp(-1.0, 1.0),
                    },
                    CurveShape::Curve { .. } => CurveShape::Linear,
                    other => other,
                },
            })
            .collect();
        points.sort_by(|a, b| a.x.total_cmp(&b.x));
        points.truncate(MAX_LFO_POINTS);
        if points.is_empty() {
            points.push(LfoPoint::linear(0.0, 1.0));
        }
        Self { points }
    }

    /// Value at `phase` (cycles; any real, wrapped into the cycle), see [`lfo_value_at`].
    pub fn value_at(&self, phase: f64) -> f64 {
        lfo_value_at(&self.points, phase)
    }
}

/// Value of the cycle drawn by `pts` (sorted by `x`) at `phase` (cycles; any real, wrapped
/// into the cycle); no point = 1. Allocation-free: the engine's real-time copy of a curve
/// (a fixed array) evaluates through it too.
pub fn lfo_value_at(pts: &[LfoPoint], phase: f64) -> f64 {
    let Some(first) = pts.first() else {
        return 1.0;
    };
    let last = pts[pts.len() - 1];
    if pts.len() == 1 {
        return f64::from(first.y);
    }
    let p = phase - phase.floor();
    let i = pts.partition_point(|q| f64::from(q.x) <= p);
    let (a, b, x0, x1) = if i == 0 {
        // Before the first point: the wrap segment from the previous cycle's last point.
        (last, *first, f64::from(last.x) - 1.0, f64::from(first.x))
    } else if i == pts.len() {
        (last, *first, f64::from(last.x), f64::from(first.x) + 1.0)
    } else {
        let (a, b) = (pts[i - 1], pts[i]);
        (a, b, f64::from(a.x), f64::from(b.x))
    };
    segment(a, b, x0, x1, p)
}

/// Value on the segment from `a` (at `x0`) to `b` (at `x1`) at `p`.
fn segment(a: LfoPoint, b: LfoPoint, x0: f64, x1: f64, p: f64) -> f64 {
    let span = x1 - x0;
    if span <= 0.0 {
        return f64::from(b.y);
    }
    let t = (p - x0) / span;
    let f = match a.curve {
        CurveShape::Linear => t.clamp(0.0, 1.0),
        CurveShape::Step => 0.0,
        CurveShape::Curve { tension } => curve_fraction(t, tension),
    };
    f64::from(a.y) + f64::from(b.y - a.y) * f
}

/// A lane of the shaper ([`ShaperCurves`]): what its drawn cycle moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum ShaperLane {
    /// Gain (`y` = linear gain).
    Volume,
    /// The filter's cutoff (`y` = position between the shaper's low and high cutoffs, on
    /// a log scale).
    Cutoff,
    /// Balance (`y` 0 = left, 0.5 = centre, 1 = right).
    Pan,
}

impl ShaperLane {
    /// Every lane, in panel order.
    pub const ALL: [ShaperLane; 3] = [Self::Volume, Self::Cutoff, Self::Pan];
}

/// The shaper's three drawn cycles (`BuiltinDevice::Shaper`), each played at its own
/// tempo-synced rate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ShaperCurves {
    pub volume: LfoCurve,
    pub cutoff: LfoCurve,
    pub pan: LfoCurve,
}

impl Default for ShaperCurves {
    /// Volume: the classic pump; cutoff: a curved rise over the cycle that drops back at
    /// its end; pan: a triangle from the centre to the right, the left and back.
    fn default() -> Self {
        Self {
            volume: LfoCurve::default(),
            cutoff: LfoCurve {
                points: vec![
                    LfoPoint::curved(0.0, 0.15, 0.4),
                    LfoPoint::linear(0.95, 1.0),
                ],
            },
            pan: LfoCurve {
                points: vec![
                    LfoPoint::linear(0.0, 0.5),
                    LfoPoint::linear(0.25, 1.0),
                    LfoPoint::linear(0.75, 0.0),
                ],
            },
        }
    }
}

impl ShaperCurves {
    /// The curve of `lane`.
    pub fn lane(&self, lane: ShaperLane) -> &LfoCurve {
        match lane {
            ShaperLane::Volume => &self.volume,
            ShaperLane::Cutoff => &self.cutoff,
            ShaperLane::Pan => &self.pan,
        }
    }

    /// These curves with `lane` replaced by `curve` (sanitized).
    pub fn with(&self, lane: ShaperLane, curve: &LfoCurve) -> Self {
        let mut c = self.clone();
        *match lane {
            ShaperLane::Volume => &mut c.volume,
            ShaperLane::Cutoff => &mut c.cutoff,
            ShaperLane::Pan => &mut c.pan,
        } = curve.sanitized();
        c
    }

    /// `Err` when a lane's curve isn't valid ([`LfoCurve::check`]).
    pub fn check(&self) -> Result<(), String> {
        for lane in ShaperLane::ALL {
            self.lane(lane).check()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaper_curves_default_valid_and_replace_one_lane() {
        let c = ShaperCurves::default();
        assert!(c.check().is_ok());
        let flat = LfoCurve {
            points: vec![LfoPoint::linear(0.0, 2.0)],
        };
        let d = c.with(ShaperLane::Pan, &flat);
        assert_eq!(d.pan.points, vec![LfoPoint::linear(0.0, 1.0)]);
        assert_eq!(d.volume, c.volume);
        assert_eq!(d.cutoff, c.cutoff);
    }

    #[test]
    fn evaluates_segments_wrap_and_edges() {
        let c = LfoCurve {
            points: vec![
                LfoPoint::linear(0.25, 0.0),
                LfoPoint::linear(0.5, 1.0),
                LfoPoint {
                    x: 0.75,
                    y: 0.5,
                    curve: CurveShape::Step,
                },
            ],
        };
        assert!(c.check().is_ok());
        assert!((c.value_at(0.375) - 0.5).abs() < 1e-9);
        assert!((c.value_at(0.5) - 1.0).abs() < 1e-9);
        // Step holds through the wrap, before the first point too.
        assert!((c.value_at(0.9) - 0.5).abs() < 1e-9);
        assert!((c.value_at(0.1) - 0.5).abs() < 1e-9);
        assert!((c.value_at(1.375) - 0.5).abs() < 1e-9);
        // A vertical edge: the later point wins from its x on.
        let e = LfoCurve {
            points: vec![
                LfoPoint::linear(0.0, 1.0),
                LfoPoint::linear(0.5, 1.0),
                LfoPoint::linear(0.5, 0.0),
            ],
        };
        assert!((e.value_at(0.4999) - 1.0).abs() < 1e-3);
        assert!(e.value_at(0.5).abs() < 1e-9);
        assert!((e.value_at(0.75) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn curved_segments_follow_the_automation_law() {
        let c = LfoCurve {
            points: vec![LfoPoint::curved(0.0, 0.0, 1.0), LfoPoint::linear(1.0, 1.0)],
        };
        assert!((c.value_at(0.5) - 0.5f64.powi(4)).abs() < 1e-9);
    }

    #[test]
    fn check_and_sanitize() {
        assert!(LfoCurve::default().check().is_ok());
        let bad = LfoCurve {
            points: vec![
                LfoPoint::linear(0.8, 2.0),
                LfoPoint::curved(0.2, f32::NAN, 0.0),
                LfoPoint::curved(-1.0, 0.5, 3.0),
            ],
        };
        assert!(bad.check().is_err());
        let s = bad.sanitized();
        assert!(s.check().is_ok());
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.points[0], LfoPoint::curved(0.0, 0.5, 1.0));
        assert!(LfoCurve { points: vec![] }.sanitized().check().is_ok());
        let many = LfoCurve {
            points: vec![LfoPoint::linear(0.5, 0.5); 100],
        };
        assert!(many.check().is_err());
        assert_eq!(many.sanitized().points.len(), MAX_LFO_POINTS);
    }
}
