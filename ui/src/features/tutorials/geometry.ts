// Pure layout of the tour overlay: the holes around the highlighted parts and where the
// bubble goes.
import type { TourPlacement } from "./types";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Size {
  w: number;
  h: number;
}

/** Space between a highlighted part and its hole's edge (px). */
export const HOLE_PAD = 6;
/** Space between the hole and the bubble, and between the bubble and the window's edges (px). */
export const BUBBLE_GAP = 12;

export const sameRect = (a: Rect, b: Rect) => a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;

export const sameRects = (a: ReadonlyArray<Rect>, b: ReadonlyArray<Rect>) => a.length === b.length && a.every((r, i) => sameRect(r, b[i]!));

/** The rect of an element, rounded (so a still layout gives equal rects frame after frame). */
export function rectOf(el: Element): Rect | null {
  const r = el.getBoundingClientRect();
  if (r.width <= 0 && r.height <= 0) return null;
  return { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) };
}

/** Each part grown by `pad` and clipped to the viewport. */
export function holes(rects: ReadonlyArray<Rect>, viewport: Size, pad = HOLE_PAD): Rect[] {
  const out: Rect[] = [];
  for (const r of rects) {
    const x = Math.max(0, r.x - pad);
    const y = Math.max(0, r.y - pad);
    const x2 = Math.min(viewport.w, r.x + r.w + pad);
    const y2 = Math.min(viewport.h, r.y + r.h + pad);
    if (x2 > x && y2 > y) out.push({ x, y, w: x2 - x, h: y2 - y });
  }
  return out;
}

export function union(rects: ReadonlyArray<Rect>): Rect | null {
  if (rects.length === 0) return null;
  const x = Math.min(...rects.map((r) => r.x));
  const y = Math.min(...rects.map((r) => r.y));
  const x2 = Math.max(...rects.map((r) => r.x + r.w));
  const y2 = Math.max(...rects.map((r) => r.y + r.h));
  return { x, y, w: x2 - x, h: y2 - y };
}

/** An SVG path covering the viewport except the holes (even-odd fill: clicks there pass through). */
export function dimPath(viewport: Size, hs: ReadonlyArray<Rect>): string {
  const box = (r: Rect) => `M${r.x} ${r.y}h${r.w}v${r.h}h${-r.w}z`;
  return [box({ x: 0, y: 0, w: viewport.w, h: viewport.h }), ...hs.map(box)].join("");
}

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, Math.max(lo, hi)));

export type Side = "top" | "bottom" | "left" | "right" | "center";

/**
 * Where the bubble goes: beside `area` on the requested side (or, for "auto", the first of
 * below, right, left and above that fits it), else centered; always kept inside the viewport.
 */
export function placeBubble(
  area: Rect | null,
  bubble: Size,
  viewport: Size,
  placement: TourPlacement = "auto",
  gap = BUBBLE_GAP,
): { x: number; y: number; side: Side } {
  const center = () => ({ x: Math.round((viewport.w - bubble.w) / 2), y: Math.round((viewport.h - bubble.h) / 2), side: "center" as const });
  if (!area || placement === "center") return center();
  const room: Record<Exclude<Side, "center">, number> = {
    bottom: viewport.h - (area.y + area.h) - gap,
    top: area.y - gap,
    right: viewport.w - (area.x + area.w) - gap,
    left: area.x - gap,
  };
  const fits = (side: Exclude<Side, "center">) => (side === "top" || side === "bottom" ? room[side] >= bubble.h + gap : room[side] >= bubble.w + gap);
  // The first side that fits, in this order (under the top bar, beside the rail and panels).
  const order: Array<Exclude<Side, "center">> = ["bottom", "right", "left", "top"];
  const side = placement !== "auto" && fits(placement) ? placement : (order.find(fits) ?? null);
  if (!side) {
    // Nothing fits beside it (a big area, a dialog): over the area's lower part, or centered.
    return center();
  }
  const maxX = viewport.w - bubble.w - gap;
  const maxY = viewport.h - bubble.h - gap;
  switch (side) {
    case "bottom":
      return { x: clamp(Math.round(area.x + area.w / 2 - bubble.w / 2), gap, maxX), y: area.y + area.h + gap, side };
    case "top":
      return { x: clamp(Math.round(area.x + area.w / 2 - bubble.w / 2), gap, maxX), y: area.y - gap - bubble.h, side };
    case "right":
      return { x: area.x + area.w + gap, y: clamp(Math.round(area.y + area.h / 2 - bubble.h / 2), gap, maxY), side };
    case "left":
      return { x: area.x - gap - bubble.w, y: clamp(Math.round(area.y + area.h / 2 - bubble.h / 2), gap, maxY), side };
  }
}
