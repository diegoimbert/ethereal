//! The `ConvolutionReverb` node: pre-delay → [`Convolver`] → low/high cut → width → gain,
//! mixed with the dry signal.
//!
//! IR swaps (`Device::SetIr`) arrive as an [`IrSwap`] through `Node::set_data`: a convolver
//! built off the audio thread. When the live shaping params differ from the ones it was
//! built with, it is rebuilt a bounded slice per block before it plays (it isn't heard
//! yet). Then the old and new convolvers both run and the wet signal crossfades (equal
//! power, [`SWAP_MS`]); the new one starts from silence, the old one's tail fades out. The
//! replaced convolvers go back to the GC thread with the next `IrSwap` (or with the node):
//! nothing is freed on the audio thread.
//!
//! Without an IR (`ir: None`, unknown factory id, unresolved media) the device passes the
//! dry signal through.

use ether_core::node::NodeData;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::ParamId;
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use super::convolution_reverb as p;
use super::convolver::{BUILD_UNITS_PER_TICK, Convolver};
use super::ir::{IrBase, IrInput, Shaping};
use crate::dsp::delay_line::DelayLine;
use crate::dsp::svf::{Coefs, Shape, Svf};
use crate::util;

/// Wet crossfade between two IRs.
pub const SWAP_MS: f32 = 200.0;
const MAX_PRE_DELAY_MS: f32 = 250.0;
const PRE_GLIDE_MS: f32 = 60.0;
const FILTER_GLIDE_MS: f32 = 15.0;
const Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Filter settings that mean "off" (the ends of their ranges).
const LOW_CUT_OFF: f64 = 20.0;
const HIGH_CUT_OFF: f64 = 20_000.0;
/// Rebuild budget per block for a convolver waiting to be swapped in.
const PENDING_UNITS: usize = 4 * BUILD_UNITS_PER_TICK;

/// `Node::set_data` payload of a convolution reverb (built by [`super::ir_swap`]): the new
/// convolver (`None` = no IR). On the way back (to the GC thread) it carries whatever the
/// node let go of.
pub struct IrSwap {
    pub(crate) next: Option<Box<Convolver>>,
    // Boxed on purpose: unboxing a node's convolver would free its box on the audio thread.
    #[allow(clippy::vec_box)]
    pub(crate) replaced: Vec<Box<Convolver>>,
    pub(crate) replaced_input: Option<IrInput>,
}

impl IrSwap {
    /// Non-RT. `next` plus room for the two convolvers a swap can hand back.
    pub(crate) fn new(next: Option<Box<Convolver>>) -> Self {
        Self {
            next,
            replaced: Vec::with_capacity(RETIRED + 1),
            replaced_input: None,
        }
    }

    /// Convolvers handed back by the node (retired or superseded), for tests.
    pub fn replaced_count(&self) -> usize {
        self.replaced.len()
    }
}

/// Convolvers waiting for the GC thread (at most two between swaps: the one fading out
/// when a swap arrives and the one the queued swap replaces).
const RETIRED: usize = 2;

/// The convolution reverb.
pub struct ConvolutionReverb {
    values: [f64; p::COUNT],
    sample_rate: f32,
    max_block: usize,
    /// The IR to build in `prepare` while no convolver holds it (moved into the convolver
    /// once built; a convolver keeps its own input for rebuilds at a new rate).
    input: Option<IrInput>,
    built_rate: f32,
    conv: Option<Box<Convolver>>,
    /// A swap waiting for its kernel to match the live shaping (`Some(None)` = to no IR).
    pending: Option<Option<Box<Convolver>>>,
    /// Swap in flight: the convolver fading out and the progress.
    from: Option<Box<Convolver>>,
    swapping: bool,
    swap_pos: usize,
    swap_len: usize,
    /// Whether the outgoing side of the swap had an IR (dry level crossfade).
    from_had_ir: bool,
    retired: [Option<Box<Convolver>>; RETIRED],
    pre: [DelayLine; 2],
    pre_cur: f32,
    pre_coef: f32,
    low: Coefs,
    high: Coefs,
    filter_coef: f64,
    svf: [[Svf; 2]; 2],
    width: Smoother,
    gain: Smoother,
    mix: Smoother,
    /// Pre-delayed convolver input, wet outputs (current and outgoing), per channel.
    cin: [Vec<f32>; 2],
    wet: [Vec<f32>; 2],
    wet_from: [Vec<f32>; 2],
}

