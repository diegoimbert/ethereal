/**
 * Automation lanes on the WebGL arranger.
 *
 * The lanes stay React components (`AutomationLaneView` with `gl`): headers, value scales,
 * focus, keyboard, menus, the drag tooltip and the marquee are DOM, and the lane's SVG keeps
 * taking the pointer with the lane's gesture code. With `gl`, the SVG draws nothing
 * itself (no element per point): the lane publishes what it would draw here, and the
 * `automation` layer draws it on the arranger canvas (background, step gridlines, envelope,
 * breakpoints), at the lane's place in its row as laid out by the DOM (so the open/close and
 * show/hide animations line up with the headers).
 */

import type { AutomationPoint, AutomationPointId, TrackId } from "@/generated";
import { shapeFraction } from "@/features/automation/curve";
import { POINT_RADIUS, valueToY } from "@/features/automation/geometry";
import type { StepLine, ValueRange } from "@/features/automation/valueAxis";
import type { Row } from "../../layout";
import { arrangementView } from "../../uiStore";
import { glRenderer } from "../active";
import type { Pipeline } from "../glContext";
import type { Hit } from "../hitTest";
import type { ArrangerLayer, LaneLayerContext, LayerOutput } from "../layers";
import { writeRect, type XSpec } from "../pipelines";
import type { ViewWindow } from "../scene";
import { lanePalette, over } from "./lanePalette";
import { writeSegment, type ClipY } from "./shapes";
import { WindowTokens } from "./viewWindow";

/** What one automation lane draws (published by `AutomationLaneView` with `gl`). */
export interface GlAutomationLane {
  track: TrackId;
  /** The lane's root element (`.eth-auto-lane`), for its place in the row. */
  el: HTMLElement;
  /** Sorted by time. */
  points: ReadonlyArray<AutomationPoint>;
  selected: ReadonlySet<AutomationPointId>;
  /** Lane (SVG) height. */
  height: number;
  range: ValueRange;
  lines: ReadonlyArray<StepLine>;
  disabled: boolean;
  /** View beats of lane time 0. */
  offset: number;
  /**
   * An empty lane's baseline: the target's current value (normalized), drawn grayed and
   * dashed across the lane; `null` (or absent) when the lane has points.
   */
  baseline?: number | null;
}

/** Where a lane is in its row (px from the row top): its top, and the visible part. */
export interface LanePlace {
  top: number;
  clipTop: number;
  clipBottom: number;
}

const published = new Map<object, GlAutomationLane>();
const versions = new Map<TrackId, number>();

function bump(track: TrackId): void {
  versions.set(track, (versions.get(track) ?? 0) + 1);
  glRenderer()?.requestRender();
}

/** Publishes (or with `null` withdraws) the lane keyed by `key` (one key per mounted lane). */
export function publishAutomationLane(key: object, lane: GlAutomationLane | null): void {
  const prev = published.get(key);
  if (lane) published.set(key, lane);
  else published.delete(key);
  if (prev && prev.track !== lane?.track) bump(prev.track);
  if (lane) bump(lane.track);
}

/** The published lanes of a track. */
export function automationLanesOf(track: TrackId): GlAutomationLane[] {
  const out: GlAutomationLane[] = [];
  for (const l of published.values()) if (l.track === track) out.push(l);
  return out;
}

/** Changes whenever a lane of `track` is published or withdrawn. */
export function automationVersion(track: TrackId): number {
  return versions.get(track) ?? 0;
}

/** A lane's place in its row from the DOM (transforms of animations included). */
export function placeFromDom(lane: GlAutomationLane): LanePlace | null {
  const row = lane.el.closest<HTMLElement>(".eth-arr-row");
  const area = lane.el.parentElement;
  if (!row || !area || !lane.el.isConnected) return null;
  const r = row.getBoundingClientRect().top;
  const a = area.getBoundingClientRect();
  return { top: lane.el.getBoundingClientRect().top - r, clipTop: a.top - r, clipBottom: a.bottom - r };
}

/** Beyond the lane's first and last points the value holds: the line runs off the view. */
const FAR = 1e6;
/** The whole lane's width. */
const FULL_X: XSpec = { b0: -FAR, b1: FAR };
/** Envelope stroke (`.eth-auto-lane__line`: 1.5 px). */
const LINE_RADIUS = 0.75;
/** Breakpoints (`.eth-auto-lane__point`: r 4, stroke 1.5 centered). */
const POINT_STROKE = 1.5;
/** Opacity of a disabled lane's envelope and points. */
const DISABLED_OPACITY = 0.35;
/** The empty-lane baseline's dashes and gaps (px; `.eth-auto-lane__baseline`: dasharray 4 4). */
const BASELINE_DASH = 4;

