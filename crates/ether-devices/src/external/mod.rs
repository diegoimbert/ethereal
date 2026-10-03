//! v0.3 devices owned by the `external-instrument` node (contracts-4 froze the param
//! tables; see docs/ROADMAP.md "v0.3" and CONTRACTS.md §13.7).
//!
//! The nodes read and write hardware through the engine's `ether_core::hw_io` buffers (the
//! routing is in the device kind, `ether_model::ExternalRouting`, compiled into
//! `TrackDesc::hw_io`), report `LATENCY` (ms → samples at the prepared rate) as
//! `Node::latency` for PDC, and send an External Instrument's input notes/MIDI to the
//! hardware MIDI ring. Routing changes (`External::SetRouting`) reach live nodes through
//! `EngineBridge::update_builtin` → `Node::set_data`.
//!
//! **Param ids are stable and append-only.**
//!
//! # External Instrument (`BuiltinDeviceType::ExternalInstrument`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Return | `Gain` | -60 ..= 24 Decibels, default 0 |
//! | 1 | Hardware | `Latency` | 0 ..= 500 Milliseconds, default 0 |
//!
//! # External Audio Effect (`BuiltinDeviceType::ExternalAudioEffect`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Send | `Send Gain` | -24 ..= 24 Decibels, default 0 |
//! | 1 | Return | `Return Gain` | -24 ..= 24 Decibels, default 0 |
//! | 2 | Output | `Mix` | 0 ..= 100 Percent, default 100 |
//! | 3 | Hardware | `Latency` | 0 ..= 500 Milliseconds, default 0 |
//! | 4 | Return | `Invert Phase` | toggle, default off |

use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::contract::{FactoryPreset, descriptor as build, param, toggle};
use crate::util;

/// Param ids of `ExternalInstrument` (stable, append-only).
pub mod external_instrument {
    use ether_core::protocol::model::ParamId;
    pub const GAIN: ParamId = ParamId(0);
    pub const LATENCY: ParamId = ParamId(1);
    /// Number of params.
    pub const COUNT: usize = 2;
}

/// Param ids of `ExternalAudioEffect` (stable, append-only).
pub mod external_audio_effect {
    use ether_core::protocol::model::ParamId;
    pub const SEND_GAIN: ParamId = ParamId(0);
    pub const RETURN_GAIN: ParamId = ParamId(1);
    pub const MIX: ParamId = ParamId(2);
    pub const LATENCY: ParamId = ParamId(3);
    pub const INVERT_PHASE: ParamId = ParamId(4);
    /// Number of params.
    pub const COUNT: usize = 5;
}

fn latency(id: u32) -> ether_core::protocol::devices::ParamInfo {
    param(
        id,
        "Latency",
        "Hardware",
        ParamUnit::Milliseconds,
        (0.0, 500.0, 0.0),
        ParamScale::Linear,
    )
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::ExternalInstrument => build(
            BuiltinDeviceType::ExternalInstrument,
            "External Instrument",
            DeviceCategory::Instrument,
            vec![
                param(
                    0,
                    "Gain",
                    "Return",
                    ParamUnit::Decibels,
                    (-60.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                latency(1),
            ],
            0,
            2,
            true,
            0,
        ),
        BuiltinDeviceType::ExternalAudioEffect => build(
            BuiltinDeviceType::ExternalAudioEffect,
            "External Audio Effect",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Send Gain",
                    "Send",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Return Gain",
                    "Return",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                latency(3),
                toggle(4, "Invert Phase", "Return", false),
            ],
            2,
            2,
            false,
            0,
        ),
        other => panic!("{other:?} is not an external device"),
    }
}

/// Non-RT. A new instance of an external device (`routing` is compiled into
/// `TrackDesc::hw_io` by the controller; the node itself holds none).
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    match device.device_type() {
        BuiltinDeviceType::ExternalInstrument => Box::new(ExternalNode::new(Kind::Instrument)),
        BuiltinDeviceType::ExternalAudioEffect => Box::new(ExternalNode::new(Kind::Effect)),
        other => panic!("{other:?} is not an external device"),
    }
}

/// Upper bound of the `Latency` param (ms).
pub const MAX_LATENCY_MS: f64 = 500.0;
const SMOOTH_MS: f32 = 20.0;

/// Latency param (ms) → samples at `sample_rate`.
pub fn latency_samples(ms: f64, sample_rate: f32) -> u32 {
    (ms.clamp(0.0, MAX_LATENCY_MS) * f64::from(sample_rate) / 1000.0).round() as u32
}

