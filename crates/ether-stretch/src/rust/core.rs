//! Port of `SignalsmithStretch<float>` (signalsmith-stretch.h v1.3.1, MIT, © Geraint Luff /
//! Signalsmith Audio Ltd.): the phase vocoder with multi-channel phase locking, vertical
//! (across-frequency) phase prediction, peak-based pitch mapping and split computation.
//!
//! Left out (Ethereal never uses them): formant shifting/compensation, custom frequency maps,
//! `flush` and `exact`. Everything is allocated in [`Core::new`]; every other method is
//! real-time safe.

use rustfft::num_complex::Complex32 as C;

use super::stft::{InputBuf, OutputBuf, Stft};

const NOISE_FLOOR: f32 = 1e-15;
/// Time-stretch ratio before phases start being randomised.
const MAX_CLEAN_STRETCH: f32 = 2.0;
const SMOOTH_ENERGY_STEPS: usize = 3;
/// The main prediction is heavy (up to 4 blended predictions): spread over this many steps.
const SPLIT_MAIN_PREDICTION: usize = 8;

#[inline]
fn norm(c: C) -> f32 {
    c.re * c.re + c.im * c.im
}

/// `a · conj(b)`.
#[inline]
fn mul_conj(a: C, b: C) -> C {
    a * b.conj()
}

#[derive(Clone, Copy, Default)]
struct Band {
    input: C,
    prev_input: C,
    output: C,
    input_energy: f32,
}

#[derive(Clone, Copy, Default)]
struct Prediction {
    energy: f32,
    input: C,
}

impl Prediction {
    #[inline]
    fn make_output(&self, mut phase: C) -> C {
        let mut phase_norm = norm(phase);
        if phase_norm <= NOISE_FLOOR {
            // Prediction too weak: fall back to the input.
            phase = self.input;
            phase_norm = norm(self.input) + NOISE_FLOOR;
        }
        phase * (self.energy / phase_norm).sqrt()
    }
}

#[derive(Clone, Copy)]
struct Peak {
    input: f32,
    output: f32,
}

#[derive(Clone, Copy, Default)]
struct MapPoint {
    input_bin: f32,
    freq_grad: f32,
}

#[derive(Clone, Copy)]
struct BlockProcess {
    samples_since_last: usize,
    steps: usize,
    step: usize,
    new_spectrum: bool,
    reanalyse_prev: bool,
    mapped_frequencies: bool,
    time_factor: f32,
}

impl Default for BlockProcess {
    fn default() -> Self {
        Self {
            samples_since_last: usize::MAX,
            steps: 0,
            step: 0,
            new_spectrum: false,
            reanalyse_prev: false,
            mapped_frequencies: false,
            time_factor: 1.0,
        }
    }
}

pub(crate) struct Core {
    split: bool,
    channels: usize,
    bands: usize,
    stft: Stft,
    stashed_input: InputBuf,
    stashed_output: OutputBuf,
    bp: BlockProcess,
    silence_counter: usize,
    silence_first: bool,
    freq_multiplier: f32,
    freq_tonality_limit: f32,
    prev_input_offset: i64,
    did_seek: bool,
    seek_time_factor: f32,
    channel_bands: Vec<Band>,
    peaks: Vec<Peak>,
    energy: Vec<f32>,
    smoothed_energy: Vec<f32>,
    output_map: Vec<MapPoint>,
    channel_predictions: Vec<Prediction>,
    process_spectrum_steps: usize,
    smooth_energy_state: f32,
    rng: u64,
}

/// Where a `process` call reads its input from: `channels` planar slices (missing channels
/// read silence, samples past a slice's end too).
#[derive(Clone, Copy)]
pub(crate) struct Inputs<'a>(pub(crate) &'a [&'a [f32]]);

impl Inputs<'_> {
    #[inline]
    fn get(&self, c: usize, i: usize) -> f32 {
        self.0
            .get(c)
            .and_then(|ch| ch.get(i))
            .copied()
            .unwrap_or(0.0)
    }
}

