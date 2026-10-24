//! Built-in `Maximizer`: a mastering loudness maximizer. A lookahead true-peak brickwall
//! limiter driven by a threshold (the signal is raised by `-Threshold` and held under the
//! `Ceiling`), with a program-dependent release, a variable attack inside a fixed lookahead,
//! a stereo link, an optional soft clipper ahead of it and a level-matched monitor.
//!
//! Signal path, per frame:
//! 1. the input times `-Threshold` dB (smoothed), then the soft clipper (`Clip` > 0): a
//!    smooth knee that rounds the peaks above `ceiling + 6 dB · (1 - clip)` towards it,
//!    so the fastest transients cost no gain reduction;
//! 2. per channel, the true peak ([`TruePeak`], 4x oversampled below 96 kHz) of that
//!    signal, and the gain each channel needs: `r = min(1, ceiling / peak)`. The stereo
//!    link blends each channel's gain with the linked one (the lower of the two) in dB;
//! 3. per channel, the gain path of a lookahead limiter: the required gain delayed by
//!    `lookahead - attack`, its minimum over the lookahead window, a release envelope
//!    (instant down, exponential up; with `Auto Release` it also never rises above a slow
//!    average of itself, so sustained reduction recovers slowly and lone peaks quickly),
//!    then a moving average over `attack` samples, which ramps the gain down over the
//!    attack time and stays at or under the gain every sample of the window needs;
//! 4. the audio delayed by the lookahead (plus the true-peak detector's delay) times the
//!    gain, clamped at the (delayed) ceiling against rounding and parameter moves, then
//!    with `Match Level` divided by the threshold gain (to hear the limiting at the input's
//!    loudness).
//!
//! The latency is fixed ([`LOOKAHEAD_MS`] plus [`TruePeak::delay`]); `Attack` only moves
//! where the gain ramp starts inside it (the rest of the lookahead holds the reduction
//! after the peak). Sample peaks never pass the ceiling; true peaks stay within a fraction
//! of a dB of it (the tests).
//!
//! Analysis (the panel's history and meters): `AnalysisKind::Levels`, see [`Frame`].
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | name           | range                         |
//! |----|----------------|-------------------------------|
//! | 0  | `Threshold`    | -30 ..= 0 dB, default 0       |
//! | 1  | `Ceiling`      | -12 ..= 0 dB, default -1      |
//! | 2  | `Release`      | 1 ..= 1000 ms (log), 60       |
//! | 3  | `Auto Release` | toggle, default on            |
//! | 4  | `Attack`       | 0.1 ..= 5 ms (log), default 2 |
//! | 5  | `Stereo Link`  | 0 ..= 100 %, default 100      |
//! | 6  | `Clip`         | 0 ..= 100 %, default 0        |
//! | 7  | `Match Level`  | toggle, default off           |

use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::{DeviceLayout, LayoutItem, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus, Smoother,
};

use crate::contract::{
    FactoryPreset, descriptor as build, item, knob, layout, param, section, toggle,
};
use crate::dsp::delay_line::DelayLine;
use crate::dsp::sliding_min::SlidingMin;
use crate::dsp::true_peak::TruePeak;
use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const THRESHOLD: ParamId = ParamId(0);
    pub const CEILING: ParamId = ParamId(1);
    pub const RELEASE: ParamId = ParamId(2);
    pub const AUTO_RELEASE: ParamId = ParamId(3);
    pub const ATTACK: ParamId = ParamId(4);
    pub const STEREO_LINK: ParamId = ParamId(5);
    pub const CLIP: ParamId = ParamId(6);
    pub const MATCH_LEVEL: ParamId = ParamId(7);
    /// Number of params.
    pub const COUNT: usize = 8;
}
use params as p;

/// The lookahead (ms); the latency is this plus the true-peak detector's delay.
pub const LOOKAHEAD_MS: f32 = 5.0;
/// Slow average of the auto release, in multiples of `Release`.
const AUTO_RELEASE_SLOW: f32 = 6.0;
/// The soft clipper's range above the ceiling at `Clip` 0+ (dB).
const CLIP_RANGE_DB: f32 = 6.0;
/// History: one point per this many milliseconds, [`HISTORY`] points (4 s).
pub const HISTORY_STEP_MS: f32 = 20.0;
pub const HISTORY: usize = 200;
/// Level reported for silence (dB).
const FLOOR_DB: f32 = -90.0;

