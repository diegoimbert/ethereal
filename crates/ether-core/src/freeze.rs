//! Frozen-track playback (v0.2, owned by the `freeze-bounce` node; CONTRACTS.md §12.3).
//!
//! A frozen track's desc carries [`FrozenDesc`] (`TrackDesc::frozen`); the controller then
//! compiles it **without clips and without device chain** (its nodes are not instantiated).
//! The engine calls [`render_frozen`] where clips would render (pre-wired in `engine.rs`,
//! once per sub-block, playing only): it adds the render at song time into the track's
//! buffer, before the (empty) chain, fader, sends and meters.
//!
//! Mapping: output sample `o` of the sub-block (`TransportInfo::seconds` = song time of
//! sample 0) reads media frame `round((seconds + o / sample_rate - start_seconds) ·
//! media_rate)`; the media is resampled to the engine rate by the controller when loaded
//! (like every source), so at equal rates this is an integer offset. Outside the media:
//! silence. Loop wraps and locates need no state (position-addressed reads).
//! Mono renders play on both channels; wider ones use their first two channels.

use std::sync::Arc;

use ether_protocol::model::MediaId;
use serde::{Deserialize, Serialize};

use crate::media::AudioSource;
use crate::mixer::Stereo;
use crate::transport::TransportInfo;

/// A frozen track's render (`Track::freeze`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrozenDesc {
    /// The render (registered with `EngineHandle::add_source` like clip media).
    pub media: MediaId,
    /// Song time of the media's first frame (seconds).
    pub start_seconds: f64,
}

/// Samples read from the source per chunk (stack scratch: no allocation).
const CHUNK: usize = 256;

/// RT. Add the frozen render for the sub-block into `out[..][..frames]`. Returns `false` on a
/// source underrun (counted like clip underruns). A render whose media is not loaded (yet) is
/// silent, like a clip whose source is missing. Mono renders play on both channels.
pub(crate) fn render_frozen(
    desc: &FrozenDesc,
    sources: &[(MediaId, Arc<dyn AudioSource>)],
    info: &TransportInfo,
    sample_rate: f64,
    out: &mut Stereo,
    frames: usize,
) -> bool {
    let Ok(i) = sources.binary_search_by(|e| e.0.cmp(&desc.media)) else {
        return true;
    };
    let source = &*sources[i].1;
    add_frozen(source, desc.start_seconds, info.seconds, sample_rate, out, frames)
}

/// RT. [`render_frozen`] for one source: media frame of output sample `o` is
/// `round((seconds - start_seconds) · sample_rate) + o`.
fn add_frozen(
    source: &dyn AudioSource,
    start_seconds: f64,
    seconds: f64,
    sample_rate: f64,
    out: &mut Stereo,
    frames: usize,
) -> bool {
    let total = source.frames() as i64;
    let first = ((seconds - start_seconds) * sample_rate).round();
    if !first.is_finite() {
        return true;
    }
    let first = first as i64;
    // Output range overlapping the media: [o0, o1).
    let o0 = (-first).clamp(0, frames as i64) as usize;
    let o1 = (total - first).clamp(0, frames as i64) as usize;
    if o0 >= o1 {
        return true;
    }
    let channels = source.channels().max(1);
    let mut ok = true;
    let mut buf = [0.0f32; CHUNK];
    let [l, r] = out;
    let mut o = o0;
    while o < o1 {
        let n = CHUNK.min(o1 - o);
        let start = (first + o as i64) as u64;
        for (ch, dst) in [(0u16, &mut *l), (1u16, &mut *r)] {
            let src_ch = ch.min(channels - 1);
            ok &= source.read(src_ch, start, &mut buf[..n]);
            for (d, s) in dst[o..o + n].iter_mut().zip(&buf[..n]) {
                *d += s;
            }
        }
        o += n;
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ramp(u16, u64);
    impl AudioSource for Ramp {
        fn channels(&self) -> u16 {
            self.0
        }
        fn frames(&self) -> u64 {
            self.1
        }
        fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
            for (i, o) in out.iter_mut().enumerate() {
                let f = start + i as u64;
                *o = if f < self.1 {
                    f as f32 + 0.5 * channel as f32
                } else {
                    0.0
                };
            }
            true
        }
    }

    fn run(src: &Ramp, start: f64, seconds: f64, frames: usize) -> Stereo {
        let mut out = [vec![0.0; frames], vec![0.0; frames]];
        assert!(add_frozen(src, start, seconds, 10.0, &mut out, frames));
        out
    }

    #[test]
    fn reads_at_song_time() {
        let src = Ramp(2, 100);
        let out = run(&src, 0.0, 1.0, 4);
        assert_eq!(out[0], vec![10.0, 11.0, 12.0, 13.0]);
        assert_eq!(out[1], vec![10.5, 11.5, 12.5, 13.5]);
    }

    #[test]
    fn silence_before_start_and_after_end() {
        let src = Ramp(1, 5);
        // Song second 0.8 with a render starting at 1.0: two silent samples first.
        let out = run(&src, 1.0, 0.8, 4);
        assert_eq!(out[0], vec![0.0, 0.0, 0.0, 1.0]);
        // Mono plays on both channels.
        assert_eq!(out[1], out[0]);
        let out = run(&src, 0.0, 0.3, 4);
        assert_eq!(out[0], vec![3.0, 4.0, 0.0, 0.0]);
        let out = run(&src, 0.0, 9.0, 4);
        assert!(out[0].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn adds_into_the_buffer_across_chunks() {
        let n = CHUNK * 2 + 7;
        let src = Ramp(2, 10_000);
        let mut out = [vec![1.0; n], vec![1.0; n]];
        assert!(add_frozen(&src, 0.0, 0.0, 10.0, &mut out, n));
        for (i, s) in out[0].iter().enumerate() {
            assert_eq!(*s, i as f32 + 1.0);
        }
    }

    #[test]
    fn missing_media_is_silent() {
        let desc = FrozenDesc {
            media: MediaId(ether_protocol::model::Ulid(9)),
            start_seconds: 0.0,
        };
        let mut out = [vec![0.0; 8], vec![0.0; 8]];
        assert!(render_frozen(
            &desc,
            &[],
            &TransportInfo::STOPPED,
            48_000.0,
            &mut out,
            8
        ));
        assert!(out[0].iter().all(|&s| s == 0.0));
    }
}
