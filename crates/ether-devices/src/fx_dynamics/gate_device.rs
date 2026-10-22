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
//! `AnalysisKind::Levels [gain dB, detector peak dBFS]` for the layout meter and the transfer
//! curve's level dot (the second value appended by the effects pass).

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus,
};

use super::gate as p;
use super::shared::{
    GainRamp, Params, SIDECHAIN_CHANNELS, Svf, SvfCoefs, level_at_least, samples, usable_sidechain,
};
use crate::dsp::delay_line::{DelayLine, LookaheadTap};
use crate::util::{self, LateEvents, Route, amp_to_db, db_to_amp};

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
    lookahead: LookaheadTap,
    /// Gate mode: open (with hysteresis).
    open: bool,
    /// Samples left before a closing gate / rising reduction may start releasing.
    hold_left: u32,
    /// Current gain reduction in dB (>= 0).
    gr_db: f32,
    output: GainRamp,
    /// Lowest applied gain (dB) since the last analysis read.
    meter_db: f32,
    /// Loudest detector peak (linear) since the last analysis read (the curve's level dot).
    meter_peak: f32,
    /// Silence reporting.
    tail: util::Tail,
    /// With lookahead the gain meets the audio `lookahead` samples into the line: every
    /// param but the lookahead itself waits that long.
    late: LateEvents,
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
            lookahead: LookaheadTap::default(),
            open: true,
            hold_left: 0,
            gr_db: 0.0,
            meter_db: 0.0,
            meter_peak: 0.0,
            tail: util::Tail::default(),
            late: LateEvents::new(),
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
        self.update_lookahead(false);
        self.output
            .set(db_to_amp(self.params.get(p::OUTPUT)), false);
    }

    fn update_hpf(&mut self) {
        let fc = self.params.get(p::SIDECHAIN_HPF);
        self.hpf = (fc > HPF_OFF_HZ + 1e-3)
            .then(|| SvfCoefs::new(f64::from(fc), f64::from(self.sample_rate)));
    }

    fn update_lookahead(&mut self, smooth: bool) {
        let n = (self.params.get(p::LOOKAHEAD) * 0.001 * self.sample_rate).round() as usize;
        self.lookahead.set(n.min(self.delay[0].max_delay()), smooth);
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
            p::LOOKAHEAD => self.update_lookahead(smooth),
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
        let delayed = self.lookahead.active();
        // Level comparisons on the linear peak (exact near the thresholds,
        // see `level_at_least`), and the gain chain recomputed only when `gr_db` moves.
        let thr_amp = db_to_amp(threshold);
        let close_amp = db_to_amp(close_at);
        let low = close_at.min(threshold);
        let low_amp = db_to_amp(low);
        // Expander: at or under this level the reduction is the whole range.
        let full_db = if ratio > 1.0 {
            threshold - range / (ratio - 1.0)
        } else {
            f32::NEG_INFINITY
        };
        // Clearly under it (0.05 dB and the level margin): the min is exactly `range`.
        let full_amp = db_to_amp(full_db - 0.05) * (1.0 - 1e-3);
        let mut cached: Option<(u32, f32, f32)> = None;
        for i in start..end {
            let peak = self.detect(key, i);
            self.meter_peak = self.meter_peak.max(peak);
            // Target reduction (dB, >= 0) and whether we may release now.
            let target = if expander {
                if level_at_least(peak, threshold, thr_amp) {
                    self.hold_left = hold;
                    0.0
                } else if ratio > 1.0 && full_amp > 1e-8 && peak <= full_amp {
                    // `(threshold - level) * (ratio - 1) > range`: the min is `range`.
                    range
                } else {
                    let level = amp_to_db(peak);
                    ((threshold - level) * (ratio - 1.0)).min(range)
                }
            } else {
                if level_at_least(peak, threshold, thr_amp) {
                    self.open = true;
                    self.hold_left = hold;
                } else if level_at_least(peak, close_at, close_amp) {
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
            } else if self.hold_left > 0 && !level_at_least(peak, low, low_amp) {
                self.hold_left -= 1;
            }
            self.gr_db = self.gr_db.clamp(0.0, range);
            let (gain, db) = match cached {
                Some((bits, gain, db)) if bits == self.gr_db.to_bits() => (gain, db),
                _ => {
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
                    let db = amp_to_db(gain);
                    cached = Some((self.gr_db.to_bits(), gain, db));
                    (gain, db)
                }
            };
            let out_gain = self.output.tick();
            let applied = gain * out_gain;
            if db < self.meter_db {
                self.meter_db = db;
            }
            for (o, out) in audio.outputs.iter_mut().enumerate() {
                let x = inputs.get(o).map_or(0.0, |ch| ch[i]);
                let x = if delayed {
                    let line = &mut self.delay[o.min(1)];
                    line.push(x);
                    self.lookahead.read(line)
                } else {
                    x
                };
                out[i] = x * applied;
            }
            self.lookahead.advance();
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
        let lookahead = self.lookahead.len();
        LateEvents::split(
            self,
            |s| &mut s.late,
            ctx,
            |kind| match kind {
                EventKind::Param {
                    param: p::LOOKAHEAD,
                    ..
                } => Route::Now,
                _ => Route::Late(lookahead),
            },
            |s, a, b| s.render(audio, sidechain, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        // Silent once the look-ahead line emptied and the gate rests where silence leaves
        // it (closed / fully reduced, nothing on hold): waking it later is then the same
        // as having run it.
        let settled = self.hold_left == 0
            && self.gr_db == self.silence_reduction()
            && !self.output.is_smoothing()
            && self.late.is_idle()
            && util::all_zero(sidechain, frames);
        let hold =
            self.lookahead.len() + 1 + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }

    /// Gain reduction (dB) a silent key settles at.
    fn silence_reduction(&self) -> f32 {
        let floor = self.params.get(p::RANGE);
        let range = (-floor).max(1.0);
        let threshold = self.params.get(p::THRESHOLD);
        let level = amp_to_db(0.0);
        if self.params.on(p::MODE) {
            if level >= threshold {
                0.0
            } else {
                ((threshold - level) * (self.params.get(p::RATIO) - 1.0))
                    .min(range)
                    .clamp(0.0, range)
            }
        } else if level >= threshold {
            0.0
        } else {
            range
        }
    }
}

impl Node for Gate {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.alloc(config.sample_rate);
        self.late.prepare(self.delay[0].max_delay(), config);
        self.reset();
    }

    fn reset(&mut self) {
        LateEvents::flush(self, |s| &mut s.late, |s, kind| {
            if let EventKind::Param { param, value } = *kind {
                s.apply_param(param, value, false);
            }
        });
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
        self.meter_peak = 0.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, &[])
    }

    fn latency(&self) -> u32 {
        self.lookahead.len() as u32
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
            f.push(amp_to_db(self.meter_peak).max(FLOOR_OFF_DB));
        }
        self.meter_db = 0.0;
        self.meter_peak = 0.0;
    }
}

impl Device for Gate {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::Gate)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.late.readback(id, self.params.plain(id))
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.late.cancel(id);
        self.apply_param(id, value, false);
    }
}
