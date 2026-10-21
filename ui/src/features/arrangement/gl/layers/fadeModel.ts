/**
 * What the clip-fades layer draws and hit-tests, as pure functions of a lane's items (no
 * GL): which clips are crossfaded, which parts of each clip are not covered by clips drawn
 * above it, and where `ClipFades`' handles sit (in lane px) at a zoom.
 */

import type { ClipId, FadeCurve } from "@/generated";
import { audioOf, isCrossfade } from "@/features/clip-editing/clipEditing";
import { fadeGain } from "@/features/clip-editing/fades";
import { CLIP_INSET, isCompactLane, titleHeight } from "../../laneBodyGeometry";
import type { LaneItem } from "../../laneItems";
import { boxX, isRaised, type LaneView } from "../hitTest";

export type FadeHandle = "fade-in" | "fade-out" | "fade-in-curve" | "fade-out-curve";
export const FADE_HANDLES: ReadonlyArray<FadeHandle> = ["fade-in", "fade-out", "fade-in-curve", "fade-out-curve"];

/** Visible handle size (`--eth-space-md`) and the invisible grab margin around it (`--eth-space-md`). */
export const HANDLE_PX = 8;
export const GRAB_PX = 8;
/** Length handles stay this far inside the clip's edges (`max(fade, --eth-space-md)`). */
export const HANDLE_INSET_PX = 8;
/** Curve handles show on fades at least this wide (px). */
export const CURVE_HANDLE_MIN_PX = 8;

/** The crossfade neighbour's curve on each side of a clip (its fade-out for `in`, its fade-in for `out`). */
export interface CrossfadeViews {
  in: FadeCurve | null;
  out: FadeCurve | null;
}

/**
 * Crossfades between the lane's audio clips (by take lane), as `ClipFades` reads them from
 * the document (`crossfadeSide`): neighbours in start order that overlap as a crossfade.
 */
export function laneCrossfades(items: ReadonlyArray<LaneItem>): Map<ClipId, CrossfadeViews> {
  const byLane = new Map<string, LaneItem["clip"][]>();
  const seen = new Set<ClipId>();
  for (const it of items) {
    if (it.ghost || seen.has(it.clip.id) || it.clip.content.type !== "Audio") continue;
    seen.add(it.clip.id);
    const key = it.clip.lane ?? "";
    let list = byLane.get(key);
    if (!list) byLane.set(key, (list = []));
    list.push(it.clip);
  }
  const out = new Map<ClipId, CrossfadeViews>();
  const views = (id: ClipId) => {
    let v = out.get(id);
    if (!v) out.set(id, (v = { in: null, out: null }));
    return v;
  };
  for (const list of byLane.values()) {
    list.sort((a, b) => a.start - b.start);
    for (let i = 0; i + 1 < list.length; i++) {
      const a = list[i]!;
      const b = list[i + 1]!;
      if (!isCrossfade(a, b)) continue;
      views(a.id).out = audioOf(b)!.fade_in_curve;
      views(b.id).in = audioOf(a)!.fade_out_curve;
    }
  }
  return out;
}

/** The lane's items in draw order: base clips by start, then raised ones (selected, dragged). */
export function drawOrder(items: ReadonlyArray<LaneItem>, selected: ReadonlySet<ClipId>): LaneItem[] {
  const base: LaneItem[] = [];
  const raised: LaneItem[] = [];
  for (const it of items) (isRaised(it, selected) ? raised : base).push(it);
  return [...base, ...raised];
}

/**
 * Per item of `order` (draw order), the spans of its beats no later (higher) clip covers.
 * Copy ghosts are translucent: they cover nothing.
 */
export function visibleSpans(order: ReadonlyArray<LaneItem>): Array<Array<[number, number]>> {
  return order.map((it, k) => {
    const s = it.bounds.start;
    const e = s + it.bounds.length;
    const cover: Array<[number, number]> = [];
    for (let j = k + 1; j < order.length; j++) {
      const o = order[j]!;
      if (o.ghost) continue;
      const os = o.bounds.start;
      const oe = os + o.bounds.length;
      if (oe > s && os < e) cover.push([os, oe]);
    }
    if (cover.length === 0) return [[-Infinity, Infinity]];
    cover.sort((a, b) => a[0] - b[0]);
    const spans: Array<[number, number]> = [];
    let at = -Infinity;
    for (const [os, oe] of cover) {
      if (os > at) spans.push([at, os]);
      at = Math.max(at, oe);
    }
    if (at < e) spans.push([at, Infinity]);
    return spans.filter(([a, b]) => Math.min(b, e) > Math.max(a, s));
  });
}

