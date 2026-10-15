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

const bounds = new WeakMap<Clip, ClipBounds>();

/**
 * `boundsOf(clip)`, the same object for the same clip object: the scene keeps its data for
 * clips that didn't change (clips are immutable snapshots: an edit makes a new object).
 */
export function stableBoundsOf(clip: Clip): ClipBounds {
  let b = bounds.get(clip);
  if (!b) bounds.set(clip, (b = Object.freeze(boundsOf(clip))));
  return b;
}

/**
 * The clips drawn in `track`'s lane, sorted by start: its own clips (unless a move drag
 * takes them to another lane) plus clips dragged in from other lanes and copy ghosts.
 */
export function laneItems(clips: Readonly<Record<ClipId, Clip>>, track: TrackId, preview: DragPreview | null): LaneItem[] {
  return laneItemsOf(
    Object.values(clips).filter((c) => c.track === track),
    track,
    preview,
    (id) => clips[id],
  );
}

/**
 * `laneItems` from the lane's own clips (`own`: the clips of `track`, e.g. `clipsOfTrack`)
 * and the drag preview; `clipOf` looks up clips dragged in from other lanes.
 */
export function laneItemsOf(own: ReadonlyArray<Clip>, track: TrackId, preview: DragPreview | null, clipOf: (id: ClipId) => Clip | undefined): LaneItem[] {
  const out: LaneItem[] = [];
  for (const clip of own) {
    if (!isArrangementClip(clip)) continue;
    const p = preview?.bounds.get(clip.id);
    if (!p) {
      out.push({ clip, bounds: stableBoundsOf(clip), dragging: false, ghost: false });
      continue;
    }
    if (preview!.copy) {
      out.push({ clip, bounds: stableBoundsOf(clip), dragging: false, ghost: false });
      if (p.track === track) out.push({ clip, bounds: p, dragging: true, ghost: true });
    } else if (p.track === track) {
      out.push({ clip, bounds: p, dragging: true, ghost: false });
    }
  }
  // Clips dragged here from other lanes.
  if (preview) {
    for (const [id, p] of preview.bounds) {
      if (p.track !== track) continue;
      const clip = clipOf(id);
      if (!clip || clip.track === track || !isArrangementClip(clip)) continue;
      out.push({ clip, bounds: p, dragging: true, ghost: preview.copy });
    }
  }
  return out.sort((a, b) => a.bounds.start - b.bounds.start || (a.clip.id < b.clip.id ? -1 : 1));
}

/**
 * Whether the drag preview changes what `track`'s lane shows: it moves one of the lane's
 * own clips (`own`), or brings a clip into the lane. Lanes select the preview only then, so
 * a drag re-renders the lanes it touches, not all of them.
 */
export function previewTouches(preview: DragPreview | null, track: TrackId, own: ReadonlySet<ClipId>): boolean {
  if (!preview) return false;
  for (const [id, p] of preview.bounds) if (p.track === track || own.has(id)) return true;
  return false;
}