impl Core {
    /// Non-RT. `configure(channels, block, interval, split)`.
    pub(crate) fn new(channels: usize, block: usize, interval: usize, split: bool) -> Self {
        let stft = Stft::new(channels, block, interval + 1, interval);
        let bands = stft.bands();
        let mut s = Self {
            split,
            channels,
            bands,
            stashed_input: stft.input.clone(),
            stashed_output: stft.output.clone(),
            stft,
            bp: BlockProcess::default(),
            silence_counter: 0,
            silence_first: true,
            freq_multiplier: 1.0,
            freq_tonality_limit: 1.0,
            prev_input_offset: -1,
            did_seek: false,
            seek_time_factor: 1.0,
            channel_bands: vec![Band::default(); bands * channels],
            peaks: Vec::with_capacity(bands / 2 + 2),
            energy: vec![0.0; bands],
            smoothed_energy: vec![0.0; bands],
            output_map: vec![MapPoint::default(); bands],
            channel_predictions: vec![Prediction::default(); bands * channels],
            process_spectrum_steps: 0,
            smooth_energy_state: 0.0,
            rng: 0x2545_F491_4F6C_DD1D,
        };
        s.reset();
        s
    }

    pub(crate) fn channels(&self) -> usize {
        self.channels
    }

    pub(crate) fn input_latency(&self) -> usize {
        self.stft.analysis_latency()
    }

    pub(crate) fn output_latency(&self) -> usize {
        self.stft.synthesis_latency() + if self.split { self.stft.interval() } else { 0 }
    }

    pub(crate) fn reset(&mut self) {
        self.stft.reset(0.1);
        self.stashed_input.copy_from(&self.stft.input);
        self.stashed_output.copy_from(&self.stft.output);
        self.prev_input_offset = -1;
        self.channel_bands.fill(Band::default());
        self.silence_counter = 0;
        self.did_seek = false;
        self.bp = BlockProcess::default();
    }

    /// `setTransposeFactor(multiplier)` (no tonality limit).
    pub(crate) fn set_transpose_factor(&mut self, multiplier: f32) {
        self.freq_multiplier = multiplier;
        self.freq_tonality_limit = 1.0;
    }

    /// Provide pre-roll (ideally a block + an interval) without affecting the speed.
    pub(crate) fn seek(&mut self, inputs: Inputs<'_>, input_samples: usize, playback_rate: f64) {
        let tmp_len = self.stft.block() + self.stft.interval();
        let start_index = input_samples.saturating_sub(tmp_len);
        let pad_start = tmp_len + start_index - input_samples;
        let mut total_energy = 0.0f32;
        for c in 0..self.channels {
            for i in 0..tmp_len {
                let s = if i < pad_start {
                    0.0
                } else {
                    inputs.get(c, start_index + i - pad_start)
                };
                total_energy += s * s;
                self.stft.write_input(c, i, s);
            }
        }
        self.stft.move_input(tmp_len);
        if total_energy >= NOISE_FLOOR {
            self.silence_counter = 0;
            self.silence_first = true;
        }
        self.did_seek = true;
        let interval = self.stft.interval() as f64;
        self.seek_time_factor = if playback_rate * interval > 1.0 {
            (1.0 / playback_rate) as f32
        } else {
            interval as f32
        };
    }

    /// Copy input up to `to_index` (at most a block + interval of it) into the STFT.
    fn copy_input(&mut self, inputs: Inputs<'_>, prev_copied: &mut usize, to_index: usize) {
        let length =
            (self.stft.block() + self.stft.interval()).min(to_index.saturating_sub(*prev_copied));
        let offset = to_index - length;
        for c in 0..self.channels {
            for i in 0..length {
                let v = inputs.get(c, i + offset);
                self.stft.write_input(c, i, v);
            }
        }
        self.stft.move_input(length);
        *prev_copied = to_index;
    }

