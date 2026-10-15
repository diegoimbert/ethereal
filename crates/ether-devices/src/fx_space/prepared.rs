//! IRs prepared on one thread and assembled into a [`Convolver`] on another in bounded steps
//! (#514): the web's controller Worker reads, normalizes and partitions the IR
//! ([`PreparedIr`], the expensive part), streams it to the AudioWorklet as flat `f32`
//! segments, and the Worklet copies them into a [`ConvolverAssembly`] and zero-fills the
//! rest, a bounded amount per render quantum, so no quantum does more than a copy of a few
//! hundred KiB. The result is the same convolver [`Convolver::new`] builds.
//!
//! # Stream
//!
//! [`SEGMENTS`] segments, in order; segment `2 * kind + channel` with kinds base IR (the
//! normalized time-domain IR, `frames` values; empty for channel 1 of a mono IR), head taps
//! (`B` values), stage-1 spectra (`p1` partitions of `B + 1` bins) and stage-2 spectra
//! (`p2` partitions of `T + 1` bins); spectra are `re, im` pairs. `p1` and `p2` follow from
//! the [`IrHeader`] (kernel length of `frames` shaped by `shaping`).

use realfft::num_complex::Complex32;

use super::convolver::{B, Convolver, Ffts, Kernel, P1, T, build_task, parts, prefault, retarget};
use super::ir::{IrBase, IrInput, MAX_IR_SECONDS, Shaping};

/// Segments of a prepared IR stream.
pub const SEGMENTS: usize = 8;
const BASE: usize = 0;
const HEAD: usize = 1;
const H1: usize = 2;
const H2: usize = 3;

/// What a [`ConvolverAssembly`] needs to allocate before the stream arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrHeader {
    /// Channels of the base IR (1 or 2).
    pub channels: u8,
    /// Frames of the base IR.
    pub frames: u32,
    /// Shaping the kernel was built for.
    pub shaping: Shaping,
}

impl IrHeader {
    fn kernel_parts(&self) -> (usize, usize, usize) {
        let len = shaped_len(self.frames as usize, self.shaping);
        let (p1, p2) = parts(len);
        (len, p1, p2)
    }

    /// `f32` values in segment `seg` (`< SEGMENTS`).
    pub fn segment_len(&self, seg: usize) -> usize {
        let (_, p1, p2) = self.kernel_parts();
        let ch = seg % 2;
        match seg / 2 {
            BASE if ch < self.channels as usize => self.frames as usize,
            BASE => 0,
            HEAD => B,
            H1 => 2 * p1 * (B + 1),
            H2 => 2 * p2 * (T + 1),
            _ => 0,
        }
    }

    /// `f32` values in the whole stream.
    pub fn stream_len(&self) -> usize {
        (0..SEGMENTS).map(|s| self.segment_len(s)).sum()
    }
}

/// `IrBase::shaped_len` from a length alone.
fn shaped_len(len: usize, shaping: Shaping) -> usize {
    IrBase {
        channels: Vec::new(),
        len,
    }
    .shaped_len(shaping)
}

fn as_f32(c: &[Complex32]) -> &[f32] {
    // SAFETY: `Complex32` is `#[repr(C)] { re: f32, im: f32 }`: `2 * len` `f32`s.
    unsafe { std::slice::from_raw_parts(c.as_ptr().cast::<f32>(), c.len() * 2) }
}

/// Non-RT. An IR read, normalized and partitioned for one shaping, ready to stream.
pub struct PreparedIr {
    base: IrBase,
    kernel: Kernel,
}

impl PreparedIr {
    /// Non-RT. Read `input` at `sample_rate` and build its kernel for `shaping` (what
    /// [`Convolver::new`] does first). `None` when the IR can't be read.
    pub fn new(input: &IrInput, sample_rate: f32, shaping: Shaping) -> Option<Self> {
        let base = IrBase::load(input, sample_rate)?;
        let (_, p2) = parts(base.shaped_len(shaping));
        let mut kernel = Kernel::with_capacity(p2);
        retarget(&base, &mut kernel, shaping, p2);
        let mut f = Ffts::new();
        for task in 0..2 * kernel.tasks_per_channel() {
            build_task(&base, &mut kernel, task, &mut f);
        }
        Some(Self { base, kernel })
    }

