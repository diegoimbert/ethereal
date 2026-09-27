//! Post-processing and file encoding of a rendered export, all stepped a bounded number of
//! frames at a time so a long file never blocks a controller tick: peak scan for
//! normalization ([`PeakScan`]; the gain is applied while encoding), TPDF dither for 16-bit,
//! WAV (`hound`) and FLAC (`flacenc`, frames written as they are encoded).

use std::io::{Cursor, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use ether_core::protocol::export::{AudioContainer, BitDepth};
use flacenc::bitsink::ByteSink;
use flacenc::component::{BitRepr, Stream, StreamInfo};
use flacenc::config;
use flacenc::error::{Verified, Verify};
use flacenc::source::{Fill, FrameBuf};

/// Peak level normalized files are scaled to: -0.1 dBFS.
pub(crate) const NORMALIZE_PEAK: f32 = 0.988_553_1;

/// FLAC block size (per channel).
const FLAC_BLOCK: usize = 4096;

/// Size of the FLAC header written in front of the frames: `fLaC` + one STREAMINFO block.
const FLAC_HEADER: usize = 4 + 4 + 34;

fn frames_of(channels: &[Vec<f32>]) -> usize {
    channels.first().map_or(0, Vec::len)
}

/// Peak of the signal, scanned in steps.
#[derive(Default)]
pub(crate) struct PeakScan {
    pos: usize,
    peak: f32,
}

impl PeakScan {
    /// Scan about `budget` more frames. `Some(peak)` when done.
    pub fn step(&mut self, channels: &[Vec<f32>], budget: usize) -> Option<f32> {
        let total = frames_of(channels);
        let end = self.pos.saturating_add(budget.max(1)).min(total);
        for c in channels {
            for s in &c[self.pos..end] {
                if s.is_finite() {
                    self.peak = self.peak.max(s.abs());
                }
            }
        }
        self.pos = end;
        (self.pos >= total).then_some(self.peak)
    }

    pub fn progress(&self, total: usize) -> f32 {
        if total == 0 {
            1.0
        } else {
            self.pos as f32 / total as f32
        }
    }
}

/// Gain that puts `peak` at [`NORMALIZE_PEAK`] (1 for silence).
pub(crate) fn normalize_gain(peak: f32) -> f32 {
    if peak <= 1e-9 {
        1.0
    } else {
        NORMALIZE_PEAK / peak
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

/// In-memory WAV target shared with the `hound` writer (which owns its writer and only
/// gives it back through `finalize`).
#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Cursor<Vec<u8>>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("unpoisoned").write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for SharedBuf {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.lock().expect("unpoisoned").seek(pos)
    }
}

/// WAV encoder stepped over an in-memory signal.
pub(crate) struct WavEncoder {
    writer: Option<hound::WavWriter<SharedBuf>>,
    buf: SharedBuf,
    bits: Option<u32>,
    dither: Option<Dither>,
    pos: usize,
}

fn wav_err(e: hound::Error) -> String {
    format!("WAV encoding failed: {e}")
}

impl WavEncoder {
    pub fn new(
        channels: usize,
        frames: usize,
        sample_rate: u32,
        depth: BitDepth,
    ) -> Result<Self, String> {
        let bits_per_sample = match depth {
            BitDepth::Int16 => 16,
            BitDepth::Int24 => 24,
            BitDepth::Float32 => 32,
        };
        let spec = hound::WavSpec {
            channels: channels as u16,
            sample_rate,
            bits_per_sample,
            sample_format: match depth {
                BitDepth::Float32 => hound::SampleFormat::Float,
                _ => hound::SampleFormat::Int,
            },
        };
        let buf = SharedBuf(Arc::new(Mutex::new(Cursor::new(Vec::with_capacity(
            44 + frames * channels * bits_per_sample as usize / 8,
        )))));
        let writer = hound::WavWriter::new(buf.clone(), spec).map_err(wav_err)?;
        let bits = int_bits(depth);
        Ok(Self {
            writer: Some(writer),
            buf,
            bits,
            dither: (bits == Some(16)).then(|| Dither::new(DITHER_SEED)),
            pos: 0,
        })
    }

    /// Write about `budget` more frames (times `gain`). `Some(bytes)` once the file is done.
    pub fn step(
        &mut self,
        channels: &[Vec<f32>],
        gain: f32,
        budget: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        let total = frames_of(channels);
        let end = self.pos.saturating_add(budget.max(1)).min(total);
        let w = self.writer.as_mut().ok_or("WAV encoder finished")?;
        for i in self.pos..end {
            for c in channels {
                let s = c[i] * gain;
                match self.bits {
                    None => w
                        .write_sample(if s.is_finite() { s } else { 0.0 })
                        .map_err(wav_err)?,
                    Some(bits) => w
                        .write_sample(quantize(s, bits, self.dither.as_mut()))
                        .map_err(wav_err)?,
                }
            }
        }
        self.pos = end;
        if self.pos < total {
            return Ok(None);
        }
        self.writer
            .take()
            .expect("checked")
            .finalize()
            .map_err(wav_err)?;
        let bytes = std::mem::take(self.buf.0.lock().expect("unpoisoned").get_mut());
        Ok(Some(bytes))
    }

    pub fn progress(&self, total: usize) -> f32 {
        if total == 0 {
            1.0
        } else {
            self.pos as f32 / total as f32
        }
    }
}

/// FLAC encoder stepped over an in-memory signal (fixed block size; the last block may be
/// shorter). Frames are serialized as they are encoded after room for the header, which is
/// filled in at the end. MD5 is left unset (all zero = "not computed", allowed).
pub(crate) struct FlacEncoder {
    config: Verified<config::Encoder>,
    /// Verbatim coding for a last block shorter than the FLAC minimum (flacenc's
    /// predictors don't handle those).
    short_config: Verified<config::Encoder>,
    info: StreamInfo,
    bytes: Vec<u8>,
    frame_count: usize,
    fb: FrameBuf,
    bits: u32,
    dither: Option<Dither>,
    pos: usize,
    interleaved: Vec<i32>,
}

fn flac_err(e: impl std::fmt::Display) -> String {
    format!("FLAC: {e}")
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
        let info =
            StreamInfo::new(sample_rate as usize, channels, bits as usize).map_err(flac_err)?;
        let fb = FrameBuf::with_size(channels, FLAC_BLOCK).map_err(flac_err)?;
        Ok(Self {
            config,
            short_config,
            info,
            bytes: vec![0; FLAC_HEADER],
            frame_count: 0,
            fb,
            bits,
            dither: (bits == 16).then(|| Dither::new(DITHER_SEED)),
            pos: 0,
            interleaved: Vec::new(),
        })
    }

    pub fn progress(&self, total: usize) -> f32 {
        if total == 0 {
            1.0
        } else {
            (self.pos as f32 / total as f32).min(1.0)
        }
    }

    /// Encode about `budget` more frames (times `gain`; at least one FLAC block).
    /// `Some(bytes)` once the file is done.
    pub fn step(
        &mut self,
        channels: &[Vec<f32>],
        gain: f32,
        budget: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        let total = frames_of(channels);
        let stop = self.pos.saturating_add(budget.max(1));
        while self.pos < total && self.pos < stop {
            let n = FLAC_BLOCK.min(total - self.pos);
            if n != self.fb.size() {
                self.fb.resize(n);
            }
            self.interleaved.clear();
            for i in self.pos..self.pos + n {
                for c in channels {
                    self.interleaved
                        .push(quantize(c[i] * gain, self.bits, self.dither.as_mut()));
                }
            }
            self.fb
                .fill_interleaved(&self.interleaved)
                .map_err(flac_err)?;
            let config = if n < 64 {
                &self.short_config
            } else {
                &self.config
            };
            let frame =
                flacenc::encode_fixed_size_frame(config, &self.fb, self.frame_count, &self.info)
                    .map_err(|e| format!("FLAC encoding failed: {e:?}"))?;
            self.info.update_frame_info(&frame);
            let mut sink = ByteSink::new();
            frame.write(&mut sink).map_err(flac_err)?;
            self.bytes.extend_from_slice(sink.as_slice());
            self.frame_count += 1;
            self.pos += n;
        }
        if self.pos < total {
            return Ok(None);
        }
        self.finish().map(Some)
    }

    fn finish(&mut self) -> Result<Vec<u8>, String> {
        let mut info = self.info.clone();
        info.set_total_samples(self.pos);
        // Fixed-size stream: the (shorter) last block doesn't count as the minimum.
        info.set_block_sizes(FLAC_BLOCK, FLAC_BLOCK)
            .map_err(flac_err)?;
        if self.frame_count == 0 {
            info.set_frame_sizes(0, 0).map_err(flac_err)?;
        }
        let mut sink = ByteSink::new();
        Stream::with_stream_info(info)
            .write(&mut sink)
            .map_err(flac_err)?;
        let header = sink.as_slice();
        if header.len() != FLAC_HEADER {
            return Err(format!("FLAC: unexpected header size {}", header.len()));
        }
        let mut bytes = std::mem::take(&mut self.bytes);
        bytes[..FLAC_HEADER].copy_from_slice(header);
        Ok(bytes)
    }
}

