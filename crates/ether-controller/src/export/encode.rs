//! Post-processing and file encoding of a rendered export, fed a block at a time so a long
//! file never blocks a controller tick nor sits whole in memory: peak scan for
//! normalization ([`PeakScan`]; the gain is applied while encoding), TPDF dither for 16-bit,
//! WAV (`hound`), FLAC (`flacenc`) and MP3 (`rusty_mp3`, 320 kbps CBR with a gapless tag:
//! [`Mp3Encoder`]), each written into a [`Sink`] as it is encoded (a file streamed to disk,
//! or memory), its header filled in at the end.

use std::io::{Cursor, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use ether_core::protocol::export::{AudioContainer, BitDepth};
use flacenc::bitsink::ByteSink;
use flacenc::component::{BitRepr, Stream, StreamInfo};
use flacenc::config;
use flacenc::error::{Verified, Verify};
use flacenc::source::{Fill, FrameBuf};

use crate::store::ExportFile;

/// Peak level normalized files are scaled to: -0.1 dBFS.
pub const NORMALIZE_PEAK: f32 = 0.988_553_1;

/// FLAC block size (per channel).
const FLAC_BLOCK: usize = 4096;

/// Size of the FLAC header written in front of the frames: `fLaC` + one STREAMINFO block.
const FLAC_HEADER: usize = 4 + 4 + 34;

/// Peak of a signal fed block by block (non-finite samples are ignored).
#[derive(Clone, Copy, Debug, Default)]
pub struct PeakScan(f32);

impl PeakScan {
    pub fn scan(&mut self, block: &[&[f32]]) {
        for c in block {
            for s in *c {
                if s.is_finite() {
                    self.0 = self.0.max(s.abs());
                }
            }
        }
    }

    pub fn peak(self) -> f32 {
        self.0
    }
}

/// Gain that puts `peak` at [`NORMALIZE_PEAK`] (1 for silence).
pub fn normalize_gain(peak: f32) -> f32 {
    if peak <= 1e-9 {
        1.0
    } else {
        NORMALIZE_PEAK / peak
    }
}

/// Deterministic TPDF dither source (xorshift64*), so exports are reproducible.
pub struct Dither(u64);

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
    pub(crate) fn next(&mut self) -> f32 {
        self.uniform() - self.uniform()
    }
}

/// Bits per sample of an integer depth.
pub fn int_bits(depth: BitDepth) -> Option<u32> {
    match depth {
        BitDepth::Int16 => Some(16),
        BitDepth::Int24 => Some(24),
        BitDepth::Float32 => None,
    }
}

/// Quantize one sample to a signed `bits`-bit integer (scale `2^(bits-1)`, clipped).
pub fn quantize(x: f32, bits: u32, dither: Option<&mut Dither>) -> i32 {
    let scale = (1i64 << (bits - 1)) as f64;
    let x = if x.is_finite() { x as f64 } else { 0.0 };
    let d = dither.map_or(0.0, |d| d.next() as f64);
    (x * scale + d).round().clamp(-scale, scale - 1.0) as i32
}

/// Seed of the 16-bit dither (fixed: identical renders give identical files).
pub const DITHER_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Where an encoder writes a file: memory (downloads, and stores that only take
/// whole files) or a file the store streams to disk ([`ProjectStore::create_export`]).
/// Seekable: WAV, FLAC and MP3 headers are filled in once the file is done.
///
/// [`ProjectStore::create_export`]: crate::store::ProjectStore::create_export
pub enum Sink {
    Memory(Cursor<Vec<u8>>),
    File(Box<dyn ExportFile>),
}

impl Sink {
    /// An in-memory file, `capacity` bytes reserved.
    pub fn memory(capacity: usize) -> Self {
        Self::Memory(Cursor::new(Vec::with_capacity(capacity)))
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Memory(c) => c.write(buf),
            Self::File(f) => f.write(buf),
        }
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Memory(c) => c.write_all(buf),
            Self::File(f) => f.write_all(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Memory(_) => Ok(()),
            Self::File(f) => f.flush(),
        }
    }
}

