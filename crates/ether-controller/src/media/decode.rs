//! Incremental decoding (symphonia), so a long file never blocks the controller: the
//! header is probed up front (`MediaRef` metadata), packets are decoded a budget at a time
//! from `tick()`. Same options and error policy as `ether_media::decode` (shared in
//! `ether_media::source`: gapless; corrupt packets skipped), plus detection of chained
//! Ogg streams, which are truncated to the first stream (reported so the controller can
//! warn).

use std::io::Cursor;
use std::sync::Arc;

use ether_media::source::{self, AudioDecoder, FormatReader, Next};
use ether_media::stream::PeakBuilder;
use ether_media::{DecodedAudio, MediaError, PeakMipmap};

pub struct IncrementalDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    /// From the header (0 = unknown until the first packet).
    pub sample_rate: u32,
    pub channels: usize,
    /// Total frames from the header, if known.
    pub n_frames: Option<u64>,
    out: Vec<Vec<f32>>,
    /// Frames to reserve per channel on the first decoded packet (not up front, so a
    /// header-only probe or a peaks-only decode reserves nothing; bounded by the file size,
    /// as headers can lie).
    reserve: usize,
    /// One decoded packet (planar), for the peaks.
    scratch: Vec<Vec<f32>>,
    /// A chained stream was cut off (only the first stream is imported).
    pub truncated: bool,
    done: bool,
    /// Peaks-only mode (the samples feed the peaks and are dropped).
    peaks: Option<PeaksSink>,
    /// Stop after this many frames (an IR only needs its first seconds).
    limit: Option<usize>,
}

struct PeaksSink {
    /// Document length (source frames).
    frames: u64,
    builder: Option<PeakBuilder>,
    fed: usize,
}

impl IncrementalDecoder {
    pub fn new(bytes: Arc<[u8]>, extension: Option<&str>) -> Result<Self, MediaError> {
        let bytes_len = bytes.len();
        let source::Opened {
            format,
            decoder,
            track_id,
            sample_rate,
            channels,
            n_frames,
        } = source::open(Box::new(Cursor::new(bytes)), extension)?;
        Ok(Self {
            reserve: ether_media::reserve_frames(n_frames, bytes_len, channels),
            format,
            decoder,
            track_id,
            sample_rate,
            channels,
            n_frames,
            out: vec![Vec::new(); channels],
            scratch: Vec::new(),
            truncated: false,
            done: false,
            peaks: None,
            limit: None,
        })
    }

    /// Keep only the waveform peaks of a media streamed from disk (cut
    /// or padded to `frames`, the document length, like the whole-file path), dropping the
    /// samples as they are decoded. Call before the first [`Self::step`].
    pub fn peaks_only(&mut self, frames: u64) {
        let channels = self.out.len();
        self.peaks = Some(PeaksSink {
            frames,
            builder: (channels > 0).then(|| PeakBuilder::new(channels, frames)),
            fed: 0,
        });
    }

    /// Decode only the first `frames` frames (the rest of the file is not decoded).
    /// Call before the first [`Self::step`].
    pub fn limit(&mut self, frames: usize) {
        self.limit = Some(frames);
    }

    /// Frames decoded so far.
    pub fn decoded_frames(&self) -> usize {
        match &self.peaks {
            Some(p) => p.fed,
            None => self.out.first().map_or(0, Vec::len),
        }
    }

    /// The peaks of a [`Self::peaks_only`] decode.
    pub fn finish_peaks(self) -> Result<PeakMipmap, MediaError> {
        let Some(p) = self.peaks else {
            return Err(MediaError::Decode("not a peaks-only decode".into()));
        };
        p.builder
            .unwrap_or_else(|| PeakBuilder::new(self.channels.max(1), p.frames))
            .finish()
    }

