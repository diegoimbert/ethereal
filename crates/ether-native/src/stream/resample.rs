//! Tap audio (engine rate, interleaved stereo) → 48 kHz → 20 ms (960-frame) frames.
//!
//! When the engine does not run at 48 kHz, a synchronous FFT resampler
//! (`rubato::FftFixedInOut`: fixed input and output chunks at the exact rational ratio, so
//! the output count never drifts from `frames_in * 48000 / sr`) converts it; its delay
//! (`output_delay()`) is trimmed once, so the output sample `n` is the input at
//! `n * sr / 48000` (the index the anchors assume, `anchors.rs`). Memory is bounded: the
//! 48 kHz FIFO keeps at most [`MAX_FIFO_FRAMES`] frames and drops the oldest whole frames
//! beyond that; the stream index still advances over them (the RTP timestamps skip, the
//! anchors stay right).

use std::collections::VecDeque;

use rubato::{FftFixedInOut, Resampler};

/// Opus frame: 20 ms at 48 kHz.
pub const FRAME: usize = 960;
/// Bound of the 48 kHz FIFO (1 s).
pub const MAX_FIFO_FRAMES: usize = 48_000;

pub struct Framer {
    rs: Option<Rs>,
    /// Interleaved stereo at 48 kHz.
    fifo: VecDeque<f32>,
    /// Stream index (`n`) of the FIFO's first frame.
    next_n: u64,
}

struct Rs {
    r: FftFixedInOut<f32>,
    input: [Vec<f32>; 2],
    fill: usize,
    output: [Vec<f32>; 2],
    /// Output frames still to discard (the resampler's delay).
    skip: usize,
}

impl Framer {
    /// A framer for audio at `sr` whose first output frame is stream sample `n_base`.
    pub fn new(sr: u32, n_base: u64) -> Self {
        let rs = (sr != 48_000 && sr > 0).then(|| {
            // ~10 ms input chunks (rubato rounds up to a multiple of sr/gcd).
            let chunk = (sr as usize / 100).max(1);
            let r = FftFixedInOut::<f32>::new(sr as usize, 48_000, chunk, 2)
                .expect("valid sample rates");
            let n_in = r.input_frames_next();
            let n_out = r.output_frames_max();
            let skip = r.output_delay();
            Rs {
                r,
                input: [vec![0.0; n_in], vec![0.0; n_in]],
                fill: 0,
                output: [vec![0.0; n_out], vec![0.0; n_out]],
                skip,
            }
        });
        Self {
            rs,
            fifo: VecDeque::with_capacity(2 * (MAX_FIFO_FRAMES + FRAME)),
            next_n: n_base,
        }
    }

    /// Push interleaved stereo frames at the engine rate.
    pub fn push(&mut self, interleaved: &[f32]) {
        match self.rs.as_mut() {
            None => self.fifo.extend(interleaved.iter().copied()),
            Some(rs) => {
                for &[l, r] in interleaved.as_chunks::<2>().0 {
                    rs.input[0][rs.fill] = l;
                    rs.input[1][rs.fill] = r;
                    rs.fill += 1;
                    if rs.fill == rs.input[0].len() {
                        rs.fill = 0;
                        let Ok((_, n)) = rs.r.process_into_buffer(&rs.input, &mut rs.output, None)
                        else {
                            continue;
                        };
                        let skip = rs.skip.min(n);
                        rs.skip -= skip;
                        for i in skip..n {
                            self.fifo.push_back(rs.output[0][i]);
                            self.fifo.push_back(rs.output[1][i]);
                        }
                    }
                }
            }
        }
        let excess = self.fifo.len().saturating_sub(2 * MAX_FIFO_FRAMES);
        if excess > 0 {
            // Keep whole frames aligned to the 20 ms grid.
            let drop = excess.div_ceil(2 * FRAME) * 2 * FRAME;
            let drop = drop.min(self.fifo.len());
            self.fifo.drain(..drop);
            self.next_n += (drop / 2) as u64;
        }
    }

