//! Volume LFO: the gain follows a drawn LFO cycle (`ether_model::LfoCurve`), for sidechain
//! pumping without a sidechain, gates, chops and tremolos.
//!
//! - **Gain**: `1 - Depth × (1 - curve(phase))`, per sample, so 0 % depth is bit-exact
//!   unity and 100 % follows the curve. The right channel reads the curve `Stereo Phase`
//!   degrees later.
//! - **Phase**: synced (the default) and playing, it is computed from the song position at
//!   every sample (`position / cycle beats + Phase`): phase 0 falls on every multiple of the
//!   cycle from beat 0, wherever playback starts, and a bounce is sample-exact. Stopped, a
//!   synced LFO runs on at the tempo; free (Hz) it runs on its own and restarts at phase 0
//!   when the transport starts (and on `reset`), so renders are deterministic.
//! - **Smoothing**: a one-pole on the gain (`Smoothing` ms, default 1) tames the hard edges
//!   of a drawn curve, a curve swap or a phase jump: the gain moves at most
//!   `1 - e^(-1/(ms·sr))` of the way per sample. 0 ms = the curve as drawn.
//! - **Curve edits** arrive as a [`CurveData`] through `Node::set_data` (swapped, never
//!   allocated or freed here); the smoothing glides from the old curve to the new one.
//! - Zero latency, memoryless on the audio (silent in, silent out).

use ether_core::node::NodeData;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, LfoCurve, LfoPoint, MAX_LFO_POINTS, ParamId, lfo_value_at,
};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::shared::{Params, Ramp, input};
use super::volume_lfo as p;
use crate::contract::SYNC_RATE_BEATS;
use crate::util::{self, db_to_amp};

const N: usize = p::COUNT;

/// A curve as the node plays it: a fixed array (no allocation on the audio thread).
#[derive(Clone, Copy, Debug)]
pub struct CurveData {
    points: [LfoPoint; MAX_LFO_POINTS],
    len: usize,
}

impl CurveData {
    /// Non-RT. The playable copy of `curve` (sanitized: sorted, clamped, at most 64 points).
    pub fn new(curve: &LfoCurve) -> Self {
        let curve = curve.sanitized();
        let mut points = [LfoPoint::linear(0.0, 1.0); MAX_LFO_POINTS];
        let len = curve.points.len().min(MAX_LFO_POINTS);
        points[..len].copy_from_slice(&curve.points[..len]);
        Self { points, len }
    }

    /// Value at `phase` (cycles), `0..=1`.
    #[inline]
    pub fn at(&self, phase: f64) -> f32 {
        lfo_value_at(&self.points[..self.len], phase) as f32
    }
}

/// Non-RT. `Node::set_data` payload of a curve edit (bridges' `update_builtin`); `None` for
/// another device.
pub fn curve_data(device: &BuiltinDevice) -> Option<NodeData> {
    match device {
        BuiltinDevice::VolumeLfo { curve } => Some(Box::new(CurveData::new(curve))),
        _ => None,
    }
}

pub(super) struct VolumeLfo {
    params: Params<N>,
    sample_rate: f32,
    curve: CurveData,
    /// Phase (cycles, `0..1`) before the offsets.
    phase: f64,
    was_playing: bool,
    depth: Ramp,
    gain: Ramp,
    /// Smoothed per-channel curve gains.
    smoothed: [f32; 2],
    /// One-pole coefficient of the smoothing (0 = none).
    coef: f32,
    tail: util::Tail,
}

impl VolumeLfo {
    pub(super) fn new(desc: &DeviceDescriptor, curve: &LfoCurve) -> Self {
        let mut s = Self {
            params: Params::<N>::new(desc),
            sample_rate: 48_000.0,
            curve: CurveData::new(curve),
            phase: 0.0,
            was_playing: false,
            depth: Ramp::new(1.0),
            gain: Ramp::new(1.0),
            smoothed: [1.0; 2],
            coef: 0.0,
            tail: util::Tail::default(),
        };
        for id in 0..N as u32 {
            s.sync_param(ParamId(id), false);
        }
        s.reset();
        s
    }

    fn sync_param(&mut self, id: ParamId, smooth: bool) {
        let v = self.params.get(id);
        match id {
            p::DEPTH => self.depth.set(v * 0.01, smooth),
            p::OUTPUT => self.gain.set(db_to_amp(v), smooth),
            p::SMOOTHING => {
                self.coef = if v <= 0.0 {
                    0.0
                } else {
                    util::tau_coef(v, self.sample_rate)
                };
            }
            _ => {}
        }
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.params.set(id, value) {
            self.sync_param(id, smooth);
        }
    }

