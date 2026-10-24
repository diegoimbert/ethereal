//! Incremental sample-rate conversion, stepped from `tick()` a budget at a time.
//!
//! Identical algorithm and alignment to `ether_media::resample` (`ether_media::sinc_resampler`:
//! sinc 256 / oversampling 1024 / Blackman-Harris², exact time alignment by the padding
//! and dropping of `ether_media::alignment` for `ratio = p / q`), so the result matches
//! the one-shot version; only the work is split across calls.

use std::sync::Arc;

use ether_media::{
    DecodedAudio, MediaError, alignment, process_chunk, resampled_len, resampler_buffers,
    sinc_resampler,
};
use rubato::{Async, Resampler};

const CHUNK: usize = 1024;

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

struct Running {
    resampler: Async<f32>,
    pad: usize,
    drop: usize,
    total_in: usize,
    pos: usize,
    inbuf: Vec<Vec<f32>>,
    outbuf: Vec<Vec<f32>>,
}

impl Running {
    /// The resampler and its alignment for `frames` input frames.
    fn new(
        source_rate: u32,
        target_rate: u32,
        n_ch: usize,
        frames: usize,
    ) -> Result<Self, MediaError> {
        let ratio = target_rate as f64 / source_rate as f64;
        let resampler = sinc_resampler(ratio, CHUNK, n_ch)?;
        let g = gcd(target_rate, source_rate);
        let (p, q) = ((target_rate / g) as usize, (source_rate / g) as usize);
        let (pad, drop) = alignment(p, q);
        let (inbuf, outbuf) = resampler_buffers(&resampler);
        Ok(Self {
            resampler,
            pad,
            drop,
            total_in: pad + frames,
            pos: 0,
            inbuf,
            outbuf,
        })
    }
}

pub struct IncrementalResampler {
    source: Arc<DecodedAudio>,
    target_rate: u32,
    expected: usize,
    out: Vec<Vec<f32>>,
    running: Option<Running>,
    done: bool,
}

impl IncrementalResampler {
    pub fn new(source: Arc<DecodedAudio>, target_rate: u32) -> Result<Self, MediaError> {
        ether_media::check_rate(target_rate)?;
        ether_media::check_rate(source.sample_rate)?;
        let frames = source.frames();
        let n_ch = source.channels.len();
        let expected = resampled_len(frames, source.sample_rate, target_rate);
        let trivial = source.sample_rate == target_rate || frames == 0 || n_ch == 0;
        let running = if trivial {
            None
        } else {
            Some(Running::new(source.sample_rate, target_rate, n_ch, frames)?)
        };
        let out = match &running {
            Some(r) => (0..n_ch)
                .map(|_| Vec::with_capacity(r.drop + expected + r.resampler.output_frames_max()))
                .collect(),
            None => Vec::new(),
        };
        Ok(Self {
            source,
            target_rate,
            expected,
            out,
            running,
            done: trivial,
        })
    }

    /// Fraction done, 0..=1.
    pub fn progress(&self) -> f32 {
        match &self.running {
            _ if self.done => 1.0,
            Some(r) => (r.pos as f32 / r.total_in.max(1) as f32).min(1.0),
            None => 1.0,
        }
    }

    /// Process about `budget` input frames. Returns `true` when done.
    pub fn step(&mut self, budget: usize) -> Result<bool, MediaError> {
        let Some(r) = self.running.as_mut() else {
            return Ok(true);
        };
        let stop_at = r.pos.saturating_add(budget.max(1));
        while self.out[0].len() < r.drop + self.expected {
            if r.pos >= stop_at {
                return Ok(false);
            }
            let need = r.resampler.input_frames_next();
            for (dst, src) in r.inbuf.iter_mut().zip(&self.source.channels) {
                dst.resize(need, 0.0);
                for (i, d) in dst.iter_mut().enumerate() {
                    let v = r.pos + i;
                    *d = if v >= r.pad && v < r.total_in {
                        src[v - r.pad]
                    } else {
                        0.0
                    };
                }
            }
            let (_, n) = process_chunk(&mut r.resampler, &r.inbuf, &mut r.outbuf)?;
            if n == 0 && r.pos > r.total_in + 4 * CHUNK {
                break;
            }
            for (o, b) in self.out.iter_mut().zip(&r.outbuf) {
                o.extend_from_slice(&b[..n]);
            }
            r.pos += need;
        }
        self.done = true;
        Ok(true)
    }

    pub fn finish(self) -> DecodedAudio {
        match self.running {
            None => {
                let mut audio = Arc::unwrap_or_clone(self.source);
                audio.sample_rate = self.target_rate;
                audio
            }
            Some(r) => {
                let mut out = self.out;
                for ch in &mut out {
                    ch.drain(..r.drop.min(ch.len()));
                    ch.resize(self.expected, 0.0);
                }
                DecodedAudio {
                    sample_rate: self.target_rate,
                    channels: out,
                }
            }
        }
    }
}

/// [`IncrementalResampler`] over a signal that arrives block by block (an export
/// render): the same chunks with the same zero padding go through the same resampler, so the
/// output is identical, but only about one chunk of input is held at a time. The signal's
/// length must be known up front (it sets the alignment and the output length).
pub struct StreamResampler {
    /// `None` for an empty signal.
    running: Option<Running>,
    frames: usize,
    /// Output frames of the whole signal.
    expected: usize,
    /// Input frames received so far.
    received: usize,
    /// Received input not consumed yet; `held[_][0]` is input frame `first`.
    held: Vec<Vec<f32>>,
    first: usize,
    /// Output frames the resampler produced so far (the alignment drop included).
    produced: usize,
}

