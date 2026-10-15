//! Port of Signalsmith Linear's `DynamicSTFT<float, false, true>` (`stft.h`): a
//! self-normalising STFT on the modified real FFT, with a Kaiser window forced to perfect
//! reconstruction. Only what Signalsmith Stretch uses (same analysis and synthesis
//! channels, Kaiser window, centred offsets).

use rustfft::num_complex::Complex32 as C;

use super::fft::{ModifiedRealFft, fast_size_above};

const ALMOST_ZERO: f32 = 1e-30;

/// Input history ring (`DynamicSTFT::Input`); saved/restored by the stretcher.
#[derive(Clone)]
pub(crate) struct InputBuf {
    pub(crate) pos: usize,
    /// `channels × len`, planar.
    pub(crate) buf: Vec<f32>,
}

impl InputBuf {
    /// RT. Copy `other` (same size) into `self`.
    pub(crate) fn copy_from(&mut self, other: &InputBuf) {
        self.pos = other.pos;
        self.buf.copy_from_slice(&other.buf);
    }
}

/// Output accumulation ring (`DynamicSTFT::Output`).
#[derive(Clone)]
pub(crate) struct OutputBuf {
    pub(crate) pos: usize,
    /// `channels × block`, planar.
    pub(crate) buf: Vec<f32>,
    pub(crate) window_products: Vec<f32>,
}

impl OutputBuf {
    /// RT. Copy `other` (same size) into `self`.
    pub(crate) fn copy_from(&mut self, other: &OutputBuf) {
        self.pos = other.pos;
        self.buf.copy_from_slice(&other.buf);
        self.window_products.copy_from_slice(&other.window_products);
    }

    /// The (normalised) output sample of `channel` at the read position.
    #[inline]
    pub(crate) fn read(&self, channel: usize, block: usize) -> f32 {
        self.buf[channel * block + self.pos] / self.window_products[self.pos]
    }

    /// Advance the read position by one sample, clearing what it leaves behind.
    #[inline]
    pub(crate) fn advance_one(&mut self, channels: usize, block: usize) {
        for c in 0..channels {
            self.buf[c * block + self.pos] = 0.0;
        }
        self.window_products[self.pos] = ALMOST_ZERO;
        self.pos += 1;
        if self.pos >= block {
            self.pos = 0;
        }
    }

    fn advance(&mut self, samples: usize, channels: usize, block: usize) {
        for i in 0..samples {
            let i2 = (self.pos + i) % block;
            for c in 0..channels {
                self.buf[c * block + i2] = 0.0;
            }
            self.window_products[i2] = ALMOST_ZERO;
        }
        self.pos = (self.pos + samples) % block;
    }
}

pub(crate) struct Stft {
    fft: ModifiedRealFft,
    channels: usize,
    block: usize,
    fft_size: usize,
    bins: usize,
    input_len: usize,
    interval: usize,
    /// Window centre (analysis and synthesis).
    offset: usize,
    window: Vec<f32>,
    pub(crate) input: InputBuf,
    pub(crate) output: OutputBuf,
    spectrum: Vec<C>,
    time: Vec<f32>,
}

impl Stft {
    /// Non-RT. `configure(channels, channels, block, extra_history)` +
    /// `setInterval(interval, kaiser)`.
    pub(crate) fn new(
        channels: usize,
        block: usize,
        extra_history: usize,
        interval: usize,
    ) -> Self {
        let fft_size = fast_size_above(block.div_ceil(2).max(1)) * 2;
        let bins = fft_size / 2;
        let input_len = block + extra_history;
        let mut window = vec![0.0f32; block];
        kaiser_fill(&mut window, block as f64 / interval as f64);
        force_perfect_reconstruction(&mut window, interval);
        let mut s = Self {
            fft: ModifiedRealFft::new(fft_size),
            channels,
            block,
            fft_size,
            bins,
            input_len,
            interval,
            offset: block / 2,
            window,
            input: InputBuf {
                pos: 0,
                buf: vec![0.0; input_len * channels],
            },
            output: OutputBuf {
                pos: 0,
                buf: vec![0.0; block * channels],
                window_products: vec![0.0; block],
            },
            spectrum: vec![C::default(); bins * channels],
            time: vec![0.0; fft_size],
        };
        s.reset(1.0);
        s
    }

    pub(crate) fn block(&self) -> usize {
        self.block
    }
    pub(crate) fn fft_size(&self) -> usize {
        self.fft_size
    }
    pub(crate) fn interval(&self) -> usize {
        self.interval
    }
    pub(crate) fn bands(&self) -> usize {
        self.bins
    }
    pub(crate) fn analysis_latency(&self) -> usize {
        self.block - self.offset
    }
    pub(crate) fn synthesis_latency(&self) -> usize {
        self.offset
    }
    pub(crate) fn bin_to_freq(&self, b: f32) -> f32 {
        (b + 0.5) / self.fft_size as f32
    }
    pub(crate) fn freq_to_bin(&self, f: f32) -> f32 {
        f * self.fft_size as f32 - 0.5
    }