    pub(crate) fn process(
        &mut self,
        inputs: Inputs<'_>,
        input_samples: usize,
        outputs: &mut [&mut [f32]],
        output_samples: usize,
    ) {
        let mut prev_copied = 0usize;
        let mut total_energy = 0.0f32;
        for ch in inputs.0.iter().take(self.channels) {
            for &s in &ch[..input_samples.min(ch.len())] {
                total_energy += s * s;
            }
        }

        if total_energy < NOISE_FLOOR {
            if self.silence_counter >= 2 * self.stft.block() {
                if self.silence_first {
                    self.silence_first = false;
                    self.bp = BlockProcess::default();
                    for b in &mut self.channel_bands {
                        *b = Band::default();
                    }
                }
                for (c, out) in outputs.iter_mut().enumerate().take(self.channels) {
                    for (o, v) in out.iter_mut().take(output_samples).enumerate() {
                        *v = if input_samples > 0 {
                            inputs.get(c, o % input_samples)
                        } else {
                            0.0
                        };
                    }
                }
                self.copy_input(inputs, &mut prev_copied, input_samples);
                return;
            } else {
                self.silence_counter += input_samples;
            }
        } else {
            self.silence_counter = 0;
            self.silence_first = true;
        }

        let interval = self.stft.interval();
        let block = self.stft.block();
        for output_index in 0..output_samples {
            let new_block = self.bp.samples_since_last >= interval;
            if new_block {
                self.start_block(
                    inputs,
                    &mut prev_copied,
                    output_index,
                    input_samples,
                    output_samples,
                );
            }
            let mut process_to = if new_block { self.bp.steps } else { 0 };
            if self.split {
                let ratio = (self.bp.samples_since_last + 1) as f32 / interval as f32;
                process_to = self
                    .bp
                    .steps
                    .min(((self.bp.steps as f32 + 0.999) * ratio) as usize);
            }
            while self.bp.step < process_to {
                let step = self.bp.step;
                self.bp.step += 1;
                self.run_step(step);
            }

            self.bp.samples_since_last += 1;
            let out = if self.split {
                &mut self.stashed_output
            } else {
                &mut self.stft.output
            };
            for (c, ch) in outputs.iter_mut().enumerate().take(self.channels) {
                if let Some(v) = ch.get_mut(output_index) {
                    *v = out.read(c, block);
                }
            }
            out.advance_one(self.channels, block);
        }

        self.copy_input(inputs, &mut prev_copied, input_samples);
        self.prev_input_offset -= input_samples as i64;
    }

    fn start_block(
        &mut self,
        inputs: Inputs<'_>,
        prev_copied: &mut usize,
        output_index: usize,
        input_samples: usize,
        output_samples: usize,
    ) {
        let interval = self.stft.interval();
        let bp = &mut self.bp;
        bp.step = 0;
        bp.steps = 0;
        bp.samples_since_last = 0;

        // Where in the input should the new spectrum come from?
        let input_offset =
            (output_index as f32 * input_samples as f32 / output_samples as f32).round() as i64;
        let input_interval = input_offset - self.prev_input_offset;
        self.prev_input_offset = input_offset;

        self.copy_input(inputs, prev_copied, input_offset.max(0) as usize);
        self.stashed_input.copy_from(&self.stft.input);
        if self.split {
            // Read from the current output; synthesis goes one interval ahead.
            self.stashed_output.copy_from(&self.stft.output);
            self.stft.move_output(interval);
        }

        let channels = self.channels;
        let bp = &mut self.bp;
        bp.new_spectrum = self.did_seek || input_interval > 0;
        bp.mapped_frequencies = self.freq_multiplier != 1.0;
        if bp.new_spectrum {
            bp.reanalyse_prev = self.did_seek || (input_interval - interval as i64).abs() > 1;
            if bp.reanalyse_prev {
                bp.steps += channels + 1;
            }
            bp.steps += channels + 1;
        }
        bp.time_factor = if self.did_seek {
            self.seek_time_factor
        } else {
            interval as f32 / (input_interval as f32).max(1.0)
        };
        self.did_seek = false;

        self.update_process_spectrum_steps();
        self.bp.steps += self.process_spectrum_steps;
        self.bp.steps += channels + 1;
    }

    fn run_step(&mut self, mut step: usize) {
        let channels = self.channels;
        let bands = self.bands;
        let interval = self.stft.interval();
        if self.bp.new_spectrum {
            if self.bp.reanalyse_prev {
                // Analyse the past input.
                if step < channels {
                    self.stft.analyse(step, &self.stashed_input, interval);
                    return;
                }
                step -= channels;
                if step < 1 {
                    for c in 0..channels {
                        let spec = self.stft.spectrum(c);
                        for (b, s) in self.channel_bands[c * bands..(c + 1) * bands]
                            .iter_mut()
                            .zip(spec.iter())
                        {
                            b.prev_input = *s;
                        }
                    }
                    return;
                }
                step -= 1;
            }
            // Analyse the latest (stashed) input.
            if step < channels {
                self.stft.analyse(step, &self.stashed_input, 0);
                return;
            }
            step -= channels;
            if step < 1 {
                for c in 0..channels {
                    let spec = self.stft.spectrum(c);
                    for (b, s) in self.channel_bands[c * bands..(c + 1) * bands]
                        .iter_mut()
                        .zip(spec.iter())
                    {
                        b.input = *s;
                    }
                }
                return;
            }
            step -= 1;
        }

        if step < self.process_spectrum_steps {
            self.process_spectrum(step);
            return;
        }
        step -= self.process_spectrum_steps;

        if step < 1 {
            for c in 0..channels {
                let bands_c = &self.channel_bands[c * bands..(c + 1) * bands];
                let spec = self.stft.spectrum(c);
                for (s, b) in spec.iter_mut().zip(bands_c) {
                    *s = b.output;
                }
            }
            return;
        }
        step -= 1;

        if step < channels {
            self.stft.synthesise_step(step);
        }
    }

