//! Incremental decoding (symphonia), so a long file never blocks the controller: the
//! header is probed up front (`MediaRef` metadata), packets are decoded a budget at a time
//! from `tick()`. Same options and error policy as `ether_media::decode` (gapless; corrupt
//! packets skipped), plus detection of chained Ogg streams, which are truncated to the
//! first stream (reported so the controller can warn).

use std::io::Cursor;

use ether_media::{DecodedAudio, MediaError};
use symphonia::core::audio::{AudioBuffer, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

fn map_err(e: SymError) -> MediaError {
    match e {
        SymError::Unsupported(what) => MediaError::Unsupported(what.to_string()),
        other => MediaError::Decode(other.to_string()),
    }
}

pub(crate) struct IncrementalDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    /// From the header (0 = unknown until the first packet).
    pub sample_rate: u32,
    pub channels: usize,
    /// Total frames from the header, if known.
    pub n_frames: Option<u64>,
    out: Vec<Vec<f32>>,
    scratch: Option<AudioBuffer<f32>>,
    /// A chained stream was cut off (only the first stream is imported).
    pub truncated: bool,
    done: bool,
}

impl IncrementalDecoder {
    pub fn new(bytes: Vec<u8>, extension: Option<&str>) -> Result<Self, MediaError> {
        let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), MediaSourceStreamOptions::default());
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
            .or_else(|| format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL))
            .ok_or_else(|| MediaError::Unsupported("no audio track".into()))?;
        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(0);
        let channels = track.codec_params.channels.map_or(0, |c| c.count());
        let n_frames = track.codec_params.n_frames;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(map_err)?;
        let mut out = vec![Vec::new(); channels];
        if let Some(n) = n_frames {
            let n = (n as usize).min(1 << 28);
            for ch in &mut out {
                ch.reserve(n);
            }
        }
        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            channels,
            n_frames,
            out,
            scratch: None,
            truncated: false,
            done: false,
        })
    }

    /// Frames decoded so far.
    pub fn decoded_frames(&self) -> usize {
        self.out.first().map_or(0, Vec::len)
    }

    /// Decode until at least `budget` more frames are produced or the stream ends.
    /// Returns `true` when done.
    pub fn step(&mut self, budget: usize) -> Result<bool, MediaError> {
        let target = self.decoded_frames().saturating_add(budget.max(1));
        while !self.done && self.decoded_frames() < target {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.done = true;
                    break;
                }
                Err(SymError::ResetRequired) => {
                    // Chained/changed stream: keep the first one.
                    self.truncated = true;
                    self.done = true;
                    break;
                }
                Err(e) => return Err(map_err(e)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(buf) => buf,
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => continue,
                Err(e) => return Err(map_err(e)),
            };
            let spec = *decoded.spec();
            if decoded.frames() == 0 {
                continue;
            }
            let n_ch = spec.channels.count();
            if self.out.is_empty() {
                self.out = vec![Vec::new(); n_ch];
                self.channels = n_ch;
            } else if self.out.len() != n_ch {
                return Err(MediaError::Decode(format!(
                    "channel count changed mid-stream ({} -> {n_ch})",
                    self.out.len()
                )));
            }
            if self.sample_rate == 0 {
                self.sample_rate = spec.rate;
            }
            let buf = match &mut self.scratch {
                Some(b) if *b.spec() == spec && b.capacity() >= decoded.capacity() => b,
                slot => slot.insert(AudioBuffer::new(decoded.capacity() as u64, spec)),
            };
            decoded.convert(buf);
            for (c, out) in self.out.iter_mut().enumerate() {
                out.extend_from_slice(buf.chan(c));
            }
        }
        Ok(self.done)
    }

    /// Decode everything that is left.
    pub fn run_to_end(&mut self) -> Result<(), MediaError> {
        while !self.step(1 << 20)? {}
        Ok(())
    }

    pub fn finish(self) -> Result<DecodedAudio, MediaError> {
        if self.sample_rate == 0 {
            return Err(MediaError::Decode("unknown sample rate".into()));
        }
        Ok(DecodedAudio {
            sample_rate: self.sample_rate,
            channels: self.out,
        })
    }
}

/// `true` if `bytes` is an Ogg file with more than one logical stream in sequence (a
/// chained Ogg): a beginning-of-stream page after an end-of-stream page.
pub(crate) fn is_chained_ogg(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"OggS") {
        return false;
    }
    let mut seen_eos = false;
    let mut i = 0;
    while i + 27 <= bytes.len() {
        if &bytes[i..i + 4] != b"OggS" {
            // Resync on the next capture pattern.
            match bytes[i + 1..].windows(4).position(|w| w == b"OggS") {
                Some(p) => {
                    i += 1 + p;
                    continue;
                }
                None => break,
            }
        }
        let header_type = bytes[i + 5];
        if header_type & 0x02 != 0 && seen_eos {
            return true;
        }
        if header_type & 0x04 != 0 {
            seen_eos = true;
        }
        let segments = bytes[i + 26] as usize;
        if i + 27 + segments > bytes.len() {
            break;
        }
        let body: usize = bytes[i + 27..i + 27 + segments].iter().map(|&b| b as usize).sum();
        i += 27 + segments + body;
    }
    false
}
