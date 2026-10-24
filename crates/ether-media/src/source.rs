//! symphonia plumbing shared by the whole-file ([`crate::decode`]), streaming
//! ([`crate::stream::ChunkDecoder`]) and incremental (controller) decoders, so they all
//! probe, pick a track, read packets and place decoded audio the same way.
//!
//! Options: gapless (the decoder trims encoder delay and padding, `AudioDecoderOptions`'
//! default), default demuxer limits (symphonia's guards against crafted files).

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, Track, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::packet::Packet;

use crate::MediaError;

pub use symphonia::core::codecs::audio::AudioDecoder;
pub use symphonia::core::formats::FormatReader;

pub fn map_err(e: SymError) -> MediaError {
    match e {
        SymError::Unsupported(what) => MediaError::Unsupported(what.to_string()),
        other => MediaError::Decode(other.to_string()),
    }
}

/// Probe `source`'s container. `extension` is a hint (with or without the leading dot,
/// any case); the container is sniffed from the bytes either way.
pub fn probe(
    source: Box<dyn MediaSource>,
    extension: Option<&str>,
) -> Result<Box<dyn FormatReader>, MediaError> {
    let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = extension {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        if !ext.is_empty() {
            hint.with_extension(&ext);
        }
    }
    symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(map_err)
}

/// The track to import: the default audio track, else the first audio track with a known
/// codec.
pub fn audio_track(format: &dyn FormatReader) -> Option<&Track> {
    format
        .default_track(TrackType::Audio)
        .filter(|t| {
            t.codec_params
                .as_ref()
                .and_then(|p| p.audio())
                .is_some_and(|p| p.codec != CODEC_ID_NULL_AUDIO)
        })
        .or_else(|| format.first_track_known_codec(TrackType::Audio))
}

/// Header facts of a track: (sample rate, channels, frames), 0 / `None` when unknown.
pub fn track_info(track: &Track) -> (u32, usize, Option<u64>) {
    let params = track.codec_params.as_ref().and_then(|p| p.audio());
    let rate = params.and_then(|p| p.sample_rate).unwrap_or(0);
    let channels = params
        .and_then(|p| p.channels.as_ref())
        .map_or(0, |c| c.count());
    (rate, channels, track.num_frames)
}

/// A probed file with a decoder for its audio track.
pub struct Opened {
    pub format: Box<dyn FormatReader>,
    pub decoder: Box<dyn AudioDecoder>,
    pub track_id: u32,
    /// From the header (0 = unknown until the first packet).
    pub sample_rate: u32,
    /// From the header (0 = unknown until the first packet, e.g. AAC in MP4).
    pub channels: usize,
    /// Total frames from the header (after gapless trimming), if known.
    pub n_frames: Option<u64>,
}

/// Probe `source` and make a decoder for its audio track.
pub fn open(source: Box<dyn MediaSource>, extension: Option<&str>) -> Result<Opened, MediaError> {
    let format = probe(source, extension)?;
    let track = audio_track(format.as_ref())
        .ok_or_else(|| MediaError::Unsupported("no audio track".into()))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| MediaError::Unsupported("no audio track".into()))?;
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(map_err)?;
    let track_id = track.id;
    let (sample_rate, channels, n_frames) = track_info(track);
    if sample_rate != 0 {
        crate::check_rate(sample_rate)?;
    }
    Ok(Opened {
        format,
        decoder,
        track_id,
        sample_rate,
        channels,
        n_frames,
    })
}

/// What [`next_packet`] read.
pub enum Next {
    Packet(Packet),
    /// End of the stream.
    End,
    /// A chained/changed stream (a new track list): only the first stream is imported.
    Reset,
}

/// The next packet of `track_id` (other tracks' packets are skipped).
pub fn next_packet(format: &mut dyn FormatReader, track_id: u32) -> Result<Next, MediaError> {
    loop {
        match format.next_packet() {
            Ok(Some(p)) if p.track_id == track_id => return Ok(Next::Packet(p)),
            Ok(Some(_)) => {}
            Ok(None) => return Ok(Next::End),
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(Next::End);
            }
            Err(SymError::ResetRequired) => return Ok(Next::Reset),
            Err(e) => return Err(map_err(e)),
        }
    }
}

/// Decode `packet`. `Ok(None)`: a corrupt or truncated packet, to skip (as players do).
pub fn decode_packet<'a>(
    decoder: &'a mut dyn AudioDecoder,
    packet: &Packet,
) -> Result<Option<GenericAudioBufferRef<'a>>, MediaError> {
    match decoder.decode(packet) {
        Ok(buf) => Ok(Some(buf)),
        Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => Ok(None),
        Err(e) => Err(map_err(e)),
    }
}

/// Source frame of the first decoded (trimmed) frame of `packet`: its timestamp plus the
/// encoder delay the decoder trims (a delay packet has a negative timestamp).
pub fn packet_start(packet: &Packet) -> u64 {
    let ts = packet
        .pts
        .get()
        .saturating_add_unsigned(packet.trim_start.get());
    ts.max(0) as u64
}

/// Append `decoded` to `out` (one `Vec` per channel, `out.len()` == its channel count),
/// converted to `f32`.
pub fn append_planar(decoded: &GenericAudioBufferRef<'_>, out: &mut [Vec<f32>]) {
    let n = decoded.frames();
    let starts: Vec<usize> = out.iter().map(Vec::len).collect();
    for o in out.iter_mut() {
        o.resize(o.len() + n, 0.0);
    }
    let mut dst: Vec<&mut [f32]> = out
        .iter_mut()
        .zip(starts)
        .map(|(o, s)| &mut o[s..])
        .collect();
    decoded.copy_to_slice_planar(&mut dst);
}

/// Channel count of a decoded buffer.
pub fn channels_of(decoded: &GenericAudioBufferRef<'_>) -> usize {
    decoded.spec().channels().count()
}
