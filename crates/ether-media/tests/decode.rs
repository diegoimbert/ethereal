mod common;

use common::*;
use ether_media::{DecodedAudio, MediaError, decode};

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

/// The header's frame count is not trusted for the up-front reservation (it used to
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

/// A header's sample rate outside `SAMPLE_RATES` is refused when the file is opened, before it
/// can size any buffer (and the resamplers refuse it too).
#[test]
fn sample_rate_out_of_range_is_refused() {
    let tone = vec![vec![0.25f32; 64]];
    for rate in [1, 999, 768_001, 4_000_000] {
        let wav = common::wav_pcm16(&tone, rate);
        assert!(
            matches!(decode(&wav, Some("wav")), Err(MediaError::Decode(_))),
            "{rate} Hz"
        );
        let audio = DecodedAudio {
            sample_rate: rate,
            channels: tone.clone(),
        };
        assert!(ether_media::resample(&audio, 48_000).is_err(), "{rate} Hz");
    }
    for rate in [8_000, 768_000] {
        let wav = common::wav_pcm16(&tone, rate);
        assert_eq!(decode(&wav, Some("wav")).unwrap().sample_rate, rate);
    }
}

/// A FLAC of `frames` frames of `block` samples on 2 channels, every subframe CONSTANT
/// (3 bytes per channel per frame, whatever the block size).
fn flac_constant(frames: u32, block: u16) -> Vec<u8> {
    fn crc8(data: &[u8]) -> u8 {
        let mut c = 0u8;
        for &b in data {
            c ^= b;
            for _ in 0..8 {
                c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
            }
        }
        c
    }
    fn crc16(data: &[u8]) -> u16 {
        let mut c = 0u16;
        for &b in data {
            c ^= u16::from(b) << 8;
            for _ in 0..8 {
                c = if c & 0x8000 != 0 { (c << 1) ^ 0x8005 } else { c << 1 };
            }
        }
        c
    }
    let mut out = b"fLaC".to_vec();
    out.extend_from_slice(&[0x80, 0, 0, 34]);
    out.extend_from_slice(&block.to_be_bytes());
    out.extend_from_slice(&block.to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    // 48 kHz (20 bits), 2 channels (3 bits: 1), 16 bits (5 bits: 15), 0 total samples (36 bits).
    let packed: u64 = (48_000u64 << 44) | (1 << 41) | (15 << 36);
    out.extend_from_slice(&packed.to_be_bytes());
    out.extend_from_slice(&[0; 16]);
    for n in 0..frames {
        let mut f = vec![0xFF, 0xF8, 0x70, 0x10];
        assert!(n < 2048);
        if n < 0x80 {
            f.push(n as u8);
        } else {
            f.push(0xC0 | (n >> 6) as u8);
            f.push(0x80 | (n & 0x3F) as u8);
        }
        f.extend_from_slice(&(block - 1).to_be_bytes());
        f.push(crc8(&f));
        for _ in 0..2 {
            f.extend_from_slice(&[0x00, 0x10, 0x00]);
        }
        let crc = crc16(&f);
        f.extend_from_slice(&crc.to_be_bytes());
        out.extend_from_slice(&f);
    }
    out
}

/// A small file that would decode to far more samples than its size allows is refused
/// (and a normal one of the same kind decodes).
#[test]
fn decoding_is_bounded_by_the_file_size() {
    let small = flac_constant(4, 4096);
    let audio = decode(&small, Some("flac")).unwrap();
    assert_eq!(audio.frames(), 4 * 4096);
    let bomb = flac_constant(300, 65_535);
    assert!(bomb.len() < 8 * 1024);
    assert!(
        300 * 65_535 * 2 > ether_media::max_decoded_samples(bomb.len()),
        "the fixture must pass the bound"
    );
    match decode(&bomb, Some("flac")) {
        Err(MediaError::Decode(e)) => assert!(e.contains("exceed"), "{e}"),
        other => panic!("{:?}", other.map(|a| a.frames())),
    }
}
