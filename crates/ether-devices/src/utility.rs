//! Built-in `Utility` device (roadmap v2).
//!
//! Gain, balance pan, stereo width, phase invert (L/R) and mono. Per sample:
//! 1. input channel mode (`Channel`: stereo, left or right on both sides, swapped),
//! 2. DC filter (5 Hz high-pass, when on),
//! 3. phase invert per channel,
//! 4. mid/side: `M = (L+R)/2`, `S = (L-R)/2 · width` (`Mono` forces width 0). In the
//!    `Mid/Side` width mode the width knob is replaced by `Mid/Side` balance: towards -100 %
//!    the side fades out (mid only), towards +100 % the mid fades out (side only). `Bass
//!    Mono` high-passes the side (12 dB/oct at `Bass Mono Freq`), so the lows are mono,
//! 5. balance pan: panning right attenuates the left channel linearly (and vice versa),
//!    centre is unity,
//! 6. gain (`Mute` = silence).
//!
//! Every stage is smoothed (≈20 ms), toggles included, so switching never clicks (the DC
//! filter switches at once: it only removes inaudible offset). With the appended params at
//! their defaults the output is bit-exact with the utility before them.
//!
//! Analysis (the window's meters): `AnalysisKind::Levels
//! [left peak dBFS, right peak dBFS]` of the output since the last read.
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | name      | range                    |
//! |----|-----------|--------------------------|
//! | 0  | `Gain`    | -36 ..= 36 dB            |
//! | 1  | `Pan`     | -1 ..= 1                 |
//! | 2  | `Width`   | 0 ..= 200 %              |
//! | 3  | `Invert L`| toggle                   |
//! | 4  | `Invert R`| toggle                   |
//! | 5  | `Mono`    | toggle                   |
//! | 6  | `Channel` | Stereo / Left / Right / Swap |
//! | 7  | `Width Mode` | Width / Mid/Side      |
//! | 8  | `Mid/Side`| -100 ..= 100 % (Mid/Side mode) |
//! | 9  | `Bass Mono` | toggle                 |
//! | 10 | `Bass Mono Freq` | 50 ..= 500 Hz (log), default 120 |
//! | 11 | `DC Filter` | toggle                 |
//! | 12 | `Mute`    | toggle                   |

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus, Smoother,
};

use crate::dsp::cut_filter::{CutFilter, HIGH_OFF_HZ};
use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const GAIN: ParamId = ParamId(0);
    pub const PAN: ParamId = ParamId(1);
    pub const WIDTH: ParamId = ParamId(2);
    pub const INVERT_L: ParamId = ParamId(3);
    pub const INVERT_R: ParamId = ParamId(4);
    pub const MONO: ParamId = ParamId(5);
    /// Input channel mode ([`super::CHANNEL_LABELS`]).
    pub const CHANNEL: ParamId = ParamId(6);
    /// Width / Mid/Side ([`super::WIDTH_MODE_LABELS`]).
    pub const WIDTH_MODE: ParamId = ParamId(7);
    /// Mid/side balance (percent, Mid/Side mode).
    pub const MID_SIDE: ParamId = ParamId(8);
    /// Mono below `BASS_MONO_FREQ` (toggle).
    pub const BASS_MONO: ParamId = ParamId(9);
    /// Bass mono crossover (Hz).
    pub const BASS_MONO_FREQ: ParamId = ParamId(10);
    /// DC filter (toggle).
    pub const DC_FILTER: ParamId = ParamId(11);
    /// Mute (toggle).
    pub const MUTE: ParamId = ParamId(12);
}