    pub(crate) fn spectrum(&mut self, channel: usize) -> &mut [C] {
        &mut self.spectrum[channel * self.bins..(channel + 1) * self.bins]
    }

    /// RT.
    pub(crate) fn reset(&mut self, product_weight: f32) {
        self.input.pos = self.block % self.input_len;
        self.output.pos = 0;
        self.input.buf.fill(0.0);
        self.output.buf.fill(0.0);
        self.spectrum.fill(C::default());
        self.output.window_products.fill(0.0);
        self.add_window_product();
        let wp = &mut self.output.window_products;
        for i in (0..self.block.saturating_sub(self.interval)).rev() {
            wp[i] += wp[i + self.interval];
        }
        for v in wp.iter_mut() {
            *v = *v * product_weight + ALMOST_ZERO;
        }
        self.move_output(self.interval);
    }

    /// RT. Write `value` at `offset` past the input position of `channel`.
    #[inline]
    pub(crate) fn write_input(&mut self, channel: usize, offset: usize, value: f32) {
        let i = (self.input.pos + offset) % self.input_len;
        self.input.buf[channel * self.input_len + i] = value;
    }

    pub(crate) fn move_input(&mut self, samples: usize) {
        self.input.pos = (self.input.pos + samples) % self.input_len;
    }

    pub(crate) fn move_output(&mut self, samples: usize) {
        self.output.advance(samples, self.channels, self.block);
    }

    /// RT. Analyse `channel` of `input` (a saved input state), `samples_in_past` before its
    /// write position, into [`Self::spectrum`].
    pub(crate) fn analyse(&mut self, channel: usize, input: &InputBuf, samples_in_past: usize) {
        let len = self.input_len;
        let off = self.offset;
        let offset_pos = (len * 2 + input.pos - self.block - samples_in_past) % len;
        let buf = &input.buf[channel * len..(channel + 1) * len];
        let fft_size = self.fft_size;
        for i in 0..off {
            let w = -self.window[i];
            self.time[i + fft_size - off] = buf[(offset_pos + i) % len] * w;
        }
        for i in off..self.block {
            self.time[i - off] = buf[(offset_pos + i) % len] * self.window[i];
        }
        self.time[self.block - off..fft_size - off].fill(0.0);
        let bins = self.bins;
        self.fft.fft(
            &self.time,
            &mut self.spectrum[channel * bins..(channel + 1) * bins],
        );
    }

    /// RT. One synthesis step: `step` = channel (step 0 also adds the window product).
    pub(crate) fn synthesise_step(&mut self, step: usize) {
        if step == 0 {
            self.add_window_product();
        }
        let channel = step;
        let bins = self.bins;
        self.fft.ifft(
            &self.spectrum[channel * bins..(channel + 1) * bins],
            &mut self.time,
        );
        let block = self.block;
        let off = self.offset;
        let fft_size = self.fft_size;
        let pos = self.output.pos;
        let buf = &mut self.output.buf[channel * block..(channel + 1) * block];
        for i in 0..off {
            let w = -self.window[i];
            buf[(pos + i) % block] += self.time[i + fft_size - off] * w;
        }
        for i in off..block {
            buf[(pos + i) % block] += self.time[i - off] * self.window[i];
        }
    }

    fn add_window_product(&mut self) {
        let scale = self.fft_size as f32;
        let pos = self.output.pos;
        for (i, w) in self.window.iter().enumerate() {
            self.output.window_products[(pos + i) % self.block] += w * w * scale;
        }
    }
}

fn bessel0(x: f64) -> f64 {
    let mut result = 0.0;
    let mut term = 1.0;
    let mut m = 0.0;
    while term > 1e-4 {
        result += term;
        m += 1.0;
        term *= (x * x) / (4.0 * m * m);
    }
    result
}

/// `Kaiser::withBandwidth(bandwidth, true).fill(...)`.
fn kaiser_fill(data: &mut [f32], bandwidth: f64) {
    let bw = bandwidth
        + 8.0 / ((bandwidth + 3.0) * (bandwidth + 3.0))
        + 0.25 * (3.0 - bandwidth).max(0.0);
    let bw = bw.max(2.0);
    let beta = (bw * bw * 0.25 - 1.0).sqrt() * std::f64::consts::PI;
    let inv_b0 = 1.0 / bessel0(beta);
    let inv_size = 1.0 / data.len() as f64;
    for (i, d) in data.iter_mut().enumerate() {
        let r = (2.0 * i as f64 + 1.0) * inv_size - 1.0;
        let arg = (1.0 - r * r).sqrt();
        *d = (bessel0(beta * arg) * inv_b0) as f32;
    }
}

fn force_perfect_reconstruction(data: &mut [f32], interval: usize) {
    for i in 0..interval.min(data.len()) {
        let sum2: f64 = data[i..]
            .iter()
            .step_by(interval)
            .map(|&v| v as f64 * v as f64)
            .sum();
        let factor = 1.0 / sum2.sqrt();
        for v in data[i..].iter_mut().step_by(interval) {
            *v = (*v as f64 * factor) as f32;
        }
    }
}
