/**
 * Automation point clipboard: copy / cut / paste / duplicate (cmd-C / X / V / D and the
 * lanes' right-click menus), in the arrangement clipboard's way (`features/arrangement/
 * clipboard.ts`): copy snapshots the selected points; paste puts them at a time (the
 * playhead, or where the lane was right-clicked) keeping their relative timing, as one undo
 * step, then selects the pasted points and (when stopped) moves the playhead to their end,
 * so pasting again continues the pattern.
 *
 * Pasting replaces the points already in the pasted span. Into another parameter, values
 * are remapped: through the plain value when both params share a unit (clamped to the
 * target's range), else by normalized value; either way snapped to the target's steps.
 */

import type {
  AutomationLane,
  AutomationOwner,
  AutomationPoint,
  AutomationPointId,
  AutomationTarget,
  Beats,
  Command,
  CurveShape,
  ParamInfo,
  PointSpec,
} from "@/generated";
import { paramToNormalized, paramToPlain } from "@/domain/paramScale";
import { pointsOfLane, useProjectStore } from "@/state";
import { itemSelection, type ItemSelectionStore } from "@/timeline";
import { cmd, newId, type EngineTransport } from "@/transport";
import { advanceCursor } from "@/features/time-edits/marker";
import { sendEdit } from "./gesture";
import { rangeWriteCommand, sliceRange, writeRange } from "./range";
import { snapValue } from "./valueAxis";
import { batchStep, countedStep, keyedStep, type StepLabel } from "@/features/arrangement/editMath";

export interface CopiedPoint {
  /** Offset from the first copied point (beats). */
  dt: Beats;
  /** Normalized value in the source param. */
  value: number;
  curve: CurveShape;
}

export interface PointClipboard {
  points: CopiedPoint[];
  /** Beats from the first to the last point. */
  span: Beats;
  /** Source param (for remapping). */
  info: ParamInfo;
  targetKey: string;
  /**
   * A copied time RANGE (a lane's time selection), not points: `points` hold the range's
   * edge values at `dt` 0 and `span` (its length), and a paste writes over that length
   * (`range.ts`), keeping the curve around it.
   */
  range?: boolean;
}

let clipboard: PointClipboard | null = null;

export function hasPointClipboard(): boolean {
  return clipboard !== null && clipboard.points.length > 0;
}

export function pointClipboard(): PointClipboard | null {
  return clipboard;
}

/** Tests: forget the clipboard. */
export function clearPointClipboard(): void {
  clipboard = null;
}

/** Snapshot of `points` (any order). */
export function snapshotPoints(points: ReadonlyArray<AutomationPoint>, info: ParamInfo, targetKey: string): PointClipboard | null {
  if (points.length === 0) return null;
  const sorted = [...points].sort((a, b) => a.time - b.time);
  const t0 = sorted[0]!.time;
  return {
    points: sorted.map((p) => ({ dt: p.time - t0, value: p.value, curve: p.curve })),
    span: sorted[sorted.length - 1]!.time - t0,
    info,
    targetKey,
  };
}

/** Copy `points`. Returns how many were copied. */
export function copyPoints(points: ReadonlyArray<AutomationPoint>, info: ParamInfo, targetKey: string): number {
  const snap = snapshotPoints(points, info, targetKey);
  if (snap) clipboard = snap;
  return snap?.points.length ?? 0;
}

/** Snapshot of the time range `[start, end]` of a lane (its points sorted), or null for an empty lane. */
export function snapshotRange(points: ReadonlyArray<AutomationPoint>, start: Beats, end: Beats, info: ParamInfo, targetKey: string): PointClipboard | null {
  const piece = sliceRange(points, start, end);
  if (!piece) return null;
  return { points: piece.points.map((p) => ({ dt: p.time, value: p.value, curve: p.curve })), span: piece.length, info, targetKey, range: true };
}

/**
 * Copy the time range `[start, end]` of a lane (its points sorted). Returns `false` (and
 * keeps the clipboard) when the lane has no points.
 */
export function copyRange(points: ReadonlyArray<AutomationPoint>, start: Beats, end: Beats, info: ParamInfo, targetKey: string): boolean {
  const snap = snapshotRange(points, start, end, info, targetKey);
  if (snap) clipboard = snap;
  return snap !== null;
}

/**
 * One undo step writing `clip` (a range copy; `null` = nothing: a delete) over
 * `[at, at + length]` of `dest` (its points `points`, sorted), keeping the curve outside
 * (see `range.ts`). Values are remapped into another parameter like pasted points.
 */
export function rangeEditCommand(
  clip: PointClipboard | null,
  dest: PasteTarget,
  points: ReadonlyArray<AutomationPoint>,
  at: Beats,
  length: Beats,
  label: StepLabel,
): Command | null {
  const same = clip?.targetKey === dest.targetKey;
  const piece = clip ? clip.points.map((p) => ({ time: p.dt, value: p.value, curve: p.curve })) : [];
  const write = writeRange(points, Math.max(0, at), length, piece, { mapValue: clip ? (v) => remapValue(v, clip.info, dest.info, same) : undefined });
  return rangeWriteCommand(write, dest, label);
}

