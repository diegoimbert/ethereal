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
import type { TimelineViewStore } from "./viewStore";

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
  /**
   * The element shows a horizontal timeline (`view`), its lanes from `originPx()` px to the
   * element's right edge: the drag's start stays on its beat while the view scrolls, and the
   * view scrolls while the pointer is past the lanes' left or right edge (the element itself
   * never scrolls sideways).
   */
  timeline?: { view: TimelineViewStore; originPx?: () => number };
  /**
   * Section mode: while Shift is held (pressed or released mid-drag switches modes), the
   * drag selects no items (the selection stays as it was at the press) and previews a time
   * section instead: `onPreview` follows the drag (`null` when back in item mode), `onEnd`
   * gets the final rectangle. With it, Shift no longer means "add" (Cmd/Ctrl toggles).
   */
  section?: {
    onPreview(rect: Rect | null): void;
    onEnd(rect: Rect): void;
    /**
     * A Shift-click (no drag): extend or retract the section to the click (`extendOrRetract`).
     * Returns `false` when there is nothing to extend: the click then goes on as a plain one
     * (the default deselect, `onClick`).
     */
    onShiftClick?(point: Point, e: PointerEvent): boolean;
  };
}

export interface MarqueeHandle {
  /**
   * The live marquee rectangle (element-local px) as drawn, within the element's box (the hit
   * test gets the whole drag: past the edges while the view scrolls), or `null` when not
   * dragging (or in section mode).
   */
  rect: Rect | null;
  /**
   * The sides of `rect` the element's box cut (the drag goes on past them), space-separated
   * ("left top"...): drawn without their border, so the box reads as going on past the edge.
   */
  cut: string;
  /** Attach to the background element's `onPointerDown`. */
  onPointerDown: (e: ReactPointerEvent<Element>) => void;
}

/**
 * Pointer-driven marquee selection. Plain drag replaces the selection with the hits,
 * shift-drag adds (or, with `section`, makes a time section instead), cmd/ctrl-drag toggles
 * (relative to the selection at drag start). A plain click on the background clears the
 * kind's selection.
 */
export function useMarquee<K extends SelectableKind>(opts: MarqueeOptions<K>): MarqueeHandle {
  const [shown, setShown] = useState<{ rect: Rect; cut: string } | null>(null);
  const optsRef = useRef(opts);
  useEffect(() => {
    optsRef.current = opts;
  });
  const cleanup = useRef<(() => void) | null>(null);
  useEffect(() => () => cleanup.current?.(), []);

  const onPointerDown = useCallback((e: ReactPointerEvent<Element>) => {
    if (e.button !== 0) return;
    const el = e.currentTarget;
    // Local to the element as it is now: it may scroll during the drag (the view auto-scrolls
    // when the pointer leaves it), and the press stays where it was in its content.
    const local = (ev: { clientX: number; clientY: number }): Point => {
      const box = el.getBoundingClientRect();
      return { x: ev.clientX - box.left, y: ev.clientY - box.top };
    };
    const start = local(e);
    let pointer = { clientX: e.clientX, clientY: e.clientY };
    // On a timeline, the press is kept as a beat: its x follows the view's scroll and zoom.
    const tl = optsRef.current.timeline;
    const origin = () => tl?.originPx?.() ?? 0;
    const startBeat = tl ? beatAtPx(tl.view, start.x - origin()) : 0;
    const anchor = (): Point => (tl ? { x: origin() + pxAtBeat(tl.view, startBeat), y: start.y } : start);
    const sectionable = !!optsRef.current.section;
    const mode: SelectMode = sectionable ? selectModeFromEvent({ ...e, shiftKey: false }) : selectModeFromEvent(e);
    const { kind, store = itemSelection } = optsRef.current;
    const base = store.getState().selected[kind] as ReadonlySet<SelectableIds[K]>;
    let active = false;
    let last = start;
    let shift = e.shiftKey;
    let itemsShown = false;
    const inSection = () => sectionable && shift;

    /** Show the drag at `p` in the current mode (items or section). */
    const update = (p: Point) => {
      last = p;
      const r = rectFromPoints(anchor(), p);
      if (inSection()) {
        // Leaving item mode: the items the drag had selected go back to the press's selection.
        if (itemsShown) store.getState().select(kind, base, "replace");
        itemsShown = false;
        setShown(null);
        optsRef.current.section!.onPreview(r);
        return;
      }
      optsRef.current.section?.onPreview(null);
      itemsShown = true;
      setShown(clipTo(r, el));
      const hits = optsRef.current.hitTest(r);
      store.getState().select(kind, combineSelection(base, hits, mode), "replace");
    };
    const move = (ev: PointerEvent) => {
      pointer = { clientX: ev.clientX, clientY: ev.clientY };
      const p = local(ev);
      shift = ev.shiftKey;
      const threshold = optsRef.current.threshold ?? 3;
      if (!active && Math.hypot(p.x - start.x, p.y - start.y) < threshold) return;
      active = true;
      update(p);
      edge.check();
    };
    // Past the lanes' left / right edge: the view scrolls, faster the farther out.
    const edge = edgeScroller(
      () => {
        if (!tl || !active) return 0;
        const x = local(pointer).x;
        const right = el.clientWidth;
        if (x > right) return x - right;
        if (x < origin()) return x - origin();
        return 0;
      },
      (dx) => tl?.view.getState().scrollByPx(dx),
    );
    // The view scrolled or zoomed (edge scrolling, the wheel): the start follows its beat.
    const unsubView = tl?.view.subscribe(() => {
      if (active) update(local(pointer));
    });
    // The view scrolled under a still pointer (auto-scroll at its edge, the wheel): the
    // marquee's far corner follows the content.
    const onScroll = () => {
      if (active) update(local(pointer));
    };
    // Shift pressed or released without moving: switch modes in place.
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Shift" || !sectionable || ev.shiftKey === shift) return;
      shift = ev.shiftKey;
      if (active) update(last);
    };
    const up = (ev: PointerEvent) => {
      shift = ev.shiftKey;
      done();
      if (active) {
        const r = rectFromPoints(anchor(), local(ev));
        if (inSection()) {
          optsRef.current.section!.onPreview(null);
          optsRef.current.section!.onEnd(r);
        } else optsRef.current.onEnd?.(r);
      } else {
        // A Shift-click: Shift held at the press or the release.
        if (sectionable && (e.shiftKey || shift) && optsRef.current.section!.onShiftClick?.(start, ev)) return;
        if (mode === "replace") store.getState().clear(kind);
        optsRef.current.onClick?.(start, ev);
      }
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", done);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
      window.removeEventListener("scroll", onScroll, true);
      edge.stop();
      unsubView?.();
      cleanup.current = null;
      setShown(null);
    };
    cleanup.current?.();
    cleanup.current = done;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", done);
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    window.addEventListener("scroll", onScroll, true);
  }, []);

  return { rect: shown?.rect ?? null, cut: shown?.cut ?? "", onPointerDown };
}

