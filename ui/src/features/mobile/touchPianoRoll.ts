/**
 * Touch in the piano roll (the design's §4 "Piano roll"), mobile UI only. Two modes, the
 * toolbar's Draw toggle (a finger can't mean both drawing and selecting without a modifier key):
 *
 *   - select (default): a tap selects a note, or on empty space deselects and places the
 *     marker; a double-tap on empty space adds a note, on a note deletes it (the desktop's
 *     double-click); a drag on a selected note moves it (on its edge: resizes); a drag on empty
 *     space pans; long-press then drag on empty space draws a selection box; long-press on a
 *     note opens its menu;
 *   - draw: a tap adds a note, a drag on empty space draws one with its length, a drag on a
 *     note moves it.
 * Pinch: side by side zooms time, one above the other the key height. The keyboard plays the
 * key pressed (its own handling). Everything else is the desktop's own pointer handling,
 * replayed (`touchSurface.ts`).
 */
import { useEffect, type RefObject } from "react";
import type { TimelineViewStore } from "@/timeline";
import { startFling } from "./gestures/momentum";
import { bindTouchSurface, type TouchPolicy } from "./gestures/touchSurface";
import { useMobileUi } from "./mode";

/** The keys and other controls that follow a pressed pointer themselves. */
const DIRECT = '.eth-pr-keys, .eth-pr__zoom-grip, [role="slider"], .eth-knob, [data-touch="direct"]';
const NATIVE = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])';

type NoteProbe = HTMLElement & { ethNoteAt?: (x: number, y: number) => { selected: boolean } | null };

export function pianoRollPolicy(target: Element): TouchPolicy {
  if (target.closest(NATIVE)) return "native";
  if (target.closest(DIRECT)) return "direct";
  return "gesture";
}

export function useTouchPianoRoll(
  bodyRef: RefObject<HTMLElement | null>,
  view: TimelineViewStore,
  originPx: number,
  onVerticalZoom: (factor: number, pointerY: number) => void,
): void {
  const mobile = useMobileUi((s) => s.enabled);
  useEffect(() => {
    const body = bodyRef.current;
    if (!mobile || !body) return;
    let stopFling = () => undefined as void;
    const pan = (dx: number, dy: number) => {
      if (dx !== 0) view.getState().scrollByPx(-dx);
      if (dy !== 0) body.scrollTop -= dy;
    };
    const unbind = bindTouchSurface(body, {
      policy: pianoRollPolicy,
      onTouchStart: () => stopFling(),
      onPanStart: (g, target) => {
        const grid = target.closest<NoteProbe>(".eth-pr-grid");
        if (!grid) return "view";
        const draw = grid.classList.contains("eth-pr-grid--draw");
        const note = grid.ethNoteAt?.(g.startX, g.startY) ?? null;
        if (note && (note.selected || draw || g.after === "longpress")) return "replay";
        if (!note && (draw || g.after === "longpress")) return "replay";
        return "view";
      },
      onPan: pan,
      onPanEnd: (vx, vy) => {
        stopFling = startFling(vx, vy, pan);
      },
      onPinch: (g) => {
        const box = body.getBoundingClientRect();
        if (g.axis === "x") view.getState().zoomBy(g.scale, g.x - box.left - originPx);
        else onVerticalZoom(g.scale, g.y - box.top);
        pan(g.dx, g.dy);
      },
      autoScroll: (p, dt) => {
        const box = body.getBoundingClientRect();
        const zone = 32;
        const speed = 600;
        const vx = p.x > box.right - zone ? speed : p.x < box.left + originPx + zone ? -speed : 0;
        const vy = p.y > box.bottom - zone ? speed : p.y < box.top + zone ? -speed : 0;
        if (vx === 0 && vy === 0) return { dx: 0, moved: false };
        const before = view.getState().scrollBeats;
        if (vx !== 0) view.getState().scrollByPx((vx * dt) / 1000);
        const dx = (view.getState().scrollBeats - before) * view.getState().pxPerBeat;
        const top = body.scrollTop;
        if (vy !== 0) body.scrollTop += (vy * dt) / 1000;
        return { dx, moved: dx !== 0 || body.scrollTop !== top };
      },
    });
    return () => {
      stopFling();
      unbind();
    };
  }, [mobile, bodyRef, view, originPx, onVerticalZoom]);
}
