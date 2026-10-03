//! Zero-latency, non-uniformly partitioned stereo convolution (two FFT stages behind a
//! direct-form head), allocation-free and with a bounded amount of work per 128 samples.
//!
//! # Layout (per channel, IR `h`, input `x`)
//!
//! | part | IR range | method | output ready |
//! |------|----------|--------|--------------|
//! | head | `[0, B)` | direct FIR (`B` taps, per sample) | immediately |
//! | stage 1 | `[B, 2T)` | uniform partitioned FFT, block `B`, 31 partitions | one `B` block ahead |
//! | stage 2 | `[2T, len)` | uniform partitioned FFT, block `T` | one `T` block ahead |
//!
//! with `B` = 128 and `T` = 2048. Each stage's IR offset equals at least its own buffering
//! delay, so the sum is the exact convolution with **zero latency** (no PDC needed).
//!
//! Stage 2 starts at `2T` (not `T`), which leaves a whole `T` block of slack: its work for
//! one input block is spread over the 16 head ticks of the next block (tick 15: forward
//! FFT; ticks 0..14: the spectral multiply-accumulates, 1/14 of the partitions each;
//! tick 14: inverse FFT). So the cost per 128 samples is flat: no spike every 2048 samples.
//!
//! # Shaping changes
//!
//! `Decay`, `Size` and `Reverse` change the kernel. The convolver owns two kernels sharing
//! one input history (the frequency-domain delay lines): when the target shaping differs,
//! the spare kernel is rebuilt a few partitions per tick ([`BUILD_UNITS_PER_TICK`], bounded,
//! no allocation), then both kernels run for two `T` blocks and the output crossfades over
//! one `T` block (43 ms at 48 kHz). The reverb's tail is continuous through the change since
//! the history is shared.

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use super::ir::{IrBase, IrInput, Shaping};

/// Head block (direct FIR length, stage-1 block).
pub const B: usize = 128;
/// Stage-2 block.
pub const T: usize = 2048;
const TICKS: usize = T / B;
/// Stage-1 partitions (cover `[B, 2T)`).
const P1: usize = (2 * T - B) / B;
/// Ticks of a `T` block that carry stage-2 multiply-accumulates.
const MAC_TICKS: usize = TICKS - 2;
/// Kernel rebuild budget per tick, in units of one `2B` FFT (a stage-2 partition costs
/// [`T2_UNITS`]): about one stage-2 partition per channel per tick.
pub const BUILD_UNITS_PER_TICK: usize = 2 * T2_UNITS;
const T2_UNITS: usize = TICKS;

/// Touch every page of `v` (a fresh `vec![0; n]` is lazily mapped zero memory: its first
/// writes would page-fault on the audio thread).
fn prefault<T: Copy>(v: &mut [T]) {
    let step = (4096 / std::mem::size_of::<T>().max(1)).max(1);
    let mut i = 0;
    while i < v.len() {
        // SAFETY: `i < v.len()`; a volatile write of the same value can't be elided.
        unsafe {
            let p = v.as_mut_ptr().add(i);
            std::ptr::write_volatile(p, *p);
        }
        i += step;
    }
}

/// Stage partition counts for a kernel of `len` samples.
fn parts(len: usize) -> (usize, usize) {
    let p1 = if len > B {
        (len - B).div_ceil(B).min(P1)
    } else {
        0
    };
    let p2 = if len > 2 * T {
        (len - 2 * T).div_ceil(T)
    } else {
        0
    };
    (p1, p2)
}

/// One kernel: the head taps (time-reversed) and the partition spectra of both stages, for
/// both channels, at a capacity fixed by the convolver.
struct Kernel {
    shaping: Shaping,
    len: usize,
    head: [Vec<f32>; 2],
    h1: [Vec<Complex32>; 2],
    p1: usize,
    h2: [Vec<Complex32>; 2],
    p2: usize,
}

impl Kernel {
    fn with_capacity(cap2: usize) -> Self {
        let z = || Complex32::new(0.0, 0.0);
        Self {
            shaping: Shaping::default(),
            len: 0,
            head: [vec![0.0; B], vec![0.0; B]],
            h1: [vec![z(); P1 * (B + 1)], vec![z(); P1 * (B + 1)]],
            p1: 0,
            h2: [vec![z(); cap2 * (T + 1)], vec![z(); cap2 * (T + 1)]],
            p2: 0,
        }
    }

    /// Tasks per channel to build this kernel (head + partitions).
    fn tasks_per_channel(&self) -> usize {
        1 + self.p1 + self.p2
    }
}

