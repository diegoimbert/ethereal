//! Media: decoding (symphonia), resampling (rubato), peak mipmaps. Pure Rust, wasm-safe:
//! operates on bytes in memory; hosts do the file/OPFS I/O.
//!
//! Typical import flow (controller side): [`decode`] the file bytes, build a [`PeakMipmap`]
//! from the source-rate audio (peak requests are in source frames), [`resample`] to the
//! engine rate and hand the result to the engine wrapped in an [`InMemorySource`].

use std::sync::Arc;

use ether_core::AudioSource;

mod decode;
mod peaks;
mod resample;

pub use decode::{decode, decode_owned};
pub use peaks::{BASE_SAMPLES_PER_PEAK, PeakMipmap};
pub use resample::{resample, resampled_len};

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

/// [`AudioSource`] over fully decoded audio (already at the engine rate).
pub struct InMemorySource {
    pub audio: Arc<DecodedAudio>,
}

impl InMemorySource {
    pub fn new(audio: Arc<DecodedAudio>) -> Self {
        Self { audio }
    }
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
