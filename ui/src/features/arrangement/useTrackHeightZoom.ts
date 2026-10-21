import { useCallback, useEffect, useLayoutEffect, useRef, type RefObject } from "react";
import { ScaleFollower } from "@/timeline";
import { MAX_TRACK_HEIGHT, MIN_TRACK_HEIGHT, rowIndexAt, type Row } from "./layout";
import { useArrangementUi } from "./uiStore";

/** How far every lane height can still scale (min, max factor), each lane within its limits. */
function heightBounds(): { min: number; max: number } {
  const ui = useArrangementUi.getState();
  const all = [ui.defaultHeight, ...ui.heights.values()];
  return { min: MIN_TRACK_HEIGHT / Math.max(...all), max: MAX_TRACK_HEIGHT / Math.min(...all) };
}

/**
 * Vertical zoom of the arrangement (cmd/ctrl + shift + wheel, via `useTimelineWheel`'s
 * `onVerticalZoom`): scales every lane height by the same factor (clamped per lane), animated
 * like the horizontal zoom (`ScaleFollower`: wheel steps move a goal the heights ease to),
 * keeping the track under the pointer in place on every frame. A step re-renders no track
 * header (they are memoized on what they show: TrackRow), so it stays smooth with many rows.
 */
export function useTrackHeightZoom(scrollRef: RefObject<HTMLElement | null>, rows: ReadonlyArray<Row>): (factor: number, pointerY: number) => void {
  const rowsRef = useRef(rows);
  /** Point under the pointer: row index and fraction of its height. */
  const anchor = useRef<{ row: number; frac: number; pointerY: number } | null>(null);
  /** Set when the heights were rescaled, so the next layout re-anchors (and no other layout does). */
  const scaled = useRef(false);
  /** The heights' animation, made at the first zoom. */
  const follower = useRef<ScaleFollower | null>(null);
  useEffect(() => () => follower.current?.cancel(), []);

  useLayoutEffect(() => {
    rowsRef.current = rows;
    const a = anchor.current;
    const el = scrollRef.current;
    if (!scaled.current || !a || !el) return;
    scaled.current = false;
    const r = rows[a.row];
    if (r) el.scrollTop = r.y + a.frac * r.height - a.pointerY;
  }, [rows, scrollRef]);

  return useCallback(
    (factor: number, pointerY: number) => {
      const el = scrollRef.current;
      const rs = rowsRef.current;
      if (el && rs.length) {
        const y = el.scrollTop + pointerY;
        const i = Math.max(0, Math.min(rs.length - 1, rowIndexAt(rs, y)));
        const r = rs[i]!;
        anchor.current = { row: i, frac: (y - r.y) / r.height, pointerY };
      }
      follower.current ??= new ScaleFollower((f) => {
        const b = heightBounds();
        const g = Math.max(b.min, Math.min(b.max, f));
        if (g === 1) return;
        scaled.current = true;
        useArrangementUi.getState().scaleHeights(g);
      });
      follower.current.push(factor, heightBounds());
    },
    [scrollRef],
  );
}
