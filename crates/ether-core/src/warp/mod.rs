//! Warped audio-clip playback.
//!
//! # Modes
//! - **Repitch** (and unwarped clips): [`crate::sched::render_audio`] resamples the source
//!   (linear interpolation) along the clip's content → source mapping. Clip transpose is a
//!   playback-rate change around the clip's offset ([`repitch_source_seconds`]).
//! - **Complex**: a [`Stretcher`] (Signalsmith Stretch: the C++ on native, its pure-Rust
//!   port on the web) follows the same mapping while keeping pitch; clip transpose goes to
//!   [`Stretcher::set_transpose_semitones`]. Without a stretcher (no factory: a build
//!   without the stretch feature, the voice limit reached, or not provisioned yet) Complex
//!   falls back to Repitch. Where the mapping plays the source at its own speed (ratio 1) and the clip
//!   isn't transposed, there is nothing to stretch: the clip plays like Repitch, which is
//!   then exact, and needs no stretcher.
//! - **Transposition envelope** (`AutomationTarget::ClipTranspose`, semitones added to the
//!   clip transpose): Complex sets the stretcher's transposition at every chunk it renders
//!   (≤ the stretcher's output block, a few ms); Repitch integrates the varying playback
//!   rate along the mapping ([`RateIntegral`]), so positions stay exact after any jump.
//! - **Reverse** and **fades**: both paths read the source through
//!   [`sched::read_frames`] (reversed clips see the reversed media, so offset, loop and warp
//!   markers are on the reversed timeline) and apply [`sched::ClipEnvelope`].
//!
//! # Stretcher voices
//! Stretchers belong to tracks, pooled by concurrency: a track gets as many *voices* as
//! Complex clips can play on it at once ([`voices_needed`]: their overlaps, with a beat of
//! margin so a clip ending and the next one starting in the same sub-block both have one),
//! not one per clip. A clip takes a voice when it starts playing (the one it had if it
//! still has it, else a free one: unbound, unused in the previous sub-block, or bound to a
//! clip that doesn't overlap this sub-block) and seeks it there, as a clip always did on a
//! jump. So a song with hundreds of Complex clips holds a handful of stretchers.
//!
//! Each track job reaches only its own voices ([`WarpTable`]) and its own scratch
//! ([`WarpScratch`] in its `TrackRt`), so tracks with Complex clips run on the worker
//! threads like the others (they used to be pinned to the audio thread, where every
//! stretcher lived in one engine-wide table).
//!
//! # Threads
//! - [`WarpHandle`] (inside `EngineHandle`, controller thread): on every `publish` it diffs
//!   the voices the new desc needs against the ones it already gave the engine, creates +
//!   [`Stretcher::configure`]s the new ones (allocation happens here) and sends them over an
//!   SPSC ring, together with removals. Stretchers the audio thread gives back arrive on a
//!   return ring and are dropped here (the C++ `delete` never runs on the audio thread).
//! - [`WarpRt`] (inside `Engine`, audio thread): a fixed-capacity voice table sorted by
//!   `(track, slot)`, changed only between blocks. RT-safe: no allocation after creation.
//!
//! # Latency and jumps
//! A stretcher needs `input_latency` source frames ahead of the position it plays and its
//! output is `output_latency` frames late. The feed is compensated: after rendering output
//! sample `o`, the stretcher has consumed source up to `src(o + output_latency) +
//! input_latency`, so what comes out at `o` is `src(o)`. Whenever the content position is not
//! where the previous block left it (first play, locate, transport loop jump, clip-loop wrap,
//! a clip edit that moves content, a voice taken over by another clip), the voice
//! [`Stretcher::seek`]s with pre-roll audio ending at that feed position, so output is
//! audible immediately (after Signalsmith's short fade-in, ~50 ms) instead of after
//! the ~60–150 ms latency.

use std::sync::Arc;

use ether_protocol::model::{ClipId, TrackId, WarpMode};
use ether_stretch::{Stretcher, StretcherFactory};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::config::EngineConfig;
use serde::{Deserialize, Serialize};

use crate::graph::{ClipContentDesc, ClipDesc, ClipParamEnvelope, RenderGraphDesc, WarpDesc};
use crate::media::AudioSource;
use crate::sched::{self, Timing, for_each_piece, source_seconds};

