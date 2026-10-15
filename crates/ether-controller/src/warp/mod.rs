//! Controller-side warp logic (owned by the `warp` wave-3 node; see `docs/WAVE3.md`).
//!
//! - [`command`]: `WarpCommand` document edits (`doc/misc.rs` delegates here). Enabling warp
//!   on a clip without markers initializes two markers (content start and media end) from
//!   the BPM-detection stub ([`detect_bpm`]), so the clip follows the project tempo.
//!   Toggling warp (either way) keeps the clip where it is and keeps the audio under its
//!   start: the clip's offset and loop region move to the content beats that read the same
//!   source seconds under the new mapping ([`ContentMap`]); the arrangement start and
//!   length don't change (like Live: turning Warp on doesn't jump the clip's start).
//! - [`warp_desc`]: compiles a clip's warp settings + markers into the engine's
//!   [`WarpDesc`] (`compile.rs` delegates here).

use ether_core::graph::WarpDesc;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::*;
use ether_core::protocol::warp::WarpCommand;

use crate::doc::{DocCtx, clip_start};
use crate::tx::{CmdResult, invalid, not_found, unsupported};

#[cfg(test)]
mod tests;

/// Tempo range the BPM stub picks from.
const DETECT_MIN_BPM: f64 = 80.0;
const DETECT_MAX_BPM: f64 = 160.0;

/// BPM-detection stub (no audio analysis): the tempo in [80, 160) BPM at which `seconds` of
/// audio span a power-of-two number of 4/4 bars (loops are usually 1, 2, 4, 8... bars),
/// rounded to 0.01. `None` for an empty or absurdly long file.
pub(crate) fn detect_bpm(seconds: f64) -> Option<f64> {
    if !(seconds.is_finite() && seconds > 0.0) {
        return None;
    }
    (0..12)
        .map(|k| (1u32 << k) as f64 * 4.0 * 60.0 / seconds)
        .find(|bpm| (DETECT_MIN_BPM..DETECT_MAX_BPM).contains(bpm))
        .map(|bpm| (bpm * 100.0).round() / 100.0)
}

/// `WarpCommand::DetectTempo`: [`detect_bpm`] on the clip's media length.
pub(crate) fn detect_tempo(p: &Project, clip: ClipId) -> CmdResult<ReplyValue> {
    let c = p
        .clips
        .get(&clip)
        .ok_or_else(|| not_found(format!("clip {clip}")))?;
    let a = audio_of(c)?;
    let m = p
        .media
        .get(&a.media)
        .ok_or_else(|| not_found(format!("media {}", a.media)))?;
    let bpm = detect_bpm(m.frames as f64 / m.sample_rate.max(1) as f64);
    Ok(ReplyValue::Tempo { bpm })
}

/// Compile a clip's warp into a [`WarpDesc`] (`None` = unwarped: the engine plays content
/// beats at the tempo in effect at the clip start).
///
/// Markers are sorted by beat, deduplicated (same beat within epsilon: the first wins) and
/// must be finite. With two or more markers the mapping is piecewise linear and extends
/// past the edge markers with the edge segments' slopes. With one marker (or none) the
/// slope comes from `source_bpm` through that marker (or through the content start); with
/// no usable `source_bpm` either, the clip plays unwarped.
pub(crate) fn warp_desc(p: &Project, clip: &Clip, audio: &AudioContent) -> Option<WarpDesc> {
    if !audio.warp.enabled {
        return None;
    }
    let mut markers: Vec<(f64, f64)> = p
        .warp_markers_of(clip.id)
        .into_iter()
        .map(|m| (m.beat.0, m.source.0))
        .filter(|(b, s)| b.is_finite() && s.is_finite())
        .collect();
    markers.dedup_by(|b, a| Beats(a.0).approx_eq(Beats(b.0)));
    if markers.len() < 2 {
        let bpm = audio
            .warp
            .source_bpm
            .filter(|b| b.is_finite() && *b > 0.0)?;
        let (b0, s0) = markers.first().copied().unwrap_or((0.0, 0.0));
        markers = vec![(b0, s0), (b0 + 1.0, s0 + 60.0 / bpm)];
    }
    Some(WarpDesc {
        mode: audio.warp.mode,
        markers,
    })
}

/// The engine's content beat → source seconds mapping of an audio clip before repitch
/// (`ether_core::sched::source_seconds`: piecewise linear through the warp markers,
/// extrapolated past the edge markers with the edge slopes; unwarped, the tempo at the clip
/// start), and its inverse.
pub(crate) struct ContentMap {
    markers: Option<Vec<(f64, f64)>>,
    ref_bpm: f64,
}

