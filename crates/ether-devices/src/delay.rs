//! Delay: time (ms or synced), feedback, mix, ping-pong.
//!
//! Stereo (per-channel) feedback delay. In ping-pong mode the input is summed to mono and
//! fed to the left line only, and each line feeds back into the other, so the echoes
//! alternate left, right, left... In sync mode the time is a note division of the
//! transport tempo. Small delay-time changes glide (fractional read with linear
//! interpolation), so automation doesn't click; a jump of more than [`TIME_JUMP_MS`] (a knob
//! click, a new division) crossfades from the old tap to the new one over [`TIME_FADE_MS`]
//! instead (gliding that far would sweep the echoes' pitch by octaves). The delay
//! line is allocated in `prepare`.
//!
//! The effects pass appends (defaults = the sound before them, bit-exact):
//! - `Low Cut` / `High Cut` (6, 7; Hz, off at 20 Hz / 20 kHz): 12 dB/oct filters on what
//!   goes into the line, so each repeat is filtered again (darker / thinner echoes).
//! - `Mod Rate` / `Mod Depth` (8, 9): a sine LFO sweeping the delay time by up to
//!   ±[`MOD_MAX_MS`] (right channel a quarter cycle later): tape-style wobble and chorusing.
//! - `Freeze` (10, toggle): the line stops taking input and loops what it holds at unity
//!   feedback (the filters are bypassed while frozen); the dry signal still passes.

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::dsp::cut_filter::{CutFilter, HIGH_OFF_HZ, LOW_OFF_HZ};
use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const SYNC: ParamId = ParamId(0);
    pub const TIME: ParamId = ParamId(1);
    pub const DIVISION: ParamId = ParamId(2);
    pub const FEEDBACK: ParamId = ParamId(3);
    pub const MIX: ParamId = ParamId(4);
    pub const PING_PONG: ParamId = ParamId(5);
    /// Feedback-path low cut (Hz, 20 = off).
    pub const LOW_CUT: ParamId = ParamId(6);
    /// Feedback-path high cut (Hz, 20000 = off).
    pub const HIGH_CUT: ParamId = ParamId(7);
    /// Delay-time modulation rate (Hz).
    pub const MOD_RATE: ParamId = ParamId(8);
    /// Delay-time modulation depth (percent of ±[`super::MOD_MAX_MS`]).
    pub const MOD_DEPTH: ParamId = ParamId(9);
    /// Freeze (toggle).
    pub const FREEZE: ParamId = ParamId(10);
}

const NUM_PARAMS: usize = 11;
/// Largest delay-time modulation (ms either side) at 100 % `Mod Depth`.
pub const MOD_MAX_MS: f32 = 5.0;
/// Longest possible delay (synced times are clamped to it).
pub const MAX_DELAY_SECONDS: f32 = 5.0;
/// Glide time constant for delay-time changes.
const TIME_GLIDE_MS: f32 = 60.0;
/// Delay-time changes larger than this crossfade instead of gliding.
pub const TIME_JUMP_MS: f32 = 10.0;
/// Crossfade length of a delay-time jump.
pub const TIME_FADE_MS: f32 = 40.0;