#[cfg(test)]
mod tests;

/// Most stretcher voices at once (all tracks); further Complex clips play Repitch.
pub(crate) const MAX_STRETCH_VOICES: usize = 256;
/// Most voices of one track.
const MAX_TRACK_VOICES: usize = 16;
/// Margin (beats) after a clip's end during which its voice still counts as taken when
/// sizing a track's pool (a sub-block can hold the end of a clip and the start of the next).
const OVERLAP_MARGIN: f64 = 1.0;
/// Pre-roll handed to [`Stretcher::seek`] after a jump (seconds).
const SEEK_SECONDS: f64 = 0.2;
/// Largest input/output ratio fed in one call without skipping input (8x speed-up).
const INPUT_HEADROOM: usize = 8;
/// Content-position tolerance (beats) under which consecutive blocks count as continuous.
const CONTINUITY_EPS: f64 = 1e-6;
/// Tolerance on the source frames per output sample under which a Complex clip plays at its
/// own speed (no stretching needed).
const UNIT_RATE_EPS: f64 = 1e-9;
/// Documented worst-case onset delay after a seek (Signalsmith fades primed output in over
/// about two of its 30 ms intervals). Tests assert the signal is back within this.
#[cfg(test)]
pub(crate) const SEEK_FADE_MS: f64 = 50.0;

/// Content beat `c` → source seconds for playback at a transposed rate (Repitch): the
/// warp mapping is sped up by `2^(transpose/12)` around the content position `anchor`
/// (the clip offset, or 0 for a leading gap), so the clip start still plays the same
/// source position.
#[inline]
pub(crate) fn repitch_source_seconds(
    warp: Option<&WarpDesc>,
    transpose: f32,
    ref_bpm: f64,
    anchor: f64,
    c: f64,
) -> f64 {
    let s = source_seconds(warp, ref_bpm, c);
    if transpose == 0.0 || !transpose.is_finite() {
        return s;
    }
    let s0 = source_seconds(warp, ref_bpm, anchor);
    s0 + (s - s0) * semitone_ratio(transpose)
}

#[inline]
fn semitone_ratio(semitones: f32) -> f64 {
    (semitones as f64 / 12.0).exp2()
}

/// Largest total clip transposition (the clip transpose plus its envelope), ± semitones.
pub const MAX_TRANSPOSE: f64 = 48.0;

/// Clip transpose plus its envelope at content beat `c`, clamped to [`MAX_TRANSPOSE`].
#[inline]
pub(crate) fn transpose_at(transpose: f32, env: Option<&ClipParamEnvelope>, c: f64) -> f32 {
    match env {
        None => transpose,
        Some(e) => (transpose as f64 + e.plain_at(c)).clamp(-MAX_TRANSPOSE, MAX_TRANSPOSE) as f32,
    }
}

/// Cells of a [`RateIntegral`] table at most, and its finest cell (beats).
const RATE_MAX_CELLS: usize = 16_384;
const RATE_MIN_STEP: f64 = 1.0 / 64.0;

/// Repitch playback of a clip whose transposition varies (a transposition envelope): the
/// source position integrates the playback rate along the warp mapping,
///
/// `S(c) = S(anchor) + k · G(c)`, `G(c) = ∫_anchor^c 2^(t(u)/12) dW₁(u)`,
///
/// with `t` the total transposition ([`transpose_at`]), `W₁` the warp mapping at 60 BPM
/// ([`source_seconds`]) and `k` = 1 for warped clips, `60 / ref_bpm` unwarped (the engine's
/// tempo at the clip start). With a constant `t` this is exactly
/// [`repitch_source_seconds`]. `G` is tabulated by the controller on a uniform grid over the
/// content the clip can play (midpoint rule per cell: exact for a constant rate, the warp
/// mapping being piecewise linear), linear in between and extended past the ends at the end
/// rates. Allocates (controller side); [`RateIntegral::seconds`] is RT-safe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RateIntegral {
    /// Content beat of `cum[0]`.
    pub c0: f64,
    /// Grid step (beats).
    pub step: f64,
    /// `G` at `c0 + i · step`.
    pub cum: Vec<f64>,
    /// The warp markers drive the mapping (`k` = 1), else `k` = 60 / ref_bpm.
    pub warped: bool,
}

