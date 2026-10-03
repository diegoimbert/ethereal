//! `audio-streaming`: chunked decoding matches the whole-file path, and a cache filled by
//! the policy serves a played-through stream without underruns.

mod common;

use std::sync::Arc;

use common::{aiff_pcm16, flac16, ogg_flac16, sine, wav_f32, wav_pcm16, wav_pcm24};
use ether_core::AudioSource;
use ether_media::stream::{BytesSource, CHUNK_FRAMES, ChunkDecoder, StreamFiller};
use ether_media::{DecodedAudio, decode, resample};

fn noise(channels: usize, frames: usize) -> Vec<Vec<f32>> {
    let mut x = 0x1234_5678u32;
    (0..channels)
        .map(|_| {
            (0..frames)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    (x as f32 / u32::MAX as f32) * 1.6 - 0.8
                })
                .collect()
        })
        .collect()
}

fn decoder(bytes: &[u8], ext: &str, frames: u64, rate: u32) -> ChunkDecoder {
    ChunkDecoder::new(
        Box::new(BytesSource::new(Arc::from(bytes))),
        Some(ext),
        frames,
        rate,
    )
    .unwrap()
}

/// All chunks, in `order`, assembled into whole channels.
fn assemble(dec: &mut ChunkDecoder, order: &[u64]) -> Vec<Vec<f32>> {
    let n = dec.frames() as usize;
    let mut out = vec![vec![f32::NAN; dec.chunk_count() as usize * CHUNK_FRAMES]; dec.channels() as usize];
    let mut buf = Vec::new();
    for &k in order {
        dec.decode_chunk(k, &mut buf).unwrap();
        for (o, b) in out.iter_mut().zip(&buf) {
            assert_eq!(b.len(), CHUNK_FRAMES);
            o[k as usize * CHUNK_FRAMES..(k as usize + 1) * CHUNK_FRAMES].copy_from_slice(b);
        }
    }
    for o in &mut out {
        assert!(o[n..].iter().all(|&v| v == 0.0), "zeros past the end");
        o.truncate(n);
    }
    out
}

fn shuffled(n: u64) -> Vec<u64> {
    // Deterministic scramble with backwards and forward jumps.
    let mut v: Vec<u64> = (0..n).collect();
    let mut x = 7u64;
    for i in (1..v.len()).rev() {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        v.swap(i, (x >> 33) as usize % (i + 1));
    }
    v
}

fn max_diff(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    a.iter()
        .zip(b)
        .flat_map(|(x, y)| {
            assert_eq!(x.len(), y.len());
            x.iter().zip(y).map(|(p, q)| (p - q).abs())
        })
        .fold(0.0, f32::max)
}

#[test]
fn same_rate_chunks_are_bit_identical_to_the_whole_file() {
    let frames = CHUNK_FRAMES * 5 + 1234;
    let audio = noise(2, frames);
    for (ext, bytes) in [
        ("wav", wav_pcm16(&audio, 48_000)),
        ("wav", wav_pcm24(&audio, 48_000)),
        ("wav", wav_f32(&audio, 48_000)),
        ("aiff", aiff_pcm16(&audio, 48_000)),
    ] {
        let whole = decode(&bytes, Some(ext)).unwrap();
        let mut dec = decoder(&bytes, ext, frames as u64, 48_000);
        assert_eq!(dec.frames(), frames as u64);
        let n = dec.chunk_count();
        let seq = assemble(&mut dec, &(0..n).collect::<Vec<_>>());
        assert_eq!(seq, whole.channels, "{ext}: sequential");
        let mut dec = decoder(&bytes, ext, frames as u64, 48_000);
        let random = assemble(&mut dec, &shuffled(n));
        assert_eq!(random, whole.channels, "{ext}: random access");
    }
    // The FLAC fixture encoder is limited to 127 frames of 256: under two chunks.
    let frames = 127 * 256;
    let audio = noise(2, frames);
    for (ext, bytes) in [
        ("flac", flac16(&audio, 48_000)),
        ("ogg", ogg_flac16(&audio, 48_000)),
    ] {
        let whole = decode(&bytes, Some(ext)).unwrap();
        for order in [[0, 1], [1, 0]] {
            let mut dec = decoder(&bytes, ext, frames as u64, 48_000);
            assert_eq!(assemble(&mut dec, &order), whole.channels, "{ext} {order:?}");
        }
    }
}

