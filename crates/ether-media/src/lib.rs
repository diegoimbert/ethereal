//! Media: decoding (symphonia), resampling (rubato), peak mipmaps. Pure Rust, wasm-safe:
//! operates on bytes in memory; hosts do the file/OPFS I/O.
//!
//! Owned by the `media` node (it adds `symphonia`/`rubato` from `[workspace.dependencies]`).

use std::sync::Arc;

use ether_core::AudioSource;
use ether_core::protocol::media::{PeakData, PeakRequest};

/// Planar `f32` audio fully in memory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    /// One `Vec` per channel, equal lengths.
    pub channels: Vec<Vec<f32>>,
}

impl DecodedAudio {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MediaError {
    #[error("unsupported format: {0}")]
    Unsupported(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("resample error: {0}")]
    Resample(String),
}

/// Decode a whole file (WAV/AIFF/FLAC/MP3/OGG). `extension` is a format hint.
pub fn decode(bytes: &[u8], extension: Option<&str>) -> Result<DecodedAudio, MediaError> {
    let _ = (bytes, extension);
    todo!("media node")
}

/// High-quality resample to `target_rate` (no-op clone if equal).
pub fn resample(audio: &DecodedAudio, target_rate: u32) -> Result<DecodedAudio, MediaError> {
    let _ = (audio, target_rate);
    todo!("media node")
}

/// Min/max peak pyramid (levels at powers of two samples-per-peak, from 32 up).
#[derive(Clone, Debug, Default)]
pub struct PeakMipmap {
    _private: (),
}

impl PeakMipmap {
    pub fn build(audio: &DecodedAudio) -> Self {
        let _ = audio;
        todo!("media node")
    }

    /// Answer a `MediaCommand::GetPeaks` from the nearest level `>= samples_per_peak`.
    pub fn query(&self, request: &PeakRequest) -> PeakData {
        let _ = request;
        todo!("media node")
    }

    /// Compact binary form for host-side caching (keyed by `MediaRef::hash`).
    pub fn to_bytes(&self) -> Vec<u8> {
        todo!("media node")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MediaError> {
        let _ = bytes;
        todo!("media node")
    }
}

/// [`AudioSource`] over fully decoded audio (already at the engine rate).
pub struct InMemorySource {
    pub audio: Arc<DecodedAudio>,
}

impl AudioSource for InMemorySource {
    fn channels(&self) -> u16 {
        self.audio.channels.len() as u16
    }

    fn frames(&self) -> u64 {
        self.audio.frames() as u64
    }

    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        let Some(data) = self.audio.channels.get(channel as usize) else {
            out.fill(0.0);
            return true;
        };
        let start = start.min(data.len() as u64) as usize;
        let n = (data.len() - start).min(out.len());
        out[..n].copy_from_slice(&data[start..start + n]);
        out[n..].fill(0.0);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_source_reads_and_pads() {
        let src = InMemorySource {
            audio: Arc::new(DecodedAudio {
                sample_rate: 48_000,
                channels: vec![vec![1.0, 2.0, 3.0]],
            }),
        };
        let mut out = [9.0; 4];
        assert!(src.read(0, 1, &mut out));
        assert_eq!(out, [2.0, 3.0, 0.0, 0.0]);
    }
}
