/**
 * The geometry of a floating window (kit `FloatingWindow`), as pure functions:
 * where it may sit (inside the viewport) and how an edge or corner drag resizes it.
 */

export interface WindowRect {
  /** Left edge, in CSS px from the viewport's left. */
  x: number;
  /** Top edge, in CSS px from the viewport's top. */
  y: number;
  /** Width; absent: the window's default width. */
  w?: number;
  /** Height; absent: the window's default height (or its content's, up to the viewport). */
  h?: number;
}

export interface Viewport {
  width: number;
  height: number;
}

/**
 * How much of a window must stay on screen when it may be dragged past the viewport's edges:
 * `x` px of its title bar's width, its whole title bar height (`y`) at the top edge's side.
 */
export interface Grip {
  x: number;
  y: number;
}

export interface SizeLimits {
  minWidth: number;
  minHeight: number;
  maxWidth: number;
  maxHeight: number;
}

/** An edge or a corner a window is resized from. */
export type ResizeEdge = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";

export const RESIZE_EDGES: ReadonlyArray<ResizeEdge> = ["n", "s", "e", "w", "ne", "nw", "se", "sw"];

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), Math.max(lo, hi));

/**
 * `rect` kept inside `viewport`: its size within the limits (and the viewport), its position so
 * the whole window shows. `autoHeight` (no explicit height) is the height to keep on screen for a
 * window sized by its content (its CSS caps it at the viewport's bottom edge). With a `grip`,
 * the window may go past the left, right and bottom edges: only part of its title bar must stay
 * on screen (`grip.x` px of its width, and never above the top edge).
 */
export function clampRect(rect: WindowRect, viewport: Viewport, limits: SizeLimits, width: number, autoHeight: number, grip?: Grip): WindowRect {
  const maxW = Math.min(limits.maxWidth, viewport.width);
  const maxH = Math.min(limits.maxHeight, viewport.height);
  const w = rect.w === undefined ? undefined : Math.round(clamp(rect.w, Math.min(limits.minWidth, maxW), maxW));
  const h = rect.h === undefined ? undefined : Math.round(clamp(rect.h, Math.min(limits.minHeight, maxH), maxH));
  const effW = Math.min(w ?? width, viewport.width);
  const effH = Math.min(h ?? autoHeight, viewport.height);
  const out: WindowRect = grip
    ? {
        x: Math.round(clamp(rect.x, Math.min(0, grip.x - effW), viewport.width - Math.min(grip.x, effW))),
        y: Math.round(clamp(rect.y, 0, viewport.height - grip.y)),
      }
    : {
        x: Math.round(clamp(rect.x, 0, viewport.width - effW)),
        y: Math.round(clamp(rect.y, 0, viewport.height - effH)),
      };
  if (w !== undefined) out.w = w;
  if (h !== undefined) out.h = h;
  return out;
}

/**
 * The rect after dragging `edge` by (`dx`, `dy`) from `start` (a fully sized rect): the opposite
 * edge stays put, the size stays within the limits and the window inside the viewport (with
 * `offscreen`: its left, right and bottom edges may go past the viewport's; the top never).
 */
export function resizeRect(
  start: Required<WindowRect>,
  edge: ResizeEdge,
  dx: number,
  dy: number,
  viewport: Viewport,
  limits: SizeLimits,
  offscreen = false,
): Required<WindowRect> {
  let { x, y, w, h } = start;
  if (edge.includes("e")) {
    w = clamp(start.w + dx, limits.minWidth, offscreen ? limits.maxWidth : Math.min(limits.maxWidth, viewport.width - start.x));
  } else if (edge.includes("w")) {
    const right = start.x + start.w;
    w = clamp(start.w - dx, limits.minWidth, offscreen ? limits.maxWidth : Math.min(limits.maxWidth, right));
    x = right - w;
  }
  if (edge.includes("s")) {
    h = clamp(start.h + dy, limits.minHeight, offscreen ? limits.maxHeight : Math.min(limits.maxHeight, viewport.height - start.y));
  } else if (edge.startsWith("n")) {
    const bottom = start.y + start.h;
    h = clamp(start.h - dy, limits.minHeight, Math.min(limits.maxHeight, bottom));
    y = bottom - h;
  }
  return { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h) };
}