/// FFT plans and scratch (allocated once).
struct Ffts {
    fwd1: Arc<dyn RealToComplex<f32>>,
    inv1: Arc<dyn ComplexToReal<f32>>,
    fwd2: Arc<dyn RealToComplex<f32>>,
    inv2: Arc<dyn ComplexToReal<f32>>,
    scratch: Vec<Complex32>,
    time: Vec<f32>,
    spec: Vec<Complex32>,
}

impl Ffts {
    fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd1 = planner.plan_fft_forward(2 * B);
        let inv1 = planner.plan_fft_inverse(2 * B);
        let fwd2 = planner.plan_fft_forward(2 * T);
        let inv2 = planner.plan_fft_inverse(2 * T);
        let scratch_len = [
            fwd1.get_scratch_len(),
            inv1.get_scratch_len(),
            fwd2.get_scratch_len(),
            inv2.get_scratch_len(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        Self {
            fwd1,
            inv1,
            fwd2,
            inv2,
            scratch: vec![Complex32::new(0.0, 0.0); scratch_len],
            time: vec![0.0; 2 * T],
            spec: vec![Complex32::new(0.0, 0.0); T + 1],
        }
    }
}

/// Forward transform of `time` (length `2n`) into `spec` (`n + 1` bins), scaled by `scale`.
fn forward(
    fft: &dyn RealToComplex<f32>,
    time: &mut [f32],
    spec: &mut [Complex32],
    scratch: &mut [Complex32],
) {
    // Lengths always match the plan: the result can't be an error.
    let _ = fft.process_with_scratch(time, spec, scratch);
}

/// Inverse transform of `spec` (destroyed) into `time`.
fn inverse(
    fft: &dyn ComplexToReal<f32>,
    spec: &mut [Complex32],
    time: &mut [f32],
    scratch: &mut [Complex32],
) {
    // A real signal's DC and Nyquist bins are real; clear rounding residue so realfft
    // doesn't flag them (it computes the transform either way).
    if let Some(first) = spec.first_mut() {
        first.im = 0.0;
    }
    if let Some(last) = spec.last_mut() {
        last.im = 0.0;
    }
    let _ = fft.process_with_scratch(spec, time, scratch);
}

/// `Σ a[i]·b[i]` with eight independent accumulators (vectorizes; `len % 8 == 0`).
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    for (x, y) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
        for l in 0..8 {
            acc[l] += x[l] * y[l];
        }
    }
    (acc[0] + acc[4]) + (acc[1] + acc[5]) + (acc[2] + acc[6]) + (acc[3] + acc[7])
}

/// `acc += x·h` (complex, element-wise).
#[inline]
fn cmac(acc: &mut [Complex32], x: &[Complex32], h: &[Complex32]) {
    for ((a, x), h) in acc.iter_mut().zip(x).zip(h) {
        a.re += x.re * h.re - x.im * h.im;
        a.im += x.re * h.im + x.im * h.re;
    }
}

/// Build task `task` (head, then stage-1, then stage-2 partitions, channel by channel) of
/// `k` from `base`. Returns its cost in units.
fn build_task(base: &IrBase, k: &mut Kernel, task: usize, f: &mut Ffts) -> usize {
    let per = k.tasks_per_channel();
    let (ch, j) = (task / per, task % per);
    let (sh, len) = (k.shaping, k.len);
    if j == 0 {
        let buf = &mut f.time[..B];
        base.shaped(ch, sh, len, 0, buf);
        for (q, tap) in k.head[ch].iter_mut().enumerate() {
            *tap = buf[B - 1 - q];
        }
        1
    } else if j <= k.p1 {
        let p = j - 1;
        let time = &mut f.time[..2 * B];
        base.shaped(ch, sh, len, B + p * B, &mut time[..B]);
        time[B..].fill(0.0);
        let spec = &mut f.spec[..=B];
        forward(&*f.fwd1, time, spec, &mut f.scratch);
        let scale = 1.0 / (2 * B) as f32;
        for (d, s) in k.h1[ch][p * (B + 1)..(p + 1) * (B + 1)]
            .iter_mut()
            .zip(spec.iter())
        {
            *d = s * scale;
        }
        1
    } else {
        let p = j - 1 - k.p1;
        let time = &mut f.time[..2 * T];
        base.shaped(ch, sh, len, 2 * T + p * T, &mut time[..T]);
        time[T..].fill(0.0);
        let spec = &mut f.spec[..=T];
        forward(&*f.fwd2, time, spec, &mut f.scratch);
        let scale = 1.0 / (2 * T) as f32;
        for (d, s) in k.h2[ch][p * (T + 1)..(p + 1) * (T + 1)]
            .iter_mut()
            .zip(spec.iter())
        {
            *d = s * scale;
        }
        T2_UNITS
    }
}

