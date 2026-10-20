//! Reversing an audio clip (`AudioContent::reversed`).
//!
//! A reversed clip plays the reversed media `R[i] = M[N - 1 - i]` with its offset, loop
//! region and warp markers on that reversed timeline (the engine only mirrors the final
//! frame lookup). So toggling the flag alone would play the *other end* of the file. A
//! reverse therefore also mirrors that geometry, so that a clip playing media frames
//! `a ..= b` plays `b ..= a` once reversed: the same material, backwards.
//!
//! [`mirror`] computes the mirrored geometry. The content → source map is the engine's
//! (piecewise linear through the warp markers, extrapolated with the edge slopes; unwarped,
//! the tempo at the clip start). With `T` = the media length in seconds and `inv` its
//! inverse, the map mirrors around `K = inv(0) + inv(T)`: a marker `(b, s)` becomes
//! `(K - b, T - s)`, which maps the file's whole extent onto itself, so reversing twice
//! gives the input back. Content positions (offset, loop region, clip envelopes) mirror
//! around [`ReverseMirror::axis`]: `c -> axis - c`, with `axis = K` unless the clip is
//! repitched (`axis` then accounts for the playback rate, see [`mirror`]).
//!
//! Fades are in arrangement time at the clip edges and do not move. A looped clip mirrors
//! its loop region, and its start offset keeps its place in the loop (the mirror of the
//! offset, or the loop start when the offset is at or before the loop start).
//!
//! Exact (sample for sample) for unwarped clips, warped clips without a repitch
//! transpose, and repitched clips with a linear map; a repitch transpose over a non-linear
//! warp map pivots around the offset, so the mirror is the closest linear one there.

use crate::clip::ClipLoop;
use crate::value::Beats;
use crate::warp::{WarpMode, WarpSettings};

/// Shortest content loop the engine honours (beats; `ether_core::sched::MIN_LOOP`).
const MIN_LOOP: f64 = 1.0 / 256.0;

/// What [`mirror`] needs to know about an audio clip.
#[derive(Clone, Debug)]
pub struct ReverseInput<'a> {
    pub offset: f64,
    pub length: f64,
    pub looping: ClipLoop,
    /// The clip's warp markers `(beat, source seconds)` in beat order.
    pub markers: &'a [(f64, f64)],
    pub warp: WarpSettings,
    pub transpose: f32,
    /// Tempo at the clip start (the unwarped map's tempo).
    pub ref_bpm: f64,
    pub frames: u64,
    pub sample_rate: u32,
}

/// The mirrored geometry of a clip (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct ReverseMirror {
    /// Content positions mirror as `c -> axis - c` (loop region, clip envelopes).
    pub axis: f64,
    pub offset: f64,
    pub looping: ClipLoop,
    /// The warp markers mirrored, in beat order (the input's order reversed).
    pub markers: Vec<(f64, f64)>,
}

/// The engine's content → source map as points (≥ 2) and whether it repitches
/// (`ether_controller::warp::warp_desc` / `ether_core::warp`).
fn content_map(i: &ReverseInput<'_>) -> (Vec<(f64, f64)>, bool) {
    if i.warp.enabled {
        let mut m: Vec<(f64, f64)> = i
            .markers
            .iter()
            .copied()
            .filter(|(b, s)| b.is_finite() && s.is_finite())
            .collect();
        m.dedup_by(|b, a| Beats(a.0).approx_eq(Beats(b.0)));
        if m.len() >= 2 {
            return (m, i.warp.mode == WarpMode::Repitch);
        }
        if let Some(bpm) = i.warp.source_bpm.filter(|b| b.is_finite() && *b > 0.0) {
            let (b0, s0) = m.first().copied().unwrap_or((0.0, 0.0));
            return (
                vec![(b0, s0), (b0 + 1.0, s0 + 60.0 / bpm)],
                i.warp.mode == WarpMode::Repitch,
            );
        }
    }
    (vec![(0.0, 0.0), (1.0, 60.0 / i.ref_bpm.max(1e-6))], true)
}

/// Content beat reading source second `s` on the map `m` (sources increasing with beats).
fn inverse(m: &[(f64, f64)], s: f64) -> f64 {
    let i = m
        .partition_point(|&(_, src)| src <= s)
        .clamp(1, m.len() - 1);
    let ((b0, s0), (b1, s1)) = (m[i - 1], m[i]);
    if (s1 - s0).abs() <= 1e-12 {
        b0
    } else {
        b0 + (s - s0) * (b1 - b0) / (s1 - s0)
    }
}

