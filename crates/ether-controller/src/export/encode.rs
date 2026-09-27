//! Post-processing and file encoding of a rendered export: peak normalization, TPDF dither
//! for 16-bit, WAV (`hound`, one shot) and FLAC (`flacenc`, stepped a few frames at a time
//! so a long file never blocks a tick).

use std::io::Cursor;

use ether_core::protocol::export::{AudioContainer, BitDepth};
use flacenc::bitsink::ByteSink;
use flacenc::component::{BitRepr, Frame, Stream, StreamInfo};
use flacenc::config;
use flacenc::error::{Verified, Verify};
use flacenc::source::{Fill, FrameBuf};

/// Peak level normalized files are scaled to: -0.1 dBFS.
pub(crate) const NORMALIZE_PEAK: f32 = 0.988_553_1;

/// FLAC block size (per channel).
const FLAC_BLOCK: usize = 4096;

/// Largest absolute sample over all channels.
pub(crate) fn peak(channels: &[Vec<f32>]) -> f32 {
    channels.iter().flat_map(|c| c.iter()).fold(0.0f32, |m, s| {
        if s.is_finite() { m.max(s.abs()) } else { m }
    })
}

/// Scale so the peak sits at [`NORMALIZE_PEAK`] (silence is left alone).
pub(crate) fn normalize(channels: &mut [Vec<f32>]) {
    let p = peak(channels);
    if p <= 1e-9 {
        return;
    }
    let gain = NORMALIZE_PEAK / p;
    for c in channels.iter_mut() {
        for s in c.iter_mut() {
            *s *= gain;
        }
    }
}

/// Deterministic TPDF dither source (xorshift64*), so exports are reproducible.
pub(crate) struct Dither(u64);

impl Dither {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn uniform(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (r >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Triangular noise in (-1, 1) LSB.
    pub fn next(&mut self) -> f32 {
        self.uniform() - self.uniform()
    }
}

/// Bits per sample of an integer depth.
pub(crate) fn int_bits(depth: BitDepth) -> Option<u32> {
    match depth {
        BitDepth::Int16 => Some(16),
        BitDepth::Int24 => Some(24),
        BitDepth::Float32 => None,
    }
}

/// Quantize one sample to a signed `bits`-bit integer (scale `2^(bits-1)`, clipped).
pub(crate) fn quantize(x: f32, bits: u32, dither: Option<&mut Dither>) -> i32 {
    let scale = (1i64 << (bits - 1)) as f64;
    let x = if x.is_finite() { x as f64 } else { 0.0 };
    let d = dither.map_or(0.0, |d| d.next() as f64);
    (x * scale + d).round().clamp(-scale, scale - 1.0) as i32
}

/// Seed of the 16-bit dither (fixed: identical renders give identical files).
pub(crate) const DITHER_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Interleaved integer samples of frames `from..to`.
fn interleave_int(
    channels: &[Vec<f32>],
    from: usize,
    to: usize,
    bits: u32,
    dither: &mut Option<Dither>,
    out: &mut Vec<i32>,
) {
    out.clear();
    for i in from..to {
        for c in channels {
            out.push(quantize(c[i], bits, dither.as_mut()));
        }
    }
}

/// Encode a whole WAV file in memory.
pub(crate) fn encode_wav(
    channels: &[Vec<f32>],
    sample_rate: u32,
    depth: BitDepth,
) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: channels.len() as u16,
        sample_rate,
        bits_per_sample: match depth {
            BitDepth::Int16 => 16,
            BitDepth::Int24 => 24,
            BitDepth::Float32 => 32,
        },
        sample_format: match depth {
            BitDepth::Float32 => hound::SampleFormat::Float,
            _ => hound::SampleFormat::Int,
        },
    };
    let frames = channels.first().map_or(0, Vec::len);
    let bytes_per_sample = spec.bits_per_sample as usize / 8;
    let mut cursor = Cursor::new(Vec::with_capacity(
        44 + frames * channels.len() * bytes_per_sample,
    ));
    let e = |e: hound::Error| format!("WAV encoding failed: {e}");
    {
        let mut w = hound::WavWriter::new(&mut cursor, spec).map_err(e)?;
        match int_bits(depth) {
            None => {
                for i in 0..frames {
                    for c in channels {
                        let s = c[i];
                        w.write_sample(if s.is_finite() { s } else { 0.0 })
                            .map_err(e)?;
                    }
                }
            }
            Some(bits) => {
                let mut dither = (bits == 16).then(|| Dither::new(DITHER_SEED));
                for i in 0..frames {
                    for c in channels {
                        w.write_sample(quantize(c[i], bits, dither.as_mut()))
                            .map_err(e)?;
                    }
                }
            }
        }
        w.finalize().map_err(e)?;
    }
    Ok(cursor.into_inner())
}