/// Point `k` at `shaping` (length and partition counts; contents rebuilt by tasks).
fn retarget(base: &IrBase, k: &mut Kernel, shaping: Shaping, cap2: usize) {
    k.shaping = shaping;
    k.len = base.shaped_len(shaping);
    let (p1, p2) = parts(k.len);
    k.p1 = p1;
    k.p2 = p2.min(cap2);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// One kernel runs.
    Idle,
    /// The spare kernel is being rebuilt (`next_task`).
    Building,
    /// Built; waiting for a `T` boundary.
    Ready,
    /// Both kernels run for one `T` block (fills the spare's output pipeline).
    Armed,
    /// Both run; the output crossfades to the spare over one `T` block.
    Fading,
}

/// A stereo convolver for one IR (see the module docs). Built off the audio thread; every
/// method but [`Convolver::new`] is real-time safe.
pub struct Convolver {
    pub(crate) input: IrInput,
    base: IrBase,
    kernels: [Kernel; 2],
    active: usize,
    cap2: usize,
    f: Ffts,
    /// Position in the current `B` block, and `B` block index in the current `T` block.
    fill: usize,
    tick: usize,
    win1: [Vec<f32>; 2],
    fdl1: [Vec<Complex32>; 2],
    pos1: usize,
    /// `[kernel][channel]` stage-1 output for the current `B` block.
    out1: [[Vec<f32>; 2]; 2],
    acc1: Vec<Complex32>,
    win2: [Vec<f32>; 2],
    fdl2: [Vec<Complex32>; 2],
    pos2: usize,
    acc2: [[Vec<Complex32>; 2]; 2],
    /// `[kernel][channel]` stage-2 output for the current / next `T` block.
    out2: [[Vec<f32>; 2]; 2],
    next2: [[Vec<f32>; 2]; 2],
    target: Shaping,
    phase: Phase,
    next_task: usize,
    fade_pos: usize,
}

impl Convolver {
    /// Non-RT. A convolver for `base` with its kernel built for `shaping`.
    pub fn new(input: IrInput, base: IrBase, shaping: Shaping) -> Self {
        let (_, cap2) = parts(base.max_shaped_len());
        let mut kernels = [Kernel::with_capacity(cap2), Kernel::with_capacity(cap2)];
        let mut f = Ffts::new();
        retarget(&base, &mut kernels[0], shaping, cap2);
        for task in 0..2 * kernels[0].tasks_per_channel() {
            build_task(&base, &mut kernels[0], task, &mut f);
        }
        let z = Complex32::new(0.0, 0.0);
        let v = |n: usize| {
            let mut a = [vec![0.0f32; n], vec![0.0f32; n]];
            a.iter_mut().for_each(|x| prefault(x));
            a
        };
        let c = |n: usize| {
            let mut a = [vec![z; n], vec![z; n]];
            a.iter_mut().for_each(|x| prefault(x));
            a
        };
        for k in &mut kernels {
            for ch in 0..2 {
                prefault(&mut k.h1[ch]);
                prefault(&mut k.h2[ch]);
            }
        }
        Self {
            input,
            base,
            kernels,
            active: 0,
            cap2,
            f,
            fill: 0,
            tick: 0,
            win1: v(2 * B),
            fdl1: c(P1 * (B + 1)),
            pos1: 0,
            out1: [v(B), v(B)],
            acc1: vec![z; B + 1],
            win2: v(2 * T),
            fdl2: c(cap2 * (T + 1)),
            pos2: 0,
            acc2: [c(T + 1), c(T + 1)],
            out2: [v(T), v(T)],
            next2: [v(T), v(T)],
            target: shaping,
            phase: Phase::Idle,
            next_task: 0,
            fade_pos: 0,
        }
    }

    /// Shaping of the kernel currently heard.
    pub fn shaping(&self) -> Shaping {
        self.kernels[self.active].shaping
    }

    /// Length in samples of the kernel currently heard.
    pub fn kernel_len(&self) -> usize {
        self.kernels[self.active].len
    }

