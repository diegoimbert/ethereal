//! [`ChunkDecoder`]: decode + resample one media chunk by chunk, with seeking.
//!
//! Source side: symphonia, same options as the whole-file decode (gapless, corrupt packets
//! skipped, first stream of a chained Ogg). With gapless enabled a packet's timestamp is the
//! source frame of its first (trimmed) decoded frame, so decoded audio is placed by
//! timestamp: a seek lands exactly, whatever packet the format reader restarts from. Seeks
//! restart [`SEEK_PREROLL`] frames early (lossy codecs need a few packets to settle). The
//! source is cut or zero-padded to the document's frame count, like the whole-file path.
//!
//! Rate side: the same rubato `SincFixedIn` setup and exact alignment as
//! `ether_media::resample` (ratio `p / q` in lowest terms, `q - 1` zeros prepended, the
//! first `p - 1` outputs dropped). Sequential chunks continue one resampler, so a stream
//! played from the start matches the whole-file resample to float rounding. A jump restarts
//! the resampler at a source frame `s0` that is a multiple of `q` (so its output grid lands
//! on the global one: output `k'` of the restart is global output `s0 * p / q + k'`) at
//! least [`RESAMPLE_MARGIN`] frames before the target, past the sinc's reach.

use std::sync::Arc;

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::{AudioBuffer, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::CHUNK_FRAMES;
use crate::{MediaError, resampled_len};

/// Source frames decoded (and dropped) before a seek target.
pub const SEEK_PREROLL: u64 = 4096;
/// Source frames a restarted resampler runs before the first output kept (> sinc half
/// length, 128).
pub const RESAMPLE_MARGIN: u64 = 512;
/// Forward jumps up to this many source frames are decoded through instead of seeking.
const SKIP_FORWARD: u64 = 2 * CHUNK_FRAMES as u64;
/// Resampler input frames per call (as `ether_media::resample`).
const RS_CHUNK: usize = 1024;

fn map_err(e: SymError) -> MediaError {
    match e {
        SymError::Unsupported(what) => MediaError::Unsupported(what.to_string()),
        other => MediaError::Decode(other.to_string()),
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Sequential reader of source-rate frames with seeking.
struct SourceReader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    channels: usize,
    /// Document length (source frames): reads past it are zeros.
    frames: u64,
    /// Decoded frames not consumed yet; `buf[c][0]` is source frame `buf_pos`.
    buf: Vec<Vec<f32>>,
    buf_pos: u64,
    /// Next frame [`Self::read`] returns.
    pos: u64,
    eof: bool,
    scratch: Option<AudioBuffer<f32>>,
    /// Packets that failed to decode since the last seek (corrupt data is skipped).
    pub skipped: u64,
}

impl SourceReader {
    fn new(
        source: Box<dyn MediaSource>,
        extension: Option<&str>,
        frames: u64,
    ) -> Result<(Self, u32), MediaError> {
        let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            let ext = ext.trim_start_matches('.').to_ascii_lowercase();
            if !ext.is_empty() {
                hint.with_extension(&ext);
            }
        }
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &MetadataOptions::default())
            .map_err(map_err)?;
        let format = probed.format;
        let track = format
            .default_track()
            .filter(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .or_else(|| {
                format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            })
            .ok_or_else(|| MediaError::Unsupported("no audio track".into()))?;
        let track_id = track.id;
        let rate = track.codec_params.sample_rate.unwrap_or(0);
        let channels = track.codec_params.channels.map_or(0, |c| c.count());
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(map_err)?;
        Ok((
            Self {
                format,
                decoder,
                track_id,
                channels,
                frames,
                buf: vec![Vec::new(); channels],
                buf_pos: 0,
                pos: 0,
                eof: false,
                scratch: None,
                skipped: 0,
            },
            rate,
        ))
    }

    fn buffered_end(&self) -> u64 {
        self.buf_pos + self.buf.first().map_or(0, Vec::len) as u64
    }

    /// Make the next [`Self::read`] start at source frame `to`.
    fn seek(&mut self, to: u64) -> Result<(), MediaError> {
        if to == self.pos {
            return Ok(());
        }
        if to > self.pos && self.buf_pos != u64::MAX && to <= self.buffered_end() + SKIP_FORWARD {
            // Within (or just past) what is decoded: decode through.
            self.pos = to;
            return Ok(());
        }
        let target = to.saturating_sub(SEEK_PREROLL);
        let seeked = self.format.seek(
            SeekMode::Accurate,
            SeekTo::TimeStamp {
                ts: target,
                track_id: self.track_id,
            },
        );
        match seeked {
            Ok(_) => {}
            // Past the end of the stream: everything from here is silence.
            Err(SymError::SeekError(_)) | Err(SymError::IoError(_)) if to >= self.frames => {}
            Err(e) => return Err(map_err(e)),
        }
        self.decoder.reset();
        for c in &mut self.buf {
            c.clear();
        }
        // Placed by the first packet's timestamp.
        self.buf_pos = u64::MAX;
        self.eof = false;
        self.pos = to;
        Ok(())
    }

    /// Decode one packet into `buf` (placed by its timestamp). `false` at the end.
    fn decode_packet(&mut self) -> Result<bool, MediaError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                // Chained/changed stream: only the first one is imported.
                Err(SymError::ResetRequired) => return Ok(false),
                Err(e) => return Err(map_err(e)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let ts = packet.ts();
            let decoded = match self.decoder.decode(&packet) {
                Ok(buf) => buf,
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => {
                    self.skipped += 1;
                    continue;
                }
                Err(e) => return Err(map_err(e)),
            };
            if decoded.frames() == 0 {
                continue;
            }
            let spec = *decoded.spec();
            let n_ch = spec.channels.count();
            if n_ch != self.channels {
                if self.channels == 0 {
                    self.channels = n_ch;
                    self.buf = vec![Vec::new(); n_ch];
                } else {
                    return Err(MediaError::Decode(format!(
                        "channel count changed mid-stream ({} -> {n_ch})",
                        self.channels
                    )));
                }
            }
            let mut sbuf = match self.scratch.take() {
                Some(b) if *b.spec() == spec && b.capacity() >= decoded.capacity() => b,
                _ => AudioBuffer::new(decoded.capacity() as u64, spec),
            };
            decoded.convert(&mut sbuf);
            let n = sbuf.frames();
            if self.buf_pos == u64::MAX {
                self.buf_pos = ts;
            }
            let end = self.buffered_end();
            if ts > end {
                // A gap (skipped packet): silence, so later frames stay in place.
                let gap = (ts - end).min(1 << 20) as usize;
                for c in &mut self.buf {
                    c.resize(c.len() + gap, 0.0);
                }
            }
            let end = self.buffered_end();
            // Overlap with what is buffered (shouldn't happen): keep the new tail only.
            let skip = (end.saturating_sub(ts) as usize).min(n);
            for (c, out) in self.buf.iter_mut().enumerate() {
                out.extend_from_slice(&sbuf.chan(c)[skip..n]);
            }
            self.scratch = Some(sbuf);
            return Ok(true);
        }
    }

    /// Read `n` frames from the current position into `out[c]` (appended), advancing.
    fn read(&mut self, out: &mut [Vec<f32>], n: usize) -> Result<(), MediaError> {
        let end = self.pos + n as u64;
        // Decode until the request is buffered, the file ends, or the document ends.
        let want = end.min(self.frames);
        while !self.eof
            && self.pos < want
            && (self.buf_pos == u64::MAX || self.buffered_end() < want)
        {
            if !self.decode_packet()? {
                self.eof = true;
            }
            // Drop what lies before the read position (seek preroll, skipped frames).
            if self.buf_pos != u64::MAX && self.buf_pos < self.pos {
                let drop =
                    ((self.pos - self.buf_pos) as usize).min(self.buf.first().map_or(0, Vec::len));
                for c in &mut self.buf {
                    c.drain(..drop);
                }
                self.buf_pos += drop as u64;
            }
        }
        for (c, o) in out.iter_mut().enumerate() {
            let src = self.buf.get(c);
            for f in self.pos..end {
                let v = match src {
                    Some(b) if f < self.frames && f >= self.buf_pos && self.buf_pos != u64::MAX => {
                        b.get((f - self.buf_pos) as usize).copied().unwrap_or(0.0)
                    }
                    _ => 0.0,
                };
                o.push(v);
            }
        }
        self.pos = end;
        // Forget consumed frames.
        if self.buf_pos != u64::MAX && self.buf_pos < self.pos {
            let drop =
                ((self.pos - self.buf_pos) as usize).min(self.buf.first().map_or(0, Vec::len));
            for c in &mut self.buf {
                c.drain(..drop);
            }
            self.buf_pos += drop as u64;
        }
        Ok(())
    }
}

