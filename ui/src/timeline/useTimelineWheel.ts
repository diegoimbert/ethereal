import { useEffect, type RefObject } from "react";
import { wheelZoomFactor } from "./viewport";
import type { TimelineViewStore } from "./viewStore";

/**
 * Wheel/trackpad navigation on a timeline element: cmd/ctrl + wheel (and trackpad pinch,
 * which browsers report as ctrl + wheel) zooms around the cursor; otherwise horizontal
 * deltas (and shift + vertical) scroll the timeline. Plain vertical wheel is left to the
 * page (track list scrolling) unless `verticalScrolls` is set.
 *
 * Uses a non-passive native listener so it can `preventDefault()` (React's is passive).
 */
export function useTimelineWheel(
  ref: RefObject<HTMLElement | null>,
  view: TimelineViewStore,
  opts: { verticalScrolls?: boolean } = {},
): void {
  const verticalScrolls = opts.verticalScrolls ?? false;
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? el.clientWidth : 1;
      const dx = e.deltaX * unit;
      const dy = e.deltaY * unit;
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        const x = e.clientX - el.getBoundingClientRect().left;
        view.getState().zoomBy(wheelZoomFactor(dy), x);
        return;
      }
      const horizontal = e.shiftKey ? dx || dy : Math.abs(dx) > Math.abs(dy) || verticalScrolls ? dx || dy : 0;
      if (horizontal === 0) return;
      e.preventDefault();
      view.getState().scrollByPx(horizontal);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [ref, view, verticalScrolls]);
}