    /// Decode until at least `budget` more frames are produced or the stream ends.
    /// Returns `true` when done.
    pub fn step(&mut self, budget: usize) -> Result<bool, MediaError> {
        let target = self.decoded_frames().saturating_add(budget.max(1));
        while !self.done && self.decoded_frames() < target {
            if self.limit.is_some_and(|l| self.decoded_frames() >= l) {
                self.done = true;
                break;
            }
            let packet = match source::next_packet(self.format.as_mut(), self.track_id)? {
                Next::Packet(p) => p,
                Next::End => {
                    self.done = true;
                    break;
                }
                Next::Reset => {
                    // Chained/changed stream: keep the first one.
                    self.truncated = true;
                    self.done = true;
                    break;
                }
            };
            let Some(decoded) = source::decode_packet(self.decoder.as_mut(), &packet)? else {
                continue;
            };
            if decoded.frames() == 0 {
                continue;
            }
            let n_ch = source::channels_of(&decoded);
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
                self.sample_rate = decoded.spec().rate();
            }
            if let Some(p) = &mut self.peaks {
                self.scratch.resize_with(n_ch, Vec::new);
                for c in &mut self.scratch {
                    c.clear();
                }
                source::append_planar(&decoded, &mut self.scratch);
                let builder = p
                    .builder
                    .get_or_insert_with(|| PeakBuilder::new(n_ch, p.frames));
                let chans: Vec<&[f32]> = self.scratch.iter().map(Vec::as_slice).collect();
                builder.push(&chans);
                p.fed += self.scratch[0].len();
                continue;
            }
            if self.reserve > 0 {
                let n = self.limit.map_or(self.reserve, |l| self.reserve.min(l));
                self.reserve = 0;
                for out in &mut self.out {
                    out.reserve(n);
                }
            }
            source::append_planar(&decoded, &mut self.out);
        }
        Ok(self.done)
    }

    pub fn finish(mut self) -> Result<DecodedAudio, MediaError> {
        if self.sample_rate == 0 {
            return Err(MediaError::Decode("unknown sample rate".into()));
        }
        if let Some(l) = self.limit {
            self.out.iter_mut().for_each(|ch| ch.truncate(l));
        }
        Ok(DecodedAudio {
            sample_rate: self.sample_rate,
            channels: self.out,
        })
    }
}

/// `true` if `bytes` is an Ogg file with more than one logical stream in sequence (a
/// chained Ogg): a beginning-of-stream page after an end-of-stream page.
pub fn is_chained_ogg(bytes: &[u8]) -> bool {
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
        let body: usize = bytes[i + 27..i + 27 + segments]
            .iter()
            .map(|&b| b as usize)
            .sum();
        i += 27 + segments + body;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(header_type: u8, serial: u32, body: &[u8]) -> Vec<u8> {
        let mut p = b"OggS".to_vec();
        p.push(0); // version
        p.push(header_type);
        p.extend_from_slice(&0u64.to_le_bytes()); // granule
        p.extend_from_slice(&serial.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // sequence
        p.extend_from_slice(&0u32.to_le_bytes()); // crc (not checked)
        p.push(1);
        p.push(body.len() as u8);
        p.extend_from_slice(body);
        p
    }

    /// 1 s of 16-bit stereo silence at 48 kHz whose data chunk claims `0xFFFFFFFF` bytes
    /// (ffmpeg's placeholder when writing to a pipe).
    fn wav_unknown_length() -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&48_000u32.to_le_bytes());
        b.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        b.resize(b.len() + 48_000 * 4, 0);
        b
    }

    /// The header's frame count reserved up to 1 GiB per channel up front, even for
    /// header-only probes.
    #[test]
    fn a_lying_header_reserves_nothing_up_front_and_little_when_decoding() {
        let bytes: Arc<[u8]> = wav_unknown_length().into();
        let dec = IncrementalDecoder::new(bytes.clone(), Some("wav")).unwrap();
        // symphonia >= 0.6 reads a `0xFFFFFFFF` data chunk as "length unknown"; older
        // readers took the ~1e9 frames it claims at face value. Either way nothing is
        // reserved from it.
        assert!(
            dec.n_frames.is_none_or(|n| n > 1 << 28),
            "unknown, or the ~1e9 frames the header claims"
        );
        assert!(
            dec.out.iter().all(|ch| ch.capacity() == 0),
            "probing reserves nothing"
        );

        let mut dec = IncrementalDecoder::new(bytes.clone(), Some("wav")).unwrap();
        while !dec.step(4096).unwrap() {}
        let audio = dec.finish().unwrap();
        assert_eq!(audio.frames(), 48_000);
        for ch in &audio.channels {
            assert!(ch.capacity() <= 4 * 48_000, "{}", ch.capacity());
        }

        let mut dec = IncrementalDecoder::new(bytes, Some("wav")).unwrap();
        dec.peaks_only(48_000);
        while !dec.step(4096).unwrap() {}
        assert!(
            dec.out.iter().all(|ch| ch.capacity() == 0),
            "peaks-only keeps no samples"
        );
    }

    #[test]
    fn detects_chained_ogg() {
        let single = [
            page(0x02, 1, b"head"),
            page(0, 1, b"data"),
            page(0x04, 1, b"end"),
        ]
        .concat();
        assert!(!is_chained_ogg(&single));
        let chained = [
            single.clone(),
            page(0x02, 2, b"head"),
            page(0x04, 2, b"end"),
        ]
        .concat();
        assert!(is_chained_ogg(&chained));
        // Multiplexed streams (several BOS pages up front) are not chained.
        let muxed = [
            page(0x02, 1, b"a"),
            page(0x02, 2, b"b"),
            page(0x04, 1, b"c"),
            page(0x04, 2, b"d"),
        ]
        .concat();
        assert!(!is_chained_ogg(&muxed));
        assert!(!is_chained_ogg(b"RIFF....WAVE"));
    }
}