/// A file being encoded.
pub(crate) enum Encoder {
    Wav(Box<WavEncoder>),
    Flac(Box<FlacEncoder>),
}

impl Encoder {
    pub fn new(
        container: AudioContainer,
        depth: BitDepth,
        channels: usize,
        frames: usize,
        rate: u32,
    ) -> Result<Self, String> {
        Ok(match container {
            AudioContainer::Wav => {
                Self::Wav(Box::new(WavEncoder::new(channels, frames, rate, depth)?))
            }
            AudioContainer::Flac => Self::Flac(Box::new(FlacEncoder::new(channels, rate, depth)?)),
        })
    }

    pub fn step(
        &mut self,
        channels: &[Vec<f32>],
        gain: f32,
        budget: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        match self {
            Self::Wav(w) => w.step(channels, gain, budget),
            Self::Flac(f) => f.step(channels, gain, budget),
        }
    }

    pub fn progress(&self, total: usize) -> f32 {
        match self {
            Self::Wav(w) => w.progress(total),
            Self::Flac(f) => f.progress(total),
        }
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

    fn encode(
        container: AudioContainer,
        depth: BitDepth,
        s: &[Vec<f32>],
        gain: f32,
        budget: usize,
    ) -> (Vec<u8>, usize) {
        let mut enc = Encoder::new(container, depth, s.len(), frames_of(s), 48_000).unwrap();
        let mut steps = 1;
        loop {
            if let Some(b) = enc.step(s, gain, budget).unwrap() {
                return (b, steps);
            }
            steps += 1;
        }
    }

    #[test]
    fn peak_scan_and_gain() {
        let s = sine(10_000, 0.25);
        let mut scan = PeakScan::default();
        let mut steps = 1;
        let p = loop {
            if let Some(p) = scan.step(&s, 1000) {
                break p;
            }
            steps += 1;
        };
        assert_eq!(steps, 10);
        assert!((p - 0.25).abs() < 1e-3);
        assert!((normalize_gain(p) * p - NORMALIZE_PEAK).abs() < 1e-6);
        assert_eq!(normalize_gain(0.0), 1.0);
        assert_eq!(PeakScan::default().step(&[vec![], vec![]], 10), Some(0.0));
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
            let (bytes, steps) = encode(AudioContainer::Wav, depth, &s, 1.0, 1000);
            assert_eq!(steps, 3, "stepped");
            let d = decode(&bytes, "wav");
            assert_eq!(d.sample_rate, 48_000);
            assert_eq!(d.channels.len(), 2);
            assert_eq!(d.frames(), 3000);
            for (a, b) in d.channels.iter().zip(&s) {
                for (x, y) in a.iter().zip(b) {
                    assert!((x - y).abs() <= tol, "{depth:?}: {x} vs {y}");
                }
            }
        }
        // Gain is applied while encoding.
        let (bytes, _) = encode(AudioContainer::Wav, BitDepth::Float32, &s, 0.5, 4096);
        let d = decode(&bytes, "wav");
        assert_eq!(d.channels[0][10], s[0][10] * 0.5);
    }

    #[test]
    fn flac_round_trips() {
        // Not a multiple of the block size: the last frame is short.
        let s = sine(FLAC_BLOCK * 2 + 777, 0.5);
        for (depth, tol) in [
            (BitDepth::Int24, 2.0 / 8_388_607.0),
            (BitDepth::Int16, 2.5 / 32767.0),
        ] {
            let (bytes, steps) = encode(AudioContainer::Flac, depth, &s, 1.0, 1000);
            assert!(steps >= 3, "stepped");
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
            let (bytes, _) = encode(AudioContainer::Flac, BitDepth::Int16, &s, 1.0, 4096);
            assert_eq!(&bytes[..4], b"fLaC");
            if frames > 0 {
                assert_eq!(decode(&bytes, "flac").frames(), frames);
            }
        }
    }
}
