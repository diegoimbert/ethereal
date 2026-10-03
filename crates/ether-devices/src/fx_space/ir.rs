//! Impulse responses: the time-domain base IR of a convolver ([`IrBase`], read from project
//! media or synthesized for the factory set, energy-normalized) and the param-driven shaping
//! (`Decay`, `Size`, `Reverse`: [`Shaping`]) that turns it into the kernel the convolver
//! partitions.
//!
//! Everything here allocates and runs off the audio thread except [`IrBase::shaped`], which
//! the convolver also calls (bounded, allocation-free) when it rebuilds a kernel because a
//! shaping param moved.

use std::sync::Arc;

use ether_core::AudioSource;

use super::FACTORY_IRS;

/// Longest IR kept (seconds at the engine rate); longer media are truncated.
pub const MAX_IR_SECONDS: f64 = 10.0;
/// `Size` range (time stretch factor).
pub(crate) const SIZE_MIN: f32 = 0.5;
pub(crate) const SIZE_MAX: f32 = 1.5;
/// `Decay` lower bound (fraction of the IR kept).
pub(crate) const DECAY_MIN: f32 = 0.1;

/// Where a convolver's IR comes from (kept so `prepare` can rebuild at a new rate).
#[derive(Clone)]
pub enum IrInput {
    /// Index into [`FACTORY_IRS`].
    Factory(usize),
    /// Project media (already at the engine rate).
    Media(Arc<dyn AudioSource>),
}

impl std::fmt::Debug for IrInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Factory(i) => write!(f, "Factory({})", FACTORY_IRS[*i].id),
            Self::Media(s) => write!(f, "Media({} ch, {} frames)", s.channels(), s.frames()),
        }
    }
}

/// The IR shaping params, quantized so equal settings compare equal (`Decay` and `Size` in
/// 0.1 % steps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shaping {
    /// `Decay` in permille (100 ..= 1000).
    pub decay: u16,
    /// `Size` in permille (500 ..= 1500).
    pub size: u16,
    pub reverse: bool,
}

impl Default for Shaping {
    fn default() -> Self {
        Self {
            decay: 1000,
            size: 1000,
            reverse: false,
        }
    }
}

impl Shaping {
    /// From the plain param values (`Decay` and `Size` in percent, `Reverse` 0/1).
    pub fn from_params(decay_pct: f64, size_pct: f64, reverse: f64) -> Self {
        let q = |pct: f64, lo: f32, hi: f32| {
            let f = (pct * 0.01).clamp(lo as f64, hi as f64);
            (f * 1000.0).round() as u16
        };
        Self {
            decay: q(decay_pct, DECAY_MIN, 1.0),
            size: q(size_pct, SIZE_MIN, SIZE_MAX),
            reverse: reverse >= 0.5,
        }
    }

    pub(crate) fn decay(self) -> f64 {
        self.decay as f64 * 0.001
    }

    pub(crate) fn size(self) -> f64 {
        self.size as f64 * 0.001
    }

    /// Fraction of the kept IR covered by the `Decay` fade-out (0 at 100 %, half the kept
    /// length from 90 % down).
    fn fade_fraction(self) -> f64 {
        ((1.0 - self.decay()) * 5.0).min(0.5)
    }
}

/// A base IR at the engine rate: 1 or 2 channels, energy-normalized (a white-noise input
/// comes out at about its own level).
#[derive(Clone, Debug)]
pub struct IrBase {
    pub(crate) channels: Vec<Vec<f32>>,
    pub(crate) len: usize,
}

impl IrBase {
    /// Non-RT. Read `input` at `sample_rate`. `None` when media can't be read or is empty.
    pub fn load(input: &IrInput, sample_rate: f32) -> Option<Self> {
        let max = (MAX_IR_SECONDS * sample_rate as f64) as usize;
        let channels = match input {
            IrInput::Factory(i) => super::factory_ir::synthesize(&FACTORY_IRS[*i], sample_rate),
            IrInput::Media(src) => {
                let frames = (src.frames() as usize).min(max);
                if frames == 0 || src.channels() == 0 {
                    return None;
                }
                let n = src.channels().min(2);
                let mut chans = Vec::with_capacity(n as usize);
                for c in 0..n {
                    let mut buf = vec![0.0f32; frames];
                    if !src.read(c, 0, &mut buf) {
                        return None;
                    }
                    for v in &mut buf {
                        if !v.is_finite() {
                            *v = 0.0;
                        }
                    }
                    chans.push(buf);
                }
                chans
            }
        };
        Self::from_channels(channels, max)
    }

    /// Non-RT. From raw channels (1 or 2; truncated to `max` frames), normalized.
    pub fn from_channels(mut channels: Vec<Vec<f32>>, max: usize) -> Option<Self> {
        channels.truncate(2);
        let len = channels.iter().map(Vec::len).max()?.min(max);
        if len == 0 {
            return None;
        }
        for c in &mut channels {
            c.resize(len, 0.0);
        }
        let energy: f64 = channels
            .iter()
            .map(|c| c.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>())
            .sum::<f64>()
            / channels.len() as f64;
        if energy <= 1e-20 {
            return None;
        }
        let g = (1.0 / energy.sqrt()) as f32;
        for c in &mut channels {
            c.iter_mut().for_each(|v| *v *= g);
        }
        Some(Self { channels, len })
    }

    /// Length in frames of the kernel for `shaping`.
    pub(crate) fn shaped_len(&self, shaping: Shaping) -> usize {
        ((self.len as f64 * shaping.size() * shaping.decay()).round() as usize).max(1)
    }

    /// Longest possible shaped length (`Size` 150 %, `Decay` 100 %): kernel capacity.
    pub(crate) fn max_shaped_len(&self) -> usize {
        (self.len as f64 * SIZE_MAX as f64).ceil() as usize + 1
    }

    /// RT-safe. Write samples `[start, start + out.len())` of channel `ch` (mono IRs repeat
    /// channel 0) of the kernel for `shaping` (length `len` = [`Self::shaped_len`]) into
    /// `out`; zero past the end.
    pub(crate) fn shaped(&self, ch: usize, shaping: Shaping, len: usize, start: usize, out: &mut [f32]) {
        let src = &self.channels[ch.min(self.channels.len() - 1)];
        let size = shaping.size();
        let inv = 1.0 / size;
        // Stretching by `size` scales the energy by `size`: keep the level.
        let gain = (1.0 / size.sqrt()) as f32;
        let fade = shaping.fade_fraction();
        let fade_start = len as f64 * (1.0 - fade);
        let fade_len = (len as f64 - fade_start).max(1.0);
        let exact = shaping.size == 1000;
        for (i, o) in out.iter_mut().enumerate() {
            let n = start + i;
            if n >= len {
                *o = 0.0;
                continue;
            }
            let m = if shaping.reverse { len - 1 - n } else { n };
            let v = if exact {
                src.get(m).copied().unwrap_or(0.0)
            } else {
                let pos = m as f64 * inv;
                let j = pos as usize;
                let frac = (pos - j as f64) as f32;
                let a = src.get(j).copied().unwrap_or(0.0);
                let b = src.get(j + 1).copied().unwrap_or(0.0);
                a + (b - a) * frac
            };
            let env = if fade > 0.0 && (m as f64) >= fade_start {
                let x = (m as f64 - fade_start) / fade_len;
                (0.5 * (1.0 + (std::f64::consts::PI * x.min(1.0)).cos())) as f32
            } else {
                1.0
            };
            *o = v * env * gain;
        }
    }
}