impl Seek for Sink {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Memory(c) => c.seek(pos),
            Self::File(f) => f.seek(pos),
        }
    }
}

fn io_err(what: &str) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("{what}: could not write the file: {e}")
}

/// Write `header` at the start of `sink`, then go back to its end.
fn patch_header(sink: &mut Sink, header: &[u8], what: &str) -> Result<(), String> {
    let end = sink.seek(SeekFrom::End(0)).map_err(io_err(what))?;
    sink.seek(SeekFrom::Start(0)).map_err(io_err(what))?;
    sink.write_all(header).map_err(io_err(what))?;
    sink.seek(SeekFrom::Start(end)).map_err(io_err(what))?;
    Ok(())
}

/// The sink shared with the `hound` writer (which owns its writer and only gives it back
/// through `finalize`).
#[derive(Clone)]
struct SharedSink(Arc<Mutex<Sink>>);

impl Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("unpoisoned").write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().expect("unpoisoned").flush()
    }
}

impl Seek for SharedSink {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.lock().expect("unpoisoned").seek(pos)
    }
}

/// Frames of a block (one slice per channel).
fn block_frames(block: &[&[f32]]) -> usize {
    block.first().map_or(0, |c| c.len())
}

/// WAV encoder fed block by block.
pub struct WavEncoder {
    writer: hound::WavWriter<SharedSink>,
    sink: SharedSink,
    bits: Option<u32>,
    dither: Option<Dither>,
}

fn wav_err(e: hound::Error) -> String {
    format!("WAV encoding failed: {e}")
}

impl WavEncoder {
    pub fn new(
        channels: usize,
        sample_rate: u32,
        depth: BitDepth,
        sink: Sink,
    ) -> Result<Self, String> {
        let bits_per_sample = wav_bits(depth);
        let spec = hound::WavSpec {
            channels: channels as u16,
            sample_rate,
            bits_per_sample,
            sample_format: match depth {
                BitDepth::Float32 => hound::SampleFormat::Float,
                _ => hound::SampleFormat::Int,
            },
        };
        let sink = SharedSink(Arc::new(Mutex::new(sink)));
        let writer = hound::WavWriter::new(sink.clone(), spec).map_err(wav_err)?;
        let bits = int_bits(depth);
        Ok(Self {
            writer,
            sink,
            bits,
            dither: (bits == Some(16)).then(|| Dither::new(DITHER_SEED)),
        })
    }

    /// Write a block (times `gain`).
    pub fn push(&mut self, block: &[&[f32]], gain: f32) -> Result<(), String> {
        for i in 0..block_frames(block) {
            for c in block {
                let s = c[i] * gain;
                match self.bits {
                    None => self
                        .writer
                        .write_sample(if s.is_finite() { s } else { 0.0 })
                        .map_err(wav_err)?,
                    Some(bits) => self
                        .writer
                        .write_sample(quantize(s, bits, self.dither.as_mut()))
                        .map_err(wav_err)?,
                }
            }
        }
        Ok(())
    }

    /// Finish the file (header sizes filled in).
    pub fn finish(self) -> Result<Sink, String> {
        self.writer.finalize().map_err(wav_err)?;
        let sink = Arc::try_unwrap(self.sink.0)
            .map_err(|_| "WAV: the writer is still in use".to_string())?;
        Ok(sink.into_inner().expect("unpoisoned"))
    }
}

/// Bits per WAV sample of a depth.
pub fn wav_bits(depth: BitDepth) -> u16 {
    match depth {
        BitDepth::Int16 => 16,
        BitDepth::Int24 => 24,
        BitDepth::Float32 => 32,
    }
}