/// FLAC encoder stepped over an in-memory signal (fixed block size; the last block may be
/// shorter). MD5 is left unset (all zero = "not computed", allowed by the format).
pub(crate) struct FlacEncoder {
    config: Verified<config::Encoder>,
    /// Verbatim coding for a last block shorter than the FLAC minimum (flacenc's
    /// predictors don't handle those).
    short_config: Verified<config::Encoder>,
    info: StreamInfo,
    frames: Vec<Frame>,
    fb: FrameBuf,
    bits: u32,
    dither: Option<Dither>,
    pos: usize,
    interleaved: Vec<i32>,
}

impl FlacEncoder {
    pub fn new(channels: usize, sample_rate: u32, depth: BitDepth) -> Result<Self, String> {
        let bits = int_bits(depth).ok_or("FLAC supports 16 or 24 bits only")?;
        let mut cfg = config::Encoder::default();
        // Never spawn threads (controller thread / Web Worker).
        cfg.multithread = false;
        cfg.block_size = FLAC_BLOCK;
        let mut short = cfg.clone();
        short.subframe_coding.use_fixed = false;
        short.subframe_coding.use_lpc = false;
        let verify = |c: config::Encoder| {
            c.into_verified()
                .map_err(|(_, e)| format!("FLAC config: {e}"))
        };
        let config = verify(cfg)?;
        let short_config = verify(short)?;
        let info = StreamInfo::new(sample_rate as usize, channels, bits as usize)
            .map_err(|e| format!("FLAC: {e}"))?;
        let fb = FrameBuf::with_size(channels, FLAC_BLOCK).map_err(|e| format!("FLAC: {e}"))?;
        Ok(Self {
            config,
            short_config,
            info,
            frames: Vec::new(),
            fb,
            bits,
            dither: (bits == 16).then(|| Dither::new(DITHER_SEED)),
            pos: 0,
            interleaved: Vec::new(),
        })
    }

    /// Fraction of `total` frames encoded.
    pub fn progress(&self, total: usize) -> f32 {
        if total == 0 {
            1.0
        } else {
            (self.pos as f32 / total as f32).min(1.0)
        }
    }

    /// Encode about `budget` more frames of `channels`. `Ok(true)` when everything is in.
    pub fn step(&mut self, channels: &[Vec<f32>], budget: usize) -> Result<bool, String> {
        let total = channels.first().map_or(0, Vec::len);
        let stop = self.pos.saturating_add(budget.max(1));
        while self.pos < total && self.pos < stop {
            let n = FLAC_BLOCK.min(total - self.pos);
            if n != self.fb.size() {
                self.fb.resize(n);
            }
            interleave_int(
                channels,
                self.pos,
                self.pos + n,
                self.bits,
                &mut self.dither,
                &mut self.interleaved,
            );
            self.fb
                .fill_interleaved(&self.interleaved)
                .map_err(|e| format!("FLAC: {e}"))?;
            let config = if n < 64 {
                &self.short_config
            } else {
                &self.config
            };
            let frame =
                flacenc::encode_fixed_size_frame(config, &self.fb, self.frames.len(), &self.info)
                    .map_err(|e| format!("FLAC encoding failed: {e:?}"))?;
            self.info.update_frame_info(&frame);
            self.frames.push(frame);
            self.pos += n;
        }
        Ok(self.pos >= total)
    }

    /// The finished file.
    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        let e = |e: &dyn std::fmt::Display| format!("FLAC: {e}");
        let total = self.pos;
        self.info.set_total_samples(total);
        // Fixed-size stream: the (shorter) last block doesn't count as the minimum.
        self.info
            .set_block_sizes(FLAC_BLOCK, FLAC_BLOCK)
            .map_err(|x| e(&x))?;
        if self.frames.is_empty() {
            self.info.set_frame_sizes(0, 0).map_err(|x| e(&x))?;
        }
        let header = Stream::with_stream_info(self.info);
        let mut sink = ByteSink::new();
        header.write(&mut sink).map_err(|x| e(&x))?;
        for f in &self.frames {
            f.write(&mut sink).map_err(|x| e(&x))?;
        }
        Ok(sink.into_inner())
    }
}

