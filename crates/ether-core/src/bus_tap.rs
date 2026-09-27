//! Track input from another track (v0.2, owned by the `groups-buses` node; CONTRACTS.md
//! §12.10): `TrackInput::Track { track, tap }` → `TrackDesc::input_tap`.
//!
//! # Contract
//! - Taps: `PreFx` = the source's input bus + clips (and monitored input), right before its
//!   device chain; `PostFx` = after its chain (and bypass delay), before the fader;
//!   `PostFader` = after fader/pan/mute gate, before sends' split and the PDC output delay
//!   (the sidechain tap point).
//! - Order: the source is processed first (an ordering edge, pre-wired in `graph.rs`, and a
//!   model routing edge, so no cycles).
//! - PDC (pre-wired in `graph.rs`): the tap's latency is `in_lat(source)` for `PreFx` and
//!   `out_lat(source)` otherwise. It counts as an input of the consumer: `in_lat(consumer) =
//!   max(bus inputs, tap latency)`, and the tapped signal is delayed by `in_lat(consumer) −
//!   tap latency` ([`InputTapRt::delay`]), so it is aligned with everything else the consumer
//!   hears and with the timeline at the consumer's chain input.
//! - The consumer hears the tap when it monitors (`TrackDesc::monitor`, like a hardware
//!   input: Auto/In/Off resolved by the controller); recording from a tap records the
//!   aligned signal at the consumer's input, compensated by the consumer's `in_lat`.
//!
//! Hooks (pre-wired): [`TapBuffers::write`] at the three points of every tapped track's job,
//! [`InputTapRt::gather`] at the start of the consumer's job (the source finished in an
//! earlier level), [`InputTapRt::mix_into`] where hardware input is monitored. Buffers are
//! allocated at graph compile, only for the points some consumer taps; the audio thread
//! only copies, delays and adds.

use ether_protocol::model::{InputTap, TrackId};
use serde::{Deserialize, Serialize};

use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::mixer::{Stereo, stereo};

/// `TrackInput::Track` compiled for the engine.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputTapDesc {
    pub track: TrackId,
    pub point: InputTap,
}

/// Consumer side (in the consumer's `TrackRt`).
#[derive(Debug, Default)]
pub(crate) struct InputTapRt {
    /// Source track index.
    pub source: Option<usize>,
    pub point: Option<InputTap>,
    /// PDC delay applied to the tapped signal (samples; the delay line's length).
    #[allow(dead_code)] // kept for debugging; the line owns the delay
    pub delay: u32,
    /// The aligned tapped signal of the current sub-block (empty without a source).
    buf: Stereo,
    line: DelayLine,
    /// `buf` holds this sub-block's signal (set by [`Self::gather`]).
    ready: bool,
}

impl InputTapRt {
    /// Non-RT (graph compile).
    pub(crate) fn compile(
        source: Option<usize>,
        point: Option<InputTap>,
        delay: u32,
        config: &EngineConfig,
    ) -> Self {
        let active = source.is_some() && point.is_some();
        Self {
            source,
            point,
            delay,
            buf: if active {
                stereo(config.max_block_size)
            } else {
                [Vec::new(), Vec::new()]
            },
            line: DelayLine::new(if active { delay as usize } else { 0 }),
            ready: false,
        }
    }

    /// RT. Take the source's tapped signal for this sub-block and align it (PDC delay).
    pub(crate) fn gather(&mut self, source: &TapBuffers, frames: usize) {
        let Some(point) = self.point else { return };
        if self.buf[0].len() < frames {
            return;
        }
        match source.get(point) {
            Some(src) if src[0].len() >= frames => {
                for ch in 0..2 {
                    self.buf[ch][..frames].copy_from_slice(&src[ch][..frames]);
                }
            }
            _ => {
                for ch in 0..2 {
                    self.buf[ch][..frames].fill(0.0);
                }
            }
        }
        let [l, r] = &mut self.buf;
        self.line.process(&mut l[..frames], &mut r[..frames]);
        self.ready = true;
    }

    /// RT. Add the (aligned) tapped signal into the consumer's input when it monitors.
    pub(crate) fn mix_into(&mut self, a: &mut Stereo, frames: usize, monitor: bool) {
        if monitor && self.ready {
            for ch in 0..2 {
                for (d, s) in a[ch][..frames].iter_mut().zip(&self.buf[ch][..frames]) {
                    *d += s;
                }
            }
        }
    }

    /// RT. The aligned tapped signal of the current sub-block (`None` before the consumer's
    /// job gathered it, or without a source): what recording from a tap captures.
    #[allow(dead_code)] // read by the tap recorder
    pub(crate) fn signal(&self, frames: usize) -> Option<[&[f32]; 2]> {
        (self.ready && self.buf[0].len() >= frames)
            .then(|| [&self.buf[0][..frames], &self.buf[1][..frames]])
    }

    /// RT. Carry delay-line state over a snapshot swap.
    pub(crate) fn inherit(&mut self, old: &mut InputTapRt) {
        if old.point == self.point {
            self.line.inherit(&mut old.line);
        }
    }
}

/// Source side (in every tapped track's `TrackRt`): one buffer per tapped point.
#[derive(Debug, Default)]
pub(crate) struct TapBuffers {
    pub pre_fx: Option<Stereo>,
    pub post_fx: Option<Stereo>,
    pub post_fader: Option<Stereo>,
}

impl TapBuffers {
    /// Non-RT (graph compile): buffers for the points tapped by consumers.
    pub(crate) fn compile(points: &[InputTap], config: &EngineConfig) -> Self {
        let has = |p: InputTap| points.contains(&p).then(|| stereo(config.max_block_size));
        Self {
            pre_fx: has(InputTap::PreFx),
            post_fx: has(InputTap::PostFx),
            post_fader: has(InputTap::PostFader),
        }
    }

    fn get(&self, point: InputTap) -> Option<&Stereo> {
        match point {
            InputTap::PreFx => self.pre_fx.as_ref(),
            InputTap::PostFx => self.post_fx.as_ref(),
            InputTap::PostFader => self.post_fader.as_ref(),
        }
    }

    /// RT. Copy the track's current signal at `point` (no-op when not tapped there).
    pub(crate) fn write(&mut self, point: InputTap, a: &Stereo, frames: usize) {
        let buf = match point {
            InputTap::PreFx => &mut self.pre_fx,
            InputTap::PostFx => &mut self.post_fx,
            InputTap::PostFader => &mut self.post_fader,
        };
        if let Some(buf) = buf {
            for ch in 0..2 {
                buf[ch][..frames].copy_from_slice(&a[ch][..frames]);
            }
        }
    }
}