/// FLAC encoder fed block by block (fixed block size; the last block may be shorter).
/// Frames are written as they are encoded after room for the header, which is filled in
/// at the end. MD5 is left unset (all zero = "not computed", allowed).
pub struct FlacEncoder {
    config: Verified<config::Encoder>,
    /// Verbatim coding for a last block shorter than the FLAC minimum (flacenc's
    /// predictors don't handle those).
    short_config: Verified<config::Encoder>,
    info: StreamInfo,
    sink: Sink,
    channels: usize,
    frame_count: usize,
    fb: FrameBuf,
    bits: u32,
    dither: Option<Dither>,
    /// Frames encoded or pending.
    pos: usize,
    /// The quantized samples of the block being filled (interleaved).
    interleaved: Vec<i32>,
}

fn flac_err(e: impl std::fmt::Display) -> String {
    format!("FLAC: {e}")
}

impl FlacEncoder {
    pub fn new(
        channels: usize,
        sample_rate: u32,
        depth: BitDepth,
        mut sink: Sink,
    ) -> Result<Self, String> {
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
        sink.write_all(&[0; FLAC_HEADER]).map_err(io_err("FLAC"))?;
        Ok(Self {
            config,
            short_config,
            info,
            sink,
            channels,
            frame_count: 0,
            fb,
            bits,
            dither: (bits == 16).then(|| Dither::new(DITHER_SEED)),
            pos: 0,
            interleaved: Vec::with_capacity(FLAC_BLOCK * channels),
        })
    }

    /// Encode a block (times `gain`): every FLAC block it completes is written.
    pub fn push(&mut self, block: &[&[f32]], gain: f32) -> Result<(), String> {
        for i in 0..block_frames(block) {
            for c in block {
                self.interleaved
                    .push(quantize(c[i] * gain, self.bits, self.dither.as_mut()));
            }
            self.pos += 1;
            if self.interleaved.len() == FLAC_BLOCK * self.channels {
                self.encode_block()?;
            }
        }
        Ok(())
    }

    /// Encode and write the pending samples as one frame.
    fn encode_block(&mut self) -> Result<(), String> {
        let n = self.interleaved.len() / self.channels.max(1);
        if n != self.fb.size() {
            self.fb.resize(n);
        }
        self.fb
            .fill_interleaved(&self.interleaved)
            .map_err(flac_err)?;
        self.interleaved.clear();
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
        self.sink
            .write_all(sink.as_slice())
            .map_err(io_err("FLAC"))?;
        self.frame_count += 1;
        Ok(())
    }

    /// Finish the file: the last (short) block, then the header.
    pub fn finish(mut self) -> Result<Sink, String> {
        if !self.interleaved.is_empty() {
            self.encode_block()?;
        }
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
        patch_header(&mut self.sink, header, "FLAC")?;
        Ok(self.sink)
    }
}

/// MP3 bitrate (CBR, kbps): the most MPEG-1 Layer III allows.
pub const MP3_KBPS: u32 = 320;

/// Samples per channel in an MPEG-1 Layer III frame.
pub const MP3_FRAME: usize = 1152;

/// Delay of a Layer III decoder's synthesis filterbank, in samples. The LAME tag's encoder
/// delay leaves it out; gapless decoders skip `delay + 529` samples.
pub const MP3_DECODER_DELAY: usize = 529;

/// `rusty_mp3`'s encoder delay in the LAME-tag convention (decoder delay excluded): input
/// sample `i` comes out of a decoder at `i + MP3_ENCODER_DELAY + MP3_DECODER_DELAY`,
/// counted from the first audio frame (the test `mp3_lead_is_the_codec_delay` pins it).
pub const MP3_ENCODER_DELAY: usize = 528;

/// The rates MPEG-1 Layer III encodes.
pub const MP3_RATES: [u32; 3] = [32_000, 44_100, 48_000];

/// The MP3 rate for a signal at `rate` (when the request names none): `rate` itself when
/// MP3 has it, else 44.1 kHz for its multiples and 48 kHz for the rest.
pub fn mp3_rate(rate: u32) -> u32 {
    if MP3_RATES.contains(&rate) {
        rate
    } else if rate.is_multiple_of(11_025) {
        44_100
    } else {
        48_000
    }
}

