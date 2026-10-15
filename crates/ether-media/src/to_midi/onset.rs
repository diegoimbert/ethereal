//! Onset picking on a novelty curve (spectral flux).

/// Frames of `flux` that are onsets: local maxima (±`half` frames) above the local mean
/// plus `delta` (both relative to the curve's maximum), at least `min_gap` frames apart.
/// `gate(i)` can veto a frame (e.g. silence).
pub(crate) fn pick_peaks(
    flux: &[f32],
    half: usize,
    delta: f32,
    min_gap: usize,
    mut gate: impl FnMut(usize) -> bool,
) -> Vec<usize> {
    let max = flux.iter().copied().fold(0.0f32, f32::max);
    if max <= 1e-9 {
        return Vec::new();
    }
    let f: Vec<f32> = flux.iter().map(|v| v / max).collect();
    // Prefix sums for the local mean over [i - 12, i + 4].
    let mut pre = Vec::with_capacity(f.len() + 1);
    pre.push(0.0f64);
    for v in &f {
        pre.push(pre.last().unwrap() + *v as f64);
    }
    let n = f.len();
    let mut out: Vec<usize> = Vec::new();
    for i in 0..n {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        if f[lo..hi].iter().any(|&v| v > f[i]) {
            continue;
        }
        // Plateaus: only the first frame.
        if i > 0 && f[i - 1] == f[i] {
            continue;
        }
        let (a, b) = (i.saturating_sub(12), (i + 5).min(n));
        let mean = ((pre[b] - pre[a]) / (b - a) as f64) as f32;
        if f[i] < mean + delta {
            continue;
        }
        if !gate(i) {
            continue;
        }
        if let Some(&last) = out.last()
            && i - last < min_gap
        {
            // Keep the stronger of the two.
            if f[i] > f[last] {
                out.pop();
            } else {
                continue;
            }
        }
        out.push(i);
    }
    out
}

/// Log-compressed magnitude (onset functions are computed on it).
#[inline]
pub(crate) fn compress(mag: f32) -> f32 {
    (1.0 + 1000.0 * mag).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_clear_peaks_only() {
        let mut f = vec![0.01f32; 200];
        f[20] = 1.0;
        f[21] = 0.5;
        f[100] = 0.6;
        f[103] = 0.4; // too close to 100
        f[150] = 0.03; // noise
        let p = pick_peaks(&f, 3, 0.1, 8, |_| true);
        assert_eq!(p, vec![20, 100]);
    }
}
