/**
 * Clip-relative time for the piano roll. The editor's time axis is the clip's *content*
 * timeline (notes are content-relative, starting at 0); these helpers map song time onto
 * it and build the tempo map the ruler and grid use.
 */

import type { Beats, Clip } from "@/generated";
import { TempoMap } from "@/timeline";

/** Song position (beats) where the clip starts. The only place the piano roll reads a clip's position. */
export function clipSongStart(clip: Clip): Beats {
  return clip.start;
}

/**
 * Content position playing at song position `song`, or `null` when the clip isn't playing
 * there. Without looping the clip plays `[offset, offset + length)`; with looping it plays
 * from `offset` and then repeats `[loop.start, loop.end)`.
 */
export function songToContent(clip: Clip, song: Beats): Beats | null {
  const rel = song - clipSongStart(clip);
  if (rel < 0 || rel >= clip.length) return null;
  const pos = clip.offset + rel;
  const { enabled, start, end } = clip.looping;
  const len = end - start;
  if (!enabled || len <= 0 || pos < end) return pos;
  return start + ((pos - start) % len);
}

/**
 * First song position at which content position `content` plays (for locating from the
 * clip ruler). Positions the clip never plays map to the nearest clip boundary.
 */
export function contentToSong(clip: Clip, content: Beats): Beats {
  const start = clipSongStart(clip);
  const { enabled, start: ls, end: le } = clip.looping;
  let rel = content - clip.offset;
  if (enabled && le > ls && rel < 0 && content >= ls) rel = le - clip.offset + (content - ls);
  return start + Math.min(Math.max(0, rel), clip.length);
}

/**
 * Tempo map on the clip's content axis: the tempo and signature in effect at the clip's
 * start, with bar 1 at content beat 0.
 */
export function clipTempoMap(song: TempoMap, clip: Clip): TempoMap {
  const at = clipSongStart(clip);
  return TempoMap.constant(song.bpmAt(at), song.signatureAt(at));
}

/** End of the content the clip can play (the last loop end or `offset + length`). */
export function contentEnd(clip: Clip): Beats {
  return clip.looping.enabled ? Math.max(clip.looping.end, clip.offset) : clip.offset + clip.length;
}
