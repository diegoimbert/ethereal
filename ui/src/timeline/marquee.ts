/**
 * Marquee (rubber-band) selection: rectangle math, hit testing and a pointer hook that
 * drives an `ItemSelectionStore`.
 *
 * Coordinates are in the caller's space (usually px relative to the view's content box).
 * Views give the hook a `hitTest(rect)` that returns the ids of items intersecting it:
 * they know their own layout (clip rows, note pitches, lane heights).
 */

import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import {
  combineSelection,
  itemSelection,
  selectModeFromEvent,
  type ItemSelectionStore,
  type SelectableIds,
  type SelectableKind,
  type SelectMode,
} from "./selection";

export interface Point {
  x: number;
  y: number;
}

/** Axis-aligned rectangle, `x0 <= x1`, `y0 <= y1`. */
export interface Rect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/** Normalized rectangle spanned by two corners. */
export function rectFromPoints(a: Point, b: Point): Rect {
  return { x0: Math.min(a.x, b.x), y0: Math.min(a.y, b.y), x1: Math.max(a.x, b.x), y1: Math.max(a.y, b.y) };
}

/** Whether two rectangles overlap (touching edges count). */
export function rectsIntersect(a: Rect, b: Rect): boolean {
  return a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1;
}

/** Ids of the boxes intersecting `rect`. */
export function marqueeHits<Id>(rect: Rect, boxes: Iterable<{ id: Id; rect: Rect }>): Id[] {
  const out: Id[] = [];
  for (const b of boxes) if (rectsIntersect(rect, b.rect)) out.push(b.id);
  return out;
}

export interface MarqueeOptions<K extends SelectableKind> {
  kind: K;
  /** Ids of the items intersecting `rect` (in the element's local px space). */
  hitTest: (rect: Rect) => Iterable<SelectableIds[K]>;
  store?: ItemSelectionStore;
  /** Pixels the pointer must travel before the marquee starts (default 3). */
  threshold?: number;
  /** Called on a click (no drag) on the background, after the default deselect. */
  onClick?: (point: Point, e: PointerEvent) => void;
  /** Called when a drag ends. */
  onEnd?: (rect: Rect) => void;
}

export interface MarqueeHandle {
  /** The live marquee rectangle (element-local px), or `null` when not dragging. */
  rect: Rect | null;
  /** Attach to the background element's `onPointerDown`. */
  onPointerDown: (e: ReactPointerEvent<Element>) => void;
}

/**
 * Pointer-driven marquee selection. Plain drag replaces the selection with the hits,
 * shift-drag adds, cmd/ctrl-drag toggles (relative to the selection at drag start). A
 * plain click on the background clears the kind's selection.
 */
export function useMarquee<K extends SelectableKind>(opts: MarqueeOptions<K>): MarqueeHandle {
  const [rect, setRect] = useState<Rect | null>(null);
  const optsRef = useRef(opts);
  useEffect(() => {
    optsRef.current = opts;
  });
  const cleanup = useRef<(() => void) | null>(null);
  useEffect(() => () => cleanup.current?.(), []);

  const onPointerDown = useCallback((e: ReactPointerEvent<Element>) => {
    if (e.button !== 0) return;
    const el = e.currentTarget;
    const box = el.getBoundingClientRect();
    const local = (ev: { clientX: number; clientY: number }): Point => ({
      x: ev.clientX - box.left,
      y: ev.clientY - box.top,
    });
    const start = local(e);
    const mode: SelectMode = selectModeFromEvent(e);
    const { kind, store = itemSelection } = optsRef.current;
    const base = store.getState().selected[kind] as ReadonlySet<SelectableIds[K]>;
    let active = false;

    const move = (ev: PointerEvent) => {
      const p = local(ev);
      const threshold = optsRef.current.threshold ?? 3;
      if (!active && Math.hypot(p.x - start.x, p.y - start.y) < threshold) return;
      active = true;
      const r = rectFromPoints(start, p);
      setRect(r);
      const hits = optsRef.current.hitTest(r);
      store.getState().select(kind, combineSelection(base, hits, mode), "replace");
    };
    const up = (ev: PointerEvent) => {
      done();
      if (active) {
        optsRef.current.onEnd?.(rectFromPoints(start, local(ev)));
      } else {
        if (mode === "replace") store.getState().clear(kind);
        optsRef.current.onClick?.(start, ev);
      }
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", done);
      cleanup.current = null;
      setRect(null);
    };
    cleanup.current?.();
    cleanup.current = done;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", done);
  }, []);

  return { rect, onPointerDown };
}
