import { useEffect, useRef, type RefObject } from "react";
import { ScrollYFollower } from "./motion";
import { inputSettings, isPinch } from "./inputSettings";
import { animatePan, animateZoom } from "./viewMotion";
import type { TimelineViewStore } from "./viewStore";
import { normalizeWheel, wheelIntent, wheelScrollPx, wheelZoomFactorFor, type NormalizedWheel } from "./wheelInput";

/** Normalized deltas of a native wheel event on `el` (pages = the element's size). */
export function readWheel(e: WheelEvent, el: HTMLElement): NormalizedWheel {
  const legacy = e as WheelEvent & { wheelDeltaX?: number; wheelDeltaY?: number };
  return normalizeWheel(
    { deltaX: e.deltaX, deltaY: e.deltaY, deltaMode: e.deltaMode, wheelDeltaX: legacy.wheelDeltaX, wheelDeltaY: legacy.wheelDeltaY },
    { width: el.clientWidth, height: el.clientHeight },
  );
}

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
 * Wheel/trackpad navigation on a timeline element, all animated (see `motion.ts`), following
 * the user's input settings (`inputSettings.ts`, Settings > Input):
 *
 * - the zoom modifier (cmd/ctrl by default) + wheel, and trackpad pinch, zoom around the cursor;
 * - the zoom modifier + shift + wheel calls `onVerticalZoom`, if given;
 * - horizontal deltas (and shift + vertical) scroll the timeline;
 * - plain vertical wheel scrolls the element (`smoothScrollY`), the timeline
 *   (`verticalScrolls`), or is left to the browser. With "wheel zooms" set, plain wheel
 *   zooms and the modifier scrolls.
 *
 * Mouse-wheel notches are normalized (`wheelInput.ts`): a fixed zoom per notch whatever the
 * platform's step; smooth trackpad input is unchanged.
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
      const settings = inputSettings();
      const n = readWheel(e, el);
      const { dx, dy } = n;
      const pinch = isPinch(e);
      const mods = { ctrlKey: e.ctrlKey, metaKey: e.metaKey, altKey: e.altKey, shiftKey: e.shiftKey, pinch };
      const intent = wheelIntent(mods, n, settings, !!onVerticalZoom.current);
      // Ctrl + wheel zooms the page in a browser: never let it through.
      if (e.ctrlKey) e.preventDefault();
      const box = el.getBoundingClientRect();
      if (intent === "verticalZoom") {
        e.preventDefault();
        // With shift held, some platforms turn a vertical wheel into a horizontal one.
        const d = Math.abs(dy) >= Math.abs(dx) ? dy : dx;
        if (d) onVerticalZoom.current?.(wheelZoomFactorFor(d, n.discrete, settings, pinch), e.clientY - box.top);
        return;
      }
      if (intent === "zoom") {
        e.preventDefault();
        const d = dy || dx;
        if (!d) return;
        const o = origin.current;
        const originPx = typeof o === "function" ? o() : (o ?? 0);
        animateZoom(view, wheelZoomFactorFor(d, n.discrete, settings, pinch), zoomAnchorPx(e.clientX - box.left, originPx));
        return;
      }
      if (intent === "scrollX") {
        const horizontal = e.shiftKey && settings.shiftScrollsHorizontally ? dx || dy : dx;
        if (horizontal === 0) return;
        e.preventDefault();
        animatePan(view, wheelScrollPx(horizontal, "x", settings));
        return;
      }
      if (dy === 0) return;
      if (verticalScrolls) {
        e.preventDefault();
        animatePan(view, wheelScrollPx(dy, "x", settings));
      } else if (scrollY && el.scrollHeight > el.clientHeight) {
        // Only when the element can scroll; otherwise let the page/parent handle it.
        e.preventDefault();
        scrollY.scrollBy(wheelScrollPx(dy, "y", settings));
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      el.removeEventListener("wheel", onWheel);
      scrollY?.cancel();
    };
  }, [ref, view, verticalScrolls, smoothScrollY]);
}
