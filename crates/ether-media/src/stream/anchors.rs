//! Where playback of a streamed media starts without a prefetch hint beforehand, from the
//! published graph: clip starts, clip loop starts, the transport loop start, and the
//! position under the playhead on a locate or play. Hosts pin (anchors) or prime (locate)
//! those chunks so the first read after the jump finds them.
//!
//! Same content → source mapping as the engine's clip reader (`ether_core::sched`): the
//! content position of timeline beat `t` follows the clip offset and its content loop; the
//! source seconds come from the warp markers or, unwarped, the tempo at the clip start;
//! transpose scales around the clip offset; reversed clips mirror the frame. Only used to
//! choose chunks, so a few frames of error don't matter.

use std::collections::BTreeMap;

use ether_core::graph::{ClipContentDesc, ClipDesc, WarpDesc};
use ether_core::protocol::model::MediaId;
use ether_core::tempo::TempoMapRt;
use ether_core::RenderGraphDesc;

/// Frames before a jump target also kept (the stretcher and interpolation read a little
/// before the content position).
pub const ANCHOR_PREROLL: u64 = 2048;

const MIN_LOOP: f64 = 1e-6;

fn source_seconds(warp: Option<&WarpDesc>, ref_bpm: f64, c: f64) -> f64 {
    match warp {
        Some(w) if w.markers.len() >= 2 => {
            let m = &w.markers;
            let i = m.partition_point(|&(b, _)| b <= c).clamp(1, m.len() - 1);
            let (b0, s0) = m[i - 1];
            let (b1, s1) = m[i];
            if b1 - b0 <= 0.0 {
                return s0;
            }
            s0 + (c - b0) * (s1 - s0) / (b1 - b0)
        }
        _ => c * 60.0 / ref_bpm.max(1e-6),
    }
}

/// Content position (beats) of `clip` at timeline beat `t`, if inside the clip.
pub fn content_at(clip: &ClipDesc, t: f64) -> Option<f64> {
    let p = t - clip.start;
    if p < 0.0 || p >= clip.length.max(0.0) {
        return None;
    }
    let looping = clip
        .looping
        .filter(|&(ls, le)| le - ls >= MIN_LOOP && clip.offset < le);
    Some(match looping {
        Some((ls, le)) if p >= le - clip.offset => ls + (p - (le - clip.offset)) % (le - ls),
        _ => clip.offset + p,
    })
}

/// Engine frame of `media` played at content position `c` of `clip` (`None` if not audio).
fn frame_of(clip: &ClipDesc, c: f64, tempo: &TempoMapRt, rate: f64, total: u64) -> Option<u64> {
    let ClipContentDesc::Audio {
        warp,
        transpose,
        reversed,
        ..
    } = &clip.content
    else {
        return None;
    };
    let ref_bpm = tempo.bpm_at(clip.start);
    let mut s = source_seconds(warp.as_ref(), ref_bpm, c);
    if *transpose != 0.0 && transpose.is_finite() {
        let s0 = source_seconds(warp.as_ref(), ref_bpm, clip.offset);
        s = s0 + (s - s0) * (*transpose as f64 / 12.0).exp2();
    }
    let f = (s * rate).max(0.0);
    let last = total.saturating_sub(1) as f64;
    let f = if *reversed { (last - f).max(0.0) } else { f };
    Some((f as u64).min(total.saturating_sub(1)))
}

fn media_of(clip: &ClipDesc) -> Option<MediaId> {
    match &clip.content {
        ClipContentDesc::Audio { media, .. } if !clip.muted => Some(*media),
        _ => None,
    }
}

/// Engine frames each streamed media is read at when the playhead is at beat `beat`.
/// `frames(media)` = the media's engine-rate length if it streams, `None` otherwise.
pub fn positions_at(
    graph: &RenderGraphDesc,
    beat: f64,
    engine_rate: u32,
    frames: &dyn Fn(MediaId) -> Option<u64>,
) -> Vec<(MediaId, u64)> {
    let tempo = TempoMapRt::compile(&graph.tempo, &graph.signatures);
    let mut out = Vec::new();
    for clip in graph.tracks.iter().flat_map(|t| t.clips.iter()) {
        let Some(media) = media_of(clip) else { continue };
        let Some(total) = frames(media) else { continue };
        if let Some(c) = content_at(clip, beat)
            && let Some(f) = frame_of(clip, c, &tempo, engine_rate as f64, total)
        {
            out.push((media, f));
        }
    }
    out
}

