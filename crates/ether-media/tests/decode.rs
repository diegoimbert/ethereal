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
    assert!(out.frames() >= 800 && out.frames() <= 1000, "{}", out.frames());
    let n = out.frames();
    let want: Vec<Vec<f32>> = src.iter().map(|c| c[..n].to_vec()).collect();
    assert_close(&out.channels, &want, 1.0 / 16000.0);
}
