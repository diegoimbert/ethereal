//! Real-time TD-PSOLA (time-domain pitch-synchronous overlap-add) pitch shifting with a
//! separate formant factor, for a monophonic voice (both channels share the marks).
//!
//! - **Analysis marks** are placed one period apart along the input (the period of the
//!   control in force there: [`Psola::control`]); unvoiced stretches get marks every
//!   [`UNVOICED_MS`]. Consecutive marks are one period apart, so the grains around them
//!   hold the same waveform phase and overlap coherently.
//! - **Synthesis marks** are placed along the output `period / ratio` apart; each takes the
//!   latest analysis mark at or before it and overlap-adds the Hann-windowed grain of two
//!   periods around it. The grain's spectral envelope comes with it, so the formants stay
//!   where they are while the pitch moves (formant preservation is what PSOLA does).
//! - **Formants**: a grain is read `formant` times faster (`x(a + k·formant)`), which
//!   moves its spectral envelope by that factor without changing the pitch (the spacing
//!   does). Reading `ratio` times faster instead is the classic "chipmunk" shift.
//! - With ratio 1 and formant 1 the grains are the analysis grains in place: periodic
//!   Hann windows at 50 % overlap sum to one, so the output is the input, delayed.
//! - **Latency**: the output is the input [`Psola::latency`] samples late: enough for a
//!   grain of the longest period, read at the largest formant factor, to be in.
//!
//! Real-time: every buffer is allocated in [`Psola::new`].

/// Marks of unvoiced stretches (ms apart).
pub const UNVOICED_MS: f32 = 5.0;
/// Controls kept (newest last); older ones are dropped.
const CONTROLS: usize = 64;
/// Analysis marks kept.
const MARKS: usize = 64;

/// What the shifter does from a point of the input on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    /// Input sample index this applies from.
    pub at: u64,
    /// Period (input samples) when voiced.
    pub period: f32,
    pub voiced: bool,
    /// Pitch ratio (output / input frequency).
    pub ratio: f32,
    /// Grain read speed (formant shift factor).
    pub formant: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Mark {
    at: u64,
    half: usize,
    voiced: bool,
    ratio: f32,
    formant: f32,
}

/// The shifter (two channels).
pub struct Psola {
    max_period: usize,
    max_formant: f32,
    latency: usize,
    mask: usize,
    input: [Vec<f32>; 2],
    acc: [Vec<f32>; 2],
    /// Samples pushed.
    n: u64,
    controls: [Control; CONTROLS],
    controls_len: usize,
    marks: [Mark; MARKS],
    marks_len: usize,
    next_mark: u64,
    /// Output-time position of the next synthesis mark.
    next_syn: f64,
    unvoiced_half: usize,
    /// Place a mark only once a control at or after it is in (the controls describe the
    /// input some time after it arrives: an estimate is about the middle of its window).
    wait_for_controls: bool,
}

impl Psola {
    /// Non-RT. A shifter for periods up to `max_period` samples and formant factors up to
    /// `max_formant` (≥ 1).
    pub fn new(sample_rate: f32, max_period: usize, max_formant: f32) -> Self {
        let max_formant = max_formant.max(1.0);
        let unvoiced_half = ((UNVOICED_MS * 0.001 * sample_rate).round() as usize).max(8);
        let max_half = max_period.max(unvoiced_half);
        let latency = (max_half as f32 * (1.0 + max_formant)).ceil() as usize + 4;
        let size = (latency + 4 * max_half + 8).next_power_of_two();
        let mut s = Self {
            max_period: max_half,
            max_formant,
            latency,
            mask: size - 1,
            input: [vec![0.0; size], vec![0.0; size]],
            acc: [vec![0.0; size], vec![0.0; size]],
            n: 0,
            controls: [Control {
                at: 0,
                period: 0.0,
                voiced: false,
                ratio: 1.0,
                formant: 1.0,
            }; CONTROLS],
            controls_len: 0,
            marks: [Mark::default(); MARKS],
            marks_len: 0,
            next_mark: 0,
            next_syn: 0.0,
            unvoiced_half,
            wait_for_controls: false,
        };
        s.reset();
        s
    }

    /// Wait for a control at or after a mark before placing it (see the field).
    pub fn wait_for_controls(&mut self, on: bool) {
        self.wait_for_controls = on;
    }

