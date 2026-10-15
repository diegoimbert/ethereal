/**
 * Automation edits as protocol commands (pure). Components send them through
 * `LaneGesture`, so every drag is one undo step.
 */

import type {
  AutomationLane,
  AutomationLaneId,
  AutomationOwner,
  AutomationPoint,
  AutomationPointId,
  AutomationTarget,
  Command,
  CurveShape,
  PointEdit,
  PointSpec,
} from "@/generated";
import { cmd } from "@/transport";
import { clampTension } from "./curve";
import { batchStep, countedStep } from "@/features/arrangement/editMath";

/**
 * Add a point. Without an existing lane the lane is created in the same undo step
 * (`newLaneId` names it): showing a parameter never touches the document by itself.
 */
export function addPointCommand(
  lane: AutomationLane | null,
  owner: AutomationOwner,
  target: AutomationTarget,
  point: PointSpec,
  newLaneId: AutomationLaneId,
): Command {
  if (lane) return cmd("Automation", { type: "AddPoints", lane: lane.id, points: [point] });
  return batchStep(countedStep("undo.automation.add_points", 1, "add automation point", "add automation points"), [
    cmd("Automation", { type: "CreateLane", id: newLaneId, owner, target }),
    cmd("Automation", { type: "AddPoints", lane: newLaneId, points: [point] }),
  ]);
}

export function removePointsCommand(ids: Iterable<AutomationPointId>): Command | null {
  const list = [...ids];
  return list.length ? cmd("Automation", { type: "RemovePoints", ids: list }) : null;
}

export function editPointsCommand(edits: PointEdit[]): Command | null {
  return edits.length ? cmd("Automation", { type: "EditPoints", edits }) : null;
}

/** Set the curve of the segments starting at `ids`. */
export function setCurveCommand(ids: Iterable<AutomationPointId>, curve: CurveShape): Command | null {
  return editPointsCommand([...ids].map((id) => ({ id, time: null, value: null, curve })));
}

/**
 * Edits for dragging `points` (the selection, as it was when the drag started) by `dt`
 * beats and `dv` (normalized). The group keeps its shape: `dt` and `dv` are clamped so no
 * point goes below time 0 or outside 0..1. `snapTime` snaps the anchor's new time (the
 * dragged point); the others move by the same delta. `snapValue` does the same for the
 * anchor's value. `lockTime`/`lockValue` constrain the drag to one axis.
 */
export function moveEdits(
  points: ReadonlyArray<AutomationPoint>,
  anchor: AutomationPoint,
  dt: number,
  dv: number,
  snapTime: ((t: number) => number) | null,
  opts: {
    lockTime?: boolean;
    lockValue?: boolean;
    snapValue?: (v: number) => number;
    /** The lane's other (unmoved) points: a moved point landing on one's exact time keeps its side. */
    others?: ReadonlyArray<AutomationPoint>;
  } = {},
): PointEdit[] {
  if (points.length === 0) return [];
  let minT = Infinity;
  let minV = Infinity;
  let maxV = -Infinity;
  for (const p of points) {
    minT = Math.min(minT, p.time);
    minV = Math.min(minV, p.value);
    maxV = Math.max(maxV, p.value);
  }
  let t = opts.lockTime ? 0 : dt;
  if (!opts.lockTime && snapTime) t = snapTime(anchor.time + t) - anchor.time;
  t = Math.max(t, -minT);
  let v = opts.lockValue ? 0 : dv;
  // The anchor lands on a step (stepped params, or the step modifier); the others follow.
  if (!opts.lockValue && opts.snapValue) v = opts.snapValue(anchor.value + v) - anchor.value;
  v = Math.min(Math.max(v, -minV), 1 - maxV);
  return points.map((p) => ({
    id: p.id,
    time: opts.lockTime ? null : keepSide(p, p.time + t, opts.others ?? []),
    value: opts.lockValue ? null : p.value + v,
    curve: null,
  }));
}

/** A nudge too small to see or hear (beats), to keep two points' order at the same time. */
export const TIE_NUDGE = 1e-6;

/**
 * `time` for moved point `p`, so that landing on an unmoved point's exact time keeps `p` on the
 * side it came from. Points at the same time are ordered by id (engine and UI alike): without
 * this, a step drawn by dragging a point onto its neighbour's time could flip its two values.
 */
function keepSide(p: AutomationPoint, time: number, others: ReadonlyArray<AutomationPoint>): number {
  for (const o of others) {
    if (Math.abs(o.time - time) > TIE_NUDGE / 2) continue;
    const wasBefore = p.time < o.time || (p.time === o.time && p.id < o.id);
    const idBefore = p.id < o.id;
    if (wasBefore !== idBefore) return Math.max(0, o.time + (wasBefore ? -TIE_NUDGE : TIE_NUDGE));
  }
  return time;
}

/** Pixels of vertical drag for a full tension sweep (−1 → 1). */
export const TENSION_DRAG_PX = 160;

/**
 * New tension when bending a segment by dragging `dyUp` px upwards (negative = down):
 * dragging up bulges the curve up. For a rising segment that is a fast start (negative
 * tension), for a falling one a slow start (positive tension).
 */
export function bendTension(start: number, dyUp: number, rising: boolean): number {
  const delta = (2 * dyUp) / TENSION_DRAG_PX;
  return clampTension(rising ? start - delta : start + delta);
}

/** Current tension of a curve (Linear = 0; Step has none and bends from 0). */
export function tensionOf(curve: CurveShape): number {
  return curve.type === "Curve" ? curve.tension : 0;
}