/// Layout of the `Levels` frame (indices into its values).
pub struct Frame;

impl Frame {
    /// Deepest gain reduction since the previous frame (dB, <= 0; also what a `Meter`
    /// widget at index 0 shows).
    pub const GAIN_REDUCTION: usize = 0;
    /// Loudest input true peak since the previous frame, before the threshold gain (dBFS;
    /// after the clipper, scaled back).
    pub const INPUT: usize = 1;
    /// Loudest output true peak since the previous frame (dBTP).
    pub const OUTPUT: usize = 2;
    /// Loudest output true peak since the reset (dBTP).
    pub const OUTPUT_MAX: usize = 3;
    /// Deepest gain reduction since the reset (dB, <= 0).
    pub const GAIN_REDUCTION_MAX: usize = 4;
    /// Seconds per history point.
    pub const STEP: usize = 5;
    /// `n`, the history points that follow: `n` input levels, `n` output levels, `n` gain
    /// reductions (each oldest first, the last one now).
    pub const POINTS: usize = 6;
    pub const HISTORY: usize = 7;
}

/// Descriptor of the maximizer.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(ty, BuiltinDeviceType::Maximizer);
    let mut d = build(
        BuiltinDeviceType::Maximizer,
        "Maximizer",
        DeviceCategory::AudioEffect,
        vec![
            param(
                0,
                "Threshold",
                "Maximizer",
                ParamUnit::Decibels,
                (-30.0, 0.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                1,
                "Ceiling",
                "Maximizer",
                ParamUnit::Decibels,
                (-12.0, 0.0, -1.0),
                ParamScale::Linear,
            ),
            param(
                2,
                "Release",
                "Release",
                ParamUnit::Milliseconds,
                (1.0, 1000.0, 60.0),
                ParamScale::Log,
            ),
            toggle(3, "Auto Release", "Release", true),
            param(
                4,
                "Attack",
                "Maximizer",
                ParamUnit::Milliseconds,
                (0.1, f64::from(LOOKAHEAD_MS), 2.0),
                ParamScale::Log,
            ),
            param(
                5,
                "Stereo Link",
                "Character",
                ParamUnit::Percent,
                (0.0, 100.0, 100.0),
                ParamScale::Linear,
            ),
            param(
                6,
                "Clip",
                "Character",
                ParamUnit::Percent,
                (0.0, 100.0, 0.0),
                ParamScale::Linear,
            ),
            toggle(7, "Match Level", "Output", false),
        ],
        2,
        2,
        false,
        0,
    );
    d.layout = Some(panel());
    d
}

/// The history (input, output, gain reduction) with the gain-reduction meter beside it,
/// then the knobs.
fn panel() -> DeviceLayout {
    use WidgetSize::*;
    layout(vec![
        section(
            "maximizer",
            None,
            4,
            4,
            vec![
                LayoutItem {
                    colspan: 3,
                    ..item(
                        Widget::Maximizer {
                            threshold: p::THRESHOLD,
                            ceiling: p::CEILING,
                        },
                        Large,
                    )
                },
                LayoutItem {
                    label: Some("GR".to_owned()),
                    ..item(
                        Widget::Meter {
                            index: 0,
                            min_db: -24.0,
                            max_db: 0.0,
                        },
                        Medium,
                    )
                },
            ],
        ),
        section(
            "controls",
            Some("Maximizer"),
            2,
            4,
            vec![
                knob(p::THRESHOLD, Large),
                knob(p::CEILING, Medium),
                knob(p::ATTACK, Small),
                knob(p::RELEASE, Small),
                item(
                    Widget::Toggle {
                        param: p::AUTO_RELEASE,
                    },
                    Small,
                ),
                knob(p::STEREO_LINK, Small),
                knob(p::CLIP, Small),
                item(
                    Widget::Toggle {
                        param: p::MATCH_LEVEL,
                    },
                    Small,
                ),
            ],
        ),
    ])
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    assert_eq!(device.device_type(), BuiltinDeviceType::Maximizer);
    Box::new(Maximizer::new())
}