impl ConvolutionReverb {
    /// Non-RT. A reverb on `input` (built in `prepare`, at the engine rate).
    pub fn new(input: Option<IrInput>) -> Self {
        let infos =
            super::descriptor(ether_core::protocol::model::BuiltinDeviceType::ConvolutionReverb)
                .params;
        let mut values = [0.0; p::COUNT];
        for info in &infos {
            values[info.id.0 as usize] = info.default;
        }
        let mut r = Self {
            values,
            sample_rate: 48_000.0,
            max_block: 0,
            input,
            built_rate: 0.0,
            conv: None,
            pending: None,
            from: None,
            swapping: false,
            swap_pos: 0,
            swap_len: 1,
            from_had_ir: false,
            retired: [None, None],
            pre: Default::default(),
            pre_cur: 0.0,
            pre_coef: 0.0,
            low: Coefs::BYPASS,
            high: Coefs::BYPASS,
            filter_coef: 0.0,
            svf: Default::default(),
            width: Smoother::new(1.0, 20.0, 48_000.0),
            gain: Smoother::new(1.0, 20.0, 48_000.0),
            mix: Smoother::new(0.3, 20.0, 48_000.0),
            cin: Default::default(),
            wet: Default::default(),
            wet_from: Default::default(),
        };
        r.allocate(256);
        r
    }