/// Running resampler (engine rate != source rate).
struct Rate {
    resampler: SincFixedIn<f32>,
    p: u64,
    q: u64,
    /// Next global output frame the resampler produces (after dropping).
    next_out: u64,
    /// Outputs to discard before `next_out` (alignment drop + restart run-in).
    to_drop: u64,
    /// Leading zeros still to feed (alignment padding).
    pad: u64,
    /// Produced but not yet returned outputs (start at `next_out - pending.len()`).
    pending: Vec<Vec<f32>>,
    inbuf: Vec<Vec<f32>>,
    outbuf: Vec<Vec<f32>>,
    /// The resampler state is valid for continuing at `next_out`.
    running: bool,
}

/// Decoder of one media into engine-rate chunks of [`CHUNK_FRAMES`] frames.
pub struct ChunkDecoder {
    src: SourceReader,
    source_rate: u32,
    engine_rate: u32,
    channels: u16,
    /// Length at the engine rate.
    frames: u64,
    rate: Option<Rate>,
    /// Source-rate scratch for one resampler call.
    tmp: Vec<Vec<f32>>,
}

impl ChunkDecoder {
    /// Probe `source` (a whole media file). `source_frames` = the document's length at the
    /// media's own rate (`MediaRef::frames`); the stream is cut or padded to it.
    pub fn new(
        source: Box<dyn MediaSource>,
        extension: Option<&str>,
        source_frames: u64,
        engine_rate: u32,
    ) -> Result<Self, MediaError> {
        let (src, source_rate) = SourceReader::new(source, extension, source_frames)?;
        if source_rate == 0 || engine_rate == 0 {
            return Err(MediaError::Decode("unknown sample rate".into()));
        }
        let channels = src.channels.max(1) as u16;
        let n_ch = channels as usize;
        let frames = resampled_len(source_frames as usize, source_rate, engine_rate) as u64;
        let rate = if source_rate == engine_rate {
            None
        } else {
            let g = gcd(engine_rate, source_rate);
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: rubato::calculate_cutoff(256, WindowFunction::BlackmanHarris2),
                oversampling_factor: 1024,
                interpolation: SincInterpolationType::Cubic,
                window: WindowFunction::BlackmanHarris2,
            };
            let ratio = engine_rate as f64 / source_rate as f64;
            let resampler = SincFixedIn::<f32>::new(ratio, 1.0, params, RS_CHUNK, n_ch)
                .map_err(|e| MediaError::Resample(e.to_string()))?;
            let inbuf = resampler.input_buffer_allocate(true);
            let outbuf = resampler.output_buffer_allocate(true);
            Some(Rate {
                resampler,
                p: (engine_rate / g) as u64,
                q: (source_rate / g) as u64,
                next_out: 0,
                to_drop: 0,
                pad: 0,
                pending: vec![Vec::new(); n_ch],
                inbuf,
                outbuf,
                running: false,
            })
        };
        Ok(Self {
            src,
            source_rate,
            engine_rate,
            channels,
            frames,
            rate,
            tmp: vec![Vec::with_capacity(RS_CHUNK); n_ch],
        })
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Length at the engine rate.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }

    pub fn engine_rate(&self) -> u32 {
        self.engine_rate
    }

    /// Chunks covering the media.
    pub fn chunk_count(&self) -> u64 {
        self.frames.div_ceil(CHUNK_FRAMES as u64)
    }

    /// Decode chunk `chunk` (engine frames `[chunk * CHUNK_FRAMES, +CHUNK_FRAMES)`) into
    /// `out` (one `Vec` per channel, replaced; zero past the end).
    pub fn decode_chunk(&mut self, chunk: u64, out: &mut Vec<Vec<f32>>) -> Result<(), MediaError> {
        let n_ch = self.channels as usize;
        out.resize_with(n_ch, Vec::new);
        for o in out.iter_mut() {
            o.clear();
            o.reserve(CHUNK_FRAMES);
        }
        let start = chunk * CHUNK_FRAMES as u64;
        if start >= self.frames {
            for o in out.iter_mut() {
                o.resize(CHUNK_FRAMES, 0.0);
            }
            return Ok(());
        }
        match self.rate.is_some() {
            false => {
                self.src.seek(start)?;
                self.src.read(out, CHUNK_FRAMES)?;
            }
            true => self.resample_into(start, out)?,
        }
        // Zero past the end (the resampler's tail and the padding).
        let valid = (self.frames - start).min(CHUNK_FRAMES as u64) as usize;
        for o in out.iter_mut() {
            o.resize(CHUNK_FRAMES, 0.0);
            o[valid..].fill(0.0);
        }
        Ok(())
    }

    fn resample_into(&mut self, start: u64, out: &mut [Vec<f32>]) -> Result<(), MediaError> {
        let n_ch = self.channels as usize;
        let rate = self.rate.as_mut().expect("resampling");
        let pending_start = rate.next_out - rate.pending[0].len() as u64;
        let resident =
            rate.running && start >= pending_start && start <= rate.next_out + CHUNK_FRAMES as u64;
        if !resident {
            // Restart at s0 (a multiple of q) far enough before the target.
            let (p, q) = (rate.p, rate.q);
            let target_src = (start as u128 * q as u128 / p as u128) as u64;
            let m = target_src.saturating_sub(RESAMPLE_MARGIN) / q;
            let s0 = m * q;
            self.src.seek(s0)?;
            rate.resampler.reset();
            rate.pad = q - 1;
            rate.to_drop = p - 1;
            rate.next_out = m * p;
            for c in &mut rate.pending {
                c.clear();
            }
            rate.running = true;
        } else if start > pending_start {
            let drop = ((start - pending_start) as usize).min(rate.pending[0].len());
            for c in &mut rate.pending {
                c.drain(..drop);
            }
        }
        let end = start + CHUNK_FRAMES as u64;
        while rate.next_out < end {
            let need = rate.resampler.input_frames_next();
            // Input: remaining pad zeros, then source frames.
            let zeros = (rate.pad as usize).min(need);
            rate.pad -= zeros as u64;
            for t in &mut self.tmp {
                t.clear();
            }
            self.src.read(&mut self.tmp, need - zeros)?;
            for (c, dst) in rate.inbuf.iter_mut().enumerate() {
                dst.clear();
                dst.resize(zeros, 0.0);
                dst.extend_from_slice(&self.tmp[c.min(self.tmp.len() - 1)]);
            }
            let (_, n) = rate
                .resampler
                .process_into_buffer(&rate.inbuf, &mut rate.outbuf, None)
                .map_err(|e| MediaError::Resample(e.to_string()))?;
            let mut from = 0usize;
            if rate.to_drop > 0 {
                from = (rate.to_drop as usize).min(n);
                rate.to_drop -= from as u64;
            }
            for (pend, o) in rate.pending.iter_mut().zip(&rate.outbuf) {
                pend.extend_from_slice(&o[from..n]);
            }
            rate.next_out += (n - from) as u64;
            // The restart's run-in: outputs before `start` are not wanted.
            let pending_start = rate.next_out - rate.pending[0].len() as u64;
            if pending_start < start {
                let drop = ((start - pending_start) as usize).min(rate.pending[0].len());
                for c in &mut rate.pending {
                    c.drain(..drop);
                }
            }
        }
        for (c, o) in out.iter_mut().enumerate().take(n_ch) {
            o.extend_from_slice(&rate.pending[c][..CHUNK_FRAMES]);
        }
        for c in &mut rate.pending {
            c.drain(..CHUNK_FRAMES);
        }
        Ok(())
    }
}

/// A `MediaSource` over shared bytes (tests, and hosts that already hold a file).
pub struct BytesSource(std::io::Cursor<Arc<[u8]>>);

impl BytesSource {
    pub fn new(bytes: Arc<[u8]>) -> Self {
        Self(std::io::Cursor::new(bytes))
    }
}

impl std::io::Read for BytesSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Seek for BytesSource {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl MediaSource for BytesSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.get_ref().len() as u64)
    }
}
