import { useCallback, useEffect, useLayoutEffect, useRef, type RefObject } from "react";
import { ScaleFollower } from "@/timeline";
import { MAX_TRACK_HEIGHT, MIN_TRACK_HEIGHT, rowIndexAt, type Row } from "./layout";
import { useArrangementUi } from "./uiStore";

/**
 * Vertical zoom of the arrangement (cmd/ctrl + shift + wheel, via `useTimelineWheel`'s
 * `onVerticalZoom`): scales every lane height by the same factor (clamped per lane),
 * animated, keeping the track under the pointer in place on every frame.
 */
export function useTrackHeightZoom(
  scrollRef: RefObject<HTMLElement | null>,
  rows: ReadonlyArray<Row>,
): (factor: number, pointerY: number) => void {
  const rowsRef = useRef(rows);
  /** Point under the pointer: row index and fraction of its height. */
  const anchor = useRef<{ row: number; frac: number; pointerY: number } | null>(null);
  /** Set when the follower rescaled, so the next layout re-anchors (and no other layout does). */
  const scaled = useRef(false);
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
        scaled.current = true;
        useArrangementUi.getState().scaleHeights(f);
      });
      const ui = useArrangementUi.getState();
      const all = [ui.defaultHeight, ...ui.heights.values()];
      follower.current.push(factor, { min: MIN_TRACK_HEIGHT / Math.max(...all), max: MAX_TRACK_HEIGHT / Math.min(...all) });
    },
    [scrollRef],
  );
}