impl RateIntegral {
    /// Tabulate `G` over `lo..hi` (content beats; the anchor is included).
    pub fn build(
        warp: Option<&WarpDesc>,
        transpose: f32,
        env: &ClipParamEnvelope,
        anchor: f64,
        lo: f64,
        hi: f64,
    ) -> Self {
        let lo = lo.min(anchor);
        let hi = hi.max(anchor).max(lo + RATE_MIN_STEP);
        let step = ((hi - lo) / RATE_MAX_CELLS as f64).max(RATE_MIN_STEP);
        let cells = ((hi - lo) / step).ceil() as usize;
        let w1 = |c: f64| source_seconds(warp, 60.0, c);
        let rate = |c: f64| semitone_ratio(transpose_at(transpose, Some(env), c));
        let mut cum = Vec::with_capacity(cells + 1);
        let mut acc = 0.0;
        cum.push(acc);
        for i in 0..cells {
            let a = lo + i as f64 * step;
            let b = a + step;
            acc += (w1(b) - w1(a)) * rate(0.5 * (a + b));
            cum.push(acc);
        }
        let mut out = Self {
            c0: lo,
            step,
            cum,
            warped: matches!(warp, Some(w) if w.markers.len() >= 2),
        };
        let at_anchor = out.g(warp, transpose, env, anchor);
        for v in &mut out.cum {
            *v -= at_anchor;
        }
        out
    }

    /// `G(c)`.
    fn g(&self, warp: Option<&WarpDesc>, transpose: f32, env: &ClipParamEnvelope, c: f64) -> f64 {
        let last = self.cum.len() - 1;
        let x = (c - self.c0) / self.step;
        let edge = |i: usize| {
            let ce = self.c0 + i as f64 * self.step;
            let r = semitone_ratio(transpose_at(transpose, Some(env), ce));
            self.cum[i] + (source_seconds(warp, 60.0, c) - source_seconds(warp, 60.0, ce)) * r
        };
        if x <= 0.0 {
            edge(0)
        } else if x >= last as f64 {
            edge(last)
        } else {
            let i = x.floor() as usize;
            let f = x - i as f64;
            self.cum[i] + (self.cum[i + 1] - self.cum[i]) * f
        }
    }

    /// Source seconds of content beat `c` (see the type docs). RT-safe.
    #[inline]
    pub(crate) fn seconds(
        &self,
        warp: Option<&WarpDesc>,
        transpose: f32,
        env: &ClipParamEnvelope,
        ref_bpm: f64,
        anchor: f64,
        c: f64,
    ) -> f64 {
        let k = if self.warped { 1.0 } else { 60.0 / ref_bpm };
        source_seconds(warp, ref_bpm, anchor) + k * self.g(warp, transpose, env, c)
    }
}

/// A voice: slot `slot` of track `track`'s pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VoiceKey {
    track: TrackId,
    slot: u16,
}

enum Msg {
    Add(VoiceKey, Box<dyn Stretcher>),
    Remove(VoiceKey),
}

/// Create the two halves (non-RT: allocates the rings and the voice table).
pub(crate) fn channel(config: &EngineConfig) -> (WarpHandle, WarpRt) {
    let cap = MAX_STRETCH_VOICES * 2 + 16;
    let (tx, rx) = RingBuffer::new(cap);
    let (ret_tx, ret_rx) = RingBuffer::new(cap);
    (
        WarpHandle {
            factory: None,
            tx,
            returned: ret_rx,
            live: Vec::new(),
            max_voices: MAX_STRETCH_VOICES,
            sample_rate: config.sample_rate.max(1) as f32,
            max_block: config.max_block_size.max(1),
        },
        WarpRt {
            rx,
            ret: ret_tx,
            voices: Vec::with_capacity(MAX_STRETCH_VOICES),
            leaked: 0,
        },
    )
}

/// Voices a track needs: the most of its Complex clips that overlap (with
/// [`OVERLAP_MARGIN`] after each end), at most [`MAX_TRACK_VOICES`].
pub(crate) fn voices_needed(clips: &[ClipDesc]) -> usize {
    let mut edges: Vec<(f64, i32)> = clips
        .iter()
        .filter(|c| complex_warp(c).is_some())
        .flat_map(|c| {
            [
                (c.start, 1),
                (c.start + c.length.max(0.0) + OVERLAP_MARGIN, -1),
            ]
        })
        .collect();
    // Ends before starts at the same beat.
    edges.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let (mut now, mut most) = (0i32, 0i32);
    for (_, d) in edges {
        now += d;
        most = most.max(now);
    }
    (most.max(0) as usize).min(MAX_TRACK_VOICES)
}

