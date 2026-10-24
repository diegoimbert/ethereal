/**
 * The selection box of a drag that starts on a track lane's empty space. Like the arranger's
 * (`useMarquee`), but it isn't held inside the lane: it selects the points of every lane it
 * crosses (other parameters, other tracks) and the clips of the clip lanes it crosses
 * (`laneRegistry.ts`). Plain drag: replaces both selections; ⌘/Ctrl-drag: toggles. Dragging
 * past the lanes' left / right edge scrolls the timeline, and the press stays on its beat.
 *
 * Client px throughout; the box is a fixed overlay over the page (`.eth-auto-marquee`), cut
 * at the lanes' left and right edges.
 */

import type { PointerEvent as ReactPointerEvent } from "react";
import { combineSelection, edgeScroller, rectFromPoints, selectModeFromEvent, type ItemSelectionStore, type Point, type TimelineViewStore } from "@/timeline";
import { clipsInRect, pointsInRect } from "./laneRegistry";

/** Pixels before a press turns into a box. */
const THRESHOLD = 3;

export interface LaneMarqueeOptions {
  view: TimelineViewStore;
  /** The lane the drag started in (its box gives the lanes' left and right edges). */
  lane: HTMLElement;
  store: ItemSelectionStore;
  /** A click (no drag), after the default deselect (lane-local px). */
  onClick(p: Point, e: PointerEvent): void;
}

/** Start the box on a lane's `pointerdown`. */
export function startLaneMarquee(e: ReactPointerEvent, opts: LaneMarqueeOptions): void {
  const { view, lane, store } = opts;
  const mode = selectModeFromEvent({ ...e, shiftKey: false });
  const box0 = lane.getBoundingClientRect();
  const start = { x: e.clientX, y: e.clientY };
  // The press, kept as a beat and as a y in the lane: it follows scrolling and zoom.
  const s0 = view.getState();
  const startBeat = s0.scrollBeats + (e.clientX - box0.left) / s0.pxPerBeat;
  const startY = e.clientY - box0.top;
  const anchor = (): Point => {
    const b = lane.getBoundingClientRect();
    const s = view.getState();
    return { x: b.left + (startBeat - s.scrollBeats) * s.pxPerBeat, y: b.top + startY };
  };
  const basePoints = store.getState().selected.automationPoint;
  const baseClips = store.getState().selected.clip;
  let pointer = start;
  let active = false;
  let overlay: HTMLDivElement | null = null;

  const update = () => {
    const r = rectFromPoints(anchor(), pointer);
    const b = lane.getBoundingClientRect();
    if (!overlay) {
      overlay = document.createElement("div");
      overlay.className = "eth-auto-marquee";
      overlay.dataset.testid = "automation-marquee";
      document.body.appendChild(overlay);
    }
    const x0 = Math.max(r.x0, b.left);
    const x1 = Math.min(r.x1, b.right);
    Object.assign(overlay.style, { left: `${x0}px`, top: `${r.y0}px`, width: `${Math.max(0, x1 - x0)}px`, height: `${r.y1 - r.y0}px` });
    // Both in one change: the box selected them together (the arrangement keeps one kind
    // selected otherwise, `bindSingleSelection`).
    store.getState().selectKinds({
      clip: combineSelection(baseClips, clipsInRect(view, r), mode),
      automationPoint: combineSelection(basePoints, pointsInRect(view, r), mode),
    });
  };

  const edge = edgeScroller(
    () => {
      if (!active) return 0;
      const b = lane.getBoundingClientRect();
      if (b.width === 0) return 0;
      if (pointer.x > b.right) return pointer.x - b.right;
      if (pointer.x < b.left) return pointer.x - b.left;
      return 0;
    },
    (dx) => view.getState().scrollByPx(dx),
  );
  const move = (ev: PointerEvent) => {
    pointer = { x: ev.clientX, y: ev.clientY };
    if (!active && Math.hypot(pointer.x - start.x, pointer.y - start.y) < THRESHOLD) return;
    active = true;
    update();
    edge.check();
  };
  const onChange = () => {
    if (active) update();
  };
  const unsubView = view.subscribe(onChange);
  const done = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", done);
    window.removeEventListener("scroll", onChange, true);
    unsubView();
    edge.stop();
    overlay?.remove();
    overlay = null;
  };
  const up = (ev: PointerEvent) => {
    done();
    if (active) return;
    if (mode === "replace") store.getState().clear("automationPoint");
    const b = lane.getBoundingClientRect();
    opts.onClick({ x: start.x - b.left, y: start.y - b.top }, ev);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", done);
  window.addEventListener("scroll", onChange, true);
}