/// Samples → the latency param (ms), clamped to its range.
pub fn latency_ms(samples: u32, sample_rate: f32) -> f64 {
    (f64::from(samples) * 1000.0 / f64::from(sample_rate.max(1.0))).clamp(0.0, MAX_LATENCY_MS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Instrument,
    Effect,
}

/// External Instrument / External Audio Effect.
///
/// The engine runs it with `process_sidechain` (`ether_core::hw_io`): the sidechain is the
/// hardware return (stereo), and an effect gets its hardware send as `outputs[2..4]`.
/// Without that (no routing, the web, offline renders) it runs `process` with a silent
/// return.
///
/// - Instrument: `out = return · Gain`.
/// - Effect: `send = in · Send Gain`; `wet = return · Return Gain · (±1)`;
///   `out = dry(t - latency) · (1 - Mix) + wet · Mix`, so dry and wet line up, and PDC
///   lines both up with the rest of the mix.
///
/// `Node::latency` is the `Latency` param in samples.
struct ExternalNode {
    kind: Kind,
    descriptor: DeviceDescriptor,
    values: Vec<f64>,
    sample_rate: f32,
    latency: u32,
    /// Dry delay lines (effect), `max latency + 1` samples each.
    dry: [Vec<f32>; 2],
    pos: usize,
    /// Instrument gain, or the effect's (signed) return gain.
    gain: Smoother,
    send: Smoother,
    mix: Smoother,
}

impl ExternalNode {
    fn new(kind: Kind) -> Self {
        let ty = match kind {
            Kind::Instrument => BuiltinDeviceType::ExternalInstrument,
            Kind::Effect => BuiltinDeviceType::ExternalAudioEffect,
        };
        let descriptor = descriptor(ty);
        let values = descriptor.params.iter().map(|p| p.default).collect();
        let mut n = Self {
            kind,
            descriptor,
            values,
            sample_rate: 48_000.0,
            latency: 0,
            dry: [Vec::new(), Vec::new()],
            pos: 0,
            gain: Smoother::new(1.0, SMOOTH_MS, 48_000.0),
            send: Smoother::new(1.0, SMOOTH_MS, 48_000.0),
            mix: Smoother::new(1.0, SMOOTH_MS, 48_000.0),
        };
        n.sync(false);
        n
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values.get(id.0 as usize).copied().unwrap_or(0.0)
    }

    /// Smoother targets (gain, send, mix) from the params.
    fn targets(&self) -> [f32; 3] {
        let db = |id| util::db_to_amp(self.value(id) as f32);
        match self.kind {
            Kind::Instrument => [db(external_instrument::GAIN), 0.0, 1.0],
            Kind::Effect => {
                use external_audio_effect as p;
                let sign = if self.value(p::INVERT_PHASE) >= 0.5 {
                    -1.0
                } else {
                    1.0
                };
                [
                    db(p::RETURN_GAIN) * sign,
                    db(p::SEND_GAIN),
                    (self.value(p::MIX) * 0.01) as f32,
                ]
            }
        }
    }

    fn latency_param(&self) -> ParamId {
        match self.kind {
            Kind::Instrument => external_instrument::LATENCY,
            Kind::Effect => external_audio_effect::LATENCY,
        }
    }

    fn sync(&mut self, smooth: bool) {
        let [g, s, m] = self.targets();
        let sr = self.sample_rate;
        for (sm, t) in [(&mut self.gain, g), (&mut self.send, s), (&mut self.mix, m)] {
            if !smooth {
                *sm = Smoother::new(t, SMOOTH_MS, sr);
            } else if sm.target() != t {
                sm.set_target(t);
            }
        }
        let samples = latency_samples(self.value(self.latency_param()), sr);
        self.latency = match self.dry[0].len() {
            0 => samples,
            len => samples.min(len as u32 - 1),
        };
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(info) = self.descriptor.params.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, info.min, info.max);
        self.sync(smooth);
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, ret: &[&[f32]], a: usize, b: usize) {
        let r = |c: usize, i: usize| ret.get(c).or(ret.first()).map_or(0.0, |ch| ch[i]);
        match self.kind {
            Kind::Instrument => {
                for i in a..b {
                    let g = self.gain.tick();
                    write(audio.outputs, i, r(0, i) * g, r(1, i) * g);
                }
            }
            Kind::Effect => {
                let inputs = audio.inputs;
                let input =
                    |c: usize, i: usize| inputs.get(c).or(inputs.first()).map_or(0.0, |ch| ch[i]);
                let len = self.dry[0].len();
                let d = self.latency as usize;
                for i in a..b {
                    let (xl, xr) = (input(0, i), input(1, i));
                    let send = self.send.tick();
                    let wet_g = self.gain.tick();
                    let mix = self.mix.tick();
                    // The dry signal, delayed by the latency (aligned with the return).
                    let (dl, dr) = if d == 0 || len == 0 {
                        (xl, xr)
                    } else {
                        let j = (self.pos + len - d) % len;
                        (self.dry[0][j], self.dry[1][j])
                    };
                    if len > 0 {
                        self.dry[0][self.pos] = xl;
                        self.dry[1][self.pos] = xr;
                        self.pos = (self.pos + 1) % len;
                    }
                    let out_l = dl * (1.0 - mix) + r(0, i) * wet_g * mix;
                    let out_r = dr * (1.0 - mix) + r(1, i) * wet_g * mix;
                    if let [ol, or, sl, sr, ..] = &mut *audio.outputs {
                        ol[i] = out_l;
                        or[i] = out_r;
                        sl[i] = xl * send;
                        sr[i] = xr * send;
                    } else {
                        write(audio.outputs, i, out_l, out_r);
                    }
                }
            }
        }
    }

    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        ret: &[&[f32]],
    ) -> ProcessStatus {
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, ret, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }
}

/// Write a stereo frame to the main outputs (mono: the average).
#[inline]
fn write(outputs: &mut [&mut [f32]], i: usize, l: f32, r: f32) {
    match outputs {
        [ol, or, ..] => {
            ol[i] = l;
            or[i] = r;
        }
        [o] => o[i] = (l + r) * 0.5,
        [] => {}
    }
}

impl Node for ExternalNode {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        if self.kind == Kind::Effect {
            let len = latency_samples(MAX_LATENCY_MS, self.sample_rate) as usize + 1;
            self.dry = [vec![0.0; len], vec![0.0; len]];
            self.pos = 0;
        }
        self.sync(false);
    }

    fn reset(&mut self) {
        for d in self.dry.iter_mut() {
            d.fill(0.0);
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        // No hardware here: a silent return.
        self.run(ctx, audio, &[])
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, sidechain)
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn channels(&self) -> (u16, u16) {
        match self.kind {
            Kind::Instrument => (0, 2),
            Kind::Effect => (2, 2),
        }
    }
}

impl Device for ExternalNode {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

/// Factory presets of a type of this group (none expected: hardware differs per user).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = ty;
    &[]
}