/// Controller-side half (lives in `EngineHandle`).
pub(crate) struct WarpHandle {
    factory: Option<Arc<dyn StretcherFactory>>,
    tx: Producer<Msg>,
    returned: Consumer<Box<dyn Stretcher>>,
    /// Voices the engine has (or will have, once it drains the ring); sorted.
    live: Vec<VoiceKey>,
    /// Most voices provisioned at once (all tracks; [`MAX_STRETCH_VOICES`] unless the host
    /// lowers it): further Complex clips play Repitch.
    max_voices: usize,
    sample_rate: f32,
    max_block: usize,
}

impl WarpHandle {
    /// Non-RT. Provision the voices the tracks of `desc` need and release the others.
    /// Called by `EngineHandle::publish` before the snapshot is sent, so a new voice reaches
    /// the engine no later than the snapshot using it.
    pub(crate) fn sync(&mut self, desc: &RenderGraphDesc) {
        self.collect();
        let mut wanted: Vec<VoiceKey> = Vec::new();
        if self.factory.is_some() {
            for t in &desc.tracks {
                let n = voices_needed(&t.clips);
                for slot in 0..n {
                    if wanted.len() < self.max_voices {
                        wanted.push(VoiceKey {
                            track: t.id,
                            slot: slot as u16,
                        });
                    }
                }
            }
        }
        wanted.sort();
        wanted.dedup();

        let mut i = 0;
        while i < self.live.len() {
            let key = self.live[i];
            if wanted.binary_search(&key).is_err() {
                if self.tx.push(Msg::Remove(key)).is_err() {
                    return; // Ring full (engine not draining): retry at the next publish.
                }
                self.live.remove(i);
            } else {
                i += 1;
            }
        }
        let Some(factory) = self.factory.clone() else {
            return;
        };
        for key in wanted {
            let Err(at) = self.live.binary_search(&key) else {
                continue;
            };
            if self.tx.slots() == 0 {
                return;
            }
            let mut st = factory.create();
            st.configure(2, self.sample_rate, self.max_block);
            if self.tx.push(Msg::Add(key, st)).is_ok() {
                self.live.insert(at, key);
            }
        }
    }

    /// Drop the stretchers the audio thread returned.
    pub(crate) fn collect(&mut self) -> usize {
        let mut n = 0;
        while let Ok(st) = self.returned.pop() {
            drop(st);
            n += 1;
        }
        n
    }
}

impl crate::engine::EngineHandle {
    /// Enable pitch-preserving `WarpMode::Complex` playback with stretchers from `factory`
    /// (native hosts pass `ether_stretch::SignalsmithFactory`, the web host
    /// `ether_stretch::RustStretchFactory`). Without a factory Complex clips play Repitch.
    /// Takes effect at the next `publish`.
    pub fn set_stretcher_factory(&mut self, factory: Arc<dyn StretcherFactory>) {
        self.warp.factory = Some(factory);
    }

    /// Cap the stretcher voices of the whole engine at `max` (at most
    /// [`MAX_STRETCH_VOICES`]); Complex clips beyond it play Repitch. The web host lowers
    /// it: every voice runs on the single AudioWorklet thread. Takes effect at the next
    /// `publish`.
    pub fn set_stretch_voice_limit(&mut self, max: usize) {
        self.warp.max_voices = max.min(MAX_STRETCH_VOICES);
    }

    /// `true` once a stretcher factory is set (Complex warp is pitch-preserving).
    pub fn stretching_supported(&self) -> bool {
        self.warp.factory.is_some()
    }

    /// Number of stretchers currently provisioned for the engine (diagnostics/tests).
    pub fn stretcher_count(&self) -> usize {
        self.warp.live.len()
    }

    /// Drop the stretchers the audio thread gave back since the last publish (also done by
    /// every `publish`). Returns how many were dropped.
    pub fn collect_stretchers(&mut self) -> usize {
        self.warp.collect()
    }
}