/// `FactoryPreset` entries for `presets/maximizer/<slug>.etherpreset` files.
macro_rules! presets {
    ($($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!("maximizer/", $slug),
            json: include_str!(concat!("../presets/maximizer/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::Maximizer => presets!(
            "transparent-master",
            "streaming-safe",
            "loud-and-dense",
            "punchy-drums",
            "wide-and-free"
        ),
        _ => &[],
    }
}

/// One channel's gain path (step 3 of the module docs).
#[derive(Debug, Default)]
struct GainPath {
    /// The required gain, delayed by `lookahead - attack`.
    delay: DelayLine,
    min: SlidingMin,
    /// Release envelope (instant attack) and its slow average (auto release).
    env: f64,
    slow: f64,
    /// The last `lookahead + 1` envelope values and the running sum of the newest
    /// `attack + 1` of them.
    ring: Vec<f32>,
    at: usize,
    sum: f64,
    gain: f32,
}

impl GainPath {
    fn new(lookahead: usize) -> Self {
        let mut g = Self {
            delay: DelayLine::new(lookahead.max(1)),
            min: SlidingMin::new(lookahead + 1),
            ring: vec![1.0; lookahead + 1],
            ..Self::default()
        };
        g.clear(1);
        g
    }

    fn clear(&mut self, window: usize) {
        self.delay.clear();
        for _ in 0..=self.delay.max_delay() {
            self.delay.push(1.0);
        }
        self.min.clear();
        self.env = 1.0;
        self.slow = 1.0;
        self.ring.fill(1.0);
        self.at = 0;
        self.sum = window as f64;
        self.gain = 1.0;
    }

    /// The running sum of the newest `window` envelope values, recomputed.
    fn resum(&mut self, window: usize) {
        let n = self.ring.len();
        self.sum = (1..=window)
            .map(|k| f64::from(self.ring[(self.at + n - k) % n]))
            .sum();
    }

    /// RT. One required gain in; the gain to apply to the sample leaving the lookahead.
    #[inline]
    fn step(&mut self, required: f32, s: &Shape) -> f32 {
        let r = if s.hold == 0 {
            required
        } else {
            self.delay.tap(s.hold)
        };
        self.delay.push(required);
        let m = f64::from(self.min.push(r));
        self.env = if m < self.env || m - self.env < 1e-6 {
            m
        } else {
            m - (m - self.env) * s.release
        };
        // Auto release: a slow average follows the envelope both ways and the applied
        // envelope is the lower of the two, so a drop is caught at once and the recovery
        // is fast back to the average reduction, then slow.
        // It follows reductions at `slow` and recoveries twice as fast.
        let c = if self.env < self.slow {
            s.slow
        } else {
            s.slow_up
        };
        self.slow += (self.env - self.slow) * (1.0 - c);
        if self.env >= 1.0 && self.slow > 1.0 - 1e-3 {
            // Recovered to within 0.01 dB: settled (else it creeps to unity for seconds).
            self.slow = self.env;
        }
        let env = if s.auto {
            self.env.min(self.slow)
        } else {
            self.env
        };
        let n = self.ring.len();
        let e = env as f32;
        let leaving = self.ring[(self.at + n - s.window) % n];
        self.ring[self.at] = e;
        self.sum += f64::from(e) - f64::from(leaving);
        self.at += 1;
        if self.at == n {
            self.at = 0;
            self.resum(s.window);
        }
        self.gain = ((self.sum / s.window as f64) as f32).min(1.0);
        self.gain
    }

    fn settled(&self) -> bool {
        (1.0 - self.env).abs() < 1e-6 && self.slow == self.env && (1.0 - self.gain).abs() < 1e-6
    }
}

/// The gain paths' shared settings (per sample rate and params).
#[derive(Clone, Copy, Debug)]
struct Shape {
    /// Delay of the required gain before the window (`lookahead - attack`).
    hold: usize,
    /// Samples of the moving average (`attack + 1`).
    window: usize,
    /// Release and slow-average coefficients (per sample).
    release: f64,
    slow: f64,
    slow_up: f64,
    auto: bool,
}

/// The built-in maximizer.
pub struct Maximizer {
    values: [f64; p::COUNT],
    ranges: [(f64, f64); p::COUNT],
    sample_rate: f32,
    lookahead: usize,
    shape: Shape,
    drive: Smoother,
    ceiling: Smoother,
    link: Smoother,
    clip: Smoother,
    detect: [TruePeak; 2],
    out_peak: [TruePeak; 2],
    /// The previous detector value of each channel (a value covers the samples on both
    /// sides of it).
    prev: [f32; 2],
    paths: [GainPath; 2],
    /// Audio, ceiling and threshold gain travelling with it.
    audio: [DelayLine; 2],
    ceiling_line: DelayLine,
    drive_line: DelayLine,
    // Meters: since the previous analysis read.
    meter_gain: f32,
    meter_in: f32,
    meter_out: f32,
    // Since the reset.
    max_out: f32,
    max_gain: f32,
    // History (rings of `HISTORY` points; `len` valid).
    hist_in: Vec<f32>,
    hist_out: Vec<f32>,
    hist_gr: Vec<f32>,
    hist_at: usize,
    hist_len: usize,
    point_len: usize,
    point_pos: usize,
    point_in: f32,
    point_out: f32,
    point_gain: f32,
    was_playing: bool,
    tail: util::Tail,
}

impl Default for Maximizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Maximizer {
    /// Non-RT. A maximizer with default params (sized for 48 kHz until `prepare`).
    pub fn new() -> Self {
        let d = descriptor(BuiltinDeviceType::Maximizer);
        let mut values = [0.0; p::COUNT];
        let mut ranges = [(0.0, 0.0); p::COUNT];
        for info in &d.params {
            values[info.id.0 as usize] = info.default;
            ranges[info.id.0 as usize] = (info.min, info.max);
        }
        let mut m = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            lookahead: 1,
            shape: Shape {
                hold: 0,
                window: 1,
                release: 0.0,
                slow: 0.0,
                slow_up: 0.0,
                auto: true,
            },
            drive: Smoother::new(1.0, 20.0, 48_000.0),
            ceiling: Smoother::new(1.0, 20.0, 48_000.0),
            link: Smoother::new(1.0, 20.0, 48_000.0),
            clip: Smoother::new(0.0, 20.0, 48_000.0),
            detect: [TruePeak::new(48_000.0), TruePeak::new(48_000.0)],
            out_peak: [TruePeak::new(48_000.0), TruePeak::new(48_000.0)],
            prev: [0.0; 2],
            paths: [GainPath::default(), GainPath::default()],
            audio: [DelayLine::default(), DelayLine::default()],
            ceiling_line: DelayLine::default(),
            drive_line: DelayLine::default(),
            meter_gain: 1.0,
            meter_in: 0.0,
            meter_out: 0.0,
            max_out: 0.0,
            max_gain: 1.0,
            hist_in: vec![FLOOR_DB; HISTORY],
            hist_out: vec![FLOOR_DB; HISTORY],
            hist_gr: vec![0.0; HISTORY],
            hist_at: 0,
            hist_len: 0,
            point_len: 960,
            point_pos: 0,
            point_in: 0.0,
            point_out: 0.0,
            point_gain: 1.0,
            was_playing: false,
            tail: util::Tail::default(),
        };
        m.allocate();
        m
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    /// The audio's delay (the latency).
    fn delay(&self) -> usize {
        self.lookahead + self.detect[0].delay()
    }

    fn allocate(&mut self) {
        let sr = self.sample_rate;
        self.lookahead = ((LOOKAHEAD_MS * 0.001 * sr).round() as usize).max(1);
        self.detect = [TruePeak::new(sr), TruePeak::new(sr)];
        self.out_peak = [TruePeak::new(sr), TruePeak::new(sr)];
        self.paths = [GainPath::new(self.lookahead), GainPath::new(self.lookahead)];
        let d = self.delay();
        self.audio = [DelayLine::new(d + 1), DelayLine::new(d + 1)];
        self.ceiling_line = DelayLine::new(d + 1);
        self.drive_line = DelayLine::new(d + 1);
        self.drive = Smoother::new(self.drive_target(), 20.0, sr);
        self.ceiling = Smoother::new(util::db_to_amp(self.value(p::CEILING)), 20.0, sr);
        self.link = Smoother::new(self.value(p::STEREO_LINK) * 0.01, 20.0, sr);
        self.clip = Smoother::new(self.value(p::CLIP) * 0.01, 20.0, sr);
        self.point_len = ((HISTORY_STEP_MS * 0.001 * sr).round() as usize).max(1);
        self.update_shape();
        self.reset();
        self.clear_meters();
    }

    fn drive_target(&self) -> f32 {
        util::db_to_amp(-self.value(p::THRESHOLD))
    }

    fn update_shape(&mut self) {
        let sr = self.sample_rate;
        let attack =
            ((self.value(p::ATTACK) * 0.001 * sr).round() as usize).clamp(0, self.lookahead);
        let release = self.value(p::RELEASE);
        let window = attack + 1;
        let changed = window != self.shape.window;
        self.shape = Shape {
            hold: self.lookahead - attack,
            window,
            release: f64::from(util::tau_coef(release, sr)),
            slow: f64::from(util::tau_coef(release * AUTO_RELEASE_SLOW, sr)),
            slow_up: f64::from(util::tau_coef(release * AUTO_RELEASE_SLOW * 0.5, sr)),
            auto: self.value(p::AUTO_RELEASE) >= 0.5,
        };
        if changed {
            for g in &mut self.paths {
                g.resum(window);
            }
        }
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        let v = self.value(id);
        let (smoother, target) = match id {
            p::THRESHOLD => (&mut self.drive, util::db_to_amp(-v)),
            p::CEILING => (&mut self.ceiling, util::db_to_amp(v)),
            p::STEREO_LINK => (&mut self.link, v * 0.01),
            p::CLIP => (&mut self.clip, v * 0.01),
            _ => {
                self.update_shape();
                return;
            }
        };
        if smooth {
            smoother.set_target(target);
        } else {
            smoother.set_immediate(target);
        }
    }

    /// Current gain reduction (dB, >= 0): the deeper of the two channels.
    pub fn gain_reduction(&self) -> f32 {
        let g = self.paths[0].gain.min(self.paths[1].gain);
        -util::amp_to_db(g).min(0.0)
    }

    /// Loudest output true peak since the reset (dBTP).
    pub fn output_true_peak_max(&self) -> f32 {
        util::amp_to_db(self.max_out).max(FLOOR_DB)
    }

    fn clear_meters(&mut self) {
        self.meter_gain = 1.0;
        self.meter_in = 0.0;
        self.meter_out = 0.0;
        self.max_out = 0.0;
        self.max_gain = 1.0;
        self.hist_at = 0;
        self.hist_len = 0;
        self.point_pos = 0;
        self.point_in = 0.0;
        self.point_out = 0.0;
        self.point_gain = 1.0;
    }

    /// RT. One frame through the soft clipper.
    #[inline]
    fn soft_clip(x: f32, ceiling: f32, clip: f32) -> f32 {
        if clip <= 0.0 {
            return x;
        }
        // Knee from half the clip level up to it: linear below, a tanh-shaped approach
        // to `level` above (continuous slope).
        let level = ceiling * util::db_to_amp(CLIP_RANGE_DB * (1.0 - clip));
        let knee = level * 0.5;
        let a = x.abs();
        if a <= knee {
            return x;
        }
        let span = level - knee;
        let y = knee + span * fast_tanh((a - knee) / span);
        y.copysign(x)
    }

    #[inline]
    fn frame(&mut self, x: [f32; 2]) -> [f32; 2] {
        let drive = self.drive.tick();
        let ceiling = self.ceiling.tick();
        let link = self.link.tick();
        let clip = self.clip.tick();
        let mut req = [1.0f32; 2];
        let mut driven = [0.0f32; 2];
        for c in 0..2 {
            let v = Self::soft_clip(x[c] * drive, ceiling, clip);
            driven[c] = v;
            let tp = self.detect[c].push(v);
            let peak = tp.max(self.prev[c]);
            self.prev[c] = tp;
            // The meters show the input before the threshold gain.
            self.meter_in = self.meter_in.max(peak / drive.max(1e-6));
            req[c] = if peak > ceiling { ceiling / peak } else { 1.0 };
        }
        let linked = req[0].min(req[1]);
        let mut gain = [1.0f32; 2];
        for c in 0..2 {
            // Blend in dB: `linked^link · own^(1 - link)` (between the two: never above
            // the channel's own need).
            let r = if link >= 1.0 || req[c] == linked {
                linked
            } else if link <= 0.0 {
                req[c]
            } else {
                linked.powf(link) * req[c].powf(1.0 - link)
            };
            gain[c] = self.paths[c].step(r, &self.shape);
        }
        let d = self.delay();
        let ceil_d = self.ceiling_line.tap(d);
        let drive_d = self.drive_line.tap(d);
        self.ceiling_line.push(ceiling);
        self.drive_line.push(drive);
        let matched = if self.value(p::MATCH_LEVEL) >= 0.5 {
            1.0 / drive_d.max(1e-6)
        } else {
            1.0
        };
        let mut out = [0.0f32; 2];
        let mut out_peak = 0.0f32;
        for c in 0..2 {
            let delayed = self.audio[c].tap(d);
            self.audio[c].push(driven[c]);
            let y = (delayed * gain[c]).clamp(-ceil_d, ceil_d);
            out_peak = out_peak.max(self.out_peak[c].push(y));
            out[c] = y * matched;
        }
        let g = gain[0].min(gain[1]);
        self.meter_gain = self.meter_gain.min(g);
        self.meter_out = self.meter_out.max(out_peak);
        self.max_out = self.max_out.max(out_peak);
        self.max_gain = self.max_gain.min(g);
        self.point_in = self
            .point_in
            .max(self.prev[0].max(self.prev[1]) / drive.max(1e-6));
        self.point_out = self.point_out.max(out_peak);
        self.point_gain = self.point_gain.min(g);
        self.point_pos += 1;
        if self.point_pos >= self.point_len {
            self.hist_in[self.hist_at] = util::amp_to_db(self.point_in).max(FLOOR_DB);
            self.hist_out[self.hist_at] = util::amp_to_db(self.point_out).max(FLOOR_DB);
            self.hist_gr[self.hist_at] = util::amp_to_db(self.point_gain).min(0.0);
            self.hist_at = (self.hist_at + 1) % HISTORY;
            self.hist_len = (self.hist_len + 1).min(HISTORY);
            self.point_pos = 0;
            self.point_in = 0.0;
            self.point_out = 0.0;
            self.point_gain = 1.0;
        }
        out
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| {
            inputs
                .get(c)
                .or_else(|| inputs.first())
                .map_or(0.0, |ch| if ch[i].is_finite() { ch[i] } else { 0.0 })
        };
        for i in start..end {
            let y = self.frame([input(0, i), input(1, i)]);
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = if c < 2 { y[c] } else { 0.0 };
            }
        }
    }
}

/// `tanh` to about 1e-4 (a Padé approximant, clamped), for the clipper's knee.
#[inline]
fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    (x * (27.0 + x2) / (27.0 + 9.0 * x2)).clamp(-1.0, 1.0)
}