/// The geometry that makes the clip play the same material backwards once its `reversed`
/// flag is toggled (either way: the mirror is its own inverse). `None` when the media
/// length is unknown or the map is degenerate (the flag is then toggled alone).
pub fn mirror(i: &ReverseInput<'_>) -> Option<ReverseMirror> {
    if i.frames == 0 || i.sample_rate == 0 {
        return None;
    }
    let sr = i.sample_rate as f64;
    // The clip plays whole frames: frames `a .. a + n` reversed are `N - a - n .. N - a`
    // of the reversed media, so the media mirrors over its full length `N / sr`.
    let t = i.frames as f64 / sr;
    let (map, repitch) = content_map(i);
    let k = inverse(&map, 0.0) + inverse(&map, t);
    let rate = if repitch && i.transpose != 0.0 && i.transpose.is_finite() {
        (i.transpose as f64 / 12.0).exp2()
    } else {
        1.0
    };
    // Repitched, `length` beats read `rate` times as much source, pivoting at the offset:
    // frames `a, a + r, .., a + r (n - 1)` reversed start at `N - 1 - a - r (n - 1)`.
    // One source frame in beats, at the map's slope (exact for a linear map).
    let axis = if rate == 1.0 {
        k
    } else {
        let (b0, s0) = map[0];
        let (b1, s1) = map[1];
        let frame = if s1 > s0 {
            (b1 - b0) / (s1 - s0) / sr
        } else {
            0.0
        };
        k + (1.0 - rate) * (i.length - frame)
    };
    if !(k.is_finite() && axis.is_finite()) {
        return None;
    }
    let m = |c: f64| axis - c;
    let lp = i.looping;
    let looping = ClipLoop {
        enabled: lp.enabled,
        start: Beats(m(lp.end.0).max(0.0)),
        end: Beats(m(lp.start.0)),
    };
    let looped = lp.enabled && lp.end.0 - lp.start.0 >= MIN_LOOP && i.offset < lp.end.0;
    let offset = if looped {
        let o = m(i.offset);
        if o >= looping.end.0 - Beats::EPSILON {
            looping.start.0
        } else {
            o
        }
    } else {
        m(i.offset) - i.length
    };
    let markers = i
        .markers
        .iter()
        .rev()
        .map(|&(b, s)| (k - b, t - s))
        .collect();
    Some(ReverseMirror {
        axis,
        offset,
        looping,
        markers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(markers: &[(f64, f64)]) -> ReverseInput<'_> {
        ReverseInput {
            offset: 1.0,
            length: 2.0,
            looping: ClipLoop {
                enabled: false,
                start: Beats(0.0),
                end: Beats(8.0),
            },
            markers,
            warp: WarpSettings::default(),
            transpose: 0.0,
            ref_bpm: 120.0,
            frames: 48_000 * 4,
            sample_rate: 48_000,
        }
    }

    #[test]
    fn unwarped_offset_mirrors_over_the_file() {
        // 4 s at 120 bpm = 8 beats; content 1..3 plays from 5..7 of the reversed file.
        let r = mirror(&input(&[])).unwrap();
        assert!((r.axis - 8.0).abs() < 1e-12);
        assert!((r.offset - 5.0).abs() < 1e-12);
        assert_eq!((r.looping.start.0, r.looping.end.0), (0.0, 8.0));
    }

    #[test]
    fn twice_is_identity() {
        let markers = [(0.0, 0.0), (2.0, 1.5), (6.0, 4.0)];
        let mut i = input(&markers);
        i.warp.enabled = true;
        i.looping = ClipLoop {
            enabled: true,
            start: Beats(1.0),
            end: Beats(3.0),
        };
        i.offset = 1.5;
        let once = mirror(&i).unwrap();
        let back = once.markers.clone();
        let mut j = i.clone();
        j.markers = &back;
        j.offset = once.offset;
        j.looping = once.looping;
        let twice = mirror(&j).unwrap();
        assert!((twice.offset - 1.5).abs() < 1e-12);
        assert_eq!(twice.looping, i.looping);
        for (a, b) in twice.markers.iter().zip(&markers) {
            assert!((a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12);
        }
    }

    #[test]
    fn looped_offset_at_the_loop_start_stays_at_the_loop_start() {
        let mut i = input(&[]);
        i.looping = ClipLoop {
            enabled: true,
            start: Beats(1.0),
            end: Beats(3.0),
        };
        let r = mirror(&i).unwrap();
        assert_eq!((r.looping.start.0, r.looping.end.0), (5.0, 7.0));
        assert_eq!(r.offset, 5.0);
    }
}
