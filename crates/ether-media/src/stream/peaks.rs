//! [`PeakBuilder`]: waveform peaks from audio fed block by block, so a streamed media's
//! one decode pass keeps only the peaks (the samples are dropped as they go). The result
//! is identical to `PeakMipmap::build` over the whole (fitted) audio.

use crate::{BASE_SAMPLES_PER_PEAK, MediaError, PeakMipmap};

const SCALE: f32 = i16::MAX as f32;

// Same quantization as `PeakMipmap::build` (min rounded down, max up, clamped, NaN = 0).
fn q_min(v: f32) -> i16 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
    (v * SCALE).floor().max(-SCALE) as i16
}

fn q_max(v: f32) -> i16 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
    (v * SCALE).ceil().min(SCALE) as i16
}

/// Incremental base-level peaks of a media of known length.
pub struct PeakBuilder {
    frames: u64,
    fed: u64,
    min: Vec<Vec<i16>>,
    max: Vec<Vec<i16>>,
    lo: Vec<f32>,
    hi: Vec<f32>,
}

impl PeakBuilder {
    /// Peaks of `channels` channels over exactly `frames` frames (audio past it is ignored,
    /// missing audio counts as silence, as the whole-file path fits to the document).
    pub fn new(channels: usize, frames: u64) -> Self {
        let n = frames.div_ceil(BASE_SAMPLES_PER_PEAK as u64) as usize;
        Self {
            frames,
            fed: 0,
            min: (0..channels).map(|_| Vec::with_capacity(n)).collect(),
            max: (0..channels).map(|_| Vec::with_capacity(n)).collect(),
            lo: vec![f32::INFINITY; channels],
            hi: vec![f32::NEG_INFINITY; channels],
        }
    }

    pub fn frames_fed(&self) -> u64 {
        self.fed
    }

    /// Feed the next `channels[c][..n]` frames.
    pub fn push(&mut self, channels: &[&[f32]]) {
        let n = channels.first().map_or(0, |c| c.len());
        let n = (n as u64).min(self.frames - self.fed) as usize;
        let spp = BASE_SAMPLES_PER_PEAK as u64;
        let mut i = 0;
        while i < n {
            let in_block = (self.fed % spp) as usize;
            let take = (spp as usize - in_block).min(n - i);
            for (c, (lo, hi)) in self.lo.iter_mut().zip(self.hi.iter_mut()).enumerate() {
                let data = channels.get(c).copied().unwrap_or(&[]);
                for &s in data.get(i..i + take).unwrap_or(&[]) {
                    *lo = lo.min(s);
                    *hi = hi.max(s);
                }
            }
            i += take;
            self.fed += take as u64;
            if self.fed % spp == 0 {
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        for c in 0..self.lo.len() {
            self.min[c].push(q_min(self.lo[c]));
            self.max[c].push(q_max(self.hi[c]));
            self.lo[c] = f32::INFINITY;
            self.hi[c] = f32::NEG_INFINITY;
        }
    }

    /// The peaks (silence for frames never fed).
    pub fn finish(mut self) -> Result<PeakMipmap, MediaError> {
        let spp = BASE_SAMPLES_PER_PEAK as u64;
        // Pad with zeros up to the document length.
        let zeros = [0.0f32; BASE_SAMPLES_PER_PEAK as usize];
        while self.fed < self.frames {
            let take = (spp - self.fed % spp).min(self.frames - self.fed) as usize;
            let chans: Vec<&[f32]> = (0..self.lo.len()).map(|_| &zeros[..take]).collect();
            self.push(&chans);
        }
        if self.fed % spp != 0 {
            self.flush();
        }
        // The cache encoding of `PeakMipmap` (level 0); coarser levels are derived on load.
        let channels = self.min.len() as u16;
        let n = self.min.first().map_or(0, Vec::len);
        let mut out = Vec::with_capacity(19 + n * 4 * channels as usize);
        out.extend_from_slice(b"EPKM");
        out.push(1);
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(&(n as u32).to_le_bytes());
        for v in self.min.iter().chain(&self.max) {
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        PeakMipmap::from_bytes(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DecodedAudio;

    #[test]
    fn matches_whole_file_peaks() {
        let frames = 10_007usize;
        let audio = DecodedAudio {
            sample_rate: 48_000,
            channels: vec![
                (0..frames).map(|i| (i as f32 * 0.013).sin()).collect(),
                (0..frames).map(|i| (i as f32 * 0.07).cos() * 0.5).collect(),
            ],
        };
        let whole = PeakMipmap::build(&audio);
        for block in [1usize, 31, 32, 33, 1000, 20_000] {
            let mut b = PeakBuilder::new(2, frames as u64);
            let mut at = 0;
            while at < frames {
                let n = block.min(frames - at);
                b.push(&[&audio.channels[0][at..at + n], &audio.channels[1][at..at + n]]);
                at += n;
            }
            assert_eq!(b.finish().unwrap(), whole, "block {block}");
        }
    }

    #[test]
    fn fits_to_the_document_length() {
        let audio = DecodedAudio {
            sample_rate: 48_000,
            channels: vec![vec![0.5; 100]],
        };
        // Longer document: padded with silence, like `fit_frames`.
        let mut padded = audio.clone();
        padded.channels[0].resize(150, 0.0);
        let mut b = PeakBuilder::new(1, 150);
        b.push(&[&audio.channels[0]]);
        assert_eq!(b.finish().unwrap(), PeakMipmap::build(&padded));
        // Shorter: cut.
        let mut cut = audio.clone();
        cut.channels[0].truncate(70);
        let mut b = PeakBuilder::new(1, 70);
        b.push(&[&audio.channels[0]]);
        assert_eq!(b.finish().unwrap(), PeakMipmap::build(&cut));
    }
}