/// Frames an MP3 of `frames` input frames decodes to after the gapless trim: the input
/// rounded up so that it, the encoder delay and the decoder delay fill whole codec frames.
/// The tail padding is then exactly [`MP3_DECODER_DELAY`], so whatever a decoder trims, the
/// extra samples it outputs are at the start (see [`Mp3Encoder`]).
pub fn mp3_trimmed_frames(frames: usize) -> usize {
    let delay = MP3_ENCODER_DELAY + MP3_DECODER_DELAY;
    (frames + delay).div_ceil(MP3_FRAME) * MP3_FRAME - delay
}

/// Bytes of an unpadded 320 kbps Layer III frame at `rate`: the size of the Info frame,
/// whose room is kept at the start of the file until the end.
fn mp3_info_frame_len(rate: u32) -> usize {
    (144 * MP3_KBPS as usize * 1000) / rate as usize
}

/// CRC-16/ARC (polynomial 0x8005 reflected, initial 0): the LAME tag's CRCs.
fn crc16(crc: u16, bytes: &[u8]) -> u16 {
    bytes.iter().fold(crc, |mut crc, &b| {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
        crc
    })
}

/// MP3 encoder fed block by block: 320 kbps CBR, mono or stereo (more channels: the first
/// two).
///
/// Gapless layout. The input is the signal, then silence up to [`mp3_trimmed_frames`],
/// then [`MP3_ENCODER_DELAY`] + [`MP3_DECODER_DELAY`] more silent samples, which makes a
/// whole number of codec frames (the encoder pads no partial frame, and the tail of the
/// signal is never cut). The file starts with an Info frame (Xing layout: frame count,
/// byte count, TOC) carrying a LAME-format tag: encoder delay [`MP3_ENCODER_DELAY`],
/// padding [`MP3_DECODER_DELAY`]. A decoder that applies the tag outputs exactly
/// `mp3_trimmed_frames(n)` samples; one that does not outputs the delay (and maybe a silent
/// frame for the Info frame) first, then the same samples. Either way the extra samples are
/// at the start: `decoded length - mp3_trimmed_frames(n)` is the lead to skip, which is how
/// the static share listener aligns the files.
///
/// The audio frames are written as they are encoded, after room for the Info frame, which
/// is written last (its counts and CRC cover the whole file).
pub struct Mp3Encoder {
    enc: rusty_mp3::Mp3Encoder,
    rate: u32,
    channels: usize,
    sink: Sink,
    /// Signal frames pushed so far.
    pos: usize,
    interleaved: Vec<f32>,
    /// Audio frames written, their bytes and CRC.
    audio_frames: u32,
    audio_bytes: usize,
    audio_crc: u16,
}

impl Mp3Encoder {
    pub fn new(channels: usize, rate: u32, mut sink: Sink) -> Result<Self, String> {
        if !MP3_RATES.contains(&rate) {
            return Err(format!("MP3 supports 32, 44.1 or 48 kHz, not {rate} Hz"));
        }
        if channels == 0 {
            return Err("MP3: no audio channels".into());
        }
        sink.write_all(&vec![0; mp3_info_frame_len(rate)])
            .map_err(io_err("MP3"))?;
        Ok(Self {
            enc: rusty_mp3::Mp3Encoder::new(rusty_mp3::Mp3EncoderConfig {
                bitrate_kbps: MP3_KBPS,
                vbr_quality: None,
            }),
            rate,
            channels: channels.min(2),
            sink,
            pos: 0,
            interleaved: Vec::new(),
            audio_frames: 0,
            audio_bytes: 0,
            audio_crc: 0,
        })
    }

    /// Input frames for a signal of `frames`: a whole number of codec frames.
    fn input_frames(frames: usize) -> usize {
        mp3_trimmed_frames(frames) + MP3_ENCODER_DELAY + MP3_DECODER_DELAY
    }

    /// Write the encoded audio frames.
    fn drain(&mut self) -> Result<(), String> {
        while let Ok(p) = self.enc.next_packet() {
            self.sink.write_all(&p).map_err(io_err("MP3"))?;
            self.audio_crc = crc16(self.audio_crc, &p);
            self.audio_bytes += p.len();
            self.audio_frames += 1;
        }
        Ok(())
    }

