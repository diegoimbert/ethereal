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
//! earlier level), [`InputTapRt::mix_into`] where hardware input is monitored. Placeholders
//! until `groups-buses` lands (no buffers are allocated, nothing is mixed).

use ether_protocol::model::{InputTap, TrackId};
use serde::{Deserialize, Serialize};

use crate::config::EngineConfig;
use crate::mixer::Stereo;

/// `TrackInput::Track` compiled for the engine.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputTapDesc {
    pub track: TrackId,
    pub point: InputTap,
}

/// Consumer side (in the consumer's `TrackRt`).
#[allow(dead_code)] // read once groups-buses lands
#[derive(Debug, Default)]
pub(crate) struct InputTapRt {
    /// Source track index.
    pub source: Option<usize>,
    pub point: Option<InputTap>,
    /// PDC delay applied to the tapped signal (samples).
    pub delay: u32,
}

impl InputTapRt {
    /// Non-RT (graph compile).
    pub(crate) fn compile(
        source: Option<usize>,
        point: Option<InputTap>,
        delay: u32,
        config: &EngineConfig,
    ) -> Self {
        let _ = config;
        Self {
            source,
            point,
            delay,
        }
    }

    /// RT. Take the source's tapped signal for this sub-block (placeholder: no-op).
    pub(crate) fn gather(&mut self, source: &TapBuffers, frames: usize) {
        let _ = (source, frames);
    }

    /// RT. Add the (aligned) tapped signal into the consumer's input (placeholder: no-op).
    pub(crate) fn mix_into(&mut self, a: &mut Stereo, frames: usize, monitor: bool) {
        let _ = (a, frames, monitor);
    }

    /// RT. Carry delay-line state over a snapshot swap.
    pub(crate) fn inherit(&mut self, old: &mut InputTapRt) {
        let _ = old;
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
    /// Non-RT (graph compile): buffers for the points tapped by consumers (placeholder:
    /// none).
    pub(crate) fn compile(points: &[InputTap], config: &EngineConfig) -> Self {
        let _ = (points, config);
        Self::default()
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