pub(crate) fn complex_warp(clip: &ClipDesc) -> Option<&WarpDesc> {
    match &clip.content {
        ClipContentDesc::Audio {
            warp:
                Some(
                    w @ WarpDesc {
                        mode: WarpMode::Complex,
                        ..
                    },
                ),
            ..
        } => Some(w),
        _ => None,
    }
}

/// One stretcher voice.
pub(crate) struct Voice {
    key: VoiceKey,
    st: Box<dyn Stretcher>,
    /// Clip playing through this voice, and its timeline extent `[start, end)` when it took
    /// the voice.
    clip: Option<(ClipId, f64, f64)>,
    /// Engine sample time at the end of the last sub-block this voice rendered.
    used_until: u64,
    /// Content beat expected at the next rendered sample (NaN = seek first).
    next_c: f64,
    /// Next source frame to feed (integer-valued).
    next_src: f64,
}

/// Audio-thread half (lives in `Engine`).
pub(crate) struct WarpRt {
    rx: Consumer<Msg>,
    ret: Producer<Box<dyn Stretcher>>,
    /// Sorted by key; capacity [`MAX_STRETCH_VOICES`] (never grows).
    voices: Vec<Voice>,
    /// Stretchers leaked because the return ring was full (should stay 0).
    pub(crate) leaked: u64,
}

impl WarpRt {
    /// RT. Apply provisioning messages from the handle (between blocks).
    pub(crate) fn drain(&mut self) {
        while let Ok(msg) = self.rx.pop() {
            match msg {
                Msg::Add(key, st) => match self.voices.binary_search_by(|v| v.key.cmp(&key)) {
                    Ok(i) => {
                        let old = std::mem::replace(&mut self.voices[i].st, st);
                        self.voices[i].next_c = f64::NAN;
                        self.retire(old);
                    }
                    Err(i) if self.voices.len() < self.voices.capacity() => {
                        self.voices.insert(
                            i,
                            Voice {
                                key,
                                st,
                                clip: None,
                                used_until: 0,
                                next_c: f64::NAN,
                                next_src: 0.0,
                            },
                        );
                    }
                    Err(_) => self.retire(st),
                },
                Msg::Remove(key) => {
                    if let Ok(i) = self.voices.binary_search_by(|v| v.key.cmp(&key)) {
                        let v = self.voices.remove(i);
                        self.retire(v.st);
                    }
                }
            }
        }
    }

    fn retire(&mut self, st: Box<dyn Stretcher>) {
        if let Err(rtrb::PushError::Full(st)) = self.ret.push(st) {
            std::mem::forget(st);
            self.leaked += 1;
        }
    }

    /// RT. Shared access to the voices for this block's track jobs.
    pub(crate) fn table(&mut self) -> WarpTable {
        WarpTable {
            ptr: self.voices.as_mut_ptr(),
            len: self.voices.len(),
        }
    }
}

/// Raw view of the voice table handed to the track jobs (`crate::parallel`, "Unsafe
/// sharing"): each job takes the voices of its own track only.
#[derive(Clone, Copy)]
pub(crate) struct WarpTable {
    ptr: *mut Voice,
    len: usize,
}

impl WarpTable {
    /// The voices of `track` (possibly none).
    ///
    /// # Safety
    /// The table's `WarpRt` outlives the use and isn't touched through another path while
    /// jobs run (it only changes in `drain`, between blocks); `track` is the caller's own
    /// track: track ids are unique in a snapshot and each job runs one track, so no two
    /// jobs get overlapping slices. Keys are only read.
    pub(crate) unsafe fn track_voices<'a>(self, track: TrackId) -> &'a mut [Voice] {
        // Bounds by binary search on the immutable keys, through raw pointers (no
        // reference to another job's voices is ever formed).
        let key_at = |i: usize| {
            // SAFETY: `i < len`; `key` is never written while jobs run.
            unsafe { std::ptr::addr_of!((*self.ptr.add(i)).key).read() }
        };
        let lower = |pred: &dyn Fn(VoiceKey) -> bool| {
            let (mut lo, mut hi) = (0usize, self.len);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if pred(key_at(mid)) {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        let start = lower(&|k| k.track < track);
        let end = lower(&|k| k.track <= track);
        if start == end {
            return &mut [];
        }
        // SAFETY: `start..end` are this track's voices only (see above).
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(start), end - start) }
    }
}