    fn update_process_spectrum_steps(&mut self) {
        let mut n = 0;
        if self.bp.new_spectrum {
            n += self.channels;
        }
        if self.bp.mapped_frequencies {
            n += SMOOTH_ENERGY_STEPS + 1; // + findPeaks
        }
        n += 1; // output map
        n += self.channels; // preliminary phase-vocoder prediction
        n += SPLIT_MAIN_PREDICTION;
        if self.bp.new_spectrum {
            n += 1; // .input -> .prev_input
        }
        self.process_spectrum_steps = n;
    }

    #[inline]
    fn band(&self, c: usize, index: i32) -> Option<&Band> {
        if index < 0 || index as usize >= self.bands {
            None
        } else {
            Some(&self.channel_bands[c * self.bands + index as usize])
        }
    }

    #[inline]
    fn frac_input(&self, c: usize, low: i32, frac: f32) -> C {
        let lo = self.band(c, low).map_or(C::default(), |b| b.input);
        let hi = self.band(c, low + 1).map_or(C::default(), |b| b.input);
        lo + (hi - lo) * frac
    }

    #[inline]
    fn frac_prev_input(&self, c: usize, low: i32, frac: f32) -> C {
        let lo = self.band(c, low).map_or(C::default(), |b| b.prev_input);
        let hi = self.band(c, low + 1).map_or(C::default(), |b| b.prev_input);
        lo + (hi - lo) * frac
    }

    #[inline]
    fn frac_energy(&self, c: usize, low: i32, frac: f32) -> f32 {
        let lo = self.band(c, low).map_or(0.0, |b| b.input_energy);
        let hi = self.band(c, low + 1).map_or(0.0, |b| b.input_energy);
        lo + (hi - lo) * frac
    }

    #[inline]
    fn frac_input_at(&self, c: usize, index: f32) -> C {
        let low = index.floor();
        self.frac_input(c, low as i32, index - low)
    }