/// Synced divisions: label and length in beats (quarter notes).
pub const DIVISIONS: [(&str, f64); 11] = [
    ("1/32", 0.125),
    ("1/16", 0.25),
    ("1/16 D", 0.375),
    ("1/8 T", 1.0 / 3.0),
    ("1/8", 0.5),
    ("1/8 D", 0.75),
    ("1/4 T", 2.0 / 3.0),
    ("1/4", 1.0),
    ("1/4 D", 1.5),
    ("1/2", 2.0),
    ("1/1", 4.0),
];

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::{choice, param};
    let labels: Vec<&str> = DIVISIONS.iter().map(|(l, _)| *l).collect();
    vec![
        choice(0, "Sync", "Time", ParamUnit::Toggle, &["Off", "On"], 0),
        param(
            1,
            "Time",
            "Time",
            ParamUnit::Milliseconds,
            (1.0, 2000.0, 375.0),
            ParamScale::Log,
        ),
        choice(2, "Division", "Time", ParamUnit::None, &labels, 4),
        param(
            3,
            "Feedback",
            "Delay",
            ParamUnit::Percent,
            (0.0, 95.0, 35.0),
            ParamScale::Linear,
        ),
        param(
            4,
            "Mix",
            "Delay",
            ParamUnit::Percent,
            (0.0, 100.0, 30.0),
            ParamScale::Linear,
        ),
        choice(
            5,
            "Ping-pong",
            "Delay",
            ParamUnit::Toggle,
            &["Off", "On"],
            0,
        ),
        // --- devices pass effects ---
        param(
            6,
            "Low Cut",
            "Filter",
            ParamUnit::Hertz,
            (LOW_OFF_HZ, 20_000.0, LOW_OFF_HZ),
            ParamScale::Log,
        ),
        param(
            7,
            "High Cut",
            "Filter",
            ParamUnit::Hertz,
            (LOW_OFF_HZ, HIGH_OFF_HZ, HIGH_OFF_HZ),
            ParamScale::Log,
        ),
        param(
            8,
            "Mod Rate",
            "Modulation",
            ParamUnit::Hertz,
            (0.05, 10.0, 0.5),
            ParamScale::Log,
        ),
        param(
            9,
            "Mod Depth",
            "Modulation",
            ParamUnit::Percent,
            (0.0, 100.0, 0.0),
            ParamScale::Linear,
        ),
        choice(10, "Freeze", "Delay", ParamUnit::Toggle, &["Off", "On"], 0),
    ]
}

/// Descriptor of the delay type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: Some(layout()),
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Delay,
        },
        name: "Delay".to_owned(),
        category: DeviceCategory::AudioEffect,
        params: param_infos(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
        implemented: true,
    }
}

/// Declarative panel (drawn by the shared renderer in the device's window): the echo
/// display (with its time / sync / division / feedback / ping-pong / freeze controls), then
/// the feedback filter, the modulation and the mix.
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{Widget, WidgetSize::*};
    layout(vec![
        section(
            "echoes",
            Some("Delay"),
            4,
            1,
            vec![item(
                Widget::DelayTaps {
                    time: params::TIME,
                    feedback: params::FEEDBACK,
                    sync: Some(params::SYNC),
                    division: Some(params::DIVISION),
                    ping_pong: Some(params::PING_PONG),
                    freeze: Some(params::FREEZE),
                },
                Large,
            )],
        ),
        section(
            "filter",
            Some("Filter"),
            1,
            2,
            vec![
                knob(params::LOW_CUT, Medium),
                knob(params::HIGH_CUT, Medium),
            ],
        ),
        section(
            "modulation",
            Some("Modulation"),
            1,
            2,
            vec![
                knob(params::MOD_RATE, Medium),
                knob(params::MOD_DEPTH, Medium),
            ],
        ),
        section(
            "output",
            Some("Output"),
            1,
            1,
            vec![knob(params::MIX, Large)],
        ),
    ])
}