    fn v(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    /// Shaping requested by the params.
    pub fn shaping(&self) -> Shaping {
        Shaping::from_params(self.v(p::DECAY), self.v(p::SIZE), self.v(p::REVERSE))
    }

    /// Shaping heard now (`None` without an IR).
    pub fn current_shaping(&self) -> Option<Shaping> {
        self.conv.as_ref().map(|c| c.shaping())
    }

    /// Whether an IR is playing.
    pub fn has_ir(&self) -> bool {
        self.conv.is_some()
    }

    /// Whether nothing is in flight (no queued or running swap, kernel matches the params).
    pub fn settled(&self) -> bool {
        self.pending.is_none() && !self.swapping && self.conv.as_ref().is_none_or(|c| c.settled())
    }

    fn allocate(&mut self, max_block: usize) {
        let sr = self.sample_rate;
        let pre_len = (MAX_PRE_DELAY_MS * 0.001 * sr).ceil() as usize + 4;
        self.pre = [DelayLine::new(pre_len), DelayLine::new(pre_len)];
        self.pre_coef = util::tau_coef(PRE_GLIDE_MS, sr);
        self.filter_coef = util::tau_coef(FILTER_GLIDE_MS, sr) as f64;
        self.max_block = max_block.max(1);
        let v = || vec![0.0f32; max_block.max(1)];
        self.cin = [v(), v()];
        self.wet = [v(), v()];
        self.wet_from = [v(), v()];
        self.swap_len = ((SWAP_MS * 0.001 * sr) as usize).max(1);
        let mk = |x: f32| Smoother::new(x, 20.0, sr);
        self.width = mk(self.width_target());
        self.gain = mk(self.gain_target());
        self.mix = mk(self.mix_target());
        self.low = self.low_target();
        self.high = self.high_target();
        self.pre_cur = self.pre_target();
    }

    fn width_target(&self) -> f32 {
        (self.v(p::WIDTH) * 0.01) as f32
    }

    fn gain_target(&self) -> f32 {
        util::db_to_amp(self.v(p::GAIN) as f32)
    }

    fn mix_target(&self) -> f32 {
        (self.v(p::MIX) * 0.01) as f32
    }

    fn pre_target(&self) -> f32 {
        (self.v(p::PRE_DELAY) * 0.001) as f32 * self.sample_rate
    }

    fn cut(&self, shape: Shape, hz: f64) -> Coefs {
        let sr = self.sample_rate as f64;
        Coefs::new(shape, hz.min(sr * 0.45), 0.0, Q, sr)
    }

    /// Low cut; off at its minimum (20 Hz), so the default wet path is the plain IR.
    fn low_target(&self) -> Coefs {
        let c = self.cut(Shape::LowCut, self.v(p::LOW_CUT));
        if self.v(p::LOW_CUT) <= LOW_CUT_OFF {
            c.bypassed()
        } else {
            c
        }
    }

    /// High cut; off at its maximum (20 kHz).
    fn high_target(&self) -> Coefs {
        let c = self.cut(Shape::HighCut, self.v(p::HIGH_CUT));
        if self.v(p::HIGH_CUT) >= HIGH_CUT_OFF {
            c.bypassed()
        } else {
            c
        }
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(info) = super::param_info(id) else {
            return;
        };
        let v = if value.is_nan() {
            info.default
        } else {
            util::clamp(value, info.min, info.max)
        };
        self.values[id.0 as usize] = v;
        match id {
            p::WIDTH => {
                let t = self.width_target();
                set(&mut self.width, t, smooth);
            }
            p::GAIN => {
                let t = self.gain_target();
                set(&mut self.gain, t, smooth);
            }
            p::MIX => {
                let t = self.mix_target();
                set(&mut self.mix, t, smooth);
            }
            p::PRE_DELAY if !smooth => self.pre_cur = self.pre_target(),
            p::LOW_CUT if !smooth => self.low = self.low_target(),
            p::HIGH_CUT if !smooth => self.high = self.high_target(),
            p::DECAY | p::SIZE | p::REVERSE => {
                let s = self.shaping();
                if let Some(c) = &mut self.conv {
                    c.set_target(s);
                }
            }
            _ => {}
        }
    }

    /// RT. Take a swap: queue its convolver (replacing a queued one) and hand back the
    /// retired convolvers.
    fn take_swap(&mut self, swap: &mut IrSwap) {
        let next = swap.next.take();
        swap.replaced_input = self.input.take();
        if let Some(Some(old)) = self.pending.replace(next) {
            push_bounded(&mut swap.replaced, old);
        }
        for slot in &mut self.retired {
            if let Some(old) = slot.take() {
                push_bounded(&mut swap.replaced, old);
            }
        }
    }

    /// RT. Start the queued swap once its kernel matches the live shaping.
    fn start_pending(&mut self) {
        if self.swapping {
            return;
        }
        let target = self.shaping();
        match &mut self.pending {
            None => return,
            Some(Some(c)) => {
                c.set_target(target);
                if !c.prepare_idle(PENDING_UNITS) {
                    return;
                }
            }
            Some(None) => {}
        }
        let Some(next) = self.pending.take() else {
            return;
        };
        self.from_had_ir = self.conv.is_some();
        self.from = self.conv.take();
        self.conv = next;
        self.swapping = true;
        self.swap_pos = 0;
    }

    fn finish_swap(&mut self) {
        self.swapping = false;
        if let Some(old) = self.from.take() {
            match self.retired.iter_mut().find(|s| s.is_none()) {
                Some(slot) => *slot = Some(old),
                // Unreachable by construction (see `RETIRED`); never free on this thread.
                None => std::mem::forget(old),
            }
        }
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let mut a = start;
        while a < end {
            let b = end.min(a + self.max_block);
            self.render_chunk(audio, a, b);
            a = b;
        }
    }

    fn render_chunk(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let n = end - start;
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| {
            inputs
                .get(c)
                .or_else(|| inputs.first())
                .map_or(0.0, |ch| ch[i])
        };
        // Pre-delay (gliding) into the convolver input.
        let pre_t = self.pre_target();
        for i in 0..n {
            self.pre_cur = pre_t + (self.pre_cur - pre_t) * self.pre_coef;
            for c in 0..2 {
                self.pre[c].push(input(c, start + i));
                self.cin[c][i] = self.pre[c].read(self.pre_cur + 1.0);
            }
        }
        let [cl, cr] = &self.cin;
        match &mut self.conv {
            Some(conv) => {
                let [wl, wr] = &mut self.wet;
                conv.process([&cl[..n], &cr[..n]], [&mut wl[..n], &mut wr[..n]], n);
            }
            None => self.wet.iter_mut().for_each(|w| w[..n].fill(0.0)),
        }
        if self.swapping {
            match &mut self.from {
                Some(from) => {
                    let [wl, wr] = &mut self.wet_from;
                    from.process([&cl[..n], &cr[..n]], [&mut wl[..n], &mut wr[..n]], n);
                }
                None => self.wet_from.iter_mut().for_each(|w| w[..n].fill(0.0)),
            }
        }
        let (low_t, high_t) = (self.low_target(), self.high_target());
        let has_to = if self.conv.is_some() { 1.0 } else { 0.0 };
        let has_from = if self.from_had_ir { 1.0 } else { 0.0 };
        for i in 0..n {
            // IR swap: equal-power on the (uncorrelated) wet signals, linear on the IR
            // presence (dry level).
            let (mut wl, mut wr, presence) = if self.swapping {
                let g = (self.swap_pos as f32 / self.swap_len as f32).min(1.0);
                let (to, fr) = (
                    (g * std::f32::consts::FRAC_PI_2).sin(),
                    (g * std::f32::consts::FRAC_PI_2).cos(),
                );
                self.swap_pos += 1;
                (
                    self.wet[0][i] * to + self.wet_from[0][i] * fr,
                    self.wet[1][i] * to + self.wet_from[1][i] * fr,
                    has_to * g + has_from * (1.0 - g),
                )
            } else {
                (self.wet[0][i], self.wet[1][i], has_to)
            };
            if self.swapping && self.swap_pos >= self.swap_len {
                self.finish_swap();
            }
            self.low.glide(&low_t, self.filter_coef);
            self.high.glide(&high_t, self.filter_coef);
            let mut w = [wl as f64, wr as f64];
            for (c, x) in w.iter_mut().enumerate() {
                let y = self.svf[0][c].tick(*x, &self.low);
                *x = self.svf[1][c].tick(y, &self.high);
            }
            (wl, wr) = (w[0] as f32, w[1] as f32);
            let width = self.width.tick();
            let gain = self.gain.tick();
            let mix = self.mix.tick();
            let mid = (wl + wr) * 0.5;
            let side = (wl - wr) * 0.5 * width;
            let wet = [(mid + side) * gain, (mid - side) * gain];
            // No IR: pass the dry signal through.
            let dry_gain = presence * (1.0 - mix) + (1.0 - presence);
            for (c, ch) in audio.outputs.iter_mut().enumerate() {
                ch[start + i] = if c < 2 {
                    input(c, start + i) * dry_gain + wet[c] * mix
                } else {
                    0.0
                };
            }
        }
    }
}

fn set(s: &mut Smoother, v: f32, smooth: bool) {
    if smooth {
        s.set_target(v);
    } else {
        s.set_immediate(v);
    }
}

/// Push without growing past the reserved capacity (never allocates on the audio thread).
#[allow(clippy::vec_box)]
fn push_bounded(v: &mut Vec<Box<Convolver>>, c: Box<Convolver>) {
    if v.len() < v.capacity() {
        v.push(c);
    } else {
        std::mem::forget(c);
    }
}

impl Node for ConvolutionReverb {
    fn prepare(&mut self, config: &PrepareConfig) {
        let sr = config.sample_rate.max(1.0);
        let rate_changed = sr != self.sample_rate;
        self.sample_rate = sr;
        self.allocate(config.max_block_size.max(1));
        if rate_changed || self.built_rate != sr {
            // Rebuild at this rate from the newest IR (a queued swap supersedes the current).
            let input = match self.pending.take() {
                Some(next) => next.map(|c| c.input.clone()),
                None => self
                    .conv
                    .as_ref()
                    .map(|c| c.input.clone())
                    .or_else(|| self.input.clone()),
            };
            self.from = None;
            self.swapping = false;
            self.retired = [None, None];
            let shaping = self.shaping();
            self.conv = input.as_ref().and_then(|i| {
                IrBase::load(i, sr).map(|base| Box::new(Convolver::new(i.clone(), base, shaping)))
            });
            // Keep the input only while nothing holds it (unreadable media: retried later).
            self.input = if self.conv.is_some() { None } else { input };
            self.built_rate = sr;
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.pre.iter_mut().for_each(DelayLine::clear);
        self.svf = Default::default();
        if let Some(c) = &mut self.conv {
            c.reset();
        }
        if self.swapping {
            self.swap_pos = self.swap_len;
            self.finish_swap();
        }
        self.pre_cur = self.pre_target();
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.start_pending();
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }

    fn set_data(&mut self, data: NodeData) -> Option<NodeData> {
        let mut data = data;
        match data.downcast_mut::<IrSwap>() {
            Some(swap) => {
                self.take_swap(swap);
                Some(data)
            }
            None => Some(data),
        }
    }
}

impl Device for ConvolutionReverb {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(ether_core::protocol::model::BuiltinDeviceType::ConvolutionReverb)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