    /// Bytes held by this convolver (kernels, delay lines, buffers), for reports.
    pub fn memory_bytes(&self) -> usize {
        let k = 2 * (B + P1 * (B + 1) * 2 + self.cap2 * (T + 1) * 2) * 4;
        let state = 2 * (2 * B + P1 * (B + 1) * 2 + 2 * B + 2 * T + self.cap2 * (T + 1) * 2) * 4
            + 2 * 2 * (T + 1) * 8
            + 2 * 2 * 2 * T * 4;
        2 * k + state + self.base.len * self.base.channels.len() * 4
    }

    /// RT. Ask for `shaping`: rebuilt in the background (bounded per tick) and crossfaded.
    pub fn set_target(&mut self, shaping: Shaping) {
        self.target = shaping;
    }

    /// RT. Whether the heard kernel matches the target and no change is in flight.
    pub fn settled(&self) -> bool {
        self.phase == Phase::Idle && self.target == self.shaping()
    }

    /// RT. For a convolver that isn't playing yet: rebuild towards the target with at most
    /// `units` of work and switch kernels without a fade. True once settled.
    pub fn prepare_idle(&mut self, units: usize) -> bool {
        if self.phase == Phase::Idle && self.target != self.shaping() {
            self.start_build();
        }
        if self.phase == Phase::Building {
            self.build(units);
        }
        if self.phase == Phase::Ready {
            self.active ^= 1;
            self.phase = Phase::Idle;
        }
        self.settled()
    }

    /// RT. Clear the signal state (the kernels stay; a crossfade in flight completes).
    pub fn reset(&mut self) {
        for c in 0..2 {
            self.win1[c].fill(0.0);
            self.win2[c].fill(0.0);
            self.fdl1[c].fill(Complex32::new(0.0, 0.0));
            self.fdl2[c].fill(Complex32::new(0.0, 0.0));
            for k in 0..2 {
                self.out1[k][c].fill(0.0);
                self.out2[k][c].fill(0.0);
                self.next2[k][c].fill(0.0);
                self.acc2[k][c].fill(Complex32::new(0.0, 0.0));
            }
        }
        self.fill = 0;
        self.tick = 0;
        self.pos1 = 0;
        self.pos2 = 0;
        if matches!(self.phase, Phase::Ready | Phase::Armed | Phase::Fading) {
            self.active ^= 1;
            self.phase = Phase::Idle;
        }
    }

    fn start_build(&mut self) {
        let other = self.active ^ 1;
        retarget(&self.base, &mut self.kernels[other], self.target, self.cap2);
        self.next_task = 0;
        self.phase = Phase::Building;
    }

    fn build(&mut self, mut units: usize) {
        let other = self.active ^ 1;
        let total = 2 * self.kernels[other].tasks_per_channel();
        while units > 0 && self.next_task < total {
            let cost = build_task(
                &self.base,
                &mut self.kernels[other],
                self.next_task,
                &mut self.f,
            );
            self.next_task += 1;
            units = units.saturating_sub(cost);
        }
        if self.next_task >= total {
            self.phase = Phase::Ready;
        }
    }

    /// RT. Convolve `n` frames of `input` (2 channels) into `output`.
    pub fn process(&mut self, input: [&[f32]; 2], output: [&mut [f32]; 2], n: usize) {
        let [out_l, out_r] = output;
        let outs = [out_l, out_r];
        let mut i = 0;
        while i < n {
            let m = (n - i).min(B - self.fill);
            let a = self.active;
            let fading = self.phase == Phase::Fading;
            let t_off = self.tick * B;
            for c in 0..2 {
                let start = B + self.fill;
                self.win1[c][start..start + m].copy_from_slice(&input[c][i..i + m]);
                let w2 = T + t_off + self.fill;
                self.win2[c][w2..w2 + m].copy_from_slice(&input[c][i..i + m]);
                let w = &self.win1[c];
                let out = &mut outs[c][i..i + m];
                for (s, o) in out.iter_mut().enumerate() {
                    let pos = self.fill + s;
                    let seg = &w[pos + 1..pos + 1 + B];
                    let ya = dot(&self.kernels[a].head[c], seg)
                        + self.out1[a][c][pos]
                        + self.out2[a][c][t_off + pos];
                    *o = if fading {
                        let b = a ^ 1;
                        let yb = dot(&self.kernels[b].head[c], seg)
                            + self.out1[b][c][pos]
                            + self.out2[b][c][t_off + pos];
                        let g = (self.fade_pos + s + 1) as f32 / T as f32;
                        ya + (yb - ya) * g
                    } else {
                        ya
                    };
                }
            }
            if fading {
                self.fade_pos += m;
            }
            self.fill += m;
            i += m;
            if self.fill == B {
                self.on_tick();
                self.fill = 0;
            }
        }
    }