/** Where a clip's fade parts are, in lane px (x from the lane's left, y from the row top). */
export interface FadeGeometry {
  x0: number;
  x1: number;
  /** The body (below the title band) and the clip box's bottom. */
  top: number;
  bottom: number;
  bodyH: number;
  /** Fade lengths on screen (px). */
  fin: number;
  fout: number;
  /** Centres of the length handles (on the body's top edge). */
  inX: number;
  outX: number;
  /** Centres of the curve handles (null when the fade is too short to show one). */
  inCurve: { x: number; y: number } | null;
  outCurve: { x: number; y: number } | null;
}

/** `ClipFades`' layout for an audio item (null for other clips) in a lane `mainHeight` tall. */
export function fadeGeometry(it: LaneItem, v: LaneView, mainHeight: number): FadeGeometry | null {
  const a = audioOf(it.clip);
  // A compact lane's clips have no body: no fades shown.
  if (!a || isCompactLane(mainHeight)) return null;
  const [x0, x1] = boxX(it, v);
  const len = it.bounds.length;
  const width = Math.max(1, len * v.pxPerBeat);
  const fin = Math.min(a.fade_in, len) * v.pxPerBeat;
  const fout = Math.min(a.fade_out, len) * v.pxPerBeat;
  const top = CLIP_INSET + titleHeight(mainHeight);
  const bottom = mainHeight - CLIP_INSET;
  const bodyH = Math.max(0, bottom - top);
  const curve = (fadePx: number, cx: number, c: FadeCurve) => (fadePx < CURVE_HANDLE_MIN_PX ? null : { x: cx, y: top + (1 - fadeGain(c, 0.5)) * bodyH });
  return {
    x0,
    x1,
    top,
    bottom,
    bodyH,
    fin,
    fout,
    inX: Math.max(x0 + fin, x0 + HANDLE_INSET_PX),
    outX: Math.min(x0 + width - fout, x1 - HANDLE_INSET_PX),
    inCurve: curve(fin, x0 + fin / 2, a.fade_in_curve),
    outCurve: curve(fout, x0 + width - fout / 2, a.fade_out_curve),
  };
}

export interface PxRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The visible rect of a handle (lane px), or null when it isn't shown. */
export function handleRect(g: FadeGeometry, h: FadeHandle): PxRect | null {
  const half = HANDLE_PX / 2;
  switch (h) {
    case "fade-in":
      return { x: g.inX - half, y: g.top, width: HANDLE_PX, height: HANDLE_PX };
    case "fade-out":
      return { x: g.outX - half, y: g.top, width: HANDLE_PX, height: HANDLE_PX };
    case "fade-in-curve":
      return g.inCurve && { x: g.inCurve.x - half, y: g.inCurve.y - half, width: HANDLE_PX, height: HANDLE_PX };
    case "fade-out-curve":
      return g.outCurve && { x: g.outCurve.x - half, y: g.outCurve.y - half, width: HANDLE_PX, height: HANDLE_PX };
  }
}

/**
 * The handle whose grab area holds the point, with `ClipFades`' priorities: the handles sit
 * above the clip's edge strips (near a corner the fade wins; lower down the edge, the strip
 * does), the length handles' grab areas reach sideways and down but never up into the title
 * band, and no handle takes the pointer over the title band (it keeps move and select).
 * Later handles are on top: the curve handles over the length handles, `out` over `in`.
 */
export function hitFadeHandle(g: FadeGeometry, x: number, y: number): FadeHandle | null {
  if (x < g.x0 || x >= g.x1 || y < g.top || y >= g.bottom) return null;
  for (const h of ["fade-out-curve", "fade-in-curve", "fade-out", "fade-in"] as const) {
    const r = handleRect(g, h);
    if (!r) continue;
    const grabTop = h === "fade-in" || h === "fade-out" ? r.y : r.y - GRAB_PX;
    if (x >= r.x - GRAB_PX && x < r.x + r.width + GRAB_PX && y >= grabTop && y < r.y + r.height + GRAB_PX) return h;
  }
  return null;
}
