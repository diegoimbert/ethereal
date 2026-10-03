//! Bitcrusher: sample-rate reduction (sample & hold, optional jitter) and bit-depth
//! quantization (optional TPDF dither), with dry/wet mix.
//!
//! - **Rate**: a phase accumulator advances by `Rate / sample rate` per sample; each time it
//!   crosses the (jittered) threshold both channels capture a new held value. No
//!   anti-alias filter: the aliasing is the effect. `Rate` at or above the sample rate
//!   captures every sample.
//! - **Jitter** randomizes each hold period by up to ±50 % (at 100 %).
//! - **Bits** is continuous (fractional depths morph smoothly): the quantizer is mid-tread
//!   with `2^(bits-1)` steps per unit, unbounded (no clipping: the output stage decides).
//! - **Dither** adds ±1 LSB triangular noise before quantizing.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use super::bitcrusher as ids;
use super::shared::{Rng, Values, db, glide};
use crate::util::split_at_events;

/// The bitcrusher device.
pub struct Bitcrusher {
    values: Values<{ ids::COUNT }>,
    sample_rate: f32,
    /// Phase advance per sample (`<= 1`).
    ratio: f32,
    /// Steps per unit of the quantizer.
    steps: f32,
    jitter: f32,
    dither: bool,
    acc: f32,
    threshold: f32,
    held: [f32; 2],
    rng: Rng,
    output: Smoother,
    mix: Smoother,
}

impl Default for Bitcrusher {
    fn default() -> Self {
        Self::new()
    }
}

impl Bitcrusher {
    /// Non-RT. A bitcrusher with default params.
    pub fn new() -> Self {
        let desc = descriptor();
        let mut s = Self {
            values: Values::new(&desc),
            sample_rate: 48_000.0,
            ratio: 1.0,
            steps: 128.0,
            jitter: 0.0,
            dither: false,
            acc: 1.0,
            threshold: 1.0,
            held: [0.0; 2],
            rng: Rng::new(0x5EED_B17C),
            output: Smoother::new(1.0, 0.0, 48_000.0),
            mix: Smoother::new(1.0, 0.0, 48_000.0),
        };
        s.sync_all();
        s
    }

    fn sync_all(&mut self) {
        for i in 0..ids::COUNT {
            self.apply(ParamId(i as u32), false);
        }
    }

    fn apply(&mut self, id: ParamId, smooth: bool) {
        match id {
            ids::BITS => self.steps = (self.values.f(ids::BITS) - 1.0).exp2(),
            ids::RATE => self.ratio = (self.values.f(ids::RATE) / self.sample_rate).min(1.0),
            ids::JITTER => self.jitter = self.values.f(ids::JITTER) * 0.01,
            ids::DITHER => self.dither = self.values.on(ids::DITHER),
            ids::OUTPUT => {
                let v = db(self.values.f(ids::OUTPUT));
                glide(&mut self.output, v, smooth);
            }
            ids::MIX => {
                let v = self.values.f(ids::MIX) * 0.01;
                glide(&mut self.mix, v, smooth);
            }
            _ => {}
        }
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.values.set(id, value) {
            self.apply(id, smooth);
        }
    }

    #[inline]
    fn quantize(&mut self, x: f32) -> f32 {
        let d = if self.dither {
            self.rng.next() - self.rng.next()
        } else {
            0.0
        };
        (x * self.steps + d).round() / self.steps
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let n_ch = audio.outputs.len().min(2);
        for i in start..end {
            if self.acc >= self.threshold {
                self.acc -= self.threshold;
                // Guard (a huge jitter step can't leave us several periods behind).
                self.acc = self.acc.min(1.0);
                self.threshold = if self.jitter > 0.0 {
                    1.0 + self.jitter * (self.rng.next() - 0.5)
                } else {
                    1.0
                };
                for c in 0..n_ch {
                    let x = audio
                        .inputs
                        .get(c)
                        .or(audio.inputs.first())
                        .map_or(0.0, |b| b[i]);
                    self.held[c] = self.quantize(x);
                }
            }
            self.acc += self.ratio;
            let out_gain = self.output.tick();
            let mix = self.mix.tick();
            for c in 0..n_ch {
                let dry = audio
                    .inputs
                    .get(c)
                    .or(audio.inputs.first())
                    .map_or(0.0, |b| b[i]);
                audio.outputs[c][i] = (dry + (self.held[c] - dry) * mix) * out_gain;
            }
        }
    }
}

fn descriptor() -> DeviceDescriptor {
    super::descriptor(BuiltinDeviceType::Bitcrusher)
}

impl Node for Bitcrusher {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        // Capture on the first sample.
        self.acc = 1.0;
        self.threshold = 1.0;
        self.held = [0.0; 2];
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.set(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }
}

impl Device for Bitcrusher {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value, false);
    }
}
