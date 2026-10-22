//! Render preview (CONTRACTS.md §13.19): while an offline render runs (an audio export,
//! stems, a share snapshot), the live engine is suspended and the transport plays the
//! render's master as far as it is rendered.
//!
//! A [`PreviewWriter`] (non-RT: the controller thread natively, the AudioWorklet's control
//! step on the web) appends the rendered master to a [`PreviewBuffer`]; the engine
//! ([`crate::EngineHandle::set_render_preview`]) reads it on the audio thread while
//! suspended, following its own transport: play, stop, locate and the loop work as usual
//! inside the rendered part, and playback stops at its end (the last frame rendered so
//! far, or the end of the render).
//!
//! # Memory
//! The buffer is a fixed table of [`CHUNK_FRAMES`]-frame chunks of interleaved 16-bit
//! stereo, allocated by the writer as it goes (never by the reader) and freed with the
//! buffer, at most [`MAX_PREVIEW_BYTES`] (about 23 minutes at 48 kHz; a longer render
//! previews its first [`MAX_PREVIEW_FRAMES`] frames). 16 bits is plenty for listening and
//! halves what the preview holds.
//!
//! # Lock-free
//! One writer, any number of readers. The writer fills a chunk, stores its pointer
//! (`Release`) before any frame of it is published, and publishes frames by raising
//! `written` (`Release`) after writing them; a reader loads `written` (`Acquire`) and only
//! reads below it, so it never sees a chunk pointer that is not stored yet nor a frame
//! being written. Chunks are never moved or freed while the buffer lives.

use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::tempo::{TempoMapRt, TempoPointDesc};

/// Frames per chunk (256 KiB of 16-bit stereo).
pub const CHUNK_FRAMES: usize = 1 << 16;
/// Most memory a preview holds.
pub const MAX_PREVIEW_BYTES: usize = 256 << 20;
/// Most frames a preview holds (16-bit stereo).
pub const MAX_PREVIEW_FRAMES: usize = MAX_PREVIEW_BYTES / 4;

/// What a preview contains: the render's master at `rate`, frame 0 at song time
/// `start_seconds` (the render's own tempo map: beats map to the rendered audio even when
/// the document's tempo is edited meanwhile).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewSpec {
    pub rate: u32,
    pub start_seconds: f64,
    /// Frames the render produces (capped at [`MAX_PREVIEW_FRAMES`] by the writer).
    pub frames: usize,
    /// The render's tempo points (empty: the default 120 bpm).
    pub tempo: Vec<TempoPointDesc>,
}

/// The rendered master so far (see the module docs).
pub struct PreviewBuffer {
    spec: PreviewSpec,
    /// `spec.tempo` compiled.
    tempo: TempoMapRt,
    chunks: Box<[AtomicPtr<i16>]>,
    written: AtomicUsize,
}

impl std::fmt::Debug for PreviewBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewBuffer")
            .field("spec", &self.spec)
            .field("written", &self.available())
            .finish()
    }
}

impl PreviewBuffer {
    pub fn spec(&self) -> &PreviewSpec {
        &self.spec
    }

    /// **RT.** Frames readable now.
    pub fn available(&self) -> usize {
        self.written.load(Ordering::Acquire)
    }

    /// Everything the render produces is in.
    pub fn complete(&self) -> bool {
        self.available() >= self.spec.frames
    }

    /// **RT.** Frame `i` as stereo `f32`. `i < available()`, else silence.
    pub fn frame(&self, i: usize) -> [f32; 2] {
        if i >= self.available() {
            return [0.0; 2];
        }
        let chunk = self.chunks[i / CHUNK_FRAMES].load(Ordering::Acquire);
        let k = (i % CHUNK_FRAMES) * 2;
        // SAFETY: `i < written`, so its chunk was stored (non-null, `CHUNK_FRAMES * 2`
        // values, never freed while `self` lives) and frame `i` was written before
        // `written` was raised past it (module docs); nothing writes it again.
        let (l, r) = unsafe { (*chunk.add(k), *chunk.add(k + 1)) };
        [f32::from(l) / 32768.0, f32::from(r) / 32768.0]
    }
}

impl Drop for PreviewBuffer {
    fn drop(&mut self) {
        for c in self.chunks.iter() {
            let p = c.load(Ordering::Acquire);
            if !p.is_null() {
                // SAFETY: allocated by `PreviewWriter::chunk` as a boxed slice of exactly
                // `CHUNK_FRAMES * 2` values, and freed only here.
                drop(unsafe {
                    Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, CHUNK_FRAMES * 2))
                });
            }
        }
    }
}

/// Appends the render to a [`PreviewBuffer`] (non-RT: allocates a chunk every
/// [`CHUNK_FRAMES`] frames). The only writer of its buffer.
pub struct PreviewWriter {
    buffer: Arc<PreviewBuffer>,
    /// Frames written (published once a push ends).
    pos: usize,
}

