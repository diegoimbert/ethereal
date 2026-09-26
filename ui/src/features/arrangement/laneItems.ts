import type { Clip, ClipId, TrackId } from "@/generated";
import { isArrangementClip } from "./clipTime";
import { boundsOf, type ClipBounds } from "./editMath";
import type { DragPreview } from "./uiStore";

/** A clip as drawn in one lane. */
export interface LaneItem {
  clip: Clip;
  bounds: ClipBounds;
  /** Being dragged (drawn at its preview position). */
  dragging: boolean;
  /** Copy-drag ghost (not a real clip yet; not interactive). */
  ghost: boolean;
}

/**
 * The clips drawn in `track`'s lane, sorted by start: its own clips (unless a move drag
 * takes them to another lane) plus clips dragged in from other lanes and copy ghosts.
 */
export function laneItems(
  clips: Readonly<Record<ClipId, Clip>>,
  track: TrackId,
  preview: DragPreview | null,
): LaneItem[] {
  const out: LaneItem[] = [];
  for (const clip of Object.values(clips)) {
    if (!isArrangementClip(clip)) continue;
    const p = preview?.bounds.get(clip.id);
    if (!p) {
      if (clip.track === track) out.push({ clip, bounds: boundsOf(clip), dragging: false, ghost: false });
      continue;
    }
    if (preview!.copy) {
      if (clip.track === track) out.push({ clip, bounds: boundsOf(clip), dragging: false, ghost: false });
      if (p.track === track) out.push({ clip, bounds: p, dragging: true, ghost: true });
    } else if (p.track === track) {
      out.push({ clip, bounds: p, dragging: true, ghost: false });
    }
  }
  return out.sort((a, b) => a.bounds.start - b.bounds.start || (a.clip.id < b.clip.id ? -1 : 1));
}
