//! Whole-file decoding with symphonia (WAV/AIFF/FLAC/MP3/OGG Vorbis).

use std::io::Cursor;

use symphonia::core::audio::{AudioBuffer, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{DecodedAudio, MediaError};

/// Frames worth reserving per channel up front for a file of `file_bytes` bytes whose
/// header claims `n_frames`. The header is not trusted (#554: a WAV data chunk of
/// `0xFFFFFFFF`, ffmpeg's placeholder when writing to a pipe, claims ~1e9 frames): the
/// reservation is at most one frame per byte per channel (exact for 8-bit PCM, generous
/// for wider PCM; compressed files grow past it as they decode) and at most 2^28 frames.
pub fn reserve_frames(n_frames: Option<u64>, file_bytes: usize, channels: usize) -> usize {
    n_frames.map_or(0, |n| {
        (n.min(1 << 28) as usize).min(file_bytes / channels.max(1))
    })
}

fn map_err(e: SymError) -> MediaError {
    match e {
        SymError::Unsupported(what) => MediaError::Unsupported(what.to_string()),
        other => MediaError::Decode(other.to_string()),
    }
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
    let mss = MediaSourceStream::new(
        Box::new(Cursor::new(bytes)),
        MediaSourceStreamOptions::default(),
    );
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
    let mut format = probed.format;

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
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut channels: Vec<Vec<f32>> = match track.codec_params.channels {
        Some(ch) => vec![Vec::new(); ch.count()],
        None => Vec::new(),
    };
    let reserve = reserve_frames(track.codec_params.n_frames, file_bytes, channels.len());
    for ch in &mut channels {
        ch.reserve(reserve);
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(map_err)?;

    let mut scratch: Option<AudioBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            // A chained/changed stream: keep what we have (v0.1 imports the first stream).
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(map_err(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            // Corrupt or truncated packet: skip it.
            Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => continue,
            Err(e) => return Err(map_err(e)),
        };
        let spec = *decoded.spec();
        if decoded.frames() == 0 {
            continue;
        }
        let n_ch = spec.channels.count();
        if channels.is_empty() {
            channels = vec![Vec::new(); n_ch];
        } else if channels.len() != n_ch {
            return Err(MediaError::Decode(format!(
                "channel count changed mid-stream ({} -> {n_ch})",
                channels.len()
            )));
        }
        if sample_rate == 0 {
            sample_rate = spec.rate;
        }

        let buf = match &mut scratch {
            Some(b) if *b.spec() == spec && b.capacity() >= decoded.capacity() => b,
            slot => slot.insert(AudioBuffer::new(decoded.capacity() as u64, spec)),
        };
        decoded.convert(buf);
        for (c, out) in channels.iter_mut().enumerate() {
            out.extend_from_slice(buf.chan(c));
        }
    }

    if sample_rate == 0 {
        return Err(MediaError::Decode("unknown sample rate".into()));
    }
    Ok(DecodedAudio {
        sample_rate,
        channels,
    })
}
