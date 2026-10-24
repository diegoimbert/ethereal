/**
 * A sample dropped on an audio clip whose sample is missing relinks that sample
 * (`@/features/media-refs` `relinkDrop`) instead of adding a clip.
 */
import type { Beats, Clip, ClipId, MediaId, TrackId } from "@/generated";
import { isArrangementClip } from "./clipTime";
import { stableBoundsOf } from "./laneItems";

/** The clip a dropped sample would relink (its box is highlighted). */
export interface RelinkTarget {
  track: TrackId;
  clip: ClipId;
  media: MediaId;
  start: Beats;
  length: Beats;
}

/**
 * The audio clip of `track` at `beat` whose sample is missing (of overlapping ones, the
 * latest start: it's drawn on top), or `null`.
 */
export function missingClipAt(clips: Readonly<Record<ClipId, Clip>>, track: TrackId, beat: Beats, missing: ReadonlySet<MediaId>): RelinkTarget | null {
  let best: RelinkTarget | null = null;
  if (missing.size === 0) return null;
  for (const clip of Object.values(clips)) {
    if (clip.track !== track || clip.content.type !== "Audio" || !isArrangementClip(clip) || !missing.has(clip.content.media)) continue;
    const b = stableBoundsOf(clip);
    if (beat < b.start || beat >= b.start + b.length) continue;
    if (best && best.start > b.start) continue;
    best = { track, clip: clip.id, media: clip.content.media, start: b.start, length: b.length };
  }
  return best;
}
