//! Clip fade shapes (roadmap v2, owned by the `clip-editing` node; see `docs/ROADMAP.md`).
//!
//! [`fade_gain`] is the one fade law shared by the engine (`sched::render_audio`, which
//! the clip-editing node switches from its linear ramp to this with a one-line call), the
//! export renderer and the UI's fade drawing (`ui/src/features/clip-editing` mirrors it).
//! Reverse playback (`ClipContentDesc::Audio::reversed`) is also implemented by the
//! clip-editing node in `sched.rs`.

use ether_protocol::model::FadeCurve;

/// Gain (0..=1) at normalized position `x` in a fade-in (0 = start, silent; 1 = end, full
/// level). A fade-out at normalized position `y` from its start uses `fade_gain(c, 1 - y)`.
/// `x` is clamped to 0..=1.
pub fn fade_gain(curve: FadeCurve, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    match curve {
        FadeCurve::Linear => x,
        FadeCurve::EqualPower => (x * std::f32::consts::FRAC_PI_2).sin(),
        // Same law as automation `CurveShape::Curve` (`x^(4^tension)`).
        FadeCurve::Curve { tension } => crate::automation::curve_fraction(x as f64, tension) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_and_shapes() {
        for c in [
            FadeCurve::Linear,
            FadeCurve::EqualPower,
            FadeCurve::Curve { tension: 0.7 },
            FadeCurve::Curve { tension: -0.7 },
        ] {
            assert_eq!(fade_gain(c, 0.0), 0.0);
            assert!((fade_gain(c, 1.0) - 1.0).abs() < 1e-6);
        }
        assert_eq!(fade_gain(FadeCurve::Linear, 0.25), 0.25);
        // Equal power: a symmetric crossfade keeps g_in² + g_out² = 1.
        let x = 0.3;
        let (a, b) = (
            fade_gain(FadeCurve::EqualPower, x),
            fade_gain(FadeCurve::EqualPower, 1.0 - x),
        );
        assert!((a * a + b * b - 1.0).abs() < 1e-6);
        assert!(fade_gain(FadeCurve::Curve { tension: 0.5 }, 0.5) < 0.5);
        assert!(fade_gain(FadeCurve::Curve { tension: -0.5 }, 0.5) > 0.5);
    }
}
