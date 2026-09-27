//! Built-in `DrumRack` device (roadmap v2, owned by the `drum-rack` node; see
//! `docs/ROADMAP.md` and CONTRACTS.md §11.12).
//!
//! The drum rack's own node in the track chain. Pad chains are separate nodes listed in
//! `TrackDesc::racks`; the engine runs them where this node sits in the chain (note routing,
//! choke groups, pad mix, PDC: `ether_core`'s drum_rack module) and mixes them into this
//! node's input. This node applies the rack's own params to that mix.
//!
//! It is an instrument (MIDI in, no audio input in its descriptor) whose node still takes
//! a stereo input (`Node::channels` = (2, 2)): the pad mix.
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | name     | range            |
//! |----|----------|------------------|
//! | 0  | `Volume` | -60 ..= 6 dB     |
//! | 1  | `Pan`    | -1 ..= 1 balance |

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const VOLUME: ParamId = ParamId(0);
    pub const PAN: ParamId = ParamId(1);
}

const NUM_PARAMS: usize = 2;
const SMOOTH_MS: f32 = 20.0;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::param;
    vec![
        param(
            0,
            "Volume",
            "Rack",
            ParamUnit::Decibels,
            (-60.0, 6.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            1,
            "Pan",
            "Rack",
            ParamUnit::Pan,
            (-1.0, 1.0, 0.0),
            ParamScale::Linear,
        ),
    ]
}

/// Descriptor of the `DrumRack` type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: None,
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::DrumRack,
        },
        name: "Drum Rack".to_owned(),
        category: DeviceCategory::Instrument,
        params: param_infos(),
        audio_inputs: 0,
        audio_outputs: 2,
        midi_input: true,
        sidechain_inputs: 0,
    }
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(DrumRack::new())
}

/// The rack node: rack volume and balance pan over the pads' mix.
#[derive(Debug)]
pub struct DrumRack {
    values: [f64; NUM_PARAMS],
    infos: Vec<ParamInfo>,
    sample_rate: f32,
    volume: Smoother,
    pan: Smoother,
}

impl Default for DrumRack {
    fn default() -> Self {
        Self::new()
    }
}

impl DrumRack {
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        for (v, info) in values.iter_mut().zip(&infos) {
            *v = info.default;
        }
        let mut s = Self {
            values,
            infos,
            sample_rate: 48_000.0,
            volume: Smoother::new(1.0, SMOOTH_MS, 48_000.0),
            pan: Smoother::new(0.0, SMOOTH_MS, 48_000.0),
        };
        s.sync_all();
        s
    }

    fn sync_all(&mut self) {
        self.volume = Smoother::new(
            util::db_to_amp(self.values[0] as f32),
            SMOOTH_MS,
            self.sample_rate,
        );
        self.pan = Smoother::new(self.values[1] as f32, SMOOTH_MS, self.sample_rate);
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(info) = self.infos.get(id.0 as usize) else {
            return;
        };
        let v = util::clamp(value, info.min, info.max);
        self.values[id.0 as usize] = v;
        let (smoother, target) = match id {
            params::VOLUME => (&mut self.volume, util::db_to_amp(v as f32)),
            params::PAN => (&mut self.pan, v as f32),
            _ => return,
        };
        if smooth {
            smoother.set_target(target);
        } else {
            smoother.set_immediate(target);
        }
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let [l_in, r_in] = match audio.inputs {
            [l, r, ..] => [*l, *r],
            [m] => [*m, *m],
            [] => {
                for ch in audio.outputs.iter_mut() {
                    ch[start..end].fill(0.0);
                }
                return;
            }
        };
        let (left, right) = match &mut *audio.outputs {
            [l, r, ..] => (l, r),
            _ => return,
        };
        for k in start..end {
            let g = self.volume.tick();
            let p = self.pan.tick().clamp(-1.0, 1.0);
            let gl = if p > 0.0 { 1.0 - p } else { 1.0 };
            let gr = if p < 0.0 { 1.0 + p } else { 1.0 };
            left[k] = l_in[k] * g * gl;
            right[k] = r_in[k] * g * gr;
        }
    }
}

impl Node for DrumRack {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
    }

    fn reset(&mut self) {}

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut pos = 0;
        for ev in ctx.events {
            let off = (ev.offset as usize).min(ctx.frames);
            if off > pos {
                self.render(audio, pos, off);
                pos = off;
            }
            if let EventKind::Param { param, value } = ev.kind {
                self.apply_param(param, value, true);
            }
        }
        if pos < ctx.frames {
            self.render(audio, pos, ctx.frames);
        }
        ProcessStatus::Continue
    }

    /// Stereo in (the pad mix the engine feeds it), stereo out.
    fn channels(&self) -> (u16, u16) {
        (2, 2)
    }
}

impl Device for DrumRack {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