    /// The end of a `B` block: stage 1, this tick's share of stage 2, rebuild work and
    /// phase changes.
    fn on_tick(&mut self) {
        let t = self.tick;
        let dual = matches!(self.phase, Phase::Armed | Phase::Fading);
        let kernels: &[usize] = if dual {
            if self.active == 0 { &[0, 1] } else { &[1, 0] }
        } else if self.active == 0 {
            &[0]
        } else {
            &[1]
        };

        // Stage 1.
        self.pos1 = (self.pos1 + 1) % P1;
        for c in 0..2 {
            let time = &mut self.f.time[..2 * B];
            time.copy_from_slice(&self.win1[c]);
            let slot = &mut self.fdl1[c][self.pos1 * (B + 1)..(self.pos1 + 1) * (B + 1)];
            forward(&*self.f.fwd1, time, slot, &mut self.f.scratch);
            for &k in kernels {
                let kern = &self.kernels[k];
                if kern.p1 == 0 {
                    self.out1[k][c].fill(0.0);
                    continue;
                }
                self.acc1.fill(Complex32::new(0.0, 0.0));
                for p in 0..kern.p1 {
                    let s = (self.pos1 + P1 - p) % P1;
                    cmac(
                        &mut self.acc1,
                        &self.fdl1[c][s * (B + 1)..(s + 1) * (B + 1)],
                        &kern.h1[c][p * (B + 1)..(p + 1) * (B + 1)],
                    );
                }
                let time = &mut self.f.time[..2 * B];
                inverse(&*self.f.inv1, &mut self.acc1, time, &mut self.f.scratch);
                self.out1[k][c].copy_from_slice(&time[B..]);
            }
            self.win1[c].copy_within(B.., 0);
        }

        // Stage 2.
        if self.cap2 > 0 {
            if t == TICKS - 1 {
                std::mem::swap(&mut self.out2, &mut self.next2);
                self.pos2 = (self.pos2 + 1) % self.cap2;
                for c in 0..2 {
                    let time = &mut self.f.time[..2 * T];
                    time.copy_from_slice(&self.win2[c]);
                    let slot = &mut self.fdl2[c][self.pos2 * (T + 1)..(self.pos2 + 1) * (T + 1)];
                    forward(&*self.f.fwd2, time, slot, &mut self.f.scratch);
                    self.win2[c].copy_within(T.., 0);
                }
            } else if t < MAC_TICKS {
                for &k in kernels {
                    let kern = &self.kernels[k];
                    let chunk = kern.p2.div_ceil(MAC_TICKS);
                    let (lo, hi) = ((t * chunk).min(kern.p2), ((t + 1) * chunk).min(kern.p2));
                    for c in 0..2 {
                        let acc = &mut self.acc2[k][c];
                        if t == 0 {
                            acc.fill(Complex32::new(0.0, 0.0));
                        }
                        for p in lo..hi {
                            let s = (self.pos2 + self.cap2 - p) % self.cap2;
                            cmac(
                                acc,
                                &self.fdl2[c][s * (T + 1)..(s + 1) * (T + 1)],
                                &kern.h2[c][p * (T + 1)..(p + 1) * (T + 1)],
                            );
                        }
                    }
                }
            } else {
                for &k in kernels {
                    for c in 0..2 {
                        let time = &mut self.f.time[..2 * T];
                        inverse(
                            &*self.f.inv2,
                            &mut self.acc2[k][c],
                            time,
                            &mut self.f.scratch,
                        );
                        self.next2[k][c].copy_from_slice(&time[T..]);
                    }
                }
            }
        }

        // Shaping changes.
        if self.phase == Phase::Idle && self.target != self.shaping() {
            self.start_build();
        }
        if self.phase == Phase::Building {
            self.build(BUILD_UNITS_PER_TICK);
        }
        if t == TICKS - 1 {
            match self.phase {
                Phase::Fading => {
                    self.active ^= 1;
                    self.phase = Phase::Idle;
                }
                Phase::Armed => {
                    self.phase = Phase::Fading;
                    self.fade_pos = 0;
                }
                Phase::Ready => self.phase = Phase::Armed,
                Phase::Idle | Phase::Building => {}
            }
        }
        self.tick = (t + 1) % TICKS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(ir: Vec<f32>) -> IrBase {
        // Bypass normalization: tests compare against a direct convolution of `ir`.
        IrBase {
            len: ir.len(),
            channels: vec![ir],
        }
    }

    fn direct(x: &[f32], h: &[f32]) -> Vec<f32> {
        let mut y = vec![0.0f32; x.len()];
        for (n, yn) in y.iter_mut().enumerate() {
            let mut acc = 0.0f64;
            for (j, &hj) in h.iter().enumerate().take(n + 1) {
                acc += hj as f64 * x[n - j] as f64;
            }
            *yn = acc as f32;
        }
        y
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
            })
            .collect()
    }

    fn run(conv: &mut Convolver, x: &[f32], blocks: &[usize]) -> Vec<f32> {
        let mut y = vec![0.0f32; x.len()];
        let mut yr = vec![0.0f32; x.len()];
        let mut i = 0;
        let mut b = 0;
        while i < x.len() {
            let n = blocks[b % blocks.len()].min(x.len() - i);
            let (l, r) = (&mut y[i..i + n], &mut yr[i..i + n]);
            conv.process([&x[i..i + n], &x[i..i + n]], [l, r], n);
            i += n;
            b += 1;
        }
        assert_eq!(y, yr, "mono IR: both channels equal");
        y
    }

    #[test]
    fn matches_direct_convolution_with_zero_latency() {
        for len in [1usize, 100, 128, 129, 3000, 4096, 4097, 9000, 20_000] {
            let h = noise(len, len as u32);
            let x = noise(30_000, 7);
            let want = direct(&x, &h);
            let mut conv = Convolver::new(IrInput::Factory(0), base(h.clone()), Shaping::default());
            let got = run(&mut conv, &x, &[64, 1, 333, 128, 4096, 7]);
            let peak = want.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let err = want
                .iter()
                .zip(&got)
                .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
            assert!(
                err <= 1e-4 * peak.max(1.0),
                "len {len}: err {err} (peak {peak})"
            );
        }
    }

    #[test]
    fn impulse_reproduces_the_ir() {
        let h = noise(10_000, 3);
        let mut x = vec![0.0f32; 16_000];
        x[0] = 1.0;
        let mut conv = Convolver::new(IrInput::Factory(0), base(h.clone()), Shaping::default());
        let got = run(&mut conv, &x, &[512]);
        for (i, (a, b)) in h.iter().zip(&got).enumerate() {
            assert!((a - b).abs() < 1e-5, "sample {i}: {a} vs {b}");
        }
        assert!(got[10_000..].iter().all(|v| v.abs() < 1e-5));
    }

    #[test]
    fn shaping_change_rebuilds_and_crossfades() {
        let h = noise(40_000, 5);
        let x = noise(120_000, 9);
        let mut conv = Convolver::new(IrInput::Factory(0), base(h.clone()), Shaping::default());
        let rev = Shaping {
            reverse: true,
            ..Shaping::default()
        };
        // Settle on the reversed kernel, then compare a late window with the direct result.
        conv.set_target(rev);
        let got = run(&mut conv, &x, &[256]);
        assert!(conv.settled());
        assert_eq!(conv.shaping(), rev);
        let hr: Vec<f32> = h.iter().rev().copied().collect();
        let want = direct(&x, &hr);
        let tail = 80_000..120_000;
        let peak = want[tail.clone()]
            .iter()
            .fold(0.0f32, |m, v| m.max(v.abs()));
        let err = want[tail.clone()]
            .iter()
            .zip(&got[tail])
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(err <= 1e-4 * peak, "err {err}");
        // No discontinuity: max sample-to-sample step stays in the range of the signal's.
        let step = |y: &[f32]| y.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(step(&got) < 1.5 * step(&want).max(step(&direct(&x, &h))));
    }

    #[test]
    fn decay_shortens_and_size_stretches() {
        let ir = IrBase {
            len: 48_000,
            channels: vec![vec![1.0; 48_000]],
        };
        let s = Shaping {
            decay: 500,
            size: 1500,
            reverse: false,
        };
        assert_eq!(ir.shaped_len(s), 36_000);
        let mut out = vec![0.0f32; 36_001];
        ir.shaped(0, s, 36_000, 0, &mut out);
        assert!(out[35_999].abs() < 1e-3, "faded out at the end");
        assert_eq!(out[36_000], 0.0);
        assert!(
            (out[0] - 1.0 / 1.5f32.sqrt()).abs() < 1e-6,
            "stretch keeps energy"
        );
    }
}
