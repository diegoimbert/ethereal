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
import { paramToNormalized, paramToPlain } from "@/features/devices/paramScale";
import { useProjectStore } from "@/state";
import { itemSelection, type ItemSelectionStore } from "@/timeline";
import { cmd, newId, type EngineTransport } from "@/transport";
import { sendEdit } from "./gesture";
import { snapValue } from "./valueAxis";

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
  opts: { keepAt?: boolean; label?: string; newLaneId?: string; ids?: () => AutomationPointId } = {},
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
  const label = opts.label ?? (points.length > 1 ? "Paste Automation Points" : "Paste Automation Point");
  return { command: cmd("Edit", { type: "Batch", label, commands }), ids: points.map((p) => p.id), laneId };
}

/** Where cmd-D puts a copy of the selection: right after it (one grid step for a lone point). */
export function duplicateAt(points: ReadonlyArray<AutomationPoint>, gridStep: Beats): Beats {
  const times = points.map((p) => p.time);
  const t0 = Math.min(...times);
  const t1 = Math.max(...times);
  return t1 + (t1 > t0 ? 0 : gridStep);
}

/**
 * Paste `clip` into `dest` at `at` (one undo step), select the pasted points and, when
 * stopped, move the playhead to their end (see clipboard.ts).
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
  const { command, ids } = pasteCommand(clip, dest, at, { keepAt, label: keepAt ? "Duplicate Automation Points" : undefined });
  await sendEdit(transport, command);
  const now = useProjectStore.getState().project;
  const created = ids.filter((id) => now?.automation_points[id]);
  if (created.length) selection.getState().select("automationPoint", created, "replace");
  if (!keepAt && !useProjectStore.getState().transport?.playing) {
    transport.send(cmd("Transport", { type: "Locate", position: Math.max(0, at) + clip.span })).catch(() => {});
  }
  return created;
}