const NUM_PARAMS: usize = 13;
const SMOOTH_MS: f32 = 20.0;
/// Labels of `Channel` (indices stable).
pub const CHANNEL_LABELS: [&str; 4] = ["Stereo", "Left", "Right", "Swap"];
/// Labels of `Width Mode` (indices stable).
pub const WIDTH_MODE_LABELS: [&str; 2] = ["Width", "Mid/Side"];
/// DC filter corner (Hz).
const DC_HZ: f32 = 5.0;
/// Output level reported for silence (dBFS).
const METER_FLOOR_DB: f32 = -90.0;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::{choice, param};
    let toggle = |id, name, group| choice(id, name, group, ParamUnit::Toggle, &["Off", "On"], 0);
    vec![
        param(
            0,
            "Gain",
            "Utility",
            ParamUnit::Decibels,
            (-36.0, 36.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            1,
            "Pan",
            "Utility",
            ParamUnit::Pan,
            (-1.0, 1.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            2,
            "Width",
            "Stereo",
            ParamUnit::Percent,
            (0.0, 200.0, 100.0),
            ParamScale::Linear,
        ),
        toggle(3, "Invert L", "Phase"),
        toggle(4, "Invert R", "Phase"),
        toggle(5, "Mono", "Stereo"),
        // --- devices pass effects ---
        choice(6, "Channel", "Utility", ParamUnit::None, &CHANNEL_LABELS, 0),
        choice(
            7,
            "Width Mode",
            "Stereo",
            ParamUnit::None,
            &WIDTH_MODE_LABELS,
            0,
        ),
        param(
            8,
            "Mid/Side",
            "Stereo",
            ParamUnit::Percent,
            (-100.0, 100.0, 0.0),
            ParamScale::Linear,
        ),
        toggle(9, "Bass Mono", "Stereo"),
        param(
            10,
            "Bass Mono Freq",
            "Stereo",
            ParamUnit::Hertz,
            (50.0, 500.0, 120.0),
            ParamScale::Log,
        ),
        toggle(11, "DC Filter", "Utility"),
        toggle(12, "Mute", "Utility"),
    ]
}

/// Descriptor of the `Utility` type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: Some(layout()),
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Utility,
        },
        name: "Utility".to_owned(),
        category: DeviceCategory::AudioEffect,
        params: param_infos(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
        implemented: true,
    }
}

/// Declarative panel (drawn by the shared renderer in the device's window): gain / pan
/// with the output meters, the stereo field (width or mid/side, mono, bass mono), and the
/// channel / phase / DC / mute switches.
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{LayoutItem, Widget, WidgetSize::*};
    let toggle = |param| item(Widget::Toggle { param }, Small);
    let choice = |param| item(Widget::Choice { param }, Small);
    let meter = |index, label: &str| LayoutItem {
        label: Some(label.to_owned()),
        ..item(
            Widget::Meter {
                index,
                min_db: -60.0,
                max_db: 6.0,
            },
            Medium,
        )
    };
    layout(vec![
        section(
            "utility",
            Some("Utility"),
            2,
            2,
            vec![
                knob(params::GAIN, Large),
                knob(params::PAN, Large),
                toggle(params::MUTE),
                toggle(params::DC_FILTER),
            ],
        ),
        section(
            "meters",
            Some("Output"),
            1,
            2,
            vec![meter(0, "L"), meter(1, "R")],
        ),
        section(
            "stereo",
            Some("Stereo"),
            2,
            3,
            vec![
                LayoutItem {
                    colspan: 3,
                    ..choice(params::WIDTH_MODE)
                },
                knob(params::WIDTH, Medium),
                knob(params::MID_SIDE, Medium),
                toggle(params::MONO),
                toggle(params::BASS_MONO),
                knob(params::BASS_MONO_FREQ, Small),
            ],
        ),
        section(
            "channels",
            Some("Channels"),
            1,
            2,
            vec![
                LayoutItem {
                    colspan: 2,
                    ..choice(params::CHANNEL)
                },
                toggle(params::INVERT_L),
                toggle(params::INVERT_R),
            ],
        ),
    ])
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(Utility::new())
}

/// Smoothed per-sample targets.
#[derive(Debug)]
struct Smoothers {
    gain: Smoother,
    /// Left/right balance gains.
    bal: [Smoother; 2],
    width: Smoother,
    /// Polarity factors (+1 / -1).
    sign: [Smoother; 2],
    /// Mid gain (Mid/Side mode; 1 otherwise).
    mid: Smoother,
    /// Input matrix `[l←L, l←R, r←L, r←R]` (channel mode; identity = stereo).
    route: [Smoother; 4],
}

