/** Where an anchored floating panel opens. */
export type Placement = "bottom-start" | "bottom-end" | "top-start" | "top-end";

/** Room kept between a floating panel (popover, menu, select list) and the window edges. */
export const VIEWPORT_MARGIN = 4;

/** `start` moved so `[start, start + size]` stays inside `[margin, viewport - margin]` (start wins when it can't). */
export function clampToViewport(start: number, size: number, viewport: number): number {
  return Math.max(VIEWPORT_MARGIN, Math.min(start, viewport - size - VIEWPORT_MARGIN));
}

/** Where a panel actually goes once measured: its placement and a horizontal nudge. */
export interface Fit {
  placement: Placement;
  dx: number;
  /** Set when the panel is taller than the room on either side (it scrolls). */
  maxHeight?: number;
}

/**
 * Keeps a `w` x `h` panel anchored at `r` inside the window: flips to the other side (above /
 * below) or alignment (start / end) when the preferred one overflows and the other has more
 * room, then nudges it horizontally inside the margins; a panel taller than the room on
 * both sides gets a max height (it scrolls).
 */
export function fitPlacement(preferred: Placement, r: DOMRect, w: number, h: number, vw: number, vh: number): Fit {
  const m = VIEWPORT_MARGIN;
  let [side, align] = preferred.split("-") as ["top" | "bottom", "start" | "end"];
  const below = vh - r.bottom - m;
  const above = r.top - m;
  if (side === "bottom" && h > below && above > below) side = "top";
  else if (side === "top" && h > above && below > above) side = "bottom";
  const room = side === "bottom" ? below : above;
  if (align === "start" && r.left + w > vw - m && r.right - w >= m) align = "end";
  else if (align === "end" && r.right - w < m && r.left + w <= vw - m) align = "start";
  const left = align === "start" ? r.left : r.right - w;
  const clamped = clampToViewport(left, w, vw);
  return {
    placement: `${side}-${align}` as Placement,
    dx: clamped - left,
    maxHeight: h > room ? Math.max(0, room) : undefined,
  };
}
