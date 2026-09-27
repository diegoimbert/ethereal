//! Warped audio-clip playback (owned by the `warp` wave-3 node; see `docs/WAVE3.md`).
//!
//! # Modes
//! - **Repitch** (and unwarped clips): [`crate::sched::render_audio`] resamples the source
//!   (linear interpolation) along the clip's content → source mapping. Clip transpose is a
//!   playback-rate change around the clip's offset ([`repitch_source_seconds`]).
//! - **Complex**: a per-clip [`Stretcher`] (Signalsmith on native) follows the same mapping
//!   while keeping pitch; clip transpose goes to [`Stretcher::set_transpose_semitones`].
//!   Without a stretcher (no factory: the web build, or not provisioned yet) Complex falls
//!   back to Repitch.
//! - **Reverse** and **fades** (clip-editing): both paths read the source through
//!   [`sched::read_frames`] (reversed clips see the reversed media, so offset, loop and warp
//!   markers are on the reversed timeline) and apply [`sched::ClipEnvelope`].
//!
//! # Threads
//! - [`WarpHandle`] (inside `EngineHandle`, controller thread): on every `publish` it diffs
//!   the Complex-warped audio clips of the new desc against the stretchers it already gave
//!   the engine, creates + [`Stretcher::configure`]s the new ones (allocation happens here)
//!   and sends them over an SPSC ring, together with removals. Stretchers the audio thread
//!   gives back arrive on a return ring and are dropped here (the C++ `delete` never runs on
//!   the audio thread).
//! - [`WarpRt`] (inside `Engine`, audio thread): a fixed-capacity, sorted voice table keyed
//!   by clip id. RT-safe: no allocation after creation.
//!
//! # Latency and jumps
//! A stretcher needs `input_latency` source frames ahead of the position it plays and its
//! output is `output_latency` frames late. The feed is compensated: after rendering output
//! sample `o`, the stretcher has consumed source up to `src(o + output_latency) +
//! input_latency`, so what comes out at `o` is `src(o)`. Whenever the content position is not
//! where the previous block left it (first play, locate, transport loop jump, clip-loop wrap,
//! a clip edit that moves content), the voice [`Stretcher::seek`]s with pre-roll audio ending
//! at that feed position, so output is audible immediately (after Signalsmith's short
//! fade-in, [`SEEK_FADE_MS`]) instead of after the ~60–120 ms latency.

use std::sync::Arc;

use ether_protocol::model::{ClipId, WarpMode};
use ether_stretch::{Stretcher, StretcherFactory};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::config::EngineConfig;
use crate::graph::{ClipContentDesc, ClipDesc, RenderGraphDesc, WarpDesc};
use crate::media::AudioSource;
use crate::sched::{self, Timing, for_each_piece, source_seconds};

#[cfg(test)]
mod tests;

/// Most Complex-warped clips with their own stretcher at once; further clips play Repitch.
pub(crate) const MAX_STRETCH_VOICES: usize = 256;
/// Pre-roll handed to [`Stretcher::seek`] after a jump (seconds).
const SEEK_SECONDS: f64 = 0.2;
/// Largest input/output ratio fed in one call without skipping input (8x speed-up).
const INPUT_HEADROOM: usize = 8;
/// Content-position tolerance (beats) under which consecutive blocks count as continuous.
const CONTINUITY_EPS: f64 = 1e-6;
/// Documented worst-case onset delay after a seek (Signalsmith fades primed output in over
/// about two of its 30 ms intervals). Tests assert the signal is back within this.
#[allow(dead_code)]
pub(crate) const SEEK_FADE_MS: f64 = 50.0;

/// Content beat `c` → source seconds for playback at a transposed rate (Repitch): the
/// warp mapping is sped up by `2^(transpose/12)` around the content position `anchor`
/// (the clip offset), so the clip start still plays the same source position.
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

enum Msg {
    Add(ClipId, Box<dyn Stretcher>),
    Remove(ClipId),
}

/// Create the two halves (non-RT: allocates the rings and the voice table).
pub(crate) fn channel(config: &EngineConfig) -> (WarpHandle, WarpRt) {
    let cap = MAX_STRETCH_VOICES * 2 + 16;
    let (tx, rx) = RingBuffer::new(cap);
    let (ret_tx, ret_rx) = RingBuffer::new(cap);
    let sr = config.sample_rate.max(1) as f64;
    let max_block = config.max_block_size.max(1);
    let seek_len = (sr * SEEK_SECONDS).ceil() as usize;
    let in_cap = (max_block * INPUT_HEADROOM).max(seek_len);
    (
        WarpHandle {
            factory: None,
            tx,
            returned: ret_rx,
            live: Vec::new(),
            sample_rate: sr as f32,
            max_block,
        },
        WarpRt {
            rx,
            ret: ret_tx,
            voices: Vec::with_capacity(MAX_STRETCH_VOICES),
            in_l: vec![0.0; in_cap],
            in_r: vec![0.0; in_cap],
            out_l: vec![0.0; max_block],
            out_r: vec![0.0; max_block],
            seek_len,
            leaked: 0,
        },
    )
}