impl Node for Maximizer {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
    }

    fn reset(&mut self) {
        for t in self.detect.iter_mut().chain(self.out_peak.iter_mut()) {
            t.reset();
        }
        self.prev = [0.0; 2];
        let window = self.shape.window;
        for g in &mut self.paths {
            g.clear(window);
        }
        for line in &mut self.audio {
            line.clear();
        }
        // The delayed ceiling and drive start at their current values.
        self.ceiling_line.clear();
        self.drive_line.clear();
        let (c, g) = (self.ceiling.current(), self.drive.current());
        for _ in 0..=self.delay() + 1 {
            self.ceiling_line.push(c);
            self.drive_line.push(g);
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let playing = ctx.transport.playing;
        if playing && !self.was_playing {
            self.clear_meters();
        }
        self.was_playing = playing;
        let frames = ctx.frames;
        util::split_at_events(
            self,
            ctx.events,
            frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply_param(param, value, !step);
                }
            },
        );
        let settled = self.paths.iter().all(GainPath::settled)
            && !self.drive.is_smoothing()
            && !self.ceiling.is_smoothing();
        let hold = self.delay() + 1 + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }

    fn latency(&self) -> u32 {
        self.delay() as u32
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        if let Some(f) = out.frame(AnalysisKind::Levels) {
            let db = |a: f32| util::amp_to_db(a).max(FLOOR_DB);
            for v in [
                util::amp_to_db(self.meter_gain).min(0.0),
                db(self.meter_in),
                db(self.meter_out),
                db(self.max_out),
                util::amp_to_db(self.max_gain).min(0.0),
                HISTORY_STEP_MS * 0.001,
                self.hist_len as f32,
            ] {
                f.push(v);
            }
            for ring in [&self.hist_in, &self.hist_out, &self.hist_gr] {
                for k in (0..self.hist_len).rev() {
                    f.push(ring[(self.hist_at + HISTORY - 1 - k) % HISTORY]);
                }
            }
        }
        self.meter_gain = 1.0;
        self.meter_in = 0.0;
        self.meter_out = 0.0;
    }
}

impl Device for Maximizer {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(BuiltinDeviceType::Maximizer)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
