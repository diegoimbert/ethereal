/**
 * Rotation keeps the timeline's centre (#522, the design's §3): when the window's width changes
 * (a phone or tablet turning), the beat at the centre of the lanes stays at the centre, where
 * the view would otherwise keep its left edge (and the playhead or the clip you were looking at
 * could end up off screen). Other width changes (the header column resized) keep the left edge.
 */
import type { TimelineViewStore } from "@/timeline";

export function keepCenterOnRotation(view: TimelineViewStore, windowWidth: () => number = () => window.innerWidth): () => void {
  let width = view.getState().widthPx;
  let windowW = windowWidth();
  return view.subscribe((s) => {
    if (s.widthPx === width) return;
    const before = width;
    width = s.widthPx;
    const w = windowWidth();
    const rotated = w !== windowW;
    windowW = w;
    if (!rotated || before <= 0 || width <= 0) return;
    // Same zoom: shifting by half the width change keeps the centre beat.
    view.getState().scrollByPx((before - width) / 2);
  });
}
