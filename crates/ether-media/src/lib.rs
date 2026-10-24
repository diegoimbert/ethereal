//! Media: decoding (symphonia), resampling (rubato), peak mipmaps. Pure Rust, wasm-safe:
//! operates on bytes in memory; hosts do the file/OPFS I/O.
//!
//! Typical import flow (controller side): [`decode`] the file bytes, build a [`PeakMipmap`]
//! from the source-rate audio (peak requests are in source frames), [`resample`] to the
//! engine rate and hand the result to the engine wrapped in an [`InMemorySource`].

use std::sync::Arc;

use ether_core::AudioSource;

mod decode;
pub mod demo;
mod peaks;
mod resample;
pub mod smf;
pub mod source;
pub mod stream;
pub mod to_midi;

pub use decode::{
    MAX_DECODED_SAMPLES, check_decoded_size, decode, decode_owned, max_decoded_samples,
    reserve_frames,
};
pub use peaks::{BASE_SAMPLES_PER_PEAK, MAX_PAD_FRAMES, PeakMipmap};
pub use resample::{
    alignment, process_chunk, resample, resampled_len, resampler_buffers, sinc_resampler,
};

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

/// Sample rates a file or a resampler may have (Hz). Real files are 8 kHz..=768 kHz; the
/// bound keeps a header's claim from sizing buffers (a 1 Hz file resampled to 48 kHz would be
/// 48,000 times its length).
pub const SAMPLE_RATES: std::ops::RangeInclusive<u32> = 1_000..=768_000;

/// `rate` if it is in [`SAMPLE_RATES`], else a decode error.
pub fn check_rate(rate: u32) -> Result<u32, MediaError> {
    if SAMPLE_RATES.contains(&rate) {
        Ok(rate)
    } else {
        Err(MediaError::Decode(format!("unsupported sample rate {rate} Hz")))
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