    /// Encode `frames` frames: `block`'s (times `gain`), or silence.
    fn encode(&mut self, block: Option<&[&[f32]]>, frames: usize, gain: f32) -> Result<(), String> {
        self.interleaved.clear();
        for i in 0..frames {
            for c in 0..self.channels {
                let s = block.and_then(|b| b.get(c)).map_or(0.0, |c| c[i] * gain);
                self.interleaved.push(if s.is_finite() { s } else { 0.0 });
            }
        }
        self.enc
            .push_pcm_f32(&self.interleaved, self.channels as u16, self.rate)
            .map_err(|e| format!("MP3 encoding failed: {e}"))?;
        self.drain()
    }

    /// Encode a block (times `gain`).
    pub fn push(&mut self, block: &[&[f32]], gain: f32) -> Result<(), String> {
        let n = block_frames(block);
        self.pos += n;
        self.encode(Some(block), n, gain)
    }

    /// Finish the file: the gapless padding, then the Info frame.
    pub fn finish(mut self) -> Result<Sink, String> {
        let pad = Self::input_frames(self.pos) - self.pos;
        self.encode(None, pad, 1.0)?;
        self.enc.finish();
        // The input is whole frames: all that is left is the encoder's Info frame, replaced
        // by ours (same header and size, with the gapless tag).
        let info = self.enc.next_packet().ok();
        self.drain()?;
        let info = info.ok_or("MP3: no frames encoded")?;
        let tag = 4 + if self.channels == 1 { 17 } else { 32 };
        if info.len() != mp3_info_frame_len(self.rate)
            || info.len() < tag + 156
            || &info[tag..tag + 4] != b"Info"
        {
            return Err("MP3: unexpected Info frame".into());
        }
        let file_bytes = u32::try_from(info.len() + self.audio_bytes)
            .map_err(|_| "MP3: the file is too large")?;
        let mut out = Vec::with_capacity(info.len());
        out.extend_from_slice(&info[..tag]);
        out.extend_from_slice(b"Info");
        // Frames, bytes, TOC and quality fields present (as LAME writes them).
        out.extend_from_slice(&0x0000_000Fu32.to_be_bytes());
        out.extend_from_slice(&self.audio_frames.to_be_bytes());
        out.extend_from_slice(&file_bytes.to_be_bytes());
        // CBR: the TOC is linear.
        out.extend((0..100u32).map(|i| (i * 256 / 100) as u8));
        out.extend_from_slice(&0u32.to_be_bytes());
        // The LAME-format tag (36 bytes; the vendor string is ours).
        out.extend_from_slice(b"Ethereal ");
        out.push(0x01); // tag revision 0, CBR
        out.push(0); // lowpass: not given
        out.extend_from_slice(&[0; 8]); // no ReplayGain
        out.push(0); // encoding flags, ATH type
        out.push(255); // bitrate (CBR, 255 = 255 kbps or more)
        let gapless = ((MP3_ENCODER_DELAY as u32) << 12) | MP3_DECODER_DELAY as u32;
        out.extend_from_slice(&gapless.to_be_bytes()[1..]);
        let source_rate = match self.rate {
            44_100 => 1,
            48_000 => 2,
            _ => 0,
        };
        let stereo_mode = u8::from(self.channels == 2);
        out.push((source_rate << 6) | (stereo_mode << 2));
        out.push(0); // MP3 gain
        out.extend_from_slice(&[0; 2]); // preset, surround
        out.extend_from_slice(&file_bytes.to_be_bytes());
        out.extend_from_slice(&self.audio_crc.to_be_bytes());
        let tag_crc = crc16(0, &out);
        out.extend_from_slice(&tag_crc.to_be_bytes());
        out.resize(info.len(), 0);
        patch_header(&mut self.sink, &out, "MP3")?;
        Ok(self.sink)
    }
}

/// A file being encoded, fed block by block ([`Encoder::push`]), then finished.
pub enum Encoder {
    Wav(Box<WavEncoder>),
    Flac(Box<FlacEncoder>),
    Mp3(Box<Mp3Encoder>),
}

