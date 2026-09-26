mod common;

use std::sync::Arc;

use common::sine;
use ether_core::AudioSource;
use ether_core::protocol::ids::MediaId;
use ether_core::protocol::media::PeakRequest;
use ether_media::{
    BASE_SAMPLES_PER_PEAK, DecodedAudio, InMemorySource, PeakMipmap, resample, resampled_len,
};

fn audio(rate: u32, channels: Vec<Vec<f32>>) -> DecodedAudio {
    DecodedAudio {
        sample_rate: rate,
        channels,
    }
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

// ---------------------------------------------------------------- resample

#[test]
fn resample_same_rate_is_identity() {
    let a = audio(48_000, sine(2, 100, 48_000, 440.0, 0.5));
    assert_eq!(resample(&a, 48_000).unwrap(), a);
}

#[test]
fn resample_lengths() {
    assert_eq!(resampled_len(44_100, 44_100, 48_000), 48_000);
    assert_eq!(resampled_len(1, 44_100, 48_000), 2);
    assert_eq!(resampled_len(0, 44_100, 48_000), 0);
    for (from, to, n) in [
        (44_100, 48_000, 10_000),
        (48_000, 44_100, 777),
        (96_000, 48_000, 5),
    ] {
        let a = audio(from, sine(2, n, from, 440.0, 0.5));
        let r = resample(&a, to).unwrap();
        assert_eq!(r.sample_rate, to);
        assert_eq!(r.channels.len(), 2);
        assert_eq!(r.frames(), resampled_len(n, from, to), "{from}->{to} n={n}");
    }
    let empty = audio(44_100, vec![vec![], vec![]]);
    let r = resample(&empty, 48_000).unwrap();
    assert_eq!((r.channels.len(), r.frames()), (2, 0));
    assert!(resample(&empty, 0).is_err());
}

/// A resampled sine must match the ideal sine at the new rate: aligned (delay compensated)
/// and with small error away from the edges.
#[test]
fn resample_preserves_sine_and_alignment() {
    for (from, to) in [(44_100u32, 48_000u32), (48_000, 44_100), (22_050, 48_000)] {
        let n = from as usize / 4;
        let a = audio(from, sine(1, n, from, 1000.0, 0.5));
        let r = resample(&a, to).unwrap();
        let ideal = &sine(1, r.frames(), to, 1000.0, 0.5)[0];
        let edge = 512;
        let got = &r.channels[0][edge..r.frames() - edge];
        let want = &ideal[edge..r.frames() - edge];
        let err: Vec<f32> = got.iter().zip(want).map(|(a, b)| a - b).collect();
        let snr = 20.0 * (rms(want) / rms(&err)).log10();
        assert!(snr > 60.0, "{from}->{to}: SNR {snr:.1} dB");
    }
}

#[test]
fn downsample_removes_content_above_nyquist() {
    // 20 kHz at 48k -> 22.05k (Nyquist 11.025k): must be filtered, not aliased.
    let a = audio(48_000, sine(1, 24_000, 48_000, 20_000.0, 0.5));
    let r = resample(&a, 22_050).unwrap();
    let mid = &r.channels[0][1000..r.frames() - 1000];
    assert!(rms(mid) < 0.005, "rms {}", rms(mid));
}

// ---------------------------------------------------------------- peaks

fn req(spp: u32, start: f64, count: f64) -> PeakRequest {
    PeakRequest {
        media: MediaId::NIL,
        samples_per_peak: spp,
        start_frame: start,
        frame_count: count,
    }
}

fn ramp(n: usize) -> Vec<f32> {
    // -1 .. +1 linear
    (0..n)
        .map(|i| -1.0 + 2.0 * i as f32 / (n - 1) as f32)
        .collect()
}

#[test]
fn peak_levels_are_powers_of_two_from_32() {
    let m = PeakMipmap::build(&audio(48_000, vec![ramp(10_000)]));
    let levels: Vec<u32> = m.levels().collect();
    assert_eq!(levels.first(), Some(&BASE_SAMPLES_PER_PEAK));
    for w in levels.windows(2) {
        assert_eq!(w[1], w[0] * 2);
    }
    // Coarsest level has exactly one peak covering everything.
    let top = *levels.last().unwrap();
    assert!(top as usize >= 10_000 && (top as usize) / 2 < 10_000);
    let p = m.query(&req(u32::MAX, 0.0, 10_000.0));
    assert_eq!(p.samples_per_peak, top);
    assert_eq!(p.min[0].len(), 1);
    assert!((p.min[0][0] + 1.0).abs() < 1e-4 && (p.max[0][0] - 1.0).abs() < 1e-4);
}

#[test]
fn peak_query_picks_level_and_aligns_range() {
    let n = 4096;
    let mut left = vec![0.0f32; n];
    let right = vec![0.25f32; n];
    left[100] = 0.9;
    left[3000] = -0.7;
    let m = PeakMipmap::build(&audio(48_000, vec![left, right]));

    // 50 is not a level: nearest >= is 64.
    let p = m.query(&req(50, 70.0, 200.0));
    assert_eq!(p.samples_per_peak, 64);
    assert_eq!(p.start_frame, 64.0); // widened down to the peak containing frame 70
    // frames [64, 270) -> peaks 1..=4
    assert_eq!(p.min[0].len(), 4);
    assert_eq!(p.min.len(), 2);
    assert!((p.max[0][0] - 0.9).abs() < 1e-4, "{:?}", p.max[0]);
    assert!(p.max[0][1..].iter().all(|&v| v.abs() < 1e-4));
    assert!(
        p.min[1]
            .iter()
            .chain(&p.max[1])
            .all(|&v| (v - 0.25).abs() < 1e-4)
    );

    // Exact level, whole file.
    let p = m.query(&req(1024, 0.0, n as f64));
    assert_eq!((p.samples_per_peak, p.min[0].len()), (1024, 4));
    assert!((p.min[0][2] + 0.7).abs() < 1e-4);
    assert!((p.max[0][0] - 0.9).abs() < 1e-4);

    // Requests below the base level use the base level.
    assert_eq!(m.query(&req(1, 0.0, 10.0)).samples_per_peak, 32);
    assert_eq!(m.query(&req(0, 0.0, 10.0)).samples_per_peak, 32);
}

#[test]
fn peak_query_clips_out_of_range() {
    let m = PeakMipmap::build(&audio(48_000, vec![ramp(1000)]));
    // Partial last peak: 1000 frames = 31.25 peaks -> 32 peaks at level 32.
    let p = m.query(&req(32, 0.0, 1e12));
    assert_eq!(p.min[0].len(), 32);
    // Past the end.
    let p = m.query(&req(32, 5000.0, 100.0));
    assert_eq!(p.min[0].len(), 0);
    // Negative / NaN inputs don't panic.
    let p = m.query(&req(32, -100.0, 150.0));
    assert_eq!((p.start_frame, p.min[0].len()), (0.0, 2));
    let p = m.query(&req(32, f64::NAN, f64::NAN));
    assert_eq!(p.min[0].len(), 0);
}

#[test]
fn peaks_are_conservative_and_clamped() {
    let data = vec![0.123_456_7f32, -0.765_432_1, 3.0, -2.0, f32::NAN];
    let m = PeakMipmap::build(&audio(48_000, vec![data.clone()]));
    let p = m.query(&req(32, 0.0, 5.0));
    assert_eq!(p.min[0], vec![-1.0]);
    assert_eq!(p.max[0], vec![1.0]);

    let m = PeakMipmap::build(&audio(48_000, vec![data[..2].to_vec()]));
    let p = m.query(&req(32, 0.0, 2.0));
    assert!(p.min[0][0] <= -0.765_432_1 && p.min[0][0] > -0.766);
    assert!(p.max[0][0] >= 0.123_456_7 && p.max[0][0] < 0.124);
}

#[test]
fn peaks_empty_audio() {
    let m = PeakMipmap::build(&audio(48_000, vec![vec![], vec![]]));
    let p = m.query(&req(256, 0.0, 1000.0));
    assert_eq!(p.min, vec![Vec::<f32>::new(), Vec::new()]);
    let back = PeakMipmap::from_bytes(&m.to_bytes()).unwrap();
    assert_eq!(back, m);
    // Default (never built) also answers.
    let p = PeakMipmap::default().query(&req(100, 0.0, 10.0));
    assert!(p.min.is_empty());
}

#[test]
fn peaks_bytes_roundtrip_and_validation() {
    let a = audio(44_100, sine(2, 12_345, 44_100, 50.0, 0.9));
    let m = PeakMipmap::build(&a);
    let bytes = m.to_bytes();
    // 2 ch * ceil(12345/32)=386 peaks * (min+max) * 2 bytes + header
    assert_eq!(bytes.len(), 19 + 2 * 386 * 4);
    let back = PeakMipmap::from_bytes(&bytes).unwrap();
    assert_eq!(back, m);
    assert_eq!((back.channels(), back.frames()), (2, 12_345));

    assert!(PeakMipmap::from_bytes(&[]).is_err());
    assert!(PeakMipmap::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(PeakMipmap::from_bytes(&extra).is_err());
    let mut magic = bytes.clone();
    magic[0] = b'X';
    assert!(PeakMipmap::from_bytes(&magic).is_err());
    let mut version = bytes;
    version[4] = 99;
    assert!(PeakMipmap::from_bytes(&version).is_err());
}

// ---------------------------------------------------------------- source

#[test]
fn in_memory_source_end_to_end() {
    let a = Arc::new(audio(48_000, sine(2, 100, 48_000, 440.0, 0.5)));
    let src = InMemorySource::new(a.clone());
    assert_eq!((src.channels(), src.frames()), (2, 100));
    let mut out = [7.0f32; 16];
    assert!(src.read(1, 90, &mut out));
    assert_eq!(&out[..10], &a.channels[1][90..]);
    assert!(out[10..].iter().all(|&s| s == 0.0));
    // Out-of-range channel and start read silence.
    assert!(src.read(5, 0, &mut out));
    assert!(out.iter().all(|&s| s == 0.0));
    out.fill(1.0);
    assert!(src.read(0, u64::MAX, &mut out));
    assert!(out.iter().all(|&s| s == 0.0));
}
