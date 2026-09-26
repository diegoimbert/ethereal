//! Min/max peak pyramids for waveform display.
//!
//! Level `k` holds one `(min, max)` pair per channel for every `32 << k` source frames. Level
//! 0 is computed from the samples; each further level merges pairs of the previous one, so
//! building costs one pass over the audio plus ~1/16 of that. Values are stored quantized
//! to `i16` (min rounded down, max rounded up, so the envelope never shrinks), clamped to
//! -1..=1.

use ether_core::protocol::media::{PeakData, PeakRequest};

use crate::{DecodedAudio, MediaError};

/// Samples per peak of level 0.
pub const BASE_SAMPLES_PER_PEAK: u32 = 32;

const MAGIC: &[u8; 4] = b"EPKM";
const FORMAT_VERSION: u8 = 1;
const SCALE: f32 = i16::MAX as f32;

#[derive(Clone, Debug, PartialEq, Eq)]
struct PeakLevel {
    samples_per_peak: u32,
    /// `[channel][peak]`.
    min: Vec<Vec<i16>>,
    max: Vec<Vec<i16>>,
}

impl PeakLevel {
    fn len(&self) -> usize {
        self.min.first().map_or(0, Vec::len)
    }
}

/// Min/max peak pyramid (levels at powers of two samples-per-peak, from 32 up).
///
/// Build it from audio at the media's *source* rate: [`PeakRequest`] frames are source
/// frames (`MediaRef::sample_rate`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeakMipmap {
    channels: u16,
    frames: u64,
    /// Finest first; never empty once built.
    levels: Vec<PeakLevel>,
}

fn q_min(v: f32) -> i16 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
    (v * SCALE).floor().max(-SCALE) as i16
}

fn q_max(v: f32) -> i16 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
    (v * SCALE).ceil().min(SCALE) as i16
}

impl PeakMipmap {
    pub fn build(audio: &DecodedAudio) -> Self {
        let frames = audio.frames();
        let spp = BASE_SAMPLES_PER_PEAK as usize;
        let mut base = PeakLevel {
            samples_per_peak: BASE_SAMPLES_PER_PEAK,
            min: Vec::with_capacity(audio.channels.len()),
            max: Vec::with_capacity(audio.channels.len()),
        };
        for ch in &audio.channels {
            let (mins, maxs) = ch[..frames]
                .chunks(spp)
                .map(|c| {
                    let (lo, hi) = c
                        .iter()
                        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &s| {
                            (lo.min(s), hi.max(s))
                        });
                    (q_min(lo), q_max(hi))
                })
                .unzip();
            base.min.push(mins);
            base.max.push(maxs);
        }

