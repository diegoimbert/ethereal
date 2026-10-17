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
import { shapeFraction, splitPointAt } from "./curve";
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

/** Moving a time section of a lane up/down (`sectionMovePlan`). */
export interface SectionPlan {
  /** Edge points to add first (new ids, in id order). */
  add: PointSpec[];
  /** What moves: the points inside the section and its inner edge points (added or existing). */
  moving: AutomationPoint[];
}

/**
 * Plan a vertical move of the section `[a, b]` of a lane (`pts` sorted by time, then id):
 * the lane outside the section must not change, so each edge becomes a jump of two points at
 * the same time (the engine orders them by id): an outer one keeping the outside value and an
 * inner one that moves with the points strictly inside.
 *
 * - Start edge: no point there → two new points with the lane's value (the outer one first;
 *   the inner one continues the split segment, `splitPointAt`). One existing point → it stays
 *   as the outer point and a new inner one follows it (a new id sorts after it). Two or more
 *   (already a jump) → the last one moves, nothing is added.
 * - End edge: no point → two new points (inner first, then the outer one continuing the
 *   segment after the edge). One existing point → it becomes the inner one (it moves) and a
 *   new outer point after it keeps its value and curve. Two or more → the first one moves.
 *
 * The points outside are untouched, so their values are bit-identical; the lane between
 * them and the edges keeps its shape exactly for linear and step segments and for the part
 * of a curve before an edge; the part of a curve after an edge gets the closest tension
 * (`splitPointAt`). `null` for a lane without points or an empty section.
 */
export function sectionMovePlan(
  pts: ReadonlyArray<AutomationPoint>,
  a: Beats,
  b: Beats,
  laneId: AutomationLaneId,
  makeId: () => AutomationPointId = newId,
): SectionPlan | null {
  if (pts.length === 0 || b - a <= RANGE_EPS) return null;
  const at = (t: Beats) => pts.filter((p) => Math.abs(p.time - t) <= RANGE_EPS);
  const inside = pts.filter((p) => p.time > a + RANGE_EPS && p.time < b - RANGE_EPS);
  const add: PointSpec[] = [];
  const LINEAR: CurveShape = { type: "Linear" };
  const created = (spec: PointSpec): AutomationPoint => ({ ...spec, lane: laneId });
  const startMoving: AutomationPoint[] = [];
  const endMoving: AutomationPoint[] = [];

  const atA = at(a);
  if (atA.length >= 2) startMoving.push(atA[atA.length - 1]!);
  else if (atA.length === 1) {
    const p = atA[0]!;
    const inner: PointSpec = { id: makeId(), time: p.time, value: p.value, curve: p.curve };
    add.push(inner);
    startMoving.push(created(inner));
  } else {
    const { value, curve } = splitPointAt(pts, a, LINEAR)!;
    add.push({ id: makeId(), time: a, value, curve: LINEAR });
    const inner: PointSpec = { id: makeId(), time: a, value, curve };
    add.push(inner);
    startMoving.push(created(inner));
  }

  const atB = at(b);
  if (atB.length >= 2) endMoving.push(atB[0]!);
  else if (atB.length === 1) {
    const q = atB[0]!;
    endMoving.push(q);
    add.push({ id: makeId(), time: q.time, value: q.value, curve: q.curve });
  } else {
    const { value, curve } = splitPointAt(pts, b, LINEAR)!;
    const inner: PointSpec = { id: makeId(), time: b, value, curve: LINEAR };
    add.push(inner);
    endMoving.push(created(inner));
    add.push({ id: makeId(), time: b, value, curve });
  }
  return { add, moving: [...startMoving, ...inside, ...endMoving] };
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