impl Encoder {
    pub fn new(
        container: AudioContainer,
        depth: BitDepth,
        channels: usize,
        rate: u32,
        sink: Sink,
    ) -> Result<Self, String> {
        Ok(match container {
            AudioContainer::Wav => {
                Self::Wav(Box::new(WavEncoder::new(channels, rate, depth, sink)?))
            }
            AudioContainer::Flac => {
                Self::Flac(Box::new(FlacEncoder::new(channels, rate, depth, sink)?))
            }
            AudioContainer::Mp3 => Self::Mp3(Box::new(Mp3Encoder::new(channels, rate, sink)?)),
        })
    }

    /// Encode a block (one slice per channel, times `gain`).
    pub fn push(&mut self, block: &[&[f32]], gain: f32) -> Result<(), String> {
        match self {
            Self::Wav(w) => w.push(block, gain),
            Self::Flac(f) => f.push(block, gain),
            Self::Mp3(m) => m.push(block, gain),
        }
    }

    /// Finish the file; the sink holds it.
    pub fn finish(self) -> Result<Sink, String> {
        match self {
            Self::Wav(w) => w.finish(),
            Self::Flac(f) => f.finish(),
            Self::Mp3(m) => m.finish(),
        }
    }
}

/// File extension and MIME type of a container.
pub fn file_type(container: AudioContainer) -> (&'static str, &'static str) {
    match container {
        AudioContainer::Wav => ("wav", "audio/wav"),
        AudioContainer::Flac => ("flac", "audio/flac"),
        AudioContainer::Mp3 => ("mp3", "audio/mpeg"),
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

    fn frames_of(channels: &[Vec<f32>]) -> usize {
        channels.first().map_or(0, Vec::len)
    }

    /// Encode `s` at `rate` fed in blocks of `block` frames: the file and the blocks fed.
    fn encode_at(
        container: AudioContainer,
        depth: BitDepth,
        s: &[Vec<f32>],
        gain: f32,
        block: usize,
        rate: u32,
    ) -> (Vec<u8>, usize) {
        let mut enc = Encoder::new(container, depth, s.len(), rate, Sink::memory(0)).unwrap();
        let (n, mut at, mut blocks) = (frames_of(s), 0, 0);
        while at < n {
            let k = block.min(n - at);
            let b: Vec<&[f32]> = s.iter().map(|c| &c[at..at + k]).collect();
            enc.push(&b, gain).unwrap();
            at += k;
            blocks += 1;
        }
        match enc.finish().unwrap() {
            Sink::Memory(c) => (c.into_inner(), blocks.max(1)),
            Sink::File(_) => unreachable!(),
        }
    }

    fn encode(
        container: AudioContainer,
        depth: BitDepth,
        s: &[Vec<f32>],
        gain: f32,
        block: usize,
    ) -> (Vec<u8>, usize) {
        encode_at(container, depth, s, gain, block, 48_000)
    }

    #[test]
    fn peak_scan_and_gain() {
        let s = sine(10_000, 0.25);
        let mut scan = PeakScan::default();
        for b in s[0].chunks(1000).zip(s[1].chunks(1000)) {
            scan.scan(&[b.0, b.1]);
        }
        let p = scan.peak();
        assert!((p - 0.25).abs() < 1e-3);
        assert!((normalize_gain(p) * p - NORMALIZE_PEAK).abs() < 1e-6);
        assert_eq!(normalize_gain(0.0), 1.0);
        let mut empty = PeakScan::default();
        empty.scan(&[&[], &[f32::NAN]]);
        assert_eq!(empty.peak(), 0.0);
    }

    /// The bytes of a file don't depend on how its signal was cut into blocks (the render's
    /// blocks, a resampler's output, the normalize path's units): streaming changes nothing.
    #[test]
    fn files_are_independent_of_the_block_size() {
        let s = sine(FLAC_BLOCK * 3 + 1234, 0.7);
        for (container, depth, rate) in [
            (AudioContainer::Wav, BitDepth::Int16, 48_000),
            (AudioContainer::Wav, BitDepth::Int24, 48_000),
            (AudioContainer::Wav, BitDepth::Float32, 48_000),
            (AudioContainer::Flac, BitDepth::Int16, 48_000),
            (AudioContainer::Flac, BitDepth::Int24, 48_000),
            (AudioContainer::Mp3, BitDepth::Int16, 44_100),
            (AudioContainer::Mp3, BitDepth::Int16, 32_000),
        ] {
            let (whole, _) = encode_at(container, depth, &s, 0.9, usize::MAX, rate);
            for block in [1, 333, 512, FLAC_BLOCK, FLAC_BLOCK + 1] {
                let (bytes, _) = encode_at(container, depth, &s, 0.9, block, rate);
                assert!(bytes == whole, "{container:?} {depth:?}: blocks of {block}");
            }
        }
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
        assert!(FlacEncoder::new(2, 48_000, BitDepth::Float32, Sink::memory(0)).is_err());
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

    /// A test signal whose alignment is unambiguous: a chirp (left) and noise (right).
    fn chirp_and_noise(frames: usize, rate: u32) -> Vec<Vec<f32>> {
        let mut seed = 0x1234_5678_u64;
        let left = (0..frames)
            .map(|i| {
                let t = i as f32 / rate as f32;
                0.4 * (2.0 * std::f32::consts::PI * (200.0 + 400.0 * t) * t).sin()
            })
            .collect();
        let right = (0..frames)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                ((seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.5
            })
            .collect();
        vec![left, right]
    }

    fn encode_mp3(s: &[Vec<f32>], rate: u32, block: usize) -> (Vec<u8>, usize) {
        encode_at(AudioContainer::Mp3, BitDepth::Int24, s, 1.0, block, rate)
    }

    /// The offset in `decoded` (frames) where `reference` lines up best (least squares over
    /// a window, on the noise channel).
    fn best_offset(reference: &[f32], decoded: &[f32], max: usize) -> usize {
        (0..max)
            .min_by(|&a, &b| {
                let err = |d: usize| -> f64 {
                    (2000..20_000)
                        .map(|i| (reference[i] as f64 - decoded[i + d] as f64).powi(2))
                        .sum()
                };
                err(a).total_cmp(&err(b))
            })
            .unwrap()
    }

    fn rusty_decode(bytes: &[u8]) -> Vec<Vec<f32>> {
        let mut dec = rusty_mp3::Mp3Decoder::new();
        dec.push(bytes);
        dec.flush();
        let mut out = vec![Vec::new(), Vec::new()];
        while let Ok(f) = dec.next_frame() {
            for (i, x) in f.samples.iter().enumerate() {
                out[i % 2].push(*x);
            }
        }
        out
    }

    #[test]
    fn mp3_rates() {
        assert_eq!(mp3_rate(48_000), 48_000);
        assert_eq!(mp3_rate(44_100), 44_100);
        assert_eq!(mp3_rate(32_000), 32_000);
        assert_eq!(mp3_rate(88_200), 44_100);
        assert_eq!(mp3_rate(96_000), 48_000);
        assert_eq!(mp3_rate(22_050), 44_100);
        assert!(Mp3Encoder::new(2, 96_000, Sink::memory(0)).is_err());
        // Whole codec frames, at most one frame of extra silence.
        for n in [0, 1, 95, 96, 1000, 48_000] {
            let t = mp3_trimmed_frames(n);
            assert!(t >= n && t < n + MP3_FRAME, "{n} -> {t}");
            assert_eq!((t + MP3_ENCODER_DELAY + MP3_DECODER_DELAY) % MP3_FRAME, 0);
        }
    }

    /// The encoded file decodes (with the gapless trim, as `ether_media` does) to the
    /// signal, at 320 kbps, stepped like the other encoders.
    #[test]
    fn mp3_decodes() {
        for rate in MP3_RATES {
            let n = rate as usize * 2 + 777;
            let s = chirp_and_noise(n, rate);
            let (bytes, steps) = encode_mp3(&s, rate, 4096);
            assert!(steps > 10, "stepped");
            // 320 kbps, plus the Info frame.
            let seconds = mp3_trimmed_frames(n) as f64 / rate as f64;
            let kbps = bytes.len() as f64 * 8.0 / seconds / 1000.0;
            assert!((315.0..340.0).contains(&kbps), "{rate}: {kbps} kbps");
            let d = decode(&bytes, "mp3");
            assert_eq!(d.sample_rate, rate);
            assert_eq!(d.channels.len(), 2);
            eprintln!(
                "{rate}: decoded {} trimmed {} n {n}",
                d.frames(),
                mp3_trimmed_frames(n)
            );
            let lead = d.frames() - mp3_trimmed_frames(n);
            assert_eq!(best_offset(&s[1], &d.channels[1][lead..], 64), 0);
            // Close to the input (a lossy codec: the chirp within a few percent).
            let (mut sig, mut err) = (0.0f64, 0.0f64);
            for (x, y) in s[0].iter().zip(&d.channels[0][lead..]) {
                sig += (*x as f64).powi(2);
                err += (*x as f64 - *y as f64).powi(2);
            }
            let snr = 10.0 * (sig / err).log10();
            assert!(snr > 20.0, "{rate}: SNR {snr:.1} dB");
        }
    }

    /// Without any gapless handling, a decoder outputs the codec delay first: the extra
    /// samples are all at the start, so `decoded - mp3_trimmed_frames` is the lead.
    #[test]
    fn mp3_lead_is_the_codec_delay() {
        let rate = 48_000;
        let n = 30_011;
        let s = chirp_and_noise(n, rate);
        let (bytes, _) = encode_mp3(&s, rate, 100_000);
        let raw = rusty_decode(&bytes);
        let lead = raw[1].len() - mp3_trimmed_frames(n);
        // This decoder outputs the Info frame as a silent frame.
        assert_eq!(lead, MP3_FRAME + MP3_ENCODER_DELAY + MP3_DECODER_DELAY);
        assert_eq!(best_offset(&s[1], &raw[1], 4000), lead);
    }

    /// The Info frame: Xing fields and the LAME-format gapless tag with valid CRCs.
    #[test]
    fn mp3_info_tag() {
        let rate = 44_100;
        let n = 10_000;
        let (bytes, _) = encode_mp3(&chirp_and_noise(n, rate), rate, 4096);
        let frame = 1044; // 144 * 320000 / 44100, no padding
        let tag = 36;
        assert_eq!(&bytes[tag..tag + 4], b"Info");
        let be32 = |o: usize| u32::from_be_bytes(bytes[o..o + 4].try_into().unwrap());
        assert_eq!(be32(tag + 4), 0xF);
        let audio_frames = be32(tag + 8) as usize;
        assert_eq!(bytes.len(), frame * (audio_frames + 1));
        assert_eq!(be32(tag + 12) as usize, bytes.len());
        let lame = tag + 120;
        assert_eq!(&bytes[lame..lame + 9], b"Ethereal ");
        let g = u32::from_be_bytes([0, bytes[lame + 21], bytes[lame + 22], bytes[lame + 23]]);
        let (delay, padding) = ((g >> 12) as usize, (g & 0xFFF) as usize);
        assert_eq!((delay, padding), (MP3_ENCODER_DELAY, MP3_DECODER_DELAY));
        assert_eq!(
            audio_frames * MP3_FRAME - delay - padding,
            mp3_trimmed_frames(n)
        );
        let crc = |o: usize| u16::from_be_bytes([bytes[o], bytes[o + 1]]);
        assert_eq!(crc(lame + 32), crc16(0, &bytes[frame..]));
        assert_eq!(crc(lame + 34), crc16(0, &bytes[..lame + 34]));
        assert_eq!(lame + 34, 190, "the tag CRC covers the first 190 bytes");
        // CRC-16/ARC check value.
        assert_eq!(crc16(0, b"123456789"), 0xBB3D);
    }
}
