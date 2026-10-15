/**
 * Time-range edits of ONE automation lane (pure): copy a range with its edge values, and
 * write a piece (or nothing) over a range while the curve outside it stays exactly as it was
 * (breakpoints are added at the range edges where needed). Mirror of the engine's
 * time-edit point functions (`ether-controller` `time_edit/points.rs`: `slice`, `overwrite`),
 * expressed with the existing commands: one `Edit::Batch` of `RemovePoints` + `AddPoints`
 * (+ `CreateLane`), so each range edit is one undo step.
 *
 * - **Delete** a range: its points go; the value before it and after it are kept, joined by a
 *   ramp across the range (no time is removed: later points stay where they are).
 *
 * Unlike the engine's time edits, the edge points are added whenever the lane has points
 * (not only when it has points beyond the edge), so even the value held before the first
 * point / after the last one is kept.
 * - **Copy**: the points inside the range, relative to its start, plus its edge values.
 * - **Paste / Duplicate**: write the copied piece over `[at, at + length]`.
 *
 * Two points at the same time form a jump; which one comes first is decided by id order
 * (`pointsOfLane` / `Project::points_of`): new ids are monotonic ULIDs, minted here in list
 * order, and points at the range edges are always new.
 */

import type {
  AutomationLane,
  AutomationLaneId,
  AutomationOwner,
  AutomationPoint,
  AutomationPointId,
  AutomationTarget,
  Beats,
  Command,
  CurveShape,
  PointSpec,
} from "@/generated";
import { cmd, newId } from "@/transport";
import { shapeFraction } from "./curve";
import { batchStep, type StepLabel } from "@/features/arrangement/editMath";

/** Same-time tolerance (beats), the engine's. */
export const RANGE_EPS = 1e-9;

export interface RangePoint {
  time: number;
  value: number;
  curve: CurveShape;
}

/** A copied range: points relative to its start (the first at 0, the last at `length`). */
export interface RangePiece {
  points: RangePoint[];
  length: Beats;
}

function segment(a: RangePoint, b: RangePoint, time: number): number {
  const span = b.time - a.time;
  if (span <= 0) return b.value;
  return a.value + (b.value - a.value) * shapeFraction(a.curve, (time - a.time) / span);
}

/** Value at `time`, right limit (a point exactly at `time` counts). `pts` sorted. */
export function valueAt(pts: ReadonlyArray<RangePoint>, time: number): number | null {
  const first = pts[0];
  if (!first) return null;
  if (time < first.time) return first.value;
  const i = pts.findIndex((p) => p.time > time);
  if (i < 0) return pts[pts.length - 1]!.value;
  return segment(pts[i - 1]!, pts[i]!, time);
}

/** Value just before `time`, left limit (points exactly at `time` don't count yet). */
export function valueBefore(pts: ReadonlyArray<RangePoint>, time: number): number | null {
  const first = pts[0];
  if (!first) return null;
  if (time <= first.time) return first.value;
  const i = pts.findIndex((p) => p.time >= time);
  if (i < 0) return pts[pts.length - 1]!.value;
  return segment(pts[i - 1]!, pts[i]!, time);
}

/** Curve of the segment running through `time` (the last point at or before it). */
function curveAt(pts: ReadonlyArray<RangePoint>, time: number): CurveShape {
  let c: CurveShape = { type: "Linear" };
  for (const p of pts) {
    if (p.time > time) break;
    c = p.curve;
  }
  return c;
}

/** The range `[a, b]` of a lane (`pts` sorted), relative to `a`, or null for an empty lane. */
export function sliceRange(pts: ReadonlyArray<RangePoint>, a: Beats, b: Beats): RangePiece | null {
  const start = valueAt(pts, a);
  const end = valueBefore(pts, b);
  if (start === null || end === null || b - a <= RANGE_EPS) return null;
  const inside = pts.filter((p) => p.time > a + RANGE_EPS && p.time < b - RANGE_EPS).map((p) => ({ time: p.time - a, value: p.value, curve: p.curve }));
  return {
    points: [{ time: 0, value: start, curve: curveAt(pts, a) }, ...inside, { time: b - a, value: end, curve: { type: "Linear" } }],
    length: b - a,
  };
}

/** What writing over a range does to a lane: the points removed and the ones added (in id order). */
export interface RangeWrite {
  remove: AutomationPointId[];
  add: PointSpec[];
}

/**
 * Write `piece` (relative times; empty = delete) over `[at, at + length]` of a lane whose
 * points are `pts` (sorted): the points in the range go, the curve outside it is kept with
 * edge breakpoints. `mapValue` remaps pasted values (another parameter).
 */
export function writeRange(
  pts: ReadonlyArray<AutomationPoint>,
  at: Beats,
  length: Beats,
  piece: ReadonlyArray<RangePoint>,
  opts: { ids?: () => AutomationPointId; mapValue?: (v: number) => number } = {},
): RangeWrite {
  const makeId = opts.ids ?? newId;
  const map = opts.mapValue ?? ((v: number) => v);
  const end = at + length;
  // The lane's value outside the range (held before its first point and after its last one)
  // is kept by edge points, whenever the lane has points at all.
  const hasBefore = pts.length > 0;
  const hasAfter = pts.length > 0;
  const remove = pts.filter((p) => p.time >= at - RANGE_EPS && p.time <= end + RANGE_EPS).map((p) => p.id);
  const add: PointSpec[] = [];
  const left = hasBefore ? valueBefore(pts, at) : null;
  // The value up to `at` is kept (then a jump to the piece's first value, if it differs).
  if (left !== null) add.push({ id: makeId(), time: at, value: left, curve: { type: "Linear" } });
  for (const p of piece) add.push({ id: makeId(), time: at + p.time, value: map(p.value), curve: p.curve });
  const right = hasAfter ? valueAt(pts, end) : null;
  if (right !== null) add.push({ id: makeId(), time: end, value: right, curve: curveAt(pts, end) });
  return { remove, add };
}

/**
 * The command of a range write, one undo step: creates the lane if needed (`newLaneId`),
 * removes the range's points, adds the new ones. `null` when it changes nothing.
 */
export function rangeWriteCommand(
  write: RangeWrite,
  dest: { lane: AutomationLane | null; owner: AutomationOwner; target: AutomationTarget; newLaneId?: AutomationLaneId },
  label: StepLabel,
): Command | null {
  if (write.remove.length === 0 && write.add.length === 0) return null;
  const commands: Command[] = [];
  const laneId = dest.lane?.id ?? dest.newLaneId ?? newId();
  if (!dest.lane) {
    if (write.add.length === 0) return null;
    commands.push(cmd("Automation", { type: "CreateLane", id: laneId, owner: dest.owner, target: dest.target }));
  }
  if (write.remove.length) commands.push(cmd("Automation", { type: "RemovePoints", ids: write.remove }));
  if (write.add.length) commands.push(cmd("Automation", { type: "AddPoints", lane: laneId, points: write.add }));
  return batchStep(label, commands);
}