/// The built-in delay.
#[derive(Debug)]
pub struct Delay {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    /// One delay line per channel (2), each `len` samples.
    lines: [Vec<f32>; 2],
    write: usize,
    /// Current (gliding) delay in samples.
    delay: f32,
    /// The target of the previous sub-block (a jump from it crossfades).
    last_target: f32,
    /// Crossfade of a time jump: the old tap's delay and its weight (1 -> 0).
    fade_delay: f32,
    fade: f32,
    snap_delay: bool,
    glide_coef: f32,
    feedback: Smoother,
    mix: Smoother,
    /// Feedback-path tone filters.
    filter: CutFilter,
    /// Modulation depth in samples (gliding) and the LFO phase (cycles).
    mod_depth: Smoother,
    mod_phase: f64,
    /// Freeze amount (0 = off, 1 = frozen; smoothed).
    freeze: Smoother,
    /// Silence reporting: an echo above the floor left the line this block.
    loud: bool,
    tail: util::Tail,
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Delay {
    /// Non-RT. A delay with default parameters (delay line sized for 48 kHz until
    /// `prepare`).
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let mut d = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            lines: [Vec::new(), Vec::new()],
            write: 0,
            delay: 0.0,
            last_target: 0.0,
            fade_delay: 0.0,
            fade: 0.0,
            snap_delay: true,
            glide_coef: 0.0,
            feedback: Smoother::new(0.0, 20.0, 48_000.0),
            mix: Smoother::new(0.0, 20.0, 48_000.0),
            filter: CutFilter::default(),
            mod_depth: Smoother::new(0.0, 20.0, 48_000.0),
            mod_phase: 0.0,
            freeze: Smoother::new(0.0, 20.0, 48_000.0),
            loud: false,
            tail: util::Tail::default(),
        };
        d.allocate();
        d
    }

    fn allocate(&mut self) {
        let len = (MAX_DELAY_SECONDS * self.sample_rate).ceil() as usize + 4;
        self.lines = [vec![0.0; len], vec![0.0; len]];
        self.write = 0;
        self.glide_coef = util::tau_coef(TIME_GLIDE_MS, self.sample_rate);
        self.feedback = Smoother::new(self.value(params::FEEDBACK) * 0.01, 20.0, self.sample_rate);
        self.mix = Smoother::new(self.value(params::MIX) * 0.01, 20.0, self.sample_rate);
        self.mod_depth = Smoother::new(self.mod_depth_target(), 20.0, self.sample_rate);
        self.freeze = Smoother::new(self.freeze_target(), 20.0, self.sample_rate);
        self.filter.prepare(self.sample_rate);
        self.update_filter(false);
        self.filter.reset();
        self.snap_delay = true;
    }

    fn mod_depth_target(&self) -> f32 {
        self.value(params::MOD_DEPTH) * 0.01 * MOD_MAX_MS * 0.001 * self.sample_rate
    }

    fn freeze_target(&self) -> f32 {
        if self.value(params::FREEZE) >= 0.5 {
            1.0
        } else {
            0.0
        }
    }

    fn update_filter(&mut self, smooth: bool) {
        self.filter.set(
            self.values[params::LOW_CUT.0 as usize],
            self.values[params::HIGH_CUT.0 as usize],
            smooth,
        );
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        let (smoother, v) = match id {
            params::FEEDBACK => {
                let v = self.value(id) * 0.01;
                (&mut self.feedback, v)
            }
            params::MIX => {
                let v = self.value(id) * 0.01;
                (&mut self.mix, v)
            }
            params::MOD_DEPTH => {
                let v = self.mod_depth_target();
                (&mut self.mod_depth, v)
            }
            params::FREEZE => {
                let v = self.freeze_target();
                (&mut self.freeze, v)
            }
            params::LOW_CUT | params::HIGH_CUT => {
                self.update_filter(smooth);
                return;
            }
            params::MOD_RATE => return,
            _ => {
                if !smooth {
                    self.snap_delay = true;
                }
                return;
            }
        };
        if smooth {
            smoother.set_target(v);
        } else {
            smoother.set_immediate(v);
        }
    }

    /// Target delay in samples for the current params and tempo.
    fn target_delay(&self, bpm: f64) -> f32 {
        let seconds = if self.value(params::SYNC) >= 0.5 {
            let bpm = if bpm > 0.0 { bpm } else { 120.0 };
            let (_, beats) =
                DIVISIONS[util::index(self.values[params::DIVISION.0 as usize], DIVISIONS.len())];
            beats * 60.0 / bpm
        } else {
            f64::from(self.value(params::TIME)) * 0.001
        };
        // In f64 so whole-sample times stay whole (0.6 s at 48 kHz is 28800, not the
        // 28800.002 of an f32 product); the read position is f64 too.
        let max = (self.lines[0].len() - 2) as f64;
        (seconds * f64::from(self.sample_rate)).clamp(1.0, max) as f32
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, bpm: f64, start: usize, end: usize) {
        let target = self.target_delay(bpm);
        if self.snap_delay {
            self.delay = target;
            self.fade = 0.0;
            self.snap_delay = false;
        } else if (target - self.last_target).abs() > TIME_JUMP_MS * 0.001 * self.sample_rate {
            // A jump: fade the current tap out (the louder one if a fade is running) and the
            // new one in, each at a fixed delay (no pitch sweep).
            if self.fade <= 0.5 {
                self.fade_delay = self.delay;
            }
            self.delay = target;
            self.fade = 1.0;
        }
        self.last_target = target;
        let plain = self.fade == 0.0
            && self.filter.is_bypass()
            && !self.mod_depth.is_smoothing()
            && self.mod_depth.current() == 0.0
            && !self.freeze.is_smoothing()
            && self.freeze.current() == 0.0;
        if !plain {
            self.render_extended(audio, target, start, end);
            return;
        }
        let len = self.lines[0].len();
        let inputs = audio.inputs;
        let channels = audio.outputs.len().min(2);
        let ping_pong = self.value(params::PING_PONG) >= 0.5;
        for i in start..end {
            self.delay = target + (self.delay - target) * self.glide_coef;
            let fb = self.feedback.tick();
            let mix = self.mix.tick();
            // Read position `delay` samples behind the write head, in f64: in f32 the
            // fraction would round to 1/32 sample at the far end of a 10 s line, so an echo
            // would depend on where the write head happens to be (it stops while the engine
            // skips the asleep device).
            let mut rp = self.write as f64 - f64::from(self.delay);
            if rp < 0.0 {
                rp += len as f64;
            }
            let i0 = rp as usize % len;
            let i1 = (i0 + 1) % len;
            let frac = (rp - rp.floor()) as f32;
            let tap = |line: &[f32]| line[i0] + (line[i1] - line[i0]) * frac;
            let wet = [tap(&self.lines[0]), tap(&self.lines[1])];
            self.loud |= wet[0].abs().max(wet[1].abs()) >= util::SILENCE_FLOOR;
            let dry = [
                inputs.first().map_or(0.0, |c| c[i]),
                inputs.get(1).map_or(0.0, |c| c[i]),
            ];
            // Line inputs: per channel, or (ping-pong, stereo) mono input into the left
            // line with the feedback crossing over.
            let fed = if ping_pong && channels == 2 {
                [(dry[0] + dry[1]) * 0.5 + wet[1] * fb, wet[0] * fb]
            } else {
                [dry[0] + wet[0] * fb, dry[1] + wet[1] * fb]
            };
            for ch in 0..channels {
                // Flush tiny values so the tail doesn't run on denormals.
                let f = fed[ch];
                self.lines[ch][self.write] = if f.abs() < 1e-20 { 0.0 } else { f };
                audio.outputs[ch][i] = dry[ch] * (1.0 - mix) + wet[ch] * mix;
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
            self.write += 1;
            if self.write == len {
                self.write = 0;
            }
        }
    }

    /// [`Self::render`] with the effects pass stages: feedback filters, time
    /// modulation and freeze (the plain loop above stays bit-identical to the device before
    /// them).
    fn render_extended(
        &mut self,
        audio: &mut AudioBuffers<'_, '_>,
        target: f32,
        start: usize,
        end: usize,
    ) {
        let len = self.lines[0].len();
        let inputs = audio.inputs;
        let channels = audio.outputs.len().min(2);
        let ping_pong = self.value(params::PING_PONG) >= 0.5;
        let rate = f64::from(self.value(params::MOD_RATE)) / f64::from(self.sample_rate);
        let max = (len - 2) as f64;
        let fade_step = 1.0 / (TIME_FADE_MS * 0.001 * self.sample_rate);
        // The LFO as a rotating phasor (one sin/cos per sub-block, perf): left = sin, right
        // = cos (a quarter cycle later).
        let (mut s, mut c) = (std::f64::consts::TAU * self.mod_phase).sin_cos();
        let (rs, rc) = (std::f64::consts::TAU * rate).sin_cos();
        for i in start..end {
            self.delay = target + (self.delay - target) * self.glide_coef;
            let fb = self.feedback.tick();
            let mix = self.mix.tick();
            let depth = f64::from(self.mod_depth.tick());
            let freeze = self.freeze.tick();
            self.filter.glide();
            (s, c) = (s * rc + c * rs, c * rc - s * rs);
            let lfo = [s * depth, c * depth];
            let mut wet = [0.0f32; 2];
            for (ch, w) in wet.iter_mut().enumerate() {
                let lfo = lfo[ch];
                let line = &self.lines[ch];
                let d = (f64::from(self.delay) + lfo).clamp(1.0, max);
                *w = read(line, self.write, d);
                if self.fade > 0.0 {
                    let old = read(
                        line,
                        self.write,
                        (f64::from(self.fade_delay) + lfo).clamp(1.0, max),
                    );
                    *w += (old - *w) * self.fade;
                }
            }
            self.fade = (self.fade - fade_step).max(0.0);
            self.loud |= wet[0].abs().max(wet[1].abs()) >= util::SILENCE_FLOOR;
            let dry = [
                inputs.first().map_or(0.0, |c| c[i]),
                inputs.get(1).map_or(0.0, |c| c[i]),
            ];
            // Frozen: no new input, unity feedback, no filtering (the loop holds).
            let take = 1.0 - freeze;
            let loop_fb = fb + (1.0 - fb) * freeze;
            let fed = if ping_pong && channels == 2 {
                [
                    (dry[0] + dry[1]) * 0.5 * take + wet[1] * loop_fb,
                    wet[0] * loop_fb,
                ]
            } else {
                [
                    dry[0] * take + wet[0] * loop_fb,
                    dry[1] * take + wet[1] * loop_fb,
                ]
            };
            for ch in 0..channels {
                let filtered = self.filter.tick(ch, fed[ch]);
                let f = filtered + (fed[ch] - filtered) * freeze;
                self.lines[ch][self.write] = if f.abs() < 1e-20 { 0.0 } else { f };
                audio.outputs[ch][i] = dry[ch] * (1.0 - mix) + wet[ch] * mix;
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
            self.write += 1;
            if self.write == len {
                self.write = 0;
            }
        }
        self.mod_phase = (self.mod_phase + rate * (end - start) as f64).fract();
    }
}

/// Linear-interpolated read `d` samples behind `write` (f64: see [`Delay::render`]).
#[inline]
fn read(line: &[f32], write: usize, d: f64) -> f32 {
    let len = line.len();
    let mut rp = write as f64 - d;
    if rp < 0.0 {
        rp += len as f64;
    }
    let i0 = rp as usize % len;
    let i1 = (i0 + 1) % len;
    let frac = (rp - rp.floor()) as f32;
    line[i0] + (line[i1] - line[i0]) * frac
}

impl Node for Delay {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.write = 0;
        self.snap_delay = true;
        self.filter.reset();
        self.mod_phase = 0.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let bpm = ctx.transport.bpm;
        self.loud = false;
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, bpm, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply_param(param, value, !step);
                }
            },
        );
        // Silent once the echoes (the wet tap, whatever the mix) stayed under the floor for
        // a whole delay time: the line then holds nothing audible.
        let n = ctx.frames;
        let silent_in = util::all_zero(audio.inputs, n);
        let quiet = silent_in
            && !self.loud
            && util::below_floor(audio.outputs.iter().map(|o| &o[..n.min(o.len())]));
        // A frozen loop never dies out; a modulated one keeps moving.
        let settled = (self.delay - self.target_delay(bpm)).abs() < 1e-3
            && !self.feedback.is_smoothing()
            && !self.mix.is_smoothing()
            && self.freeze.current() == 0.0
            && !self.freeze.is_smoothing()
            && !self.mod_depth.is_smoothing();
        let hold =
            self.delay.ceil() as usize + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail
            .status(!ctx.events.is_empty(), silent_in, quiet, settled, n, hold)
    }
}

impl Device for Delay {
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