impl StreamResampler {
    pub fn new(
        source_rate: u32,
        target_rate: u32,
        channels: usize,
        frames: usize,
    ) -> Result<Self, MediaError> {
        ether_media::check_rate(target_rate)?;
        ether_media::check_rate(source_rate)?;
        let running = if frames == 0 || channels == 0 {
            None
        } else {
            Some(Running::new(source_rate, target_rate, channels, frames)?)
        };
        Ok(Self {
            expected: if running.is_some() {
                resampled_len(frames, source_rate, target_rate)
            } else {
                0
            },
            running,
            frames,
            received: 0,
            held: vec![Vec::new(); channels],
            first: 0,
            produced: 0,
        })
    }

    /// Output frames for the whole signal.
    pub fn expected(&self) -> usize {
        self.expected
    }

    /// Take the next input frames (`block`: one slice per channel, all of one length) and
    /// append the output they complete to `out`.
    pub fn push(&mut self, block: &[&[f32]], out: &mut [Vec<f32>]) -> Result<(), MediaError> {
        let n = block.first().map_or(0, |c| c.len());
        if self.received + n > self.frames {
            return Err(MediaError::Resample("more input than announced".into()));
        }
        for (h, b) in self.held.iter_mut().zip(block) {
            h.extend_from_slice(b);
        }
        self.received += n;
        self.run(out)
    }

    /// After the whole signal was pushed: append the rest of the output to `out`.
    pub fn finish(&mut self, out: &mut [Vec<f32>]) -> Result<(), MediaError> {
        if self.received != self.frames {
            return Err(MediaError::Resample("the signal ended early".into()));
        }
        self.run(out)?;
        // As `IncrementalResampler::finish`: exactly `expected` frames.
        let missing = self.expected - self.emitted();
        for o in out.iter_mut() {
            o.resize(o.len() + missing, 0.0);
        }
        self.produced = self.running.as_ref().map_or(0, |r| r.drop) + self.expected;
        Ok(())
    }

    /// Output frames handed out so far.
    fn emitted(&self) -> usize {
        let drop = self.running.as_ref().map_or(0, |r| r.drop);
        self.produced.saturating_sub(drop).min(self.expected)
    }

    /// Process every chunk whose input is known (the loop of [`IncrementalResampler::step`]).
    fn run(&mut self, out: &mut [Vec<f32>]) -> Result<(), MediaError> {
        let Some(r) = self.running.as_mut() else {
            return Ok(());
        };
        let all_in = self.received == self.frames;
        while self.produced < r.drop + self.expected {
            let need = r.resampler.input_frames_next();
            // Virtual input: `pad` zeros, the signal, then zeros.
            let end = r.pos + need;
            if !all_in && end > r.pad + self.received {
                return Ok(());
            }
            for (dst, src) in r.inbuf.iter_mut().zip(&self.held) {
                dst.resize(need, 0.0);
                for (i, d) in dst.iter_mut().enumerate() {
                    let v = r.pos + i;
                    *d = if v >= r.pad && v < r.total_in {
                        src[v - r.pad - self.first]
                    } else {
                        0.0
                    };
                }
            }
            let (_, n) = process_chunk(&mut r.resampler, &r.inbuf, &mut r.outbuf)?;
            if n == 0 && r.pos > r.total_in + 4 * CHUNK {
                break;
            }
            // Hand out what is past the alignment drop and within the length.
            let lo = r.drop.saturating_sub(self.produced).min(n);
            let hi = (r.drop + self.expected - self.produced).min(n);
            if lo < hi {
                for (o, b) in out.iter_mut().zip(&r.outbuf) {
                    o.extend_from_slice(&b[lo..hi]);
                }
            }
            self.produced += n;
            r.pos = end;
            // Input before the next chunk is not needed any more.
            let consumed = r.pos.saturating_sub(r.pad).min(self.received);
            if consumed > self.first {
                for h in &mut self.held {
                    h.drain(..consumed - self.first);
                }
                self.first = consumed;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(frames: usize) -> Vec<Vec<f32>> {
        (0..2)
            .map(|c| {
                (0..frames)
                    .map(|i| ((i as f32 * 0.013 + c as f32).sin() * 0.7) + (i % 7) as f32 * 0.01)
                    .collect()
            })
            .collect()
    }

    /// Streamed in blocks of any size, the output is the one-shot output, sample for sample.
    #[test]
    fn streamed_equals_one_shot() {
        for (from, to) in [
            (48_000, 44_100),
            (48_000, 96_000),
            (44_100, 48_000),
            (48_000, 22_050),
        ] {
            for frames in [0usize, 1, 100, 1023, 1024, 5000, 48_001] {
                let src = signal(frames);
                let mut one = IncrementalResampler::new(
                    Arc::new(DecodedAudio {
                        sample_rate: from,
                        channels: src.clone(),
                    }),
                    to,
                )
                .unwrap();
                while !one.step(4096).unwrap() {}
                let one = one.finish().channels;
                for block in [1usize, 512, 777, 4096] {
                    let mut s = StreamResampler::new(from, to, 2, frames).unwrap();
                    let mut out = vec![Vec::new(), Vec::new()];
                    let mut at = 0;
                    while at < frames {
                        let n = block.min(frames - at);
                        let b: Vec<&[f32]> = src.iter().map(|c| &c[at..at + n]).collect();
                        s.push(&b, &mut out).unwrap();
                        at += n;
                        assert!(s.held[0].len() <= n + 2 * CHUNK, "holds about a chunk");
                    }
                    s.finish(&mut out).unwrap();
                    assert_eq!(s.expected(), one[0].len());
                    assert!(
                        out == one,
                        "{from} -> {to}, {frames} frames, blocks of {block}"
                    );
                }
            }
        }
    }
}