impl ContentMap {
    pub(crate) fn of(p: &Project, clip: &Clip, audio: &AudioContent) -> Self {
        Self {
            markers: warp_desc(p, clip, audio)
                .map(|w| w.markers)
                .filter(|m| m.len() >= 2),
            ref_bpm: p.tempo_map().bpm_at(clip_start(clip)).max(1e-6),
        }
    }

    /// Source seconds at content beat `c`.
    pub(crate) fn forward(&self, c: f64) -> f64 {
        match &self.markers {
            Some(m) => {
                let i = m.partition_point(|&(b, _)| b <= c).clamp(1, m.len() - 1);
                let ((b0, s0), (b1, s1)) = (m[i - 1], m[i]);
                if b1 - b0 <= 0.0 {
                    s0
                } else {
                    s0 + (c - b0) * (s1 - s0) / (b1 - b0)
                }
            }
            None => c * 60.0 / self.ref_bpm,
        }
    }

    /// Content beat reading source second `s` (assumes sources increase with beats).
    pub(crate) fn inverse(&self, s: f64) -> f64 {
        match &self.markers {
            Some(m) => {
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
            None => s * self.ref_bpm / 60.0,
        }
    }
}

fn audio_of(clip: &Clip) -> CmdResult<&AudioContent> {
    match &clip.content {
        ClipContent::Audio(a) => Ok(a),
        ClipContent::Midi => Err(invalid(format!("clip {} is not an audio clip", clip.id))),
    }
}

fn marker_values(beat: Beats, source: Seconds) -> CmdResult<(Beats, Seconds)> {
    if !(beat.0.is_finite() && source.0.is_finite()) {
        return Err(invalid("warp marker times must be finite"));
    }
    Ok((beat, Seconds(source.0.max(0.0))))
}

/// Closest a moved marker gets to its neighbours (beats; the warp editor's `MIN_GAP`).
pub(crate) const MARKER_MIN_GAP: f64 = 1.0 / 64.0;

/// Clamp a move of marker `m` to `(beat, source)` between its neighbours (by beat, among
/// the clip's other markers): a marker never crosses another, so a move only re-times the
/// audio between its two neighbours and the mapping outside them stays put (Live). The
/// beat keeps [`MARKER_MIN_GAP`] from each neighbour (the midpoint when they are closer);
/// the source stays within the neighbours' sources, so the mapping keeps increasing.
fn between_neighbours(
    p: &Project,
    m: &WarpMarker,
    beat: Beats,
    source: Seconds,
) -> (Beats, Seconds) {
    let others: Vec<&WarpMarker> = p
        .warp_markers_of(m.clip)
        .into_iter()
        .filter(|o| o.id != m.id)
        .collect();
    let prev = others.iter().rev().find(|o| o.beat.0 < m.beat.0);
    let next = others.iter().find(|o| o.beat.0 > m.beat.0);
    let (mut b, mut s) = (beat.0, source.0);
    match (prev, next) {
        (Some(a), Some(z)) if z.beat.0 - a.beat.0 <= 2.0 * MARKER_MIN_GAP => {
            b = (a.beat.0 + z.beat.0) / 2.0;
        }
        _ => {
            if let Some(a) = prev {
                b = b.max(a.beat.0 + MARKER_MIN_GAP);
            }
            if let Some(z) = next {
                b = b.min(z.beat.0 - MARKER_MIN_GAP);
            }
        }
    }
    if let Some(a) = prev {
        s = s.max(a.source.0);
    }
    if let Some(z) = next {
        s = s.min(z.source.0);
    }
    (Beats(b), Seconds(s))
}

/// Apply a `WarpCommand` (document edit).
pub(crate) fn command(ctx: &mut DocCtx, c: &WarpCommand) -> CmdResult<()> {
    match c {
        WarpCommand::SetWarp { clip, warp } => set_warp(ctx, *clip, *warp),
        WarpCommand::AddMarker {
            id,
            clip,
            beat,
            source,
        } => {
            if ctx.p().warp_markers.contains_key(id) {
                return Ok(());
            }
            audio_of(&ctx.clip(*clip)?)?;
            let (beat, source) = marker_values(*beat, *source)?;
            ctx.tx.insert(Entity::WarpMarker(WarpMarker {
                id: *id,
                clip: *clip,
                beat,
                source,
            }))
        }
        WarpCommand::MoveMarker { id, beat, source } => {
            let Some(m) = ctx.p().warp_markers.get(id).cloned() else {
                return Err(not_found(format!("warp marker {id}")));
            };
            let (beat, source) = marker_values(*beat, *source)?;
            let (beat, source) = between_neighbours(ctx.p(), &m, beat, source);
            if !m.beat.approx_eq(beat) {
                ctx.tx.update(EntityUpdate::WarpMarker {
                    id: *id,
                    change: WarpMarkerChange::Beat(beat),
                })?;
            }
            if m.source != source {
                ctx.tx.update(EntityUpdate::WarpMarker {
                    id: *id,
                    change: WarpMarkerChange::Source(source),
                })?;
            }
            Ok(())
        }
        WarpCommand::RemoveMarker { id } => {
            if !ctx.p().warp_markers.contains_key(id) {
                return Err(not_found(format!("warp marker {id}")));
            }
            ctx.tx.remove(EntityKey::WarpMarker(*id))
        }
        WarpCommand::DetectTempo { .. } => Err(unsupported("not a document command")),
    }
}

/// `SetWarp`. Turning warp on for a clip without markers pins the content start to the
/// media start and the media end to the beat it reaches at the detected tempo (falling
/// back to the given `source_bpm`, then the project tempo at the clip start), and records
/// that tempo as `source_bpm`. Toggling warp keeps the audio under the clip
/// ([`keep_content`]).
fn set_warp(ctx: &mut DocCtx, clip_id: ClipId, mut warp: WarpSettings) -> CmdResult<()> {
    let clip = ctx.clip(clip_id)?;
    let audio = audio_of(&clip)?.clone();
    let before = ContentMap::of(ctx.p(), &clip, &audio);
    if let Some(bpm) = warp.source_bpm
        && !(bpm.is_finite() && bpm > 0.0)
    {
        return Err(invalid("source_bpm must be positive"));
    }
    let turning_on = warp.enabled && !audio.warp.enabled;
    let has_markers = !ctx.p().warp_markers_of(clip_id).is_empty();
    let media = ctx.p().media.get(&audio.media).cloned();
    if turning_on
        && !has_markers
        && let Some(m) = media.filter(|m| m.frames > 0 && m.sample_rate > 0)
    {
        let seconds = m.frames as f64 / m.sample_rate as f64;
        let bpm = detect_bpm(seconds)
            .or(warp.source_bpm)
            .unwrap_or_else(|| ctx.p().tempo_map().bpm_at(clip.start));
        warp.source_bpm = Some(bpm);
        for (beat, source) in [(0.0, 0.0), (seconds * bpm / 60.0, seconds)] {
            let id = ctx.new_id();
            ctx.tx.insert(Entity::WarpMarker(WarpMarker {
                id,
                clip: clip_id,
                beat: Beats(beat),
                source: Seconds(source),
            }))?;
        }
    }
    if audio.warp != warp {
        ctx.set_clip(clip_id, ClipChange::Warp(warp))?;
    }
    if audio.warp.enabled != warp.enabled {
        keep_content(ctx, clip_id, &before)?;
    }
    Ok(())
}

/// After the content → source mapping of a clip changed from `before` (warp toggled): move
/// its offset and loop region to the content beats that read the same source seconds, so
/// the clip keeps playing the same audio from its (unchanged) arrangement start. A leading
/// gap (content before 0) is silence and keeps its length.
fn keep_content(ctx: &mut DocCtx, clip_id: ClipId, before: &ContentMap) -> CmdResult<()> {
    let clip = ctx.clip(clip_id)?;
    let after = ContentMap::of(ctx.p(), &clip, audio_of(&clip)?);
    let remap = |c: f64| {
        if c < 0.0 {
            return c;
        }
        let r = after.inverse(before.forward(c));
        if r.is_finite() { r.max(0.0) } else { c }
    };
    let offset = Beats(remap(clip.offset.0));
    if !offset.approx_eq(clip.offset) {
        ctx.set_clip(clip_id, ClipChange::Offset(offset))?;
    }
    let lp = clip.looping;
    let (start, end) = (Beats(remap(lp.start.0)), Beats(remap(lp.end.0)));
    if end.0 > start.0 && !(start.approx_eq(lp.start) && end.approx_eq(lp.end)) {
        ctx.set_clip(clip_id, ClipChange::Loop(ClipLoop { start, end, ..lp }))?;
    }
    Ok(())
}