// SAFETY: only dereferenced under the job contract above (each job its own slice).
unsafe impl Send for WarpTable {}
unsafe impl Sync for WarpTable {}

/// Per-track scratch of the Complex path (allocated with the snapshot, for tracks with
/// Complex-warped clips).
#[derive(Debug)]
pub(crate) struct WarpScratch {
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    seek_len: usize,
}

impl WarpScratch {
    /// Non-RT.
    pub(crate) fn new(sample_rate: f64, max_block: usize) -> Self {
        let max_block = max_block.max(1);
        let seek_len = (sample_rate * SEEK_SECONDS).ceil() as usize;
        let in_cap = (max_block * INPUT_HEADROOM).max(seek_len);
        Self {
            in_l: vec![0.0; in_cap],
            in_r: vec![0.0; in_cap],
            out_l: vec![0.0; max_block],
            out_r: vec![0.0; max_block],
            seek_len,
        }
    }
}

/// RT. Render (add) one audio clip for the sub-block: through a stretcher voice of the
/// track when it is Complex-warped, needs stretching and a voice is available, else by
/// resampling ([`sched::render_audio`]). Returns `false` on a source underrun.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    voices: &mut [Voice],
    scratch: Option<&mut WarpScratch>,
    clip: &ClipDesc,
    source: &dyn AudioSource,
    ref_bpm: f64,
    timing: &Timing<'_>,
    out: [&mut [f32]; 2],
    src_scratch: &mut [f32],
) -> bool {
    if let (Some(w), Some(scratch)) = (complex_warp(clip), scratch)
        && !voices.is_empty()
    {
        let mine = voices
            .iter()
            .position(|v| v.clip.is_some_and(|(id, _, _)| id == clip.id));
        let ClipContentDesc::Audio {
            transpose,
            transpose_envelope,
            ..
        } = &clip.content
        else {
            unreachable!("complex_warp: audio clips only")
        };
        if *transpose == 0.0 && transpose_envelope.is_none() && unit_rate(clip, w, ref_bpm, timing)
        {
            // Nothing to stretch: play it straight, and give the voice back (it seeks if
            // the clip needs stretching again).
            if let Some(i) = mine {
                voices[i].clip = None;
                voices[i].next_c = f64::NAN;
            }
            return sched::render_audio(clip, source, ref_bpm, timing, out, src_scratch);
        }
        let free = |v: &Voice| match v.clip {
            None => true,
            Some((_, start, end)) => {
                v.used_until < timing.sample_time || end <= timing.b0 || start >= timing.b1
            }
        };
        if let Some(i) = mine.or_else(|| voices.iter().position(free)) {
            let v = &mut voices[i];
            if mine.is_none() {
                v.clip = Some((clip.id, clip.start, clip.start + clip.length));
                v.next_c = f64::NAN;
            }
            v.used_until = timing.sample_time + timing.frames as u64;
            return render_complex(v, scratch, clip, w, source, ref_bpm, timing, out);
        }
    }
    sched::render_audio(clip, source, ref_bpm, timing, out, src_scratch)
}

/// The clip plays its source at its own speed over the whole sub-block: one source frame
/// per output sample at the start, middle and end of every piece.
fn unit_rate(clip: &ClipDesc, warp: &WarpDesc, ref_bpm: f64, timing: &Timing<'_>) -> bool {
    let sr = timing.sample_rate;
    let mut unit = true;
    for_each_piece(clip, timing.b0, timing.b1, |p| {
        let o_a = timing.sample_ceil(p.t0) as f64;
        let o_b = timing.sample_ceil(p.t1) as f64;
        if o_b - o_a < 2.0 {
            return;
        }
        let src_at =
            |o: f64| source_seconds(Some(warp), ref_bpm, p.c0 + (timing.beat_at(o) - p.t0)) * sr;
        let slope = |a: f64, b: f64| (src_at(b) - src_at(a)) / (b - a);
        let mid = ((o_a + o_b) * 0.5).floor();
        for (a, b) in [(o_a, o_a + 1.0), (o_a, mid), (mid, o_b), (o_b - 1.0, o_b)] {
            unit &= (slope(a, b) - 1.0).abs() < UNIT_RATE_EPS;
        }
    });
    unit
}

