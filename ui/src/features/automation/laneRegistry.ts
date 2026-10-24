/**
 * The mounted track automation lanes, so a gesture that starts in one lane can reach the
 * others on the same timeline (Bitwig / Ableton: one selection across lanes and tracks):
 *
 * - a drag on a lane's empty space selects the points of every lane it crosses, and the
 *   clips of the clip lanes it crosses (`ClipHitTester`, registered by the arrangement);
 * - dragging, nudging, deleting, copying and duplicating then act on every lane holding
 *   selected points, as one undo step (`AutomationLaneView`).
 *
 * Each `AutomationLaneView` of a TRACK lane registers itself (clip envelopes stay on their
 * own). Lanes are matched by their timeline (`view`), and hit tests run in client px, from
 * the lanes' boxes as laid out now (scroll, open/close animations included).
 */

import type { AutomationPoint, AutomationPointId, ClipId, Command, PointEdit, TrackId } from "@/generated";
import { rectsIntersect, type Rect, type TimelineViewStore } from "@/timeline";
import { pointRect, type LaneGeometry } from "./geometry";
import type { PasteTarget } from "./clipboard";

/** Drag modifiers for `RegisteredLane.dragEdits` (read from the pointer event). */
export interface DragMods {
  /** ⇧: whole value increments. */
  increments: boolean;
  lockTime: boolean;
  lockValue: boolean;
}

export interface RegisteredLane {
  view: TimelineViewStore;
  track: TrackId;
  /** Target key (`params.ts` `targetKey`). */
  key: string;
  /** The lane's root element (`.eth-auto-lane`). */
  el(): HTMLElement | null;
  /** The lane's points (sorted), as rendered. */
  points(): ReadonlyArray<AutomationPoint>;
  /** Its geometry now (lane-local px). */
  geom(): LaneGeometry;
  pasteTarget(): PasteTarget;
  /**
   * Edits moving `originals` (some of this lane's points) by `dt` beats (already snapped and
   * clamped by the grabbed lane) and `dyPx` pointer px (up is negative), with this lane's own
   * value scale, steps and clamping.
   */
  dragEdits(originals: ReadonlyArray<AutomationPoint>, anchor: AutomationPoint, dt: number, dyPx: number, mods: DragMods): PointEdit[];
  /** Edits nudging `points` (see `AutomationLaneView`'s keys); `dt` in beats. */
  nudgeEdits(points: ReadonlyArray<AutomationPoint>, dt: number, dvSteps: number, fine: boolean, precise: boolean): PointEdit[];
}

const lanes = new Set<RegisteredLane>();

/** Register a lane; returns the unregister function. */
export function registerLane(lane: RegisteredLane): () => void {
  lanes.add(lane);
  return () => {
    lanes.delete(lane);
  };
}

/** The registered lanes on `view` (display order: top to bottom). */
export function lanesOn(view: TimelineViewStore): RegisteredLane[] {
  const out = [...lanes].filter((l) => l.view === view && l.el()?.isConnected);
  const top = new Map(out.map((l) => [l, l.el()!.getBoundingClientRect().top]));
  return out.sort((a, b) => top.get(a)! - top.get(b)!);
}

/** The registered lane of `track` / `key` on `view`, or null. */
export function laneOf(view: TimelineViewStore, track: TrackId, key: string): RegisteredLane | null {
  for (const l of lanes) if (l.view === view && l.track === track && l.key === key && l.el()?.isConnected) return l;
  return null;
}

/**
 * The lane's box in client px. Without a layout (no box at all: jsdom) its geometry's height
 * from its box's corner, so the lane-local and client px agree there.
 */
export function laneBox(lane: RegisteredLane): { left: number; top: number; right: number; bottom: number } | null {
  const box = lane.el()?.getBoundingClientRect();
  if (!box) return null;
  if (box.width === 0 && box.height === 0) return { left: box.left, top: box.top, right: Infinity, bottom: box.top + lane.geom().height };
  return box.height > 0 ? box : null;
}

/** Ids of the points of the lanes on `view` inside `rect` (client px). */
export function pointsInRect(view: TimelineViewStore, rect: Rect): AutomationPointId[] {
  const out: AutomationPointId[] = [];
  for (const lane of lanesOn(view)) {
    const box = laneBox(lane);
    if (!box || box.bottom < rect.y0 || box.top > rect.y1) continue;
    const g = lane.geom();
    for (const p of lane.points()) {
      const r = pointRect(p, g);
      const c = { x0: r.x0 + box.left, y0: r.y0 + box.top, x1: r.x1 + box.left, y1: r.y1 + box.top };
      // Only the part of the lane that is on screen (a lane mid-animation is clipped).
      if (c.y0 > box.bottom || c.y1 < box.top) continue;
      if (rectsIntersect(rect, c)) out.push(p.id);
    }
  }
  return out;
}

/** The selected points of each lane on `view` (only lanes holding some). */
export function selectedByLane(view: TimelineViewStore, selected: ReadonlySet<AutomationPointId>): Array<{ lane: RegisteredLane; points: AutomationPoint[] }> {
  const out: Array<{ lane: RegisteredLane; points: AutomationPoint[] }> = [];
  if (selected.size === 0) return out;
  for (const lane of lanesOn(view)) {
    const points = lane.points().filter((p) => selected.has(p.id));
    if (points.length) out.push({ lane, points });
  }
  return out;
}

/** Clips under a rectangle in client px (the arrangement's clip lanes). */
export type ClipHitTester = (rect: Rect) => ClipId[];

const clipTesters = new Map<TimelineViewStore, ClipHitTester>();

/** The arrangement registers its clip hit test for its view; returns the unregister function. */
export function registerClipHitTester(view: TimelineViewStore, test: ClipHitTester): () => void {
  clipTesters.set(view, test);
  return () => {
    if (clipTesters.get(view) === test) clipTesters.delete(view);
  };
}

/** Clips of `view`'s clip lanes inside `rect` (client px); none without a registered tester. */
export function clipsInRect(view: TimelineViewStore, rect: Rect): ClipId[] {
  return clipTesters.get(view)?.(rect) ?? [];
}

/**
 * The commands as one list (a `Batch` is flattened: batches don't nest), for one combined
 * undo step.
 */
export function flattenCommands(commands: ReadonlyArray<Command | null>): Command[] {
  const out: Command[] = [];
  for (const c of commands) {
    if (!c) continue;
    if (c.domain === "Edit" && c.command.type === "Batch") out.push(...c.command.commands);
    else out.push(c);
  }
  return out;
}

/** Tests: forget every lane and tester. */
export function resetLaneRegistry(): void {
  lanes.clear();
  clipTesters.clear();
}
