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

/**
 * Below this lane height (px) a lane is compact: its track header on one line (TrackRow), its
 * clips a title band alone, the name centered, no body (no notes, waveform or fades). Vertical
 * zoom steps by 1.12 from 64 (57, 51, 46, 41, ...): compact from 41.
 */
export const COMPACT_LANE_HEIGHT = 43;

export const isCompactLane = (laneHeight: number): boolean => laneHeight < COMPACT_LANE_HEIGHT;

/** Height of a clip's title band in a lane `laneHeight` px tall: the whole clip when compact. */
export function titleHeight(laneHeight: number): number {
  return isCompactLane(laneHeight) ? Math.max(0, laneHeight - 2 * CLIP_INSET) : CLIP_TITLE;
}

/**
 * Where a clip's content area starts below its inset, in a lane `laneHeight` px tall: under
 * the title band; at the clip's top in a compact lane (its fades span the whole clip).
 */
export function contentTop(laneHeight: number): number {
  return isCompactLane(laneHeight) ? 0 : CLIP_TITLE;
}

/** Height of a clip's body (below its title) in a lane `laneHeight` px tall (none when compact). */
export function bodyHeight(laneHeight: number): number {
  return Math.max(0, laneHeight - 2 * CLIP_INSET - titleHeight(laneHeight));
}

/** Longest canvas side in device px (browsers cap canvases around 16k–32k). */
export const MAX_CANVAS_PX = 8192;