    /// Cycle length in beats when synced.
    fn sync_beats(&self) -> Option<f64> {
        self.params.on(p::SYNC).then(|| {
            let i = self.params.index(p::SYNC_RATE, SYNC_RATE_BEATS.len());
            SYNC_RATE_BEATS[i]
        })
    }

    /// Free-running phase increment per sample (synced: at the tempo `bpm`).
    fn free_inc(&self, bpm: f64) -> f64 {
        let sr = f64::from(self.sample_rate);
        match self.sync_beats() {
            Some(beats) => {
                let bpm = if bpm > 0.0 { bpm } else { 120.0 };
                bpm / 60.0 / beats / sr
            }
            None => f64::from(self.params.get(p::RATE)).max(0.0) / sr,
        }
    }

    /// Phase offsets `[left, right]` (cycles).
    fn offsets(&self) -> [f64; 2] {
        let phase = f64::from(self.params.get(p::PHASE)) / 360.0;
        let stereo = f64::from(self.params.get(p::STEREO_PHASE)) / 360.0;
        [phase, phase + stereo]
    }

    /// Unsmoothed gains `[left, right]` at the current phase and depth target.
    fn targets(&self, depth: f32) -> [f32; 2] {
        let o = self.offsets();
        [0, 1].map(|ch| 1.0 - depth * (1.0 - self.curve.at(self.phase + o[ch])))
    }

    crate::dsp::simd::avx2_dispatch! {
        fn render => render_avx2, render_generic(
            &mut self,
            ctx: &ProcessContext<'_>,
            audio: &mut AudioBuffers<'_, '_>,
            start: usize,
            end: usize,
        )
    }

    #[inline(always)]
    fn render_generic(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        start: usize,
        end: usize,
    ) {
        let t = ctx.transport;
        let locked = (t.playing && t.beats_per_sample > 0.0)
            .then_some(())
            .and(self.sync_beats());
        let inc = self.free_inc(t.bpm);
        let offsets = self.offsets();
        let channels = audio.outputs.len().min(2);
        let inputs = audio.inputs;
        let k = self.coef;
        for i in start..end {
            self.phase = match locked {
                Some(beats) => {
                    let ph = (t.position + t.beats_per_sample * i as f64) / beats;
                    ph - ph.floor()
                }
                None => {
                    let ph = self.phase + inc;
                    ph - ph.floor()
                }
            };
            let depth = self.depth.tick();
            let gain = self.gain.tick();
            for (ch, &offset) in offsets.iter().enumerate().take(channels) {
                let target = 1.0 - depth * (1.0 - self.curve.at(self.phase + offset));
                let sm = &mut self.smoothed[ch];
                *sm = crate::dsp::flush32(target + (*sm - target) * k);
                // A tiny input times a decaying gain can't go subnormal.
                audio.outputs[ch][i] = crate::dsp::flush32(input(inputs, ch, i) * *sm * gain);
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
        }
    }
}

impl Node for VolumeLfo {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_param(p::SMOOTHING, false);
        self.reset();
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.smoothed = self.targets(self.params.get(p::DEPTH) * 0.01);
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let c: &ProcessContext<'_> = ctx;
        let t = c.transport;
        // A free LFO restarts with the transport (deterministic renders); a synced one
        // follows the song position anyway.
        if t.playing && !self.was_playing && self.sync_beats().is_none() {
            self.phase = 0.0;
        }
        self.was_playing = t.playing;
        // Asleep for a while: the free LFO goes on as if it had run.
        let skipped = self.tail.skipped(c);
        if skipped > 0 {
            let ph = self.phase + self.free_inc(t.bpm) * skipped as f64;
            self.phase = ph - ph.floor();
            self.smoothed = self.targets(self.params.get(p::DEPTH) * 0.01);
        }
        util::split_at_events(
            self,
            c.events,
            c.frames,
            |s, a, b| s.render(c, audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply(param, value, !step);
                }
            },
        );
        // Memoryless on the audio (input × gain): silent in, silent out.
        let settled = !self.depth.is_smoothing() && !self.gain.is_smoothing();
        self.tail.status_of(c, audio, settled, 1)
    }

    fn set_data(&mut self, data: NodeData) -> Option<NodeData> {
        match data.downcast::<CurveData>() {
            Ok(mut new) => {
                // The old curve goes back in the box (dropped off the audio thread).
                std::mem::swap(&mut self.curve, &mut *new);
                Some(new)
            }
            Err(other) => Some(other),
        }
    }
}

impl Device for VolumeLfo {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::VolumeLfo)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}