/// Controller-side half (lives in `EngineHandle`).
pub(crate) struct WarpHandle {
    factory: Option<Arc<dyn StretcherFactory>>,
    tx: Producer<Msg>,
    returned: Consumer<Box<dyn Stretcher>>,
    /// Clips the engine has (or will have, once it drains the ring) a stretcher for; sorted.
    live: Vec<ClipId>,
    sample_rate: f32,
    max_block: usize,
}

impl WarpHandle {
    /// Non-RT. Provision stretchers for the Complex-warped clips of `desc` and release the
    /// ones no longer needed. Called by `EngineHandle::publish` before the snapshot is sent,
    /// so a new clip's stretcher reaches the engine no later than the snapshot using it.
    pub(crate) fn sync(&mut self, desc: &RenderGraphDesc) {
        self.collect();
        let mut wanted: Vec<ClipId> = match &self.factory {
            None => Vec::new(),
            Some(_) => desc
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .filter(|c| complex_warp(c).is_some())
                .map(|c| c.id)
                .collect(),
        };
        wanted.sort();
        wanted.dedup();
        wanted.truncate(MAX_STRETCH_VOICES);

        let mut i = 0;
        while i < self.live.len() {
            let id = self.live[i];
            if wanted.binary_search(&id).is_err() {
                if self.tx.push(Msg::Remove(id)).is_err() {
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
        for id in wanted {
            let Err(at) = self.live.binary_search(&id) else {
                continue;
            };
            if self.tx.slots() == 0 {
                return;
            }
            let mut st = factory.create();
            st.configure(2, self.sample_rate, self.max_block);
            if self.tx.push(Msg::Add(id, st)).is_ok() {
                self.live.insert(at, id);
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
    /// (native hosts pass `ether_stretch::SignalsmithFactory`). Without a factory (the web
    /// build) Complex clips play Repitch. Takes effect at the next `publish`.
    pub fn set_stretcher_factory(&mut self, factory: Arc<dyn StretcherFactory>) {
        self.warp.factory = Some(factory);
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

fn complex_warp(clip: &ClipDesc) -> Option<&WarpDesc> {
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

struct Voice {
    clip: ClipId,
    st: Box<dyn Stretcher>,
    /// Content beat expected at the next rendered sample (NaN = seek first).
    next_c: f64,
    /// Next source frame to feed (integer-valued).
    next_src: f64,
}

/// Audio-thread half (lives in `Engine`).
pub(crate) struct WarpRt {
    rx: Consumer<Msg>,
    ret: Producer<Box<dyn Stretcher>>,
    /// Sorted by clip id; capacity [`MAX_STRETCH_VOICES`] (never grows).
    voices: Vec<Voice>,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    seek_len: usize,
    /// Stretchers leaked because the return ring was full (should stay 0).
    leaked: u64,
}

impl WarpRt {
    /// RT. Apply provisioning messages from the handle.
    pub(crate) fn drain(&mut self) {
        while let Ok(msg) = self.rx.pop() {
            match msg {
                Msg::Add(clip, st) => match self.voices.binary_search_by(|v| v.clip.cmp(&clip)) {
                    Ok(i) => {
                        let old = std::mem::replace(&mut self.voices[i].st, st);
                        self.voices[i].next_c = f64::NAN;
                        self.retire(old);
                    }
                    Err(i) if self.voices.len() < self.voices.capacity() => {
                        self.voices.insert(
                            i,
                            Voice {
                                clip,
                                st,
                                next_c: f64::NAN,
                                next_src: 0.0,
                            },
                        );
                    }
                    Err(_) => self.retire(st),
                },
                Msg::Remove(clip) => {
                    if let Ok(i) = self.voices.binary_search_by(|v| v.clip.cmp(&clip)) {
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

    /// RT. Render (add) one audio clip for the sub-block: through its stretcher when it is
    /// Complex-warped and has one, else by resampling ([`sched::render_audio`]). Returns
    /// `false` on a source underrun.
    pub(crate) fn render(
        &mut self,
        clip: &ClipDesc,
        source: &dyn AudioSource,
        ref_bpm: f64,
        timing: &Timing<'_>,
        out: [&mut [f32]; 2],
        scratch: &mut [f32],
    ) -> bool {
        if let Some(w) = complex_warp(clip)
            && let Ok(i) = self.voices.binary_search_by(|v| v.clip.cmp(&clip.id))
        {
            return self.render_complex(i, clip, w, source, ref_bpm, timing, out);
        }
        sched::render_audio(clip, source, ref_bpm, timing, out, scratch)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_complex(
        &mut self,
        vi: usize,
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
            ..
        } = &clip.content
        else {
            return true;
        };
        let WarpRt {
            voices,
            in_l,
            in_r,
            out_l,
            out_r,
            seek_len,
            ..
        } = self;
        let voice = &mut voices[vi];
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
                    let g = env.at(timing.beat_at(s as f64));
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
}
