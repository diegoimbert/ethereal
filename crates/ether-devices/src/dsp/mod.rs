//! DSP building blocks for the roadmap v2 effects (`devices-2`): a TPT state-variable
//! filter with smoothable coefficients, a fractional delay line and a sliding-window
//! minimum. All buffers are allocated outside `process` (in `prepare`/`new`).

pub(crate) mod delay_line;
pub(crate) mod sliding_min;
pub(crate) mod svf;

/// Flush values that would decay into the subnormal range (the audio thread has no FTZ).
#[inline]
pub(crate) fn flush(x: f64) -> f64 {
    if x.abs() < 1e-18 { 0.0 } else { x }
}

/// Flush for `f32` signals.
#[inline]
pub(crate) fn flush32(x: f32) -> f32 {
    if x.abs() < 1e-18 { 0.0 } else { x }
}