/// Number of smoothed targets ([`Utility::targets`]).
const TARGETS: usize = 11;
const IDENTITY: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// The built-in utility.
#[derive(Debug)]
pub struct Utility {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    s: Smoothers,
    /// Side high-pass of `Bass Mono` (channel 0 only).
    bass: CutFilter,
    /// DC blocker: pole and state per channel (previous input, previous output).
    dc_pole: f32,
    dc: [(f32, f32); 2],
    /// Output peaks (linear) since the last analysis read.
    meter: [f32; 2],
    /// Silence reporting.
    tail: util::Tail,
}

impl Default for Utility {
    fn default() -> Self {
        Self::new()
    }
}

impl Utility {
    /// Non-RT. A utility with default parameters.
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let mk = |v| Smoother::new(v, SMOOTH_MS, 48_000.0);
        let mut u = Self {
            tail: util::Tail::default(),
            values,
            ranges,
            sample_rate: 48_000.0,
            s: Smoothers {
                gain: mk(1.0),
                bal: [mk(1.0), mk(1.0)],
                width: mk(1.0),
                sign: [mk(1.0), mk(1.0)],
                mid: mk(1.0),
                route: IDENTITY.map(mk),
            },
            bass: CutFilter::default(),
            dc_pole: 0.0,
            dc: [(0.0, 0.0); 2],
            meter: [0.0; 2],
        };
        u.sync_all();
        u
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    fn on(&self, id: ParamId) -> bool {
        self.value(id) >= 0.5
    }

    /// Plain targets: gain, balance L/R, effective width (side gain), signs L/R, mid gain,
    /// input matrix.
    fn targets(&self) -> [f32; TARGETS] {
        let pan = self.value(params::PAN) as f32;
        let mid_side = util::index(self.value(params::WIDTH_MODE), WIDTH_MODE_LABELS.len()) == 1;
        let (mid, side) = if mid_side {
            let b = self.value(params::MID_SIDE) as f32 * 0.01;
            (1.0 - b.max(0.0), 1.0 + b.min(0.0))
        } else {
            (1.0, self.value(params::WIDTH) as f32 * 0.01)
        };
        let width = if self.on(params::MONO) { 0.0 } else { side };
        let sign = |id| if self.on(id) { -1.0 } else { 1.0 };
        let gain = if self.on(params::MUTE) {
            0.0
        } else {
            util::db_to_amp(self.value(params::GAIN) as f32)
        };
        let route = match util::index(self.value(params::CHANNEL), CHANNEL_LABELS.len()) {
            1 => [1.0, 0.0, 1.0, 0.0],
            2 => [0.0, 1.0, 0.0, 1.0],
            3 => [0.0, 1.0, 1.0, 0.0],
            _ => IDENTITY,
        };
        [
            gain,
            1.0 - pan.max(0.0),
            1.0 + pan.min(0.0),
            width,
            sign(params::INVERT_L),
            sign(params::INVERT_R),
            mid,
            route[0],
            route[1],
            route[2],
            route[3],
        ]
    }

    fn smoothers(&mut self) -> [&mut Smoother; TARGETS] {
        let s = &mut self.s;
        let [bl, br] = &mut s.bal;
        let [sl, sr] = &mut s.sign;
        let [r0, r1, r2, r3] = &mut s.route;
        [
            &mut s.gain,
            bl,
            br,
            &mut s.width,
            sl,
            sr,
            &mut s.mid,
            r0,
            r1,
            r2,
            r3,
        ]
    }

    fn sync_all(&mut self) {
        let targets = self.targets();
        let sr = self.sample_rate;
        for (s, t) in self.smoothers().into_iter().zip(targets) {
            *s = Smoother::new(t, SMOOTH_MS, sr);
        }
        self.bass.prepare(self.sample_rate);
        self.update_bass(false);
        self.dc_pole = 1.0 - std::f32::consts::TAU * DC_HZ / self.sample_rate;
    }

