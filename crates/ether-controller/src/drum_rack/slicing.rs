//! Automatic sampler slicing (`SliceCommand::Auto`): onsets from the media's level-0 peaks
//! (the controller keeps no decoded audio; 32-frame min/max pairs resolve onsets to
//! ~0.7 ms at 48 kHz), grid and equal divisions.

use ether_core::protocol::media::PeakRequest;
use ether_core::protocol::model::MediaId;
use ether_media::{BASE_SAMPLES_PER_PEAK, PeakMipmap};

/// At most this many slices (one per MIDI key).
pub(crate) const MAX_SLICES: usize = 128;
/// Onsets closer than this (seconds) to the previous one are ignored.
const MIN_GAP: f64 = 0.05;
/// Length of the "background" window an onset must stand out from (seconds).
const WINDOW: f64 = 0.05;

/// Amplitude envelope (max |sample| over channels) per `BASE_SAMPLES_PER_PEAK` frames.
pub(crate) fn envelope(peaks: &PeakMipmap, media: MediaId) -> Vec<f32> {
    let data = peaks.query(&PeakRequest {
        media,
        samples_per_peak: BASE_SAMPLES_PER_PEAK,
        start_frame: 0.0,
        frame_count: peaks.frames() as f64,
    });
    let len = data.max.first().map_or(0, Vec::len);
    (0..len)
        .map(|i| {
            data.min
                .iter()
                .zip(&data.max)
                .map(|(lo, hi)| lo[i].abs().max(hi[i].abs()))
                .fold(0.0f32, f32::max)
        })
        .collect()
}

/// Onset times (seconds, ascending, starting with 0) in an amplitude envelope with
/// `frames_per_bin` source frames per value at `sample_rate`. `sensitivity` 0..=1 (higher =
/// more slices): a bin is an onset when it exceeds both an absolute floor (relative to the
/// loudest bin) and `ratio` × the mean of the preceding [`WINDOW`], at least [`MIN_GAP`]
/// after the previous onset.
pub(crate) fn transients(
    env: &[f32],
    frames_per_bin: u32,
    sample_rate: u32,
    sensitivity: f32,
) -> Vec<f64> {
    let s = if sensitivity.is_finite() {
        sensitivity.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let bin_secs = frames_per_bin as f64 / sample_rate.max(1) as f64;
    let window = ((WINDOW / bin_secs).round() as usize).max(1);
    let gap = ((MIN_GAP / bin_secs).round() as usize).max(1);
    let loudest = env.iter().copied().fold(0.0f32, f32::max);
    let mut out = vec![0.0];
    if loudest <= 0.0 {
        return out;
    }
    // Less sensitive = louder floor and a bigger jump over the background.
    let floor = loudest * (0.3 - 0.28 * s);
    let ratio = 6.0 - 4.5 * s;
    let mut last: Option<usize> = None;
    let mut sum = 0.0f64;
    for (i, &e) in env.iter().enumerate() {
        let n = i.min(window);
        let background = if n == 0 { 0.0 } else { (sum / n as f64) as f32 };
        let far = last.is_none_or(|l| i - l >= gap);
        if e >= floor && e > background * ratio && far {
            last = Some(i);
            if i > 0 && out.len() < MAX_SLICES {
                out.push(i as f64 * bin_secs);
            }
        }
        sum += e as f64;
        if i >= window {
            sum -= env[i - window] as f64;
        }
    }
    out
}

/// Markers every `step` seconds over `length` seconds (at most [`MAX_SLICES`]).
pub(crate) fn every(step: f64, length: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if !(step > 0.0 && step.is_finite() && length > 0.0) {
        return vec![0.0];
    }
    let mut i = 0u32;
    loop {
        let t = i as f64 * step;
        if t >= length - 1e-9 || out.len() == MAX_SLICES {
            break;
        }
        out.push(t);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_media::DecodedAudio;

    const SR: u32 = 48_000;

    /// Decaying noise bursts on a low noise floor, onsets at `hits` (seconds), gains given.
    fn signal(hits: &[(f64, f32)], secs: f64) -> DecodedAudio {
        let n = (secs * SR as f64) as usize;
        let mut x = vec![0.0f32; n];
        let mut seed = 0x1234_5678u32;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        for v in x.iter_mut() {
            *v = 0.002 * noise();
        }
        for &(t, g) in hits {
            let start = (t * SR as f64) as usize;
            for (k, v) in x[start..].iter_mut().enumerate() {
                let decay = (-(k as f32) / (0.04 * SR as f32)).exp();
                *v += g * decay * noise();
            }
        }
        DecodedAudio {
            sample_rate: SR,
            channels: vec![x.clone(), x],
        }
    }

    fn detect(audio: &DecodedAudio, sensitivity: f32) -> Vec<f64> {
        let peaks = PeakMipmap::build(audio);
        let env = envelope(&peaks, "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap());
        transients(&env, BASE_SAMPLES_PER_PEAK, SR, sensitivity)
    }

    #[test]
    fn finds_synthetic_transients_within_a_millisecond() {
        let hits = [(0.1, 0.9), (0.35, 0.6), (0.62, 0.8), (0.9, 0.3)];
        let audio = signal(&hits, 1.2);
        let found = detect(&audio, 0.5);
        assert_eq!(found.len(), hits.len() + 1, "{found:?}");
        assert_eq!(found[0], 0.0);
        for (f, (t, _)) in found[1..].iter().zip(hits) {
            assert!((f - t).abs() < 0.001, "{f} vs {t}");
        }
    }

    #[test]
    fn sensitivity_controls_quiet_hits() {
        let hits = [(0.1, 1.0), (0.5, 0.05)];
        let audio = signal(&hits, 1.0);
        assert_eq!(detect(&audio, 0.0).len(), 2, "quiet hit ignored");
        assert_eq!(detect(&audio, 1.0).len(), 3, "quiet hit found");
    }

    #[test]
    fn silence_has_one_slice() {
        let audio = DecodedAudio {
            sample_rate: SR,
            channels: vec![vec![0.0; 48_000]],
        };
        assert_eq!(detect(&audio, 1.0), vec![0.0]);
    }

    #[test]
    fn every_divides_and_caps() {
        assert_eq!(every(0.5, 2.0), vec![0.0, 0.5, 1.0, 1.5]);
        assert_eq!(every(0.001, 10.0).len(), MAX_SLICES);
        assert_eq!(every(0.0, 1.0), vec![0.0]);
    }
}