#[test]
fn document_length_cuts_or_pads() {
    let frames = CHUNK_FRAMES * 2 + 10;
    let audio = noise(1, frames);
    let bytes = wav_pcm16(&audio, 48_000);
    let whole = decode(&bytes, Some("wav")).unwrap();
    // Shorter document: cut.
    let mut dec = decoder(&bytes, "wav", 20_000, 48_000);
    let got = assemble(&mut dec, &[1, 0]);
    assert_eq!(got[0], whole.channels[0][..20_000]);
    // Longer document (header said more): zero padded.
    let mut dec = decoder(&bytes, "wav", frames as u64 + 5000, 48_000);
    let got = assemble(&mut dec, &[2, 0, 1]);
    assert_eq!(got[0][..frames], whole.channels[0][..]);
    assert!(got[0][frames..].iter().all(|&v| v == 0.0));
}

#[test]
fn resampled_chunks_match_the_whole_file_resample() {
    let frames = 44_100 * 3 + 17;
    let audio = sine(2, frames, 44_100, 997.0, 0.7);
    let bytes = wav_f32(&audio, 44_100);
    let whole = resample(&decode(&bytes, Some("wav")).unwrap(), 48_000).unwrap();
    let mut dec = decoder(&bytes, "wav", frames as u64, 48_000);
    assert_eq!(dec.frames() as usize, whole.frames());
    let n = dec.chunk_count();
    let seq = assemble(&mut dec, &(0..n).collect::<Vec<_>>());
    let d = max_diff(&seq, &whole.channels);
    assert!(d < 1e-5, "sequential differs by {d}");
    let mut dec = decoder(&bytes, "wav", frames as u64, 48_000);
    let random = assemble(&mut dec, &shuffled(n));
    let d = max_diff(&random, &whole.channels);
    assert!(d < 1e-4, "random access differs by {d}");
    // Downsampling too (96k → 48k).
    let audio = sine(1, 96_000 * 2, 96_000, 440.0, 0.5);
    let bytes = wav_f32(&audio, 96_000);
    let whole = resample(&decode(&bytes, Some("wav")).unwrap(), 48_000).unwrap();
    let mut dec = decoder(&bytes, "wav", 96_000 * 2, 48_000);
    let order = shuffled(dec.chunk_count());
    let random = assemble(&mut dec, &order);
    let d = max_diff(&random, &whole.channels);
    assert!(d < 1e-4, "96k random access differs by {d}");
}

/// Simulate playback: an "audio thread" reads 128-frame blocks and hints, a "reader"
/// fills between blocks at a fraction of real time. No underrun once primed.
#[test]
fn played_through_stream_has_no_underruns() {
    let frames = 48_000 * 40;
    let audio = noise(2, frames);
    let bytes = wav_pcm16(&audio, 48_000);
    let whole: DecodedAudio = decode(&bytes, Some("wav")).unwrap();
    let mut filler = StreamFiller::new(decoder(&bytes, "wav", frames as u64, 48_000), 12);
    let cache = filler.cache().clone();
    // Startup: head chunks resident before playback.
    filler.fill_all(0.0);
    let mut l = [0.0f32; 128];
    let mut r = [0.0f32; 128];
    let mut pos = 0u64;
    let mut now = 0.0;
    let mut checked = 0;
    while pos + 128 <= frames as u64 {
        assert!(cache.read(0, pos, &mut l), "underrun at {pos}");
        assert!(cache.read(1, pos, &mut r), "underrun at {pos}");
        cache.prefetch_hint(pos + 128);
        if pos % 4096 == 0 {
            assert_eq!(l[..], whole.channels[0][pos as usize..pos as usize + 128]);
            assert_eq!(r[..], whole.channels[1][pos as usize..pos as usize + 128]);
            checked += 1;
        }
        pos += 128;
        now += 128.0 / 48.0;
        // The reader thread wakes every ~5 ms and decodes one chunk per wake at most.
        if (pos / 128) % 2 == 0 {
            filler.fill_one(now);
        }
    }
    assert!(checked > 400);
    assert_eq!(cache.underruns(), 0);
    assert!(cache.memory_bytes() < 5 << 20);
}

/// A locate: the first read at the new position misses (the reader hasn't seen it), is
/// counted and becomes urgent; priming before the jump avoids it.
#[test]
fn locate_misses_unless_primed() {
    let frames = 48_000 * 60;
    let bytes = wav_pcm16(&noise(1, frames), 48_000);
    let mut filler = StreamFiller::new(decoder(&bytes, "wav", frames as u64, 48_000), 12);
    let cache = filler.cache().clone();
    filler.fill_all(0.0);
    let mut out = [0.0f32; 128];
    let target = 48_000 * 30;
    assert!(!cache.read(0, target, &mut out));
    assert_eq!(cache.underruns(), 1);
    filler.fill_all(1.0);
    assert!(cache.read(0, target, &mut out));
    // Primed jump.
    let target2 = 48_000 * 45;
    filler.urge_frames(&[target2], 2.0);
    while !filler.primed(&[target2]) {
        assert!(filler.fill_one(2.0));
    }
    assert!(cache.read(0, target2, &mut out));
    assert_eq!(cache.underruns(), 1);
}