    pub fn header(&self) -> IrHeader {
        IrHeader {
            channels: self.base.channels.len() as u8,
            frames: self.base.len as u32,
            shaping: self.kernel.shaping,
        }
    }

    /// The values of segment `seg` (`< SEGMENTS`; [`IrHeader::segment_len`] long).
    pub fn segment(&self, seg: usize) -> &[f32] {
        let ch = seg % 2;
        let k = &self.kernel;
        match seg / 2 {
            BASE => self.base.channels.get(ch).map_or(&[], Vec::as_slice),
            HEAD => &k.head[ch],
            H1 => as_f32(&k.h1[ch][..k.p1 * (B + 1)]),
            H2 => as_f32(&k.h2[ch][..k.p2 * (T + 1)]),
            _ => &[],
        }
    }
}

/// Complex buffers of an assembly (two channels each, from these indices): the heard
/// kernel's stage-1 and stage-2 spectra (streamed, then zero-padded to capacity), the spare
/// kernel's (zeros) and, from `K1_H2 + 2`, the stage-2 delay line (zeros).
const K0_H1: usize = 0;
const K0_H2: usize = 2;
const K1_H1: usize = 4;
const K1_H2: usize = 6;
const BUFFERS: usize = 10;

/// Final length of complex buffer `i` for a stage-2 capacity of `cap2` partitions.
fn buffer_len(i: usize, cap2: usize) -> usize {
    match i {
        K0_H1..K0_H2 | K1_H1..K1_H2 => P1 * (B + 1),
        _ => cap2 * (T + 1),
    }
}

/// A [`Convolver`] being assembled from a [`PreparedIr`] stream, in bounded steps, on the
/// thread that will run it. [`Self::begin`] allocates (without touching the memory);
/// [`Self::push`] copies stream values; [`Self::fill`] zero-fills; [`Self::finish`]
/// allocates only the convolver's small signal state.
pub struct ConvolverAssembly {
    input: IrInput,
    header: IrHeader,
    len: usize,
    p1: usize,
    p2: usize,
    cap2: usize,
    base: [Vec<f32>; 2],
    head: [Vec<f32>; 2],
    /// Indexed by `K0_H1 ..`, two channels each.
    complex: [Vec<Complex32>; BUFFERS],
    ffts: Ffts,
    /// Stream position: segment and values received in it.
    seg: usize,
    pos: usize,
}

impl ConvolverAssembly {
    /// Allocate for `header` (IR `input`, engine rate `sample_rate`). Errors for a header
    /// no [`PreparedIr`] at this rate produces (bounds the allocation).
    pub fn begin(input: IrInput, header: IrHeader, sample_rate: f32) -> Result<Self, String> {
        let max = (MAX_IR_SECONDS * sample_rate as f64) as u32;
        let s = header.shaping;
        if !(1..=2).contains(&header.channels)
            || header.frames == 0
            || header.frames > max
            || !(100..=1000).contains(&s.decay)
            || !(500..=1500).contains(&s.size)
        {
            return Err(format!("bad prepared IR header {header:?}"));
        }
        let (len, p1, p2) = header.kernel_parts();
        let (_, cap2) = parts(
            IrBase {
                channels: Vec::new(),
                len: header.frames as usize,
            }
            .max_shaped_len(),
        );
        let frames = header.frames as usize;
        let base_cap = |c: usize| {
            if c < header.channels as usize {
                frames
            } else {
                0
            }
        };
        Ok(Self {
            input,
            header,
            len,
            p1,
            p2: p2.min(cap2),
            cap2,
            base: [
                Vec::with_capacity(base_cap(0)),
                Vec::with_capacity(base_cap(1)),
            ],
            head: [Vec::with_capacity(B), Vec::with_capacity(B)],
            complex: std::array::from_fn(|i| Vec::with_capacity(buffer_len(i, cap2))),
            ffts: Ffts::new(),
            seg: 0,
            pos: 0,
        }
        .skip_empty())
    }