    /// The delay of the output (samples).
    pub fn latency(&self) -> usize {
        self.latency
    }

    /// Samples pushed so far (the index of the next input sample).
    pub fn position(&self) -> u64 {
        self.n
    }

    pub fn reset(&mut self) {
        for b in self.input.iter_mut().chain(self.acc.iter_mut()) {
            b.fill(0.0);
        }
        self.n = 0;
        self.controls_len = 0;
        self.marks_len = 0;
        self.next_mark = 0;
        self.next_syn = 0.0;
    }

    /// RT. A new control from `control.at` on (`at` not before the previous one's).
    pub fn control(&mut self, mut c: Control) {
        c.ratio = c.ratio.clamp(0.25, 4.0);
        c.formant = c.formant.clamp(1.0 / self.max_formant, self.max_formant);
        if !c.period.is_finite() || c.period < 8.0 {
            c.voiced = false;
        }
        c.period = c.period.min(self.max_period as f32);
        if self.controls_len == CONTROLS {
            self.controls.copy_within(1.., 0);
            self.controls_len -= 1;
        }
        self.controls[self.controls_len] = c;
        self.controls_len += 1;
    }

    /// The control in force at input index `at`.
    fn control_at(&self, at: u64) -> Control {
        let cs = &self.controls[..self.controls_len];
        match cs.iter().rposition(|c| c.at <= at) {
            Some(i) => cs[i],
            None => cs.first().copied().unwrap_or(Control {
                at: 0,
                period: 0.0,
                voiced: false,
                ratio: 1.0,
                formant: 1.0,
            }),
        }
    }

    #[inline]
    fn read(&self, ch: usize, pos: f64) -> f32 {
        let i = pos.floor();
        let f = (pos - i) as f32;
        let i = i as i64 as usize;
        let a = self.input[ch][i & self.mask];
        if f == 0.0 {
            return a;
        }
        let b = self.input[ch][(i + 1) & self.mask];
        a + (b - a) * f
    }

    /// Place the analysis marks whose grains are fully in.
    fn place_marks(&mut self) {
        loop {
            if self.wait_for_controls
                && self.controls[..self.controls_len]
                    .last()
                    .is_none_or(|c| c.at < self.next_mark)
            {
                return;
            }
            let c = self.control_at(self.next_mark);
            let half = if c.voiced {
                (c.period.round() as usize).clamp(8, self.max_period)
            } else {
                self.unvoiced_half
            };
            let mut at = self.next_mark;
            // Voiced after voiced: the mark goes where the waveform repeats (the lag of
            // best correlation with the previous mark's period), so the marks stay locked
            // to the waveform when the estimate is a little off (vibrato, slides).
            if c.voiced
                && let Some(prev) = self.marks[..self.marks_len].last().copied()
                && prev.voiced
            {
                let (lo, hi, w) = (half - half / 8, half + half / 8, half / 2);
                if prev.at + (hi + w) as u64 + 1 > self.n {
                    return;
                }
                at = prev.at + self.best_lag(prev.at, lo, hi, w) as u64;
            }
            let reach = (half as f32 * c.formant.max(1.0)).ceil() as u64 + 1;
            if at + reach > self.n {
                return;
            }
            if self.marks_len == MARKS {
                self.marks.copy_within(1.., 0);
                self.marks_len -= 1;
            }
            let (ratio, formant) = if c.voiced {
                (c.ratio, c.formant)
            } else {
                (1.0, 1.0)
            };
            self.marks[self.marks_len] = Mark {
                at,
                half,
                voiced: c.voiced,
                ratio,
                formant,
            };
            self.marks_len += 1;
            self.next_mark = at + half as u64;
        }
    }

    /// The lag in `lo ..= hi` where the mid signal around `from + lag` best matches the one
    /// around `from` (normalized cross-correlation over `±w` samples).
    fn best_lag(&self, from: u64, lo: usize, hi: usize, w: usize) -> usize {
        let mid = |i: u64| {
            let k = (i as usize) & self.mask;
            self.input[0][k] + self.input[1][k]
        };
        let start = from.saturating_sub(w as u64);
        let len = 2 * w;
        let ea: f32 = (0..len as u64).map(|k| mid(start + k).powi(2)).sum();
        let mut best = (lo, f32::MIN);
        for lag in lo..=hi {
            let (mut ab, mut eb) = (0.0f32, 0.0f32);
            for k in 0..len as u64 {
                let b = mid(start + k + lag as u64);
                ab += mid(start + k) * b;
                eb += b * b;
            }
            let ncc = ab / (ea * eb).sqrt().max(1e-12);
            if ncc > best.1 {
                best = (lag, ncc);
            }
        }
        best.0
    }