/// File extension and MIME type of a container.
pub(crate) fn file_type(container: AudioContainer) -> (&'static str, &'static str) {
    match container {
        AudioContainer::Wav => ("wav", "audio/wav"),
        AudioContainer::Flac => ("flac", "audio/flac"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frames: usize, amp: f32) -> Vec<Vec<f32>> {
        (0..2)
            .map(|c| {
                (0..frames)
                    .map(|i| amp * ((i as f32 * 0.03) + c as f32).sin())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn normalize_hits_the_target_peak() {
        let mut s = sine(1000, 0.25);
        normalize(&mut s);
        assert!((peak(&s) - NORMALIZE_PEAK).abs() < 1e-6);
        let mut silent = vec![vec![0.0f32; 10]; 2];
        normalize(&mut silent);
        assert_eq!(peak(&silent), 0.0);
    }

    #[test]
    fn quantize_clamps_and_dither_is_small() {
        assert_eq!(quantize(1.0, 16, None), 32767);
        assert_eq!(quantize(-2.0, 16, None), -32768);
        assert_eq!(quantize(0.5, 24, None), 4_194_304);
        let mut d = Dither::new(DITHER_SEED);
        for _ in 0..10_000 {
            let q = quantize(0.0, 16, Some(&mut d));
            assert!((-1..=1).contains(&q));
        }
    }

    fn decode(bytes: &[u8], ext: &str) -> ether_media::DecodedAudio {
        ether_media::decode(bytes, Some(ext)).unwrap()
    }

    #[test]
    fn wav_round_trips() {
        let s = sine(3000, 0.5);
        for (depth, tol) in [
            (BitDepth::Float32, 0.0),
            (BitDepth::Int24, 2.0 / 8_388_607.0),
            (BitDepth::Int16, 2.5 / 32767.0),
        ] {
            let bytes = encode_wav(&s, 44_100, depth).unwrap();
            let d = decode(&bytes, "wav");
            assert_eq!(d.sample_rate, 44_100);
            assert_eq!(d.channels.len(), 2);
            assert_eq!(d.frames(), 3000);
            for (a, b) in d.channels.iter().zip(&s) {
                for (x, y) in a.iter().zip(b) {
                    assert!((x - y).abs() <= tol, "{depth:?}: {x} vs {y}");
                }
            }
        }
    }

    #[test]
    fn flac_round_trips() {
        // Not a multiple of the block size: the last frame is short.
        let s = sine(FLAC_BLOCK * 2 + 777, 0.5);
        for (depth, tol) in [
            (BitDepth::Int24, 2.0 / 8_388_607.0),
            (BitDepth::Int16, 2.5 / 32767.0),
        ] {
            let mut enc = FlacEncoder::new(2, 48_000, depth).unwrap();
            let mut steps = 0;
            while !enc.step(&s, 1000).unwrap() {
                steps += 1;
            }
            assert!(steps >= 2, "stepped");
            let bytes = enc.finish().unwrap();
            let d = decode(&bytes, "flac");
            assert_eq!(d.sample_rate, 48_000);
            assert_eq!(d.frames(), s[0].len());
            for (a, b) in d.channels.iter().zip(&s) {
                for (x, y) in a.iter().zip(b) {
                    assert!((x - y).abs() <= tol, "{depth:?}: {x} vs {y}");
                }
            }
        }
        assert!(FlacEncoder::new(2, 48_000, BitDepth::Float32).is_err());
    }

    #[test]
    fn short_and_empty_flac() {
        for frames in [0usize, 10, 100] {
            let s = sine(frames, 0.5);
            let mut enc = FlacEncoder::new(2, 48_000, BitDepth::Int16).unwrap();
            while !enc.step(&s, 4096).unwrap() {}
            let bytes = enc.finish().unwrap();
            assert_eq!(&bytes[..4], b"fLaC");
            if frames > 0 {
                assert_eq!(decode(&bytes, "flac").frames(), frames);
            }
        }
    }
}