impl PreviewWriter {
    /// A new, empty preview; hand [`PreviewWriter::buffer`] to the engine.
    pub fn new(mut spec: PreviewSpec) -> Self {
        spec.frames = spec.frames.min(MAX_PREVIEW_FRAMES);
        let chunks = spec.frames.div_ceil(CHUNK_FRAMES);
        Self {
            buffer: Arc::new(PreviewBuffer {
                tempo: TempoMapRt::compile(&spec.tempo, &[]),
                spec,
                chunks: (0..chunks)
                    .map(|_| AtomicPtr::new(std::ptr::null_mut()))
                    .collect(),
                written: AtomicUsize::new(0),
            }),
            pos: 0,
        }
    }

    pub fn buffer(&self) -> Arc<PreviewBuffer> {
        self.buffer.clone()
    }

    /// Frames written so far.
    pub fn written(&self) -> usize {
        self.pos
    }

    /// The preview holds every frame it can.
    pub fn full(&self) -> bool {
        self.pos >= self.buffer.spec.frames
    }

    /// The chunk holding frame `i` (allocated on first use).
    fn chunk(&self, i: usize) -> *mut i16 {
        let slot = &self.buffer.chunks[i / CHUNK_FRAMES];
        let p = slot.load(Ordering::Relaxed);
        if !p.is_null() {
            return p;
        }
        let fresh: Box<[i16]> = vec![0i16; CHUNK_FRAMES * 2].into_boxed_slice();
        let p = Box::into_raw(fresh) as *mut i16;
        slot.store(p, Ordering::Release);
        p
    }

    /// Append `frames` frames given by `sample(i) -> [left, right]` (16-bit), up to the
    /// capacity.
    pub fn push_frames(&mut self, frames: usize, mut sample: impl FnMut(usize) -> [i16; 2]) {
        let n = frames.min(self.buffer.spec.frames - self.pos);
        for i in 0..n {
            let at = self.pos + i;
            let chunk = self.chunk(at);
            let k = (at % CHUNK_FRAMES) * 2;
            let [l, r] = sample(i);
            // SAFETY: `chunk` holds `CHUNK_FRAMES * 2` values and `k + 1` is below that;
            // frame `at >= written` is not read by anyone until `written` passes it.
            unsafe {
                *chunk.add(k) = l;
                *chunk.add(k + 1) = r;
            }
        }
        self.pos += n;
        self.buffer.written.store(self.pos, Ordering::Release);
    }

    /// Append planar `f32` frames (`left.len()` of them; `right` at least as long).
    pub fn push(&mut self, left: &[f32], right: &[f32]) {
        self.push_frames(left.len().min(right.len()), |i| {
            [to_i16(left[i]), to_i16(right[i])]
        });
    }

    /// Append interleaved 16-bit stereo frames (the web bridge's wire format).
    pub fn push_i16(&mut self, interleaved: &[i16]) {
        self.push_frames(interleaved.len() / 2, |i| {
            [interleaved[2 * i], interleaved[2 * i + 1]]
        });
    }
}

/// `f32` to 16 bits (clipped; the preview is for listening).
pub fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// The transport as the preview sees it (the engine's, see [`play`]).
pub(crate) struct PreviewTransport<'a> {
    pub position: &'a mut f64,
    pub playing: &'a mut bool,
    /// Loop in effect: on, start, end (beats).
    pub loop_region: (bool, f64, f64),
}

