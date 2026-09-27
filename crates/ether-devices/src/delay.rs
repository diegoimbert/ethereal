//! Delay: time (ms or synced), feedback, mix, ping-pong.
//!
//! Stereo (per-channel) feedback delay. In ping-pong mode the input is summed to mono and
//! fed to the left line only, and each line feeds back into the other, so the echoes
//! alternate left, right, left... In sync mode the time is a note division of the
//! transport tempo. Delay-time changes glide (fractional read with linear interpolation),
//! so automation doesn't click. The delay line is allocated in `prepare`.

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
    pub const SYNC: ParamId = ParamId(0);
    pub const TIME: ParamId = ParamId(1);
    pub const DIVISION: ParamId = ParamId(2);
    pub const FEEDBACK: ParamId = ParamId(3);
    pub const MIX: ParamId = ParamId(4);
    pub const PING_PONG: ParamId = ParamId(5);
}

const NUM_PARAMS: usize = 6;
/// Longest possible delay (synced times are clamped to it).
pub const MAX_DELAY_SECONDS: f32 = 5.0;
/// Glide time constant for delay-time changes.
const TIME_GLIDE_MS: f32 = 60.0;

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
    }
}

/// Declarative panel (v0.2, `device-ui`; drawn by the shared device renderer).
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{Widget, WidgetSize::*};
    layout(vec![
        section(
            "time",
            Some("Time"),
            2,
            3,
            vec![
                knob(params::TIME, Large),
                item(Widget::Toggle { param: params::SYNC }, Small),
                item(
                    Widget::Choice {
                        param: params::DIVISION,
                    },
                    Small,
                ),
            ],
        ),
        section(
            "delay",
            Some("Delay"),
            1,
            2,
            vec![
                knob(params::FEEDBACK, Large),
                item(
                    Widget::Toggle {
                        param: params::PING_PONG,
                    },
                    Small,
                ),
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
    snap_delay: bool,
    glide_coef: f32,
    feedback: Smoother,
    mix: Smoother,
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
            snap_delay: true,
            glide_coef: 0.0,
            feedback: Smoother::new(0.0, 20.0, 48_000.0),
            mix: Smoother::new(0.0, 20.0, 48_000.0),
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
        self.snap_delay = true;
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        let smoother = match id {
            params::FEEDBACK => &mut self.feedback,
            params::MIX => &mut self.mix,
            _ => {
                if !smooth {
                    self.snap_delay = true;
                }
                return;
            }
        };
        let v = self.values[id.0 as usize] as f32 * 0.01;
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
            (beats * 60.0 / bpm) as f32
        } else {
            self.value(params::TIME) * 0.001
        };
        let max = (self.lines[0].len() - 2) as f32;
        (seconds * self.sample_rate).clamp(1.0, max)
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, bpm: f64, start: usize, end: usize) {
        let target = self.target_delay(bpm);
        if self.snap_delay {
            self.delay = target;
            self.snap_delay = false;
        }
        let len = self.lines[0].len();
        let inputs = audio.inputs;
        let channels = audio.outputs.len().min(2);
        let ping_pong = self.value(params::PING_PONG) >= 0.5;
        for i in start..end {
            self.delay = target + (self.delay - target) * self.glide_coef;
            let fb = self.feedback.tick();
            let mix = self.mix.tick();
            // Read position `delay` samples behind the write head.
            let mut rp = self.write as f32 - self.delay;
            if rp < 0.0 {
                rp += len as f32;
            }
            let i0 = rp as usize % len;
            let i1 = (i0 + 1) % len;
            let frac = rp - rp.floor();
            let tap = |line: &[f32]| line[i0] + (line[i1] - line[i0]) * frac;
            let wet = [tap(&self.lines[0]), tap(&self.lines[1])];
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
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let bpm = ctx.transport.bpm;
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, bpm, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
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