    /// Uniform in `[lo, hi)` (xorshift64*; Signalsmith uses `std::default_random_engine`).
    #[inline]
    fn uniform(&mut self, lo: f32, hi: f32) -> f32 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let r = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        let u = (r >> 40) as f32 / (1u64 << 24) as f32;
        lo + (hi - lo) * u
    }

    fn process_spectrum(&mut self, mut step: usize) {
        let channels = self.channels;
        let bands = self.bands;
        let interval = self.stft.interval() as f32;
        let smoothing_bins = self.stft.fft_size() as f32 / interval;
        let long_vertical_step = smoothing_bins.round() as usize;
        let time_factor = self.bp.time_factor.max(1.0 / MAX_CLEAN_STRETCH);
        let random_time_factor = time_factor > MAX_CLEAN_STRETCH;
        let dist_lo =
            MAX_CLEAN_STRETCH * 2.0 * f32::from(u8::from(random_time_factor)) - time_factor;

        if self.bp.new_spectrum {
            if step < channels {
                let c = step;
                let tau = std::f32::consts::TAU;
                let mut rot = C::from_polar(1.0, self.stft.bin_to_freq(0.0) * interval * tau);
                let freq_step = self.stft.bin_to_freq(1.0) - self.stft.bin_to_freq(0.0);
                let rot_step = C::from_polar(1.0, freq_step * interval * tau);
                for bin in &mut self.channel_bands[c * bands..(c + 1) * bands] {
                    bin.output *= rot;
                    bin.prev_input *= rot;
                    rot *= rot_step;
                }
                return;
            }
            step -= channels;
        }
        if self.bp.mapped_frequencies {
            if step < SMOOTH_ENERGY_STEPS {
                self.smooth_energy(step, smoothing_bins);
                return;
            }
            step -= SMOOTH_ENERGY_STEPS;
            if step == 0 {
                self.find_peaks();
                return;
            }
            step -= 1;
        }
        if step == 0 {
            if self.bp.mapped_frequencies {
                self.update_output_map();
            } else {
                // Not pitch-shifting: no peaks needed.
                for b in &mut self.channel_bands {
                    b.input_energy = norm(b.input);
                }
                for (b, m) in self.output_map.iter_mut().enumerate() {
                    *m = MapPoint {
                        input_bin: b as f32,
                        freq_grad: 1.0,
                    };
                }
            }
            return;
        }
        step -= 1;

        // Preliminary output prediction from the phase vocoder.
        if step < channels {
            let c = step;
            for b in 0..bands {
                let map = self.output_map[b];
                let low_f = map.input_bin.floor();
                let low = low_f as i32;
                let frac = map.input_bin - low_f;
                let energy = self.frac_energy(c, low, frac) * map.freq_grad.max(0.0);
                let input = self.frac_input(c, low, frac);
                let prev_input = self.frac_prev_input(c, low, frac);
                let prediction = &mut self.channel_predictions[c * bands + b];
                let prev_energy = prediction.energy;
                prediction.energy = energy;
                prediction.input = input;
                let freq_twist = mul_conj(input, prev_input);
                let out = &mut self.channel_bands[c * bands + b].output;
                let phase = *out * freq_twist;
                *out = phase / (prev_energy.max(energy) + NOISE_FLOOR);
            }
            return;
        }
        step -= channels;

        if step < SPLIT_MAIN_PREDICTION {
            // Re-predict using phase differences between frequencies.
            let start_b = bands * step / SPLIT_MAIN_PREDICTION;
            let end_b = bands * (step + 1) / SPLIT_MAIN_PREDICTION;
            let lvs = long_vertical_step;
            for b in start_b..end_b {
                // The loudest channel leads; the others are phase-locked to it.
                let mut max_channel = 0;
                let mut max_energy = self.channel_predictions[b].energy;
                for c in 1..channels {
                    let e = self.channel_predictions[c * bands + b].energy;
                    if e > max_energy {
                        max_channel = c;
                        max_energy = e;
                    }
                }
                let mc = max_channel;
                let base = mc * bands;
                let prediction = self.channel_predictions[base + b];
                let map = self.output_map[b];
                let mut phase = C::default();

                // Upwards vertical steps.
                if b > 0 {
                    let btf = if random_time_factor {
                        self.uniform(dist_lo, time_factor)
                    } else {
                        time_factor
                    };
                    let down_input = self.frac_input_at(mc, map.input_bin - btf);
                    let short_twist = mul_conj(prediction.input, down_input);
                    phase += self.channel_bands[base + b - 1].output * short_twist;
                    if b >= lvs {
                        let long_down = self.frac_input_at(mc, map.input_bin - lvs as f32 * btf);
                        let long_twist = mul_conj(prediction.input, long_down);
                        phase += self.channel_bands[base + b - lvs].output * long_twist;
                    }
                }
                // Downwards vertical steps.
                if b + 1 < bands {
                    let up_prediction = self.channel_predictions[base + b + 1];
                    let up_map = self.output_map[b + 1];
                    let btf = if random_time_factor {
                        self.uniform(dist_lo, time_factor)
                    } else {
                        time_factor
                    };
                    let down_input = self.frac_input_at(mc, up_map.input_bin - btf);
                    let short_twist = mul_conj(up_prediction.input, down_input);
                    phase += mul_conj(self.channel_bands[base + b + 1].output, short_twist);
                    if b + lvs < bands {
                        let long_up_prediction = self.channel_predictions[base + b + lvs];
                        let long_up_map = self.output_map[b + lvs];
                        let long_down =
                            self.frac_input_at(mc, long_up_map.input_bin - lvs as f32 * btf);
                        let long_twist = mul_conj(long_up_prediction.input, long_down);
                        phase += mul_conj(self.channel_bands[base + b + lvs].output, long_twist);
                    }
                }

                let output = prediction.make_output(phase);
                self.channel_bands[base + b].output = output;

                // All other channels are locked in phase.
                for c in 0..channels {
                    if c != mc {
                        let cp = self.channel_predictions[c * bands + b];
                        let twist = mul_conj(cp.input, prediction.input);
                        self.channel_bands[c * bands + b].output = cp.make_output(output * twist);
                    }
                }
            }
            return;
        }
        step -= SPLIT_MAIN_PREDICTION;

        if self.bp.new_spectrum && step == 0 {
            for b in &mut self.channel_bands {
                b.prev_input = b.input;
            }
        }
    }

    /// Smoothed energy across all channels.
    fn smooth_energy(&mut self, step: usize, smoothing_bins: f32) {
        let slew = 1.0 / (1.0 + smoothing_bins * 0.5);
        let bands = self.bands;
        if step == 0 {
            self.energy.fill(0.0);
            for c in 0..self.channels {
                for (b, bin) in self.channel_bands[c * bands..(c + 1) * bands]
                    .iter_mut()
                    .enumerate()
                {
                    let e = norm(bin.input);
                    bin.input_energy = e;
                    self.energy[b] += e;
                }
            }
            self.smoothed_energy.copy_from_slice(&self.energy);
            self.smooth_energy_state = 0.0;
            return;
        }
        // Repeated smoothing passes, down and up.
        let mut e = self.smooth_energy_state;
        for v in self.smoothed_energy.iter_mut().rev() {
            e += (*v - e) * slew;
            *v = e;
        }
        for v in self.smoothed_energy.iter_mut() {
            e += (*v - e) * slew;
            *v = e;
        }
        self.smooth_energy_state = e;
    }

    fn map_freq(&self, freq: f32) -> f32 {
        if freq > self.freq_tonality_limit {
            freq + (self.freq_multiplier - 1.0) * self.freq_tonality_limit
        } else {
            freq * self.freq_multiplier
        }
    }

    /// Spectral peaks of the energy across all channels.
    fn find_peaks(&mut self) {
        self.peaks.clear();
        let bands = self.bands;
        let mut start = 0;
        while start < bands {
            if self.energy[start] > self.smoothed_energy[start] {
                let mut end = start;
                let (mut band_sum, mut energy_sum) = (0.0f32, 0.0f32);
                while end < bands && self.energy[end] > self.smoothed_energy[end] {
                    band_sum += end as f32 * self.energy[end];
                    energy_sum += self.energy[end];
                    end += 1;
                }
                let avg_band = band_sum / energy_sum;
                let avg_freq = self.stft.bin_to_freq(avg_band);
                let out = self.stft.freq_to_bin(self.map_freq(avg_freq));
                if self.peaks.len() < self.peaks.capacity() {
                    self.peaks.push(Peak {
                        input: avg_band,
                        output: out,
                    });
                }
                start = end;
            }
            start += 1;
        }
    }

    fn update_output_map(&mut self) {
        let bands = self.bands as i32;
        let (Some(&first), Some(&last)) = (self.peaks.first(), self.peaks.last()) else {
            for (b, m) in self.output_map.iter_mut().enumerate() {
                *m = MapPoint {
                    input_bin: b as f32,
                    freq_grad: 1.0,
                };
            }
            return;
        };
        let bottom_offset = first.input - first.output;
        for b in 0..bands.min(first.output.ceil() as i32) {
            self.output_map[b as usize] = MapPoint {
                input_bin: b as f32 + bottom_offset,
                freq_grad: 1.0,
            };
        }
        // Interpolate between peaks.
        for p in 1..self.peaks.len() {
            let (prev, next) = (self.peaks[p - 1], self.peaks[p]);
            let range_scale = 1.0 / (next.output - prev.output);
            let out_offset = prev.input - prev.output;
            let out_scale = next.input - next.output - prev.input + prev.output;
            let grad_scale = out_scale * range_scale;
            let start_bin = (prev.output.ceil() as i32).max(0);
            let end_bin = bands.min(next.output.ceil() as i32);
            for b in start_bin..end_bin {
                let r = (b as f32 - prev.output) * range_scale;
                let h = r * r * (3.0 - 2.0 * r);
                let out_b = b as f32 + out_offset + h * out_scale;
                let grad_h = 6.0 * r * (1.0 - r);
                let grad_b = 1.0 + grad_h * grad_scale;
                self.output_map[b as usize] = MapPoint {
                    input_bin: out_b,
                    freq_grad: grad_b,
                };
            }
        }
        let top_offset = last.input - last.output;
        for b in (last.output as i32).max(0)..bands {
            self.output_map[b as usize] = MapPoint {
                input_bin: b as f32 + top_offset,
                freq_grad: 1.0,
            };
        }
    }
}