/** A normalized value of param `from` in param `to` (see the module doc). */
export function remapValue(value: number, from: ParamInfo, to: ParamInfo, sameTarget: boolean): number {
  if (sameTarget) return snapValue(to, value);
  if (from.unit === to.unit && from.unit !== "None") {
    const plain = paramToPlain(from, value);
    const lo = Math.min(to.min, to.max);
    const hi = Math.max(to.min, to.max);
    return snapValue(to, paramToNormalized(to, Math.min(hi, Math.max(lo, plain))));
  }
  return snapValue(to, value);
}

/** Where pasted points go. */
export interface PasteTarget {
  lane: AutomationLane | null;
  owner: AutomationOwner;
  target: AutomationTarget;
  info: ParamInfo;
  targetKey: string;
}

/**
 * One undo step pasting `clip` with its first point at `at` into `dest`: creates the lane
 * if needed, clears the points in the pasted span (`keepAt`: not the one exactly at `at`,
 * for duplicate), adds the copies. Returns the command and the new point ids.
 */
export function pasteCommand(
  clip: PointClipboard,
  dest: PasteTarget,
  at: Beats,
  opts: { keepAt?: boolean; label?: StepLabel; newLaneId?: string; ids?: () => AutomationPointId } = {},
): { command: Command; ids: AutomationPointId[]; laneId: string } {
  const makeId = opts.ids ?? newId;
  const start = Math.max(0, at);
  const same = clip.targetKey === dest.targetKey;
  const points: PointSpec[] = clip.points.map((p) => ({
    id: makeId(),
    time: start + p.dt,
    value: remapValue(p.value, clip.info, dest.info, same),
    curve: p.curve,
  }));
  const commands: Command[] = [];
  const laneId = dest.lane?.id ?? opts.newLaneId ?? newId();
  if (!dest.lane) commands.push(cmd("Automation", { type: "CreateLane", id: laneId, owner: dest.owner, target: dest.target }));
  else {
    // `[start, end)`: nudged to include the last pasted time (and exclude `at` for duplicate).
    const eps = 1e-6;
    commands.push(cmd("Automation", { type: "ClearRange", lane: laneId, start: opts.keepAt ? start + eps : start, end: start + clip.span + eps }));
  }
  commands.push(cmd("Automation", { type: "AddPoints", lane: laneId, points }));
  const label = opts.label ?? countedStep("undo.automation.paste_points", points.length, "paste automation point", "paste automation points");
  return { command: batchStep(label, commands), ids: points.map((p) => p.id), laneId };
}

/**
 * Where cmd-D puts a copy of the selection: right after it (one grid step for a lone point).
 * The copy's first point then shares its time with the selection's last one. That is a
 * deterministic step, not an ambiguity: points at the same time are ordered by id, in the
 * engine (`Project::points_of`) and the UI (`pointsOfLane`) alike, and new ids (ULIDs) sort
 * after existing ones, so the original holds up to that time and the copy continues it.
 */
export function duplicateAt(points: ReadonlyArray<AutomationPoint>, gridStep: Beats): Beats {
  const times = points.map((p) => p.time);
  const t0 = Math.min(...times);
  const t1 = Math.max(...times);
  return t1 + (t1 > t0 ? 0 : gridStep);
}

/**
 * Paste `clip` into `dest` at `at` (one undo step), select the pasted points and move the
 * selection cursor to their end (`advanceCursor`; the playhead stays).
 */
export async function pastePoints(
  transport: EngineTransport,
  clip: PointClipboard | null,
  dest: PasteTarget,
  at: number,
  selection: ItemSelectionStore = itemSelection,
  keepAt = false,
): Promise<AutomationPointId[]> {
  if (!clip || clip.points.length === 0) return [];
  if (clip.range) {
    // A range copy writes over its length, keeping the curve around it (no points selected:
    // the caller selects the pasted range).
    const project = useProjectStore.getState().project;
    const points = dest.lane && project ? pointsOfLane(project, dest.lane.id) : [];
    await sendEdit(
      transport,
      rangeEditCommand(
        clip,
        dest,
        points,
        at,
        clip.span,
        keepAt ? keyedStep("undo.automation.duplicate_range", "duplicate automation") : keyedStep("undo.automation.paste_range", "paste automation"),
      ),
    );
    if (!keepAt) advanceCursor(Math.max(0, at) + clip.span);
    return [];
  }
  const { command, ids } = pasteCommand(clip, dest, at, {
    keepAt,
    label: keepAt
      ? countedStep("undo.automation.duplicate_points", clip.points.length, "duplicate automation point", "duplicate automation points")
      : undefined,
  });
  await sendEdit(transport, command);
  const now = useProjectStore.getState().project;
  const created = ids.filter((id) => now?.automation_points[id]);
  if (created.length) selection.getState().select("automationPoint", created, "replace");
  // The selection cursor (not the playhead) moves past the paste, so pastes tile.
  if (!keepAt) advanceCursor(Math.max(0, at) + clip.span);
  return created;
}
