import { useEffect, useRef, type RefObject } from "react";
import { ScrollYFollower } from "./motion";
import { wheelZoomFactor } from "./viewport";
import { animatePan, animateZoom } from "./viewMotion";
import type { TimelineViewStore } from "./viewStore";

/**
 * Zoom anchor in timeline px for a pointer at `pointerPx` inside the element. Over the left
 * pane (track headers, piano keys) the pointer sits left of the timeline: anchor on the
 * timeline's left edge instead, so zooming never shifts the visible start.
 */
export function zoomAnchorPx(pointerPx: number, originPx: number): number {
  return Math.max(0, pointerPx - originPx);
}

export interface TimelineWheelOptions {
  /**
   * Where the timeline's x = 0 is inside the element, in px from its left edge (e.g. a
   * track-header or keyboard column before the lanes). Zoom anchors on the pointer from
   * there. A function is read at each event (default 0).
   */
  originPx?: number | (() => number);
  /** Plain vertical wheel scrolls the timeline horizontally (for lanes that don't scroll vertically). */
  verticalScrolls?: boolean;
  /** Plain vertical wheel scrolls the element itself, animated (instead of natively). */
  smoothScrollY?: boolean;
  /**
   * Cmd/ctrl + shift + wheel: vertical zoom (track heights, key height). Receives the
   * zoom factor and the pointer's y relative to the element's top edge.
   */
  onVerticalZoom?: (factor: number, pointerY: number) => void;
}

/**
 * Wheel/trackpad navigation on a timeline element, all animated (see `motion.ts`):
 *
 * - cmd/ctrl + wheel (and trackpad pinch, reported as ctrl + wheel) zooms around the cursor;
 * - cmd/ctrl + shift + wheel calls `onVerticalZoom`, if given;
 * - horizontal deltas (and shift + vertical) scroll the timeline;
 * - plain vertical wheel scrolls the element (`smoothScrollY`), the timeline
 *   (`verticalScrolls`), or is left to the browser.
 *
 * Uses a non-passive native listener so it can `preventDefault()` (React's is passive).
 */
export function useTimelineWheel(
  ref: RefObject<HTMLElement | null>,
  view: TimelineViewStore,
  opts: TimelineWheelOptions = {},
): void {
  const { verticalScrolls = false, smoothScrollY = false } = opts;
  const origin = useRef(opts.originPx);
  const onVerticalZoom = useRef(opts.onVerticalZoom);
  useEffect(() => {
    onVerticalZoom.current = opts.onVerticalZoom;
    origin.current = opts.originPx;
  });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const scrollY = smoothScrollY ? new ScrollYFollower(el) : null;
    const onWheel = (e: WheelEvent) => {
      const unit =
        e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? el.clientWidth : 1;
      const dx = e.deltaX * unit;
      const dy = e.deltaY * unit;
      const mod = e.ctrlKey || e.metaKey;
      const box = el.getBoundingClientRect();
      if (mod && e.shiftKey && onVerticalZoom.current) {
        e.preventDefault();
        // With shift held, some platforms turn a vertical wheel into a horizontal one.
        const d = Math.abs(dy) >= Math.abs(dx) ? dy : dx;
        if (d) onVerticalZoom.current(wheelZoomFactor(d), e.clientY - box.top);
        return;
      }
      if (mod) {
        e.preventDefault();
        const o = origin.current;
        const originPx = typeof o === "function" ? o() : (o ?? 0);
        animateZoom(
          view,
          wheelZoomFactor(dy || dx),
          zoomAnchorPx(e.clientX - box.left, originPx),
        );
        return;
      }
      if (e.shiftKey || Math.abs(dx) > Math.abs(dy)) {
        const horizontal = e.shiftKey ? dx || dy : dx;
        if (horizontal === 0) return;
        e.preventDefault();
        animatePan(view, horizontal);
        return;
      }
      if (dy === 0) return;
      if (verticalScrolls) {
        e.preventDefault();
        animatePan(view, dy);
      } else if (scrollY && el.scrollHeight > el.clientHeight) {
        // Only when the element can scroll; otherwise let the page/parent handle it.
        e.preventDefault();
        scrollY.scrollBy(dy);
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      el.removeEventListener("wheel", onWheel);
      scrollY?.cancel();
    };
  }, [ref, view, verticalScrolls, smoothScrollY]);
}