const beatAtPx = (view: TimelineViewStore, px: number): number => {
  const s = view.getState();
  return s.scrollBeats + px / s.pxPerBeat;
};

const pxAtBeat = (view: TimelineViewStore, beat: number): number => {
  const s = view.getState();
  return (beat - s.scrollBeats) * s.pxPerBeat;
};

/** Edge scrolling speed: px per second per px the pointer is past the edge, and its cap. */
const EDGE_SPEED = 12;
const EDGE_MAX_SPEED = 2400;
/** Longest frame step counted (s): after a stall, the view doesn't jump the whole way at once. */
const EDGE_MAX_FRAME_S = 0.25;

/**
 * While `overshoot()` (px past an edge, signed) isn't 0, `scroll(dx)` each frame at a speed
 * growing with it. `check()` starts it after a pointer move; it stops by itself back inside.
 */
function edgeScroller(overshoot: () => number, scroll: (dx: number) => void): { check(): void; stop(): void } {
  let raf = 0;
  let last = 0;
  const frame = (t: number) => {
    const o = overshoot();
    if (o === 0) {
      raf = 0;
      return;
    }
    // Real elapsed time (slow frames, e.g. a heavy section preview, scroll as far per second),
    // capped only after a stall.
    const dt = last ? Math.min(EDGE_MAX_FRAME_S, (t - last) / 1000) : 1 / 60;
    last = t;
    scroll(Math.sign(o) * Math.min(EDGE_MAX_SPEED, Math.abs(o) * EDGE_SPEED) * dt);
    raf = requestAnimationFrame(frame);
  };
  return {
    check() {
      if (raf || overshoot() === 0) return;
      last = 0;
      raf = requestAnimationFrame(frame);
    },
    stop() {
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    },
  };
}

/**
 * `r` within `el`'s box, and the sides it cut: drawn past it, the marquee would make the element (or its scroll
 * container) overflow, and the browser would scroll that while dragging, then back on
 * release. An axis the element has no size on (not laid out) is left as is.
 */
function clipTo(r: Rect, el: Element): { rect: Rect; cut: string } {
  const w = el.clientWidth;
  const h = el.clientHeight;
  const cx = (v: number) => (w > 0 ? Math.min(w, Math.max(0, v)) : v);
  const cy = (v: number) => (h > 0 ? Math.min(h, Math.max(0, v)) : v);
  const rect = { x0: cx(r.x0), y0: cy(r.y0), x1: cx(r.x1), y1: cy(r.y1) };
  const cut = [rect.x0 > r.x0 && "left", rect.x1 < r.x1 && "right", rect.y0 > r.y0 && "top", rect.y1 < r.y1 && "bottom"].filter(Boolean).join(" ");
  return { rect, cut };
}
