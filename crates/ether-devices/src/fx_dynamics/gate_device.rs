//! Gate / expander (`BuiltinDeviceType::Gate`).
//!
//! Stereo-linked peak detector, keyed from the main input or, when the chain entry has a
//! sidechain source, from the sidechain (`Node::process_sidechain`). The detector signal
//! goes through a 12 dB/oct high-pass (`Sidechain HPF`, off at its 20 Hz minimum) in both
//! cases, so low rumble or a bass bleed doesn't hold the gate open.
//!
//! - **Gate**: opens when the level reaches `Threshold`, closes when it falls below
//!   `Threshold - Return` (hysteresis) for longer than `Hold`. Closed = `Floor` (the -80 dB
//!   minimum is full silence).
//! - **Expander**: below `Threshold` the gain drops by `(Threshold - level) · (Ratio - 1)`,
//!   down to `Floor`; `Hold` delays the release the same way.
//!
//! The gain moves linearly in dB: a full open (Floor → 0 dB) takes `Attack`, a full close
//! `Release`. `Flip` inverts the gate (passes what is below the threshold). `Lookahead`
//! delays the audio (not the detector) and is reported as latency. The applied gain (dB,
//! ≤ 0; the most reduced value since the last read) is published as
//! `AnalysisKind::Levels [gain dB]` for the layout meter.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus,
};

use super::gate as p;
use super::shared::{
    GainRamp, Params, SIDECHAIN_CHANNELS, Svf, SvfCoefs, samples, usable_sidechain,
};
use crate::dsp::delay_line::DelayLine;
use crate::util::{amp_to_db, db_to_amp, split_at_events};

/// `Floor` at (or below) this is full silence.
const FLOOR_OFF_DB: f32 = -80.0;
/// `Sidechain HPF` at (or below) this is off.
const HPF_OFF_HZ: f32 = 20.0;
/// Largest `Lookahead` (ms), sizes the delay lines.
const MAX_LOOKAHEAD_MS: f32 = 10.0;

/// The gate.
pub struct Gate {
    params: Params<{ p::COUNT }>,
    sample_rate: f32,
    /// Detector high-pass (`None` = off) and its state per detector channel.
    hpf: Option<SvfCoefs>,
    hpf_state: [Svf; SIDECHAIN_CHANNELS],
    /// Lookahead delay per channel and the current delay in samples.
    delay: [DelayLine; 2],
    lookahead: usize,
    /// Gate mode: open (with hysteresis).
    open: bool,
    /// Samples left before a closing gate / rising reduction may start releasing.
    hold_left: u32,
    /// Current gain reduction in dB (>= 0).
    gr_db: f32,
    output: GainRamp,
    /// Lowest applied gain (dB) since the last analysis read.
    meter_db: f32,
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    /// Non-RT. A gate with default parameters (prepared for 48 kHz).
    pub fn new() -> Self {
        let params = Params::new(&super::descriptor(BuiltinDeviceType::Gate));
        let mut g = Self {
            output: GainRamp::new(db_to_amp(params.get(p::OUTPUT))),
            params,
            sample_rate: 48_000.0,
            hpf: None,
            hpf_state: [Svf::default(); SIDECHAIN_CHANNELS],
            delay: [DelayLine::default(), DelayLine::default()],
            lookahead: 0,
            open: true,
            hold_left: 0,
            gr_db: 0.0,
            meter_db: 0.0,
        };
        g.alloc(48_000.0);
        g
    }

    /// Non-RT: size the delay lines for `sample_rate`.
    fn alloc(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        let max = (MAX_LOOKAHEAD_MS * 0.001 * self.sample_rate).ceil() as usize + 1;
        self.delay = [DelayLine::new(max), DelayLine::new(max)];
        self.sync();
    }

    fn sync(&mut self) {
        self.update_hpf();
        self.update_lookahead();
        self.output
            .set(db_to_amp(self.params.get(p::OUTPUT)), false);
    }

    fn update_hpf(&mut self) {
        let fc = self.params.get(p::SIDECHAIN_HPF);
        self.hpf = (fc > HPF_OFF_HZ + 1e-3)
            .then(|| SvfCoefs::new(f64::from(fc), f64::from(self.sample_rate)));
    }

    fn update_lookahead(&mut self) {
        let n = (self.params.get(p::LOOKAHEAD) * 0.001 * self.sample_rate).round() as usize;
        self.lookahead = n.min(self.delay[0].max_delay());
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        if !self.params.set(id, value) {
            return;
        }
        match id {
            p::OUTPUT => self
                .output
                .set(db_to_amp(self.params.get(p::OUTPUT)), smooth),
            p::SIDECHAIN_HPF => self.update_hpf(),
            p::LOOKAHEAD => self.update_lookahead(),
            _ => {}
        }
    }

    /// Current gain reduction in dB (>= 0).
    pub fn gain_reduction(&self) -> f32 {
        self.gr_db
    }