/// **RT.** Play `frames` frames of `buf` at the transport's position into `out` (left,
/// right; already zeroed), at `engine_rate` (resampled linearly from the preview's rate),
/// advancing the position and wrapping at the loop end like the live transport. Stops the
/// transport (and returns `true`) on a frame that is not rendered (yet): past what is
/// rendered so far, past the end, or before the render's start.
pub(crate) fn play(
    buf: &PreviewBuffer,
    t: PreviewTransport<'_>,
    engine_rate: u32,
    out: [&mut [f32]; 2],
    frames: usize,
) -> bool {
    if !*t.playing || engine_rate == 0 {
        return false;
    }
    let spec = buf.spec();
    let tempo = &buf.tempo;
    let rate = f64::from(spec.rate);
    let step = 1.0 / f64::from(engine_rate);
    let available = buf.available();
    let (loop_on, loop_start, loop_end) = t.loop_region;
    // The live transport only wraps when it is before the loop end.
    let wrap = (loop_on && *t.position < loop_end).then(|| {
        (
            tempo.beats_to_seconds(loop_start),
            tempo.beats_to_seconds(loop_end),
        )
    });
    let mut seconds = tempo.beats_to_seconds(*t.position);
    let [left, right] = out;
    let mut stopped = false;
    for i in 0..frames.min(left.len()).min(right.len()) {
        if let Some((start, end)) = wrap
            && seconds >= end - 1e-9
        {
            seconds = start + (seconds - end).max(0.0);
        }
        let at = (seconds - spec.start_seconds) * rate;
        if at < -1e-6 || at.floor() as usize >= available {
            stopped = true;
            break;
        }
        let at = at.max(0.0);
        let i0 = at.floor() as usize;
        let frac = (at - i0 as f64) as f32;
        let a = buf.frame(i0);
        let b = if i0 + 1 < available { buf.frame(i0 + 1) } else { a };
        left[i] = a[0] + (b[0] - a[0]) * frac;
        right[i] = a[1] + (b[1] - a[1]) * frac;
        seconds += step;
    }
    if let Some((start, end)) = wrap
        && !stopped
        && seconds >= end - 1e-9
    {
        seconds = start + (seconds - end).max(0.0);
    }
    *t.position = tempo.seconds_to_beats(seconds).max(0.0);
    if stopped {
        *t.playing = false;
    }
    stopped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 kHz, 120 bpm (the default map): frame `f` is beat `f / 500`.
    fn spec(frames: usize) -> PreviewSpec {
        PreviewSpec {
            rate: 1000,
            start_seconds: 0.0,
            frames,
            tempo: Vec::new(),
        }
    }

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32 / 32768.0).collect()
    }

    #[test]
    fn frames_published_in_order_across_chunks() {
        let mut w = PreviewWriter::new(spec(CHUNK_FRAMES + 10));
        let b = w.buffer();
        assert_eq!(b.available(), 0);
        assert_eq!(b.frame(0), [0.0; 2]);
        let l = ramp(CHUNK_FRAMES - 3);
        w.push(&l, &l);
        assert_eq!(b.available(), CHUNK_FRAMES - 3);
        w.push_i16(&[100, -100, 101, -101, 102, -102, 103, -103]);
        assert_eq!(b.available(), CHUNK_FRAMES + 1);
        assert_eq!(b.frame(CHUNK_FRAMES), [103.0 / 32768.0, -103.0 / 32768.0]);
        assert_eq!(b.frame(5), [5.0 / 32768.0; 2]);
        // Capped at the spec's length.
        w.push(&ramp(100), &ramp(100));
        assert_eq!(b.available(), CHUNK_FRAMES + 10);
        assert!(w.full() && b.complete());
    }

    #[test]
    fn plays_the_rendered_part_then_stops_at_its_end() {
        let mut w = PreviewWriter::new(spec(100));
        let b = w.buffer();
        w.push(&ramp(50), &ramp(50));
        let (mut pos, mut playing) = (0.02, true); // frame 10
        let (mut l, mut r) = (vec![0.0; 30], vec![0.0; 30]);
        let t = PreviewTransport {
            position: &mut pos,
            playing: &mut playing,
            loop_region: (false, 0.0, 0.0),
        };
        assert!(!play(&b, t, 1000, [&mut l, &mut r], 30));
        assert!(playing);
        assert!((l[0] * 32768.0 - 10.0).abs() < 1e-3 && (l[29] * 32768.0 - 39.0).abs() < 1e-3);
        assert!((pos - 0.08).abs() < 1e-9);
        // 10 more frames are rendered, 20 asked: stops at the boundary.
        let (mut l, mut r) = (vec![0.0; 20], vec![0.0; 20]);
        let t = PreviewTransport {
            position: &mut pos,
            playing: &mut playing,
            loop_region: (false, 0.0, 0.0),
        };
        assert!(play(&b, t, 1000, [&mut l, &mut r], 20));
        assert!(!playing);
        assert!((l[9] * 32768.0 - 49.0).abs() < 1e-3 && l[10] == 0.0);
        assert!((pos - 0.1).abs() < 1e-9);
    }

    #[test]
    fn loops_and_resamples() {
        let mut w = PreviewWriter::new(spec(100));
        let b = w.buffer();
        w.push(&ramp(100), &ramp(100));
        // Loop beats 0.04..0.06 (frames 20..30); engine at 2 kHz: half-frame steps.
        let (mut pos, mut playing) = (0.04, true);
        let (mut l, mut r) = (vec![0.0; 40], vec![0.0; 40]);
        let t = PreviewTransport {
            position: &mut pos,
            playing: &mut playing,
            loop_region: (true, 0.04, 0.06),
        };
        assert!(!play(&b, t, 2000, [&mut l, &mut r], 40));
        assert!((l[1] * 32768.0 - 20.5).abs() < 1e-2, "{}", l[1] * 32768.0);
        assert!((l[20] * 32768.0 - 20.0).abs() < 1e-2, "wrapped: {}", l[20] * 32768.0);
        assert!((l[39] * 32768.0 - 29.5).abs() < 1e-2);
        assert!(playing && (pos - 0.04).abs() < 1e-9);
    }

    #[test]
    fn before_the_render_start_is_not_rendered() {
        let mut w = PreviewWriter::new(PreviewSpec {
            start_seconds: 1.0,
            ..spec(100)
        });
        let b = w.buffer();
        w.push(&ramp(100), &ramp(100));
        let (mut pos, mut playing) = (0.5, true);
        let (mut l, mut r) = (vec![0.0; 4], vec![0.0; 4]);
        let t = PreviewTransport {
            position: &mut pos,
            playing: &mut playing,
            loop_region: (false, 0.0, 0.0),
        };
        assert!(play(&b, t, 1000, [&mut l, &mut r], 4));
        assert!(!playing && pos == 0.5);
    }
}