    /// Overlap-add the grains of the synthesis marks whose first sample is due.
    fn place_grains(&mut self, out_at: u64) {
        while self.marks_len > 0 {
            let s = self.next_syn.round().max(0.0) as u64;
            // The latest analysis mark at or before `s`.
            let Some(i) = self.marks[..self.marks_len].iter().rposition(|m| m.at <= s) else {
                // `s` is before every mark (start-up): skip ahead.
                self.next_syn = self.marks[0].at as f64;
                continue;
            };
            let m = self.marks[i];
            // The grain spans from the previous analysis mark to the next one (a period
            // each side when one isn't there): with ratio 1 the rising half of a grain and
            // the falling half of the one before cover the same samples and sum to one.
            let left = match i.checked_sub(1).map(|j| self.marks[j]) {
                Some(p) if m.at - p.at <= 2 * self.max_period as u64 => (m.at - p.at) as usize,
                _ => m.half,
            };
            let right = match self.marks[..self.marks_len].get(i + 1) {
                Some(n) => (n.at - m.at) as usize,
                None => m.half,
            };
            if s > out_at + left as u64 + 1 {
                return;
            }
            let (wl, wr) = (
                std::f32::consts::PI / left as f32,
                std::f32::consts::PI / right as f32,
            );
            let start = s as i64 - left as i64;
            for k in 0..left + right {
                let at = start + k as i64;
                if at < out_at as i64 {
                    continue;
                }
                let win = if k < left {
                    0.5 - 0.5 * (wl * k as f32).cos()
                } else {
                    0.5 + 0.5 * (wr * (k - left) as f32).cos()
                };
                let pos = m.at as f64 + (k as f64 - left as f64) * f64::from(m.formant);
                let idx = (at as usize) & self.mask;
                for ch in 0..2 {
                    let x = self.read(ch, pos.max(0.0));
                    self.acc[ch][idx] += win * x;
                }
            }
            self.next_syn += right as f64 / f64::from(m.ratio);
        }
    }

    /// RT. One input frame in; the output frame `latency` samples behind it.
    #[inline]
    pub fn process(&mut self, x: [f32; 2]) -> [f32; 2] {
        let i = (self.n as usize) & self.mask;
        self.input[0][i] = if x[0].is_finite() { x[0] } else { 0.0 };
        self.input[1][i] = if x[1].is_finite() { x[1] } else { 0.0 };
        self.n += 1;
        self.place_marks();
        let Some(out_at) = self.n.checked_sub(self.latency as u64 + 1) else {
            return [0.0; 2];
        };
        self.place_grains(out_at);
        let o = (out_at as usize) & self.mask;
        let y = [self.acc[0][o], self.acc[1][o]];
        self.acc[0][o] = 0.0;
        self.acc[1][o] = 0.0;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(p: &mut Psola, x: &[f32]) -> Vec<f32> {
        x.iter().map(|&v| p.process([v, v])[0]).collect()
    }

    #[test]
    fn unity_is_a_delay() {
        let sr = 48_000.0;
        let x: Vec<f32> = (0..20_000)
            .map(|i| (i as f32 * 0.031).sin() * 0.5 + (i as f32 * 0.17).sin() * 0.2)
            .collect();
        for voiced in [false, true] {
            let mut p = Psola::new(sr, 686, 1.4);
            p.control(Control {
                at: 0,
                period: 203.0,
                voiced,
                ratio: 1.0,
                formant: 1.0,
            });
            let y = run(&mut p, &x);
            let l = p.latency();
            // After the start-up, the input exactly (to rounding).
            let err = (2000..x.len() - l)
                .map(|i| (y[i + l] - x[i]).abs())
                .fold(0.0f32, f32::max);
            assert!(err < 1e-4, "voiced {voiced}: {err}");
        }
    }
}
