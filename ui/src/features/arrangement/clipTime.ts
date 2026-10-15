/**
 * Clip time math for the arrangement (pure):
 * - the one accessor for a clip's timeline position (`startOf`);
 * - timeline → content-beat segments (loop unrolling);
 * - content beats → source seconds for audio clips (warp markers / source tempo).
 */

import type { Beats, Clip, MediaRef, WarpMarker } from "@/generated";
import { BEATS_EPSILON } from "@/state";

/** Timeline start of an arrangement clip. */
export function startOf(clip: Clip): Beats {
  return clip.start;
}

/** Timeline end of an arrangement clip. */
export function endOf(clip: Clip): Beats {
  return startOf(clip) + clip.length;
}

export function isArrangementClip(clip: Clip): boolean {
  return clip.lane == null;
}

/** A piece of a clip where timeline time maps linearly onto content time. */
export interface ContentSegment {
  /** Timeline start/end of the piece. */
  t0: Beats;
  t1: Beats;
  /** Content beat played at `t0`. */
  c0: Beats;
}

/** Hard cap on unrolled loop repetitions (tiny loops on huge clips). */
const MAX_SEGMENTS = 4096;

/**
 * The clip as timeline → content segments, restricted to `[from, to)`. Unlooped clips are
 * one segment starting at `offset`. Looped clips play from `offset` up to the loop end, then
 * repeat `[loop.start, loop.end)` until the clip ends.
 */
export function contentSegments(
  clip: Pick<Clip, "length" | "offset" | "looping">,
  start: Beats,
  from: Beats = -Infinity,
  to: Beats = Infinity,
): ContentSegment[] {
  const end = start + clip.length;
  const lo = Math.max(start, from);
  const hi = Math.min(end, to);
  if (hi <= lo) return [];
  const { enabled, start: ls, end: le } = clip.looping;
  const loopLen = le - ls;
  if (!enabled || loopLen <= BEATS_EPSILON) return [{ t0: lo, t1: hi, c0: clip.offset + (lo - start) }];

  const out: ContentSegment[] = [];
  let t = start;
  let c = clip.offset;
  if (c >= le - BEATS_EPSILON) c = ls + ((((c - ls) % loopLen) + loopLen) % loopLen);
  // First piece: offset → loop end.
  const first = Math.min(end, t + (le - c));
  if (first > lo && t < hi) out.push(clampSeg({ t0: t, t1: first, c0: c }, lo, hi));
  t = first;
  // Skip whole repetitions before `lo`.
  if (t < lo) t += Math.floor((lo - t) / loopLen) * loopLen;
  while (t < hi - BEATS_EPSILON && out.length < MAX_SEGMENTS) {
    const t1 = Math.min(end, t + loopLen);
    if (t1 > lo) out.push(clampSeg({ t0: t, t1, c0: ls }, lo, hi));
    t = t1;
  }
  return out;
}

function clampSeg(s: ContentSegment, lo: Beats, hi: Beats): ContentSegment {
  const t0 = Math.max(s.t0, lo);
  return { t0, t1: Math.min(s.t1, hi), c0: s.c0 + (t0 - s.t0) };
}

/** Content beat played at timeline beat `t` (or `null` outside the clip). */
export function contentBeatAt(clip: Clip, t: Beats): Beats | null {
  const seg = contentSegments(clip, startOf(clip), t, t + BEATS_EPSILON)[0];
  return seg ? seg.c0 + (t - seg.t0) : null;
}

/**
 * Maps a clip's content beats to source seconds (audio clips).
 *
 * - Warp markers (sorted by beat) pin content beats to source time; between two markers the
 *   mapping is linear, outside them it extends with the source tempo.
 * - The source tempo is `warp.source_bpm`, or the song tempo at the clip start when unknown
 *   (what the engine assumes when it creates the clip).
 * - Unwarped clips play in real time; for display they use the same tempo fallback.
 */
export function sourceSecondsMapper(
  sourceBpm: number | null,
  songBpm: number,
  markers: ReadonlyArray<Pick<WarpMarker, "beat" | "source">>,
): (contentBeat: Beats) => number {
  const bpm = sourceBpm && sourceBpm > 0 ? sourceBpm : songBpm > 0 ? songBpm : 120;
  const spb = 60 / bpm;
  const ms = [...markers].sort((a, b) => a.beat - b.beat);
  if (ms.length === 0) return (b) => b * spb;
  const first = ms[0]!;
  const last = ms[ms.length - 1]!;
  return (b) => {
    if (b <= first.beat) return first.source + (b - first.beat) * spb;
    if (b >= last.beat) return last.source + (b - last.beat) * spb;
    let i = 1;
    while (i < ms.length - 1 && ms[i]!.beat < b) i++;
    const a = ms[i - 1]!;
    const z = ms[i]!;
    const span = z.beat - a.beat;
    return span <= BEATS_EPSILON ? a.source : a.source + ((b - a.beat) / span) * (z.source - a.source);
  };
}

/** Length of the media in content beats, given its source seconds mapper (inverse by bisection). */
export function mediaLengthInBeats(media: Pick<MediaRef, "frames" | "sample_rate">, toSeconds: (b: Beats) => number): Beats {
  const seconds = media.frames / Math.max(1, media.sample_rate);
  let lo = 0;
  let hi = 1;
  while (toSeconds(hi) < seconds && hi < 1e7) hi *= 2;
  for (let i = 0; i < 60; i++) {
    const mid = (lo + hi) / 2;
    if (toSeconds(mid) < seconds) lo = mid;
    else hi = mid;
  }
  return hi;
}