    fn update_bass(&mut self, smooth: bool) {
        let low = if self.on(params::BASS_MONO) {
            self.value(params::BASS_MONO_FREQ)
        } else {
            0.0
        };
        self.bass.set(low, HIGH_OFF_HZ, smooth);
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        if matches!(id, params::BASS_MONO | params::BASS_MONO_FREQ) {
            self.update_bass(smooth);
        }
        let targets = self.targets();
        for (s, t) in self.smoothers().into_iter().zip(targets) {
            if !smooth {
                s.set_immediate(t);
            } else if s.target() != t {
                s.set_target(t);
            }
        }
    }

    /// The effects pass stages are idle (channel mode stereo, no DC filter, no
    /// bass mono): the plain loop applies.
    fn plain(&self) -> bool {
        let route = &self.s.route;
        !self.on(params::DC_FILTER)
            && self.bass.is_bypass()
            && route
                .iter()
                .zip(IDENTITY)
                .all(|(s, v)| !s.is_smoothing() && s.current() == v)
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| inputs.get(c).map_or(0.0, |ch| ch[i]);
        let mono_in = inputs.len() == 1;
        let plain = self.plain();
        let dc = self.on(params::DC_FILTER);
        for i in start..end {
            let mut l = input(0, i);
            let mut r = if mono_in { input(0, i) } else { input(1, i) };
            if !plain {
                let [a, b, c, d] = &mut self.s.route;
                let (ll, rr) = (l, r);
                l = ll * a.tick() + rr * b.tick();
                r = ll * c.tick() + rr * d.tick();
                if dc {
                    for (x, (x1, y1)) in [&mut l, &mut r].into_iter().zip(&mut self.dc) {
                        let y = *x - *x1 + self.dc_pole * *y1;
                        *x1 = *x;
                        *y1 = crate::dsp::flush32(y);
                        *x = y;
                    }
                }
            }
            let l = l * self.s.sign[0].tick();
            let r = r * self.s.sign[1].tick();
            let mid = (l + r) * 0.5 * self.s.mid.tick();
            let mut side = (l - r) * 0.5 * self.s.width.tick();
            if !plain {
                self.bass.glide();
                side = self.bass.tick(0, side);
            }
            let gain = self.s.gain.tick();
            let out_l = (mid + side) * self.s.bal[0].tick() * gain;
            let out_r = (mid - side) * self.s.bal[1].tick() * gain;
            self.meter[0] = self.meter[0].max(out_l.abs());
            self.meter[1] = self.meter[1].max(out_r.abs());
            match audio.outputs {
                [ol, or, rest @ ..] => {
                    ol[i] = out_l;
                    or[i] = out_r;
                    rest.iter_mut().for_each(|o| o[i] = 0.0);
                }
                [o] => o[i] = (out_l + out_r) * 0.5,
                [] => {}
            }
        }
    }
}

impl Node for Utility {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        self.bass.reset();
        self.dc = [(0.0, 0.0); 2];
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply_param(param, value, !step);
                }
            },
        );
        // Silent in, silent out once no ramp moves (the DC filter and the bass-mono
        // high-pass ring out under the floor within the hold).
        let settled = !self.s.gain.is_smoothing()
            && !self.s.width.is_smoothing()
            && !self.s.mid.is_smoothing()
            && !self
                .s
                .bal
                .iter()
                .chain(&self.s.sign)
                .chain(&self.s.route)
                .any(Smoother::is_smoothing);
        let hold = if self.plain() {
            1
        } else {
            util::samples(util::BASE_HOLD_SECONDS, self.sample_rate)
        };
        self.tail.status_of(ctx, audio, settled, hold)
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        if let Some(f) = out.frame(AnalysisKind::Levels) {
            for m in self.meter {
                f.push(util::amp_to_db(m).max(METER_FLOOR_DB));
            }
        }
        self.meter = [0.0; 2];
    }
}

impl Device for Utility {
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