/// Anchors per streamed media: clip starts, clip content-loop starts and, with the
/// transport loop on, the loop start (each with [`ANCHOR_PREROLL`]), as engine frames.
pub fn anchors(
    graph: &RenderGraphDesc,
    engine_rate: u32,
    frames: &dyn Fn(MediaId) -> Option<u64>,
) -> BTreeMap<MediaId, Vec<u64>> {
    let tempo = TempoMapRt::compile(&graph.tempo, &graph.signatures);
    let rate = engine_rate as f64;
    let mut out: BTreeMap<MediaId, Vec<u64>> = BTreeMap::new();
    let mut add = |media: MediaId, f: u64| {
        let v = out.entry(media).or_default();
        for x in [f.saturating_sub(ANCHOR_PREROLL), f] {
            if !v.contains(&x) {
                v.push(x);
            }
        }
    };
    // The transport loop first: it wraps on every pass.
    if graph.loop_enabled {
        for (media, f) in positions_at(graph, graph.loop_start, engine_rate, frames) {
            add(media, f);
        }
    }
    for clip in graph.tracks.iter().flat_map(|t| t.clips.iter()) {
        let Some(media) = media_of(clip) else { continue };
        let Some(total) = frames(media) else { continue };
        if let Some(f) = frame_of(clip, clip.offset, &tempo, rate, total) {
            add(media, f);
        }
        if let Some((ls, le)) = clip.looping
            && le - ls >= MIN_LOOP
            && clip.offset < le
            && let Some(f) = frame_of(clip, ls, &tempo, rate, total)
        {
            add(media, f);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::graph::TrackDesc;
    use ether_core::protocol::model::{ClipId, TrackId, TrackKind, Ulid, WarpMode};
    use ether_core::tempo::TempoPointDesc;

    fn audio_clip(start: f64, length: f64, offset: f64, looping: Option<(f64, f64)>) -> ClipDesc {
        ClipDesc {
            id: ClipId::NIL,
            start,
            length,
            offset,
            looping,
            muted: false,
            content: ClipContentDesc::Audio {
                media: MediaId(Ulid(7)),
                gain: 1.0,
                transpose: 0.0,
                fade_in: 0.0,
                fade_out: 0.0,
                warp: None,
                fade_in_curve: Default::default(),
                fade_out_curve: Default::default(),
                reversed: false,
            },
            envelopes: vec![],
        }
    }

    fn graph(clips: Vec<ClipDesc>) -> RenderGraphDesc {
        let track = TrackDesc {
            id: TrackId::NIL,
            kind: TrackKind::Audio,
            chain: vec![],
            output: None,
            group: None,
            sends: vec![],
            volume: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            audio_input: None,
            monitor: false,
            armed: false,
            clips,
            automation: vec![],
            racks: vec![],
            frozen: None,
            chain_racks: vec![],
            modulation: Default::default(),
            input_tap: None,
            vca: None,
            expression: Default::default(),
            hw_io: vec![],
        };
        RenderGraphDesc {
            tempo: vec![TempoPointDesc {
                beat: 0.0,
                bpm: 120.0,
                curve: Default::default(),
            }],
            tracks: vec![track],
            ..Default::default()
        }
    }

    const M: MediaId = MediaId(Ulid(7));

    #[test]
    fn content_positions_follow_offset_and_loop() {
        let c = audio_clip(4.0, 16.0, 2.0, Some((1.0, 3.0)));
        assert_eq!(content_at(&c, 3.0), None);
        assert_eq!(content_at(&c, 4.0), Some(2.0));
        assert_eq!(content_at(&c, 4.5), Some(2.5));
        // Loop end (3.0) reached after 1 beat: back to the loop start.
        assert_eq!(content_at(&c, 5.0), Some(1.0));
        assert_eq!(content_at(&c, 6.5), Some(2.5));
        assert_eq!(content_at(&c, 7.0), Some(1.0));
        assert_eq!(content_at(&c, 20.0), None);
    }

    #[test]
    fn locate_and_anchor_frames() {
        let frames = |m: MediaId| (m == M).then_some(48_000 * 600);
        // 120 BPM: one beat = 0.5 s = 24 000 frames. Clip at beat 8 playing from content 4.
        let mut g = graph(vec![audio_clip(8.0, 100.0, 4.0, None)]);
        assert_eq!(positions_at(&g, 10.0, 48_000, &frames), vec![(M, 6 * 24_000)]);
        assert!(positions_at(&g, 7.0, 48_000, &frames).is_empty());
        g.loop_enabled = true;
        g.loop_start = 20.0;
        let a = anchors(&g, 48_000, &frames);
        let v = &a[&M];
        assert!(v.contains(&(16 * 24_000)), "loop start: {v:?}");
        assert!(v.contains(&(4 * 24_000)), "clip start: {v:?}");
        assert!(v.contains(&(4 * 24_000 - ANCHOR_PREROLL)));
        // Media that don't stream have no anchors.
        assert!(anchors(&g, 48_000, &|_| None).is_empty());
    }

    #[test]
    fn warp_transpose_and_reverse() {
        let frames = |_| Some(48_000 * 100);
        let mut clip = audio_clip(0.0, 64.0, 0.0, None);
        if let ClipContentDesc::Audio { warp, .. } = &mut clip.content {
            *warp = Some(WarpDesc {
                mode: WarpMode::Complex,
                markers: vec![(0.0, 1.0), (4.0, 3.0)],
            });
        }
        let g = graph(vec![clip.clone()]);
        // Beat 2 → 2 s.
        assert_eq!(positions_at(&g, 2.0, 48_000, &frames)[0].1, 96_000);
        if let ClipContentDesc::Audio { reversed, .. } = &mut clip.content {
            *reversed = true;
        }
        let g = graph(vec![clip]);
        assert_eq!(positions_at(&g, 2.0, 48_000, &frames)[0].1, 48_000 * 100 - 1 - 96_000);
    }
}
