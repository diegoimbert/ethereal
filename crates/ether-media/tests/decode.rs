mod common;

use common::*;
use ether_media::{MediaError, decode};

const RATE: u32 = 48_000;

fn assert_close(got: &[Vec<f32>], want: &[Vec<f32>], tol: f32) {
    assert_eq!(got.len(), want.len(), "channel count");
    for (c, (g, w)) in got.iter().zip(want).enumerate() {
        assert_eq!(g.len(), w.len(), "frames in channel {c}");
        for (i, (a, b)) in g.iter().zip(w).enumerate() {
            assert!((a - b).abs() <= tol, "ch {c} frame {i}: {a} vs {b}");
        }
    }
}

fn fixture() -> Vec<Vec<f32>> {
    sine(2, 1000, RATE, 440.0, 0.8)
}

/// A 16-bit WAV whose data chunk claims `0xFFFFFFFF` bytes (ffmpeg's placeholder when it
/// writes to a pipe) around `src`.
pub fn wav_unknown_length(src: &[Vec<f32>]) -> Vec<u8> {
    let mut bytes = wav_pcm16(src, RATE);
    let data = bytes.windows(4).position(|w| w == b"data").unwrap();
    bytes[data + 4..data + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes
}

/// #554: the header's frame count is not trusted for the up-front reservation (it used to
/// reserve 1 GiB per channel here).
#[test]
fn a_lying_length_does_not_reserve_the_claimed_frames() {
    let src = sine(2, RATE as usize, RATE, 440.0, 0.8);
    let out = decode(&wav_unknown_length(&src), Some("wav")).unwrap();
    assert_close(&out.channels, &src, 1.0 / 16000.0);
    for ch in &out.channels {
        assert!(ch.capacity() <= 4 * RATE as usize, "{}", ch.capacity());
    }
}

#[test]
fn wav_pcm16_stereo() {
    let src = fixture();
    let out = decode(&wav_pcm16(&src, RATE), Some("wav")).unwrap();
    assert_eq!(out.sample_rate, RATE);
    assert_close(&out.channels, &src, 1.0 / 16000.0);
}

#[test]
fn wav_pcm24_and_float() {
    let src = fixture();
    let out = decode(&wav_pcm24(&src, RATE), Some("WAV")).unwrap();
    assert_close(&out.channels, &src, 1e-6);
    let out = decode(&wav_f32(&src, 44_100), None).unwrap();
    assert_eq!(out.sample_rate, 44_100);
    assert_close(&out.channels, &src, 0.0);
}

#[test]
fn wav_mono_odd_length() {
    let src = sine(1, 333, RATE, 100.0, 0.5);
    let out = decode(&wav_pcm16(&src, RATE), Some(".wav")).unwrap();
    assert_eq!(out.frames(), 333);
    assert_close(&out.channels, &src, 1.0 / 16000.0);
}

#[test]
fn aiff_16bit() {
    let src = fixture();
    let out = decode(&aiff_pcm16(&src, 44_100), Some("aiff")).unwrap();
    assert_eq!(out.sample_rate, 44_100);
    assert_close(&out.channels, &src, 1.0 / 16000.0);
}

#[test]
fn flac_verbatim() {
    // Not a multiple of the block size: the last frame is short.
    let src = sine(2, 1000, RATE, 440.0, 0.8);
    let out = decode(&flac16(&src, RATE), Some("flac")).unwrap();
    assert_eq!(out.sample_rate, RATE);
    assert_close(&out.channels, &src, 1.0 / 16000.0);
}

#[test]
fn ogg_container() {
    let src = sine(1, 700, 44_100, 440.0, 0.5);
    let out = decode(&ogg_flac16(&src, 44_100), Some("ogg")).unwrap();
    assert_eq!(out.sample_rate, 44_100);
    assert_close(&out.channels, &src, 1.0 / 16000.0);
}

#[test]
fn mp3_silence_decodes() {
    let out = decode(&mp3_silence(20), Some("mp3")).unwrap();
    assert_eq!(out.sample_rate, 44_100);
    assert_eq!(out.channels.len(), 1);
    // 1152 frames per MPEG frame; allow for decoder delay/trim and the bit reservoir.
    assert!(
        out.frames() >= 15 * 1152 && out.frames() <= 20 * 1152,
        "{}",
        out.frames()
    );
    assert!(out.channels[0].iter().all(|s| s.abs() < 1e-6));
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    let junk: Vec<u8> = (0..4096u32).map(|i| (i * 7919 % 251) as u8).collect();
    for ext in [None, Some("wav"), Some("mp3"), Some("ogg"), Some("flac")] {
        assert!(decode(&junk, ext).is_err(), "{ext:?}");
    }
    assert!(matches!(
        decode(&[], None),
        Err(MediaError::Unsupported(_) | MediaError::Decode(_))
    ));
}

#[test]
fn truncated_wav_keeps_what_is_there() {
    let src = fixture();
    let mut bytes = wav_pcm16(&src, RATE);
    bytes.truncate(bytes.len() - 400); // lose the last 100 stereo frames
    let out = decode(&bytes, Some("wav")).unwrap();
    assert!(
        out.frames() >= 800 && out.frames() <= 1000,
        "{}",
        out.frames()
    );
    let n = out.frames();
    let want: Vec<Vec<f32>> = src.iter().map(|c| c[..n].to_vec()).collect();
    assert_close(&out.channels, &want, 1.0 / 16000.0);
}

/// Zero crossings per second / 2: the frequency of a clean sine.
fn sine_hz(x: &[f32], rate: u32) -> f32 {
    let crossings = x
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    crossings as f32 / 2.0 / (x.len() as f32 / rate as f32)
}

/// `.m4a` (AAC-LC and Apple Lossless) and raw ADTS `.aac`: 440 Hz sines made with ffmpeg
/// (`-f lavfi -i sine=frequency=440:sample_rate=44100`, see tests/fixtures).
#[test]
fn m4a_and_aac_decode() {
    for (bytes, ext, channels, secs) in [
        (
            &include_bytes!("fixtures/sine440-aac.m4a")[..],
            "m4a",
            2,
            0.5,
        ),
        (
            &include_bytes!("fixtures/sine440-alac.m4a")[..],
            "m4a",
            1,
            0.25,
        ),
        (&include_bytes!("fixtures/sine440.aac")[..], "aac", 2, 0.5),
    ] {
        for hint in [Some(ext), None] {
            let out = decode(bytes, hint).unwrap_or_else(|e| panic!("{ext} {hint:?}: {e}"));
            assert_eq!(out.sample_rate, 44_100, "{ext}");
            assert_eq!(out.channels.len(), channels, "{ext}");
            let want = (44_100.0 * secs) as usize;
            // AAC frames are 1024 samples: allow a frame of priming/padding either way.
            assert!(
                out.frames().abs_diff(want) <= 2048,
                "{ext}: {} frames",
                out.frames()
            );
            for ch in &out.channels {
                // Skip the priming ramp.
                let body = &ch[2048.min(ch.len() / 4)..ch.len() - 1024];
                let hz = sine_hz(body, 44_100);
                assert!((hz - 440.0).abs() < 5.0, "{ext}: {hz} Hz");
                let peak = body.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(peak > 0.05, "{ext}: peak {peak}");
            }
        }
    }
}