/**
 * The dashes `[beatA, beatB, ...]` of a baseline across `[from, to]` beats (pure): `dash` px
 * on, `dash` px off at `ppb`, anchored at beat 0 so rebuilds on pan keep them in place.
 */
export function baselineDashes(from: number, to: number, ppb: number, dash = BASELINE_DASH): number[] {
  const out: number[] = [];
  if (!(ppb > 0) || !(to > from)) return out;
  const len = dash / ppb;
  const period = 2 * len;
  for (let k = Math.floor(from / period); k * period < to; k++) out.push(k * period, k * period + len);
  return out;
}

/** Curves are sampled every this many px at the build zoom (as `lanePath`). */
const SAMPLE_PX = 2;

/** First index whose beat is >= `b` (points sorted by time). */
function lowerBound(points: ReadonlyArray<AutomationPoint>, offset: number, b: number): number {
  let lo = 0;
  let hi = points.length;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if (points[m]!.time + offset < b) lo = m + 1;
    else hi = m;
  }
  return lo;
}

/**
 * The envelope of `points` as straight pieces `[beatA, yA, beatB, yB]` (pure), within
 * `[from, to]` beats plus the segments crossing them: holds run off to ±`FAR` beyond the
 * first and last points, steps hold then jump, curves are sampled every `SAMPLE_PX` px at
 * `ppb` inside the window and joined straight outside it (as `lanePath`).
 */
export function envelope(
  points: ReadonlyArray<AutomationPoint>,
  offset: number,
  y: (value: number) => number,
  from: number,
  to: number,
  ppb: number,
): number[] {
  const out: number[] = [];
  const n = points.length;
  if (n === 0) return out;
  const seg = (b0: number, y0: number, b1: number, y1: number) => out.push(b0, y0, b1, y1);
  // Segments [i, i + 1] that reach the window.
  const i0 = Math.max(0, lowerBound(points, offset, from) - 1);
  const i1 = Math.min(n - 1, lowerBound(points, offset, to));
  const first = points[0]!;
  if (i0 === 0) seg(-FAR, y(first.value), first.time + offset, y(first.value));
  for (let i = i0; i < i1; i++) {
    const a = points[i]!;
    const b = points[i + 1]!;
    const ab = a.time + offset;
    const bb = b.time + offset;
    const ya = y(a.value);
    const yb = y(b.value);
    if (bb - ab <= 0 || a.curve.type === "Linear" || bb < from || ab > to) {
      seg(ab, ya, bb, yb);
    } else if (a.curve.type === "Step") {
      seg(ab, ya, bb, ya);
      seg(bb, ya, bb, yb);
    } else {
      const lo = Math.max(ab, from);
      const hi = Math.min(bb, to);
      if (lo > ab) seg(ab, ya, lo, y(a.value + (b.value - a.value) * shapeFraction(a.curve, (lo - ab) / (bb - ab))));
      const k = Math.max(2, Math.min(512, Math.ceil(((hi - lo) * ppb) / SAMPLE_PX)));
      const at = (t: number) => y(a.value + (b.value - a.value) * shapeFraction(a.curve, (t - ab) / (bb - ab)));
      let pb = lo;
      let py = at(lo);
      for (let s = 1; s <= k; s++) {
        const t = lo + ((hi - lo) * s) / k;
        const ty = at(t);
        seg(pb, py, t, ty);
        pb = t;
        py = ty;
      }
      if (hi < bb) seg(hi, py, bb, yb);
    }
  }
  const last = points[n - 1]!;
  if (i1 === n - 1) seg(last.time + offset, y(last.value), FAR, y(last.value));
  return out;
}

/** The index range of the points to draw in `[from, to]` beats (a point's radius around). */
function pointRange(points: ReadonlyArray<AutomationPoint>, offset: number, from: number, to: number, ppb: number): [number, number] {
  const pad = (POINT_RADIUS + POINT_STROKE) / ppb;
  return [lowerBound(points, offset, from - pad), lowerBound(points, offset, to + pad + 1e-9)];
}