#[allow(clippy::too_many_arguments)]
fn render_complex(
    voice: &mut Voice,
    scratch: &mut WarpScratch,
    clip: &ClipDesc,
    warp: &WarpDesc,
    source: &dyn AudioSource,
    ref_bpm: f64,
    timing: &Timing<'_>,
    out: [&mut [f32]; 2],
) -> bool {
    let ClipContentDesc::Audio {
        transpose,
        reversed,
        transpose_envelope,
        ..
    } = &clip.content
    else {
        return true;
    };
    let transpose_env = transpose_envelope.as_ref().map(|t| &t.env);
    let WarpScratch {
        in_l,
        in_r,
        out_l,
        out_r,
        seek_len,
    } = scratch;
    let Some(env) = sched::ClipEnvelope::new(clip, timing) else {
        voice.next_c = f64::NAN;
        return true;
    };
    voice.st.set_transpose_semitones(*transpose);
    let sr = timing.sample_rate;
    let in_lat = voice.st.input_latency() as f64;
    let out_lat = voice.st.output_latency() as f64;
    let cap = in_l.len();
    let max_out = out_l.len();
    let [dst_l, dst_r] = out;
    let mut ok = true;
    for_each_piece(clip, timing.b0, timing.b1, |p| {
        let o_a = timing.sample_ceil(p.t0);
        let o_b = timing.sample_ceil(p.t1);
        if o_a >= o_b {
            return;
        }
        // Content beat and source frame at (possibly fractional, possibly past the
        // sub-block: linear extrapolation) output sample `o`.
        let c_at = |o: f64| p.c0 + (timing.beat_at(o) - p.t0);
        let src_at = |o: f64| source_seconds(Some(warp), ref_bpm, c_at(o)) * sr;
        // Source frame fed once output up to sample `o` has been produced.
        let feed_at = |o: f64| (src_at(o + out_lat) + in_lat).floor();

        let continuous = (c_at(o_a as f64) - voice.next_c).abs() < CONTINUITY_EPS;
        if !continuous {
            // Jump: prime the stretcher with the audio leading up to the feed position.
            let span = (o_b - o_a) as f64;
            let rate = ((src_at(o_b as f64) - src_at(o_a as f64)) / span).abs();
            let rate = if rate.is_finite() && rate > 1e-6 {
                rate
            } else {
                1.0
            };
            let end = feed_at(o_a as f64);
            let n = *seek_len;
            ok &= sched::read_frames(
                source,
                *reversed,
                end as i64 - n as i64,
                &mut in_l[..n],
                &mut in_r[..n],
            );
            voice.st.seek(&[&in_l[..n], &in_r[..n]], rate);
            voice.next_src = end;
        }

        let mut o = o_a;
        while o < o_b {
            let len = (o_b - o).min(max_out);
            if transpose_env.is_some() {
                // The envelope's value where this chunk starts.
                let c = c_at(o as f64);
                voice
                    .st
                    .set_transpose_semitones(transpose_at(*transpose, transpose_env, c));
            }
            let target = feed_at((o + len) as f64);
            let mut n_in = (target - voice.next_src).max(0.0) as usize;
            if n_in > cap {
                // Faster than the headroom allows: skip the input we cannot feed.
                voice.next_src += (n_in - cap) as f64;
                n_in = cap;
            }
            ok &= sched::read_frames(
                source,
                *reversed,
                voice.next_src as i64,
                &mut in_l[..n_in],
                &mut in_r[..n_in],
            );
            voice.st.process(
                &[&in_l[..n_in], &in_r[..n_in]],
                n_in,
                &mut [&mut out_l[..len], &mut out_r[..len]],
                len,
            );
            voice.next_src += n_in as f64;
            for k in 0..len {
                let s = o + k;
                let t = timing.beat_at(s as f64);
                let g = env.at_content(t, p.c0 + (t - p.t0));
                dst_l[s] += out_l[k] * g;
                dst_r[s] += out_r[k] * g;
            }
            o += len;
        }
        voice.next_c = c_at(o_b as f64);
        if voice.next_src >= 0.0 {
            source.prefetch_hint(voice.next_src as u64);
        }
    });
    ok
}