    pub fn header(&self) -> IrHeader {
        self.header
    }

    fn skip_empty(mut self) -> Self {
        self.advance();
        self
    }

    fn advance(&mut self) {
        while self.seg < SEGMENTS && self.pos == self.header.segment_len(self.seg) {
            self.seg += 1;
            self.pos = 0;
        }
    }

    /// Whether the whole stream arrived.
    pub fn received(&self) -> bool {
        self.seg == SEGMENTS
    }

    /// Copy `values` (`f32` LE) at `offset` of segment `seg`: the next values of the
    /// stream (segments arrive in order, each in one or more pieces, spectra in whole
    /// `re, im` pairs).
    pub fn push(&mut self, seg: usize, offset: usize, values: &[u8]) -> Result<(), String> {
        let n = values.len() / 4;
        if seg != self.seg || offset != self.pos || !values.len().is_multiple_of(4) {
            return Err(format!(
                "prepared IR: segment {seg} at {offset} out of order (expected {} at {})",
                self.seg, self.pos
            ));
        }
        if self.pos + n > self.header.segment_len(seg) || (seg / 2 >= H1 && !n.is_multiple_of(2)) {
            return Err(format!("prepared IR: segment {seg} overruns"));
        }
        let ch = seg % 2;
        let mut f = values
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b));
        match seg / 2 {
            BASE => self.base[ch].extend(f),
            HEAD => self.head[ch].extend(f),
            kind => {
                let dst = &mut self.complex[if kind == H1 { K0_H1 } else { K0_H2 } + ch];
                while let (Some(re), Some(im)) = (f.next(), f.next()) {
                    dst.push(Complex32::new(re, im));
                }
            }
        }
        self.pos += n;
        self.advance();
        Ok(())
    }

    /// Zero-fill up to `budget` complex values (the spare kernel and the delay line at
    /// once, the heard kernel's padding once the stream arrived). True when the assembly
    /// is complete ([`Self::finish`]).
    pub fn fill(&mut self, mut budget: usize) -> bool {
        let received = self.received();
        for i in 0..BUFFERS {
            if budget == 0 {
                break;
            }
            if i < K1_H1 && !received {
                continue;
            }
            let v = self.complex[i].len();
            let n = (buffer_len(i, self.cap2) - v).min(budget);
            self.complex[i].resize(v + n, Complex32::new(0.0, 0.0));
            budget -= n;
        }
        self.complete()
    }

    fn complete(&self) -> bool {
        self.received() && (0..BUFFERS).all(|i| self.complex[i].len() == buffer_len(i, self.cap2))
    }

    /// The convolver (allocates its small, IR-independent signal state); `None` (dropping
    /// the assembly) until [`Self::fill`] returned true.
    pub fn finish(self) -> Option<Box<Convolver>> {
        if !self.complete() {
            return None;
        }
        let Self {
            input,
            header,
            len,
            p1,
            p2,
            cap2,
            base,
            head,
            complex,
            ffts,
            ..
        } = self;
        let [
            k0h1a,
            k0h1b,
            k0h2a,
            k0h2b,
            k1h1a,
            k1h1b,
            k1h2a,
            k1h2b,
            fdla,
            fdlb,
        ] = complex;
        let mut channels = Vec::from(base);
        channels.truncate(header.channels as usize);
        let mut head = head;
        for h in &mut head {
            h.resize(B, 0.0);
            prefault(h);
        }
        let kernels = [
            Kernel {
                shaping: header.shaping,
                len,
                head,
                h1: [k0h1a, k0h1b],
                p1,
                h2: [k0h2a, k0h2b],
                p2,
            },
            Kernel {
                shaping: Shaping::default(),
                len: 0,
                head: [vec![0.0; B], vec![0.0; B]],
                h1: [k1h1a, k1h1b],
                p1: 0,
                h2: [k1h2a, k1h2b],
                p2: 0,
            },
        ];
        let base = IrBase {
            channels,
            len: header.frames as usize,
        };
        Some(Box::new(Convolver::from_parts(
            input,
            base,
            kernels,
            cap2,
            [fdla, fdlb],
            ffts,
        )))
    }
}