        Self::from_base(audio.channels.len() as u16, frames as u64, base)
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Source frames covered.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Samples-per-peak of each available level, finest first.
    pub fn levels(&self) -> impl Iterator<Item = u32> + '_ {
        self.levels.iter().map(|l| l.samples_per_peak)
    }

    /// Answer a `MediaCommand::GetPeaks` from the nearest level `>= samples_per_peak` (the
    /// coarsest level if none is). The returned range is widened to whole peaks: it starts
    /// at the peak containing `start_frame` and ends at the one containing the last
    /// requested frame, clipped to the media.
    pub fn query(&self, request: &PeakRequest) -> PeakData {
        let empty = |spp: u32| PeakData {
            media: request.media,
            samples_per_peak: spp,
            start_frame: 0.0,
            min: vec![Vec::new(); self.channels as usize],
            max: vec![Vec::new(); self.channels as usize],
        };
        let wanted = request.samples_per_peak.max(1);
        let Some(level) = self
            .levels
            .iter()
            .find(|l| l.samples_per_peak >= wanted)
            .or(self.levels.last())
        else {
            return empty(wanted.next_power_of_two().max(BASE_SAMPLES_PER_PEAK));
        };
        let spp = level.samples_per_peak as f64;
        let len = level.len();
        let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
        let start = finite(request.start_frame);
        let end = start + finite(request.frame_count).max(0.0);
        // Float-to-int `as` saturates, so negative values land on 0.
        let i0 = ((start / spp).floor() as usize).min(len);
        let i1 = ((end / spp).ceil() as usize).clamp(i0, len);
        let to_f = |v: &[i16]| v.iter().map(|&x| x as f32 / SCALE).collect::<Vec<f32>>();
        PeakData {
            media: request.media,
            samples_per_peak: level.samples_per_peak,
            start_frame: i0 as f64 * spp,
            min: level.min.iter().map(|v| to_f(&v[i0..i1])).collect(),
            max: level.max.iter().map(|v| to_f(&v[i0..i1])).collect(),
        }
    }

    /// Compact binary form for host-side caching (keyed by `MediaRef::hash`).
    ///
    /// Layout (little-endian): `"EPKM"`, `u8` version, `u16` channels, `u64` frames, then
    /// only level 0 as `u32` peak count followed by per-channel `i16` mins then maxes. The
    /// coarser levels are cheap to rebuild on load.
    pub fn to_bytes(&self) -> Vec<u8> {
        let base = self.levels.first();
        let n = base.map_or(0, PeakLevel::len);
        let mut out = Vec::with_capacity(4 + 1 + 2 + 8 + 4 + n * 4 * self.channels as usize);
        out.extend_from_slice(MAGIC);
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(&(n as u32).to_le_bytes());
        if let Some(base) = base {
            for v in base.min.iter().chain(&base.max) {
                for x in v {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MediaError> {
        let bad = |why: &str| MediaError::Decode(format!("invalid peak cache: {why}"));
        let mut r = bytes;
        let mut take = |n: usize| -> Result<&[u8], MediaError> {
            if r.len() < n {
                return Err(bad("truncated"));
            }
            let (head, tail) = r.split_at(n);
            r = tail;
            Ok(head)
        };
        if take(4)? != MAGIC {
            return Err(bad("bad magic"));
        }
        if take(1)?[0] != FORMAT_VERSION {
            return Err(bad("unsupported version"));
        }
        let channels = u16::from_le_bytes(take(2)?.try_into().expect("2 bytes"));
        let frames = u64::from_le_bytes(take(8)?.try_into().expect("8 bytes"));
        let n = u32::from_le_bytes(take(4)?.try_into().expect("4 bytes")) as usize;
        let expected_n = frames.div_ceil(BASE_SAMPLES_PER_PEAK as u64);
        if n as u64 != expected_n {
            return Err(bad("peak count does not match frame count"));
        }
        let body = n
            .checked_mul(channels as usize * 4)
            .ok_or_else(|| bad("too large"))?;
        let data = take(body)?;
        if !r.is_empty() {
            return Err(bad("trailing bytes"));
        }
        let mut values = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b));
        let mut read_vecs = || -> Vec<Vec<i16>> {
            (0..channels)
                .map(|_| values.by_ref().take(n).collect())
                .collect()
        };
        let min = read_vecs();
        let max = read_vecs();
        Ok(Self::from_base(
            channels,
            frames,
            PeakLevel {
                samples_per_peak: BASE_SAMPLES_PER_PEAK,
                min,
                max,
            },
        ))
    }

    /// Derive the coarser levels by merging pairs.
    fn from_base(channels: u16, frames: u64, base: PeakLevel) -> Self {
        let mut levels = vec![base];
        loop {
            let prev = levels.last().expect("non-empty");
            if prev.len() <= 1 || prev.samples_per_peak > u32::MAX / 2 {
                break;
            }
            let merge = |v: &Vec<i16>, f: fn(i16, i16) -> i16| -> Vec<i16> {
                v.chunks(2)
                    .map(|p| p.iter().copied().reduce(f).expect("chunk non-empty"))
                    .collect()
            };
            let next = PeakLevel {
                samples_per_peak: prev.samples_per_peak * 2,
                min: prev.min.iter().map(|v| merge(v, i16::min)).collect(),
                max: prev.max.iter().map(|v| merge(v, i16::max)).collect(),
            };
            levels.push(next);
        }
        Self {
            channels,
            frames,
            levels,
        }
    }
}
