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
  return cmd("Edit", {
    type: "Batch",
    label: "Add automation point",
    commands: [
      cmd("Automation", { type: "CreateLane", id: newLaneId, owner, target }),
      cmd("Automation", { type: "AddPoints", lane: newLaneId, points: [point] }),
    ],
  });
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
 * dragged point); the others move by the same delta. `lockTime`/`lockValue` constrain the
 * drag to one axis.
 */
export function moveEdits(
  points: ReadonlyArray<AutomationPoint>,
  anchor: AutomationPoint,
  dt: number,
  dv: number,
  snapTime: ((t: number) => number) | null,
  opts: { lockTime?: boolean; lockValue?: boolean } = {},
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
  const v = opts.lockValue ? 0 : Math.min(Math.max(dv, -minV), 1 - maxV);
  return points.map((p) => ({
    id: p.id,
    time: opts.lockTime ? null : p.time + t,
    value: opts.lockValue ? null : p.value + v,
    curve: null,
  }));
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
