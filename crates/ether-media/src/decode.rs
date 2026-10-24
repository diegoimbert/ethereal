//! Whole-file decoding with symphonia (WAV/AIFF/FLAC/MP3/OGG Vorbis).

use std::io::Cursor;

use crate::source::{self, Next};
use crate::{DecodedAudio, MediaError};

/// Frames worth reserving per channel up front for a file of `file_bytes` bytes whose
/// header claims `n_frames`. The header is not trusted (a WAV data chunk of
/// `0xFFFFFFFF`, ffmpeg's placeholder when writing to a pipe, claims ~1e9 frames): the
/// reservation is at most one frame per byte per channel (exact for 8-bit PCM, generous
/// for wider PCM; compressed files grow past it as they decode) and at most 2^28 frames.
pub fn reserve_frames(n_frames: Option<u64>, file_bytes: usize, channels: usize) -> usize {
    n_frames.map_or(0, |n| {
        (n.min(1 << 28) as usize).min(file_bytes / channels.max(1))
    })
}

/// Decode a whole file (WAV/AIFF/FLAC/MP3/OGG, M4A with AAC-LC or ALAC, raw ADTS AAC).
/// `extension` is a format hint (with or without the leading dot, any case); the container is sniffed from the bytes either way.
///
/// The result is at the file's own sample rate; use [`crate::resample`] to convert it to
/// the engine rate. Corrupt packets are skipped (as players do); a file with no decodable
/// audio track is an error.
pub fn decode(bytes: &[u8], extension: Option<&str>) -> Result<DecodedAudio, MediaError> {
    decode_owned(bytes.to_vec(), extension)
}

/// Like [`decode`], but takes ownership of the bytes (avoids one copy of the file).
pub fn decode_owned(bytes: Vec<u8>, extension: Option<&str>) -> Result<DecodedAudio, MediaError> {
    let file_bytes = bytes.len();
    let source::Opened {
        mut format,
        mut decoder,
        track_id,
        mut sample_rate,
        channels: n_ch,
        n_frames,
    } = source::open(Box::new(Cursor::new(bytes)), extension)?;
    let mut channels: Vec<Vec<f32>> = vec![Vec::new(); n_ch];
    let reserve = reserve_frames(n_frames, file_bytes, channels.len());
    for ch in &mut channels {
        ch.reserve(reserve);
    }

    // Up to the end, or a chained/changed stream (`Next::Reset`): keep what we have (v0.1
    // imports the first stream).
    while let Next::Packet(packet) = source::next_packet(format.as_mut(), track_id)? {
        let Some(decoded) = source::decode_packet(decoder.as_mut(), &packet)? else {
            continue;
        };
        if decoded.frames() == 0 {
            continue;
        }
        let n_ch = source::channels_of(&decoded);
        if channels.is_empty() {
            channels = vec![Vec::new(); n_ch];
        } else if channels.len() != n_ch {
            return Err(MediaError::Decode(format!(
                "channel count changed mid-stream ({} -> {n_ch})",
                channels.len()
            )));
        }
        if sample_rate == 0 {
            sample_rate = crate::check_rate(decoded.spec().rate())?;
        }
        source::append_planar(&decoded, &mut channels);
    }

    if sample_rate == 0 {
        return Err(MediaError::Decode("unknown sample rate".into()));
    }
    Ok(DecodedAudio {
        sample_rate,
        channels,
    })
}