    /// Pop one 20 ms interleaved stereo frame (`out.len() == 2 * FRAME`) if available;
    /// returns the stream index of its first sample.
    pub fn pop_frame(&mut self, out: &mut [f32]) -> Option<u64> {
        debug_assert_eq!(out.len(), 2 * FRAME);
        if self.fifo.len() < 2 * FRAME {
            return None;
        }
        for (o, s) in out.iter_mut().zip(self.fifo.drain(..2 * FRAME)) {
            *o = s;
        }
        let n = self.next_n;
        self.next_n += FRAME as u64;
        Some(n)
    }

    /// Stream index of the next frame [`Self::pop_frame`] returns.
    pub fn next_n(&self) -> u64 {
        self.next_n
    }

    /// Frames (48 kHz) waiting in the FIFO.
    pub fn buffered(&self) -> usize {
        self.fifo.len() / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 kHz + 37 Hz (unambiguous alignment over ±600 samples).
    fn signal(t: f64) -> f64 {
        use std::f64::consts::TAU;
        0.5 * (TAU * 1000.0 * t).sin() + 0.4 * (TAU * 37.0 * t).sin()
    }

    fn feed(f: &mut Framer, frames: usize, sr: f64, start: usize, step: usize) -> usize {
        let mut done = 0;
        while done < frames {
            let n = step.min(frames - done);
            let buf: Vec<f32> = (0..n)
                .flat_map(|i| {
                    let s = signal((start + done + i) as f64 / sr) as f32;
                    [s, -s]
                })
                .collect();
            f.push(&buf);
            done += n;
        }
        done
    }

    #[test]
    fn frames_are_exactly_960_at_48k() {
        let mut f = Framer::new(48_000, 7);
        feed(&mut f, 1000, 48_000.0, 0, 37);
        let mut out = vec![0.0; 2 * FRAME];
        assert_eq!(f.pop_frame(&mut out), Some(7));
        assert_eq!(f.pop_frame(&mut out), None);
        assert_eq!(f.buffered(), 40);
        // Passthrough: bit exact.
        assert_eq!(out[2], signal(1.0 / 48_000.0) as f32);
        feed(&mut f, 920, 48_000.0, 1000, 128);
        assert_eq!(f.pop_frame(&mut out), Some(967));
        assert_eq!(f.buffered(), 0);
    }

    #[test]
    fn resampled_output_count_and_alignment() {
        let mut f = Framer::new(44_100, 0);
        let mut out = vec![0.0; 2 * FRAME];
        feed(&mut f, 44_100, 44_100.0, 0, 113);
        let mut frames = 0;
        let mut first = Vec::new();
        while let Some(n) = f.pop_frame(&mut out) {
            assert_eq!(n, (frames * FRAME) as u64);
            if frames == 10 {
                first = out.clone();
            }
            frames += 1;
        }
        // 1 s in → 48000 out minus the trimmed delay and the not-yet-full input chunk.
        let total = frames * FRAME + f.buffered();
        assert!((47_000..=48_000).contains(&total), "{total}");
        // Output sample n is the input at n * 44100/48000 (to within a sample).
        let err = |lag: f64| -> f64 {
            (0..FRAME)
                .map(|i| {
                    let want = signal(((10 * FRAME + i) as f64 + lag) / 48_000.0);
                    (first[2 * i] as f64 - want).powi(2) + (first[2 * i + 1] as f64 + want).powi(2)
                })
                .sum()
        };
        let best = (-6000..6000)
            .map(|l| l as f64 / 10.0)
            .min_by(|a, b| err(*a).total_cmp(&err(*b)))
            .unwrap();
        assert!(best.abs() <= 1.0, "resampler misaligned by {best} samples");
        let rms = (err(best) / (2 * FRAME) as f64).sqrt();
        assert!(rms < 0.01, "rms error {rms}");
    }

    #[test]
    fn fifo_is_bounded_and_drops_whole_frames() {
        let mut f = Framer::new(48_000, 0);
        feed(&mut f, 3 * 48_000, 48_000.0, 0, 480);
        assert!(f.buffered() <= MAX_FIFO_FRAMES);
        assert_eq!(f.buffered() % FRAME, 0);
        // The stream index still counts the dropped frames.
        assert_eq!(f.next_n() + f.buffered() as u64, 3 * 48_000);
    }
}
