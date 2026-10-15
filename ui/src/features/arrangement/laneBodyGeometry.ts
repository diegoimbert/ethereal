/**
 * Geometry of a clip box inside its lane, from the design tokens (the arranger canvas, `gl/`,
 * draws and hit-tests clips with it).
 */

import { radius, size, space } from "@/theme";

const px = (token: string): number => parseFloat(token);

/** Vertical inset of a clip in its lane (`.eth-clip` top/bottom). */
export const CLIP_INSET = px(space.xs);
/** Title band height (`.eth-clip__title`); the body starts below it. */
export const CLIP_TITLE = px(size.clipTitleHeight);
/** Corner radius of a clip (`.eth-clip`). */
export const CLIP_RADIUS = px(radius.md);
/** Narrowest a clip is drawn, so it stays grabbable (title bar) when zoomed far out. */
export const CLIP_MIN_PX = 8;

/** Height of a clip's body (below its title) in a lane `laneHeight` px tall. */
export function bodyHeight(laneHeight: number): number {
  return Math.max(0, laneHeight - 2 * CLIP_INSET - CLIP_TITLE);
}

/** Longest canvas side in device px (browsers cap canvases around 16k–32k). */
export const MAX_CANVAS_PX = 8192;