/** Draws one lane into `out` (pure apart from the writers; `place`: px from the row top). */
export function buildAutomationLane(
  lane: GlAutomationLane,
  place: LanePlace,
  slot: ClipY,
  win: ViewWindow,
  ctx: Pick<LaneLayerContext, "input">,
  out: Pick<LayerOutput, "rects" | "custom">,
  shape: Pipeline,
): void {
  const clip: ClipY = { y0: Math.max(place.clipTop, slot.y0), y1: Math.min(place.clipBottom, slot.y1) };
  if (clip.y1 <= clip.y0) return;
  const c = lanePalette(ctx.input.palette);
  const clipBox = { x: FULL_X, y: { a0: clip.y0, a1: clip.y1 } };
  // Background, then the step gridlines (1 px, centered on the value like the SVG lines).
  writeRect(out.rects, { x: FULL_X, y: { a0: clip.y0, a1: clip.y1 }, fill: c.bgInset });
  for (const l of lane.lines) {
    const yy = place.top + valueToY(l.value, lane.height, lane.range);
    writeRect(out.rects, { x: FULL_X, y: { a0: yy - 0.5, a1: yy + 0.5 }, fill: l.major ? c.borderLight : c.border, clip: clipBox });
  }
  const y = (v: number) => place.top + valueToY(v, lane.height, lane.range);
  const op = lane.disabled ? DISABLED_OPACITY : 1;
  const line = over(c.accent, op, c.bgInset);
  const w = out.custom(shape);
  if (lane.baseline != null && lane.points.length === 0) {
    // Flat-looking caps: a capsule reaches its radius past its ends, so the dash is inset.
    const by = y(lane.baseline);
    const color = over(c.textDim, op, c.bgInset);
    const dashes = baselineDashes(win.from, win.to, win.ppb);
    for (let i = 0; i < dashes.length; i += 2) {
      writeSegment(w, { beat: dashes[i]!, px: LINE_RADIUS, y: by }, { beat: dashes[i + 1]!, px: -LINE_RADIUS, y: by }, LINE_RADIUS, color, clip);
    }
  }
  const segs = envelope(lane.points, lane.offset, y, win.from, win.to, win.ppb);
  for (let i = 0; i < segs.length; i += 4) {
    writeSegment(w, { beat: segs[i]!, y: segs[i + 1]! }, { beat: segs[i + 2]!, y: segs[i + 3]! }, LINE_RADIUS, line, clip);
  }
  const [p0, p1] = pointRange(lane.points, lane.offset, win.from, win.to, win.ppb);
  const fill = over(c.bgInset, op, c.bgInset);
  const rim = line;
  const fillSel = over(c.selection, op, c.bgInset);
  const rimSel = over(c.text, op, c.bgInset);
  for (let i = p0; i < p1; i++) {
    const p = lane.points[i]!;
    const at = { beat: p.time + lane.offset, y: y(p.value) };
    const on = lane.selected.has(p.id);
    writeSegment(w, at, at, POINT_RADIUS + POINT_STROKE / 2, on ? fillSel : fill, clip, { color: on ? rimSel : rim, width: POINT_STROKE });
  }
}

/** The point of a lane under lane-local px (`x` from the lane's left, `y` from its top). */
export function automationPointAt(
  lane: Pick<GlAutomationLane, "points" | "offset" | "height" | "range">,
  x: number,
  y: number,
  view: { scrollBeats: number; pxPerBeat: number },
): AutomationPoint | null {
  const r = POINT_RADIUS + POINT_STROKE / 2;
  const beat = view.scrollBeats + x / view.pxPerBeat;
  const pad = r / view.pxPerBeat;
  const pts = lane.points;
  let best: AutomationPoint | null = null;
  // Later points are drawn above earlier ones: the last one hit wins.
  for (let i = lowerBound(pts, lane.offset, beat - pad); i < pts.length; i++) {
    const p = pts[i]!;
    if (p.time + lane.offset > beat + pad) break;
    const px = (p.time + lane.offset - view.scrollBeats) * view.pxPerBeat;
    const py = valueToY(p.value, lane.height, lane.range);
    if (Math.hypot(px - x, py - y) <= r) best = p;
  }
  return best;
}

/**
 * The `automation` layer: every published lane of the row's track, drawn under the clips
 * (the lanes are below the main lane: nothing overlaps).
 */
export function automationLayer(shape: Pipeline, place: (lane: GlAutomationLane) => LanePlace | null = placeFromDom): ArrangerLayer {
  const windows = new WindowTokens();
  return {
    id: "automation",
    pipelines: [shape],
    z: "under-clips",
    deps: ({ track, row, input, window }) => [automationVersion(track.id), row.height, row.laneHeight, input.palette, windows.token(track.id, window)],
    build(ctx, out) {
      const win = windows.token(ctx.track.id, ctx.window);
      const slot = { y0: ctx.row.laneHeight, y1: ctx.row.height };
      for (const lane of automationLanesOf(ctx.track.id)) {
        const at = place(lane);
        if (at) buildAutomationLane(lane, at, slot, win, ctx, out, shape);
      }
    },
    hitTest(x: number, y: number, row: Row): Hit | null {
      // For the test API: presses on the lanes land on their SVG (the lane's own gestures).
      if (y < row.laneHeight) return null;
      const view = arrangementView.getState();
      for (const lane of automationLanesOf(row.track.id)) {
        const at = place(lane);
        if (!at || y < at.clipTop || y >= at.clipBottom) continue;
        const p = automationPointAt(lane, x, y - at.top, view);
        if (p) return { clip: "", track: row.track.id, part: "layer", data: { point: p.id } };
      }
      return null;
    },
  };
}
