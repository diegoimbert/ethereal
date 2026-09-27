/**
 * Lane geometry (pure): normalized value ↔ y, lane time ↔ x, the SVG path of a lane and
 * point hit boxes.
 *
 * Lane time is the lane's own time base: arrangement beats for track lanes, content beats
 * for clip envelopes. `offset` is the view position (beats on the view's axis) of lane
 * time 0, so a clip envelope drawn in the arrangement passes the clip's start.
 */

import { beatsToPx, pxToBeats, type Rect, type TimelineViewport } from "@/timeline";
import { shapeFraction, type CurvePoint } from "./curve";
import { rangeValueToY, rangeYToValue, type ValueRange } from "./valueAxis";

/** Vertical padding so points at 0 and 1 stay fully visible. */
export const LANE_PAD = 5;
/** Radius of a breakpoint handle. */
export const POINT_RADIUS = 4;

export interface LaneGeometry {
  vp: TimelineViewport;
  /** Lane height in px. */
  height: number;
  /** View beats of lane time 0 (default 0). */
  offset?: number;
  /** Visible window of normalized values (default: all of 0..1). */
  range?: ValueRange;
}

export function valueToY(value: number, height: number, range?: ValueRange): number {
  return rangeValueToY(value, height, LANE_PAD, range);
}

export function yToValue(y: number, height: number, range?: ValueRange): number {
  return rangeYToValue(y, height, LANE_PAD, range);
}

export function timeToX(time: number, g: LaneGeometry): number {
  return beatsToPx(time + (g.offset ?? 0), g.vp);
}

export function xToTime(x: number, g: LaneGeometry): number {
  return pxToBeats(x, g.vp) - (g.offset ?? 0);
}

/** Hit box of a point (px, lane-local). */
export function pointRect(p: CurvePoint, g: LaneGeometry, radius = POINT_RADIUS): Rect {
  const x = timeToX(p.time, g);
  const y = valueToY(p.value, g.height, g.range);
  return { x0: x - radius, y0: y - radius, x1: x + radius, y1: y + radius };
}

const fmt = (n: number) => (Math.round(n * 100) / 100).toString();

/**
 * SVG path `d` of a lane across `[x0, x1]` px (sorted points). Linear segments are straight
 * lines, steps hold then jump, curves are sampled with the engine formula every
 * `samplePx` px. Segments entirely outside the range are skipped. Returns "" without
 * points.
 */
export function lanePath(points: ReadonlyArray<CurvePoint>, g: LaneGeometry, x0: number, x1: number, samplePx = 2): string {
  const first = points[0];
  if (!first) return "";
  const y = (v: number) => fmt(valueToY(v, g.height, g.range));
  const parts: string[] = [];
  const firstX = timeToX(first.time, g);
  parts.push(`M${fmt(Math.min(x0, firstX))},${y(first.value)}`, `L${fmt(firstX)},${y(first.value)}`);
  for (let i = 0; i + 1 < points.length; i++) {
    const a = points[i]!;
    const b = points[i + 1]!;
    const ax = timeToX(a.time, g);
    const bx = timeToX(b.time, g);
    if (bx < x0 || ax > x1 || bx - ax <= 0 || a.curve.type === "Linear") {
      parts.push(`L${fmt(bx)},${y(b.value)}`);
      continue;
    }
    if (a.curve.type === "Step") {
      parts.push(`L${fmt(bx)},${y(a.value)}`, `L${fmt(bx)},${y(b.value)}`);
      continue;
    }
    // Curve: sample the visible part only; the rest is joined with straight lines.
    const from = Math.max(ax, x0);
    const to = Math.min(bx, x1);
    const n = Math.max(2, Math.min(512, Math.ceil((to - from) / samplePx)));
    const at = (x: number) => a.value + (b.value - a.value) * shapeFraction(a.curve, (x - ax) / (bx - ax));
    for (let k = 0; k <= n; k++) {
      const x = from + ((to - from) * k) / n;
      parts.push(`L${fmt(x)},${y(at(x))}`);
    }
    if (to < bx) parts.push(`L${fmt(bx)},${y(b.value)}`);
  }
  const last = points[points.length - 1]!;
  const lastX = timeToX(last.time, g);
  parts.push(`L${fmt(Math.max(x1, lastX))},${y(last.value)}`);
  return parts.join("");
}

/** Index `i` of the segment (points[i] → points[i+1]) containing lane time `t`, or -1. */
export function segmentAt(points: ReadonlyArray<CurvePoint>, t: number): number {
  for (let i = 0; i + 1 < points.length; i++) {
    if (points[i]!.time <= t && t < points[i + 1]!.time) return i;
  }
  return -1;
}