    /// Detector peak of frame `i` (high-passed sidechain or main input).
    #[inline]
    fn detect(&mut self, key: &[&[f32]], i: usize) -> f32 {
        let mut peak = 0.0f32;
        for (ch, st) in key.iter().zip(self.hpf_state.iter_mut()) {
            let x = ch[i];
            let y = match &self.hpf {
                Some(c) => st.tick(f64::from(x), c).hp as f32,
                None => x,
            };
            peak = peak.max(y.abs());
        }
        peak
    }

    fn render(
        &mut self,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
        start: usize,
        end: usize,
    ) {
        let threshold = self.params.get(p::THRESHOLD);
        let close_at = threshold - self.params.get(p::HYSTERESIS);
        let floor = self.params.get(p::RANGE);
        let range = (-floor).max(1.0);
        let ratio = self.params.get(p::RATIO);
        let expander = self.params.on(p::MODE);
        let flip = self.params.on(p::FLIP);
        let sr = self.sample_rate;
        let attack_step = range / samples(self.params.get(p::ATTACK), sr);
        let release_step = range / samples(self.params.get(p::RELEASE), sr);
        let hold = (self.params.get(p::HOLD) * 0.001 * sr) as u32;
        let silent_floor = floor <= FLOOR_OFF_DB;
        let floor_amp = if silent_floor { 0.0 } else { db_to_amp(floor) };
        let off_amp = db_to_amp(FLOOR_OFF_DB);
        let inputs = audio.inputs;
        let key = if sidechain.is_empty() {
            &inputs[..inputs.len().min(SIDECHAIN_CHANNELS)]
        } else {
            sidechain
        };
        let delayed = self.lookahead > 0;
        for i in start..end {
            let level = amp_to_db(self.detect(key, i));
            // Target reduction (dB, >= 0) and whether we may release now.
            let target = if expander {
                if level >= threshold {
                    self.hold_left = hold;
                    0.0
                } else {
                    ((threshold - level) * (ratio - 1.0)).min(range)
                }
            } else {
                if level >= threshold {
                    self.open = true;
                    self.hold_left = hold;
                } else if level >= close_at {
                    if self.open {
                        self.hold_left = hold;
                    }
                } else if self.open && self.hold_left == 0 {
                    self.open = false;
                }
                if self.open { 0.0 } else { range }
            };
            if target < self.gr_db {
                self.gr_db = (self.gr_db - attack_step).max(target);
            } else if target > self.gr_db {
                if self.hold_left > 0 {
                    self.hold_left -= 1;
                } else {
                    self.gr_db = (self.gr_db + release_step).min(target);
                }
            } else if self.hold_left > 0 && level < close_at.min(threshold) {
                self.hold_left -= 1;
            }
            self.gr_db = self.gr_db.clamp(0.0, range);
            let mut gain = db_to_amp(-self.gr_db);
            if silent_floor {
                // Map [-80 dB, 0 dB] onto [0, 1] so a closed gate is silent.
                gain = ((gain - off_amp) / (1.0 - off_amp)).max(0.0);
            } else {
                gain = gain.max(floor_amp);
            }
            if flip {
                gain = (floor_amp + 1.0 - gain).min(1.0);
            }
            let out_gain = self.output.tick();
            let applied = gain * out_gain;
            let db = amp_to_db(gain);
            if db < self.meter_db {
                self.meter_db = db;
            }
            for (o, out) in audio.outputs.iter_mut().enumerate() {
                let x = inputs.get(o).map_or(0.0, |ch| ch[i]);
                let x = if delayed {
                    let line = &mut self.delay[o.min(1)];
                    line.push(x);
                    line.tap(self.lookahead + 1)
                } else {
                    x
                };
                out[i] = x * applied;
            }
            if !delayed {
                // Keep the lines filled so switching the lookahead on doesn't replay
                // stale audio.
                for (o, line) in self.delay.iter_mut().enumerate() {
                    line.push(inputs.get(o).map_or(0.0, |ch| ch[i]));
                }
            }
        }
    }

    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        let frames = ctx.frames;
        let sidechain = usable_sidechain(sidechain, frames);
        split_at_events(
            self,
            ctx.events,
            frames,
            |s, a, b| s.render(audio, sidechain, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }
}

impl Node for Gate {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.alloc(config.sample_rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.hpf_state = [Svf::default(); SIDECHAIN_CHANNELS];
        for d in &mut self.delay {
            d.clear();
        }
        // A fresh gate starts open (the first note isn't chopped); it closes after
        // Hold + Release when the input is quiet.
        self.open = true;
        self.hold_left = 0;
        self.gr_db = 0.0;
        self.meter_db = 0.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, &[])
    }

    fn latency(&self) -> u32 {
        self.lookahead as u32
    }

    fn sidechain_inputs(&self) -> u16 {
        SIDECHAIN_CHANNELS as u16
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, sidechain)
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        if let Some(f) = out.frame(AnalysisKind::Levels) {
            f.push(self.meter_db.max(FLOOR_OFF_DB));
        }
        self.meter_db = 0.0;
    }
}

impl Device for Gate {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::Gate)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}
