/**
 * A selection of clips AND track automation points: a box dragged across clip and automation
 * lanes selects both (`automation/laneMarquee.ts`, the arranger's `useMarquee` `also`).
 * Delete, cut, copy, paste and duplicate then act on both, as one undo step, whichever lane
 * has the focus (the arrangement's keys and the automation lanes' keys both come here).
 *
 * Points stay in their own lanes (same track, same parameter): a paste or a duplicate puts
 * each back into its lane, in step with the clips. Lanes that aren't shown are skipped.
 */

import type { AutomationPoint, Beats, Command } from "@/generated";
import { itemSelection } from "@/timeline";
import { cmd, newId, type EngineTransport } from "@/transport";
import { copyLanes, multiLaneClipboard, pasteLanesCommand, snapshotLanes } from "@/features/automation/clipboard";
import { flattenCommands, laneOf, selectedByLane, type RegisteredLane } from "@/features/automation/laneRegistry";
import { useProjectStore } from "@/state";
import { endOf, startOf } from "./clipTime";
import { clipboardPoints, copyClips } from "./clipboard";
import { selectedClips, sendEdit } from "./context";
import { batchStep, boundsOf, deleteCommand, keyedStep, moveCommand } from "./editMath";
import { arrangementView, useArrangementUi } from "./uiStore";
import { editPointsCommand } from "@/features/automation/edit";

/** The selected points of each track lane shown in the arrangement (lanes holding some). */
export function selectedLaneGroups(): Array<{ lane: RegisteredLane; points: AutomationPoint[] }> {
  return selectedByLane(arrangementView, itemSelection.getState().selected.automationPoint);
}

/** Clips and track automation points are both selected. */
export function isMixedSelection(): boolean {
  return selectedClips().length > 0 && selectedLaneGroups().length > 0;
}

/** The last copy holds clips and points (a paste puts both back). */
export function hasMixedClipboard(): boolean {
  return clipboardPoints() !== null;
}

const laneDest = (track: string, key: string) => laneOf(arrangementView, track, key)?.pasteTarget() ?? null;

const snapshot = (groups: ReturnType<typeof selectedLaneGroups>) =>
  snapshotLanes(groups.map((g) => ({ track: g.lane.track, key: g.lane.key, info: g.lane.pasteTarget().info, points: g.points })));

/** One undo step deleting the selected clips and track automation points (null: none). */
export function deleteMixedCommand(): Command | null {
  const ids = selectedLaneGroups().flatMap((g) => g.points.map((p) => p.id));
  const commands = [deleteCommand(selectedClips().map((c) => c.id)), ids.length ? cmd("Automation", { type: "RemovePoints", ids }) : null].filter(
    (c): c is Command => c !== null,
  );
  if (commands.length < 2) return commands[0] ?? null;
  return batchStep(keyedStep("undo.arrangement.delete_selection", "delete selection"), commands);
}

/** Copy the selected clips with the selected track automation points. Returns the clips copied. */
export function copyMixed(): number {
  const groups = selectedLaneGroups();
  // The automation side first: the clip copy keeps a reference to it (`clipboardPoints`).
  const copied = copyLanes(groups.map((g) => ({ track: g.lane.track, key: g.lane.key, info: g.lane.pasteTarget().info, points: g.points })));
  const points = copied > 0 ? multiLaneClipboard() : null;
  if (!points) return copyClips();
  return copyClips({ clip: points, start: Math.min(...groups.flatMap((g) => g.points.map((p) => p.time))) });
}

/** Copy, then delete both (the delete is its own undo step, like a clip cut). */
export async function cutMixed(transport: EngineTransport): Promise<void> {
  const command = deleteMixedCommand();
  if (copyMixed() === 0) return;
  await sendEdit(transport, command);
}

/**
 * Duplicate the selection right after itself (by its whole span: the earliest clip start or
 * point to the latest clip end or point), as one undo step; the copies become the selection.
 * The clipboard stays as it was.
 */
export async function duplicateMixed(transport: EngineTransport): Promise<void> {
  const clips = selectedClips();
  const groups = selectedLaneGroups();
  const times = groups.flatMap((g) => g.points.map((p) => p.time));
  if (clips.length === 0 || times.length === 0) return;
  const t0 = Math.min(...clips.map(startOf), ...times);
  const span = Math.max(...clips.map(endOf), ...times) - t0;
  const clipIds = clips.map(() => newId());
  const commands: Command[] = clips.map((c, i) => cmd("Clip", { type: "Duplicate", id: c.id, new_id: clipIds[i]!, start: startOf(c) + span }));
  const points = snapshot(groups);
  const lanes = points ? pasteLanesCommand(points, laneDest, Math.min(...times) + span, { keepAt: true }) : { command: null, ids: [] };
  commands.push(...flattenCommands([lanes.command]));
  await sendEdit(transport, batchStep(keyedStep("undo.arrangement.duplicate_selection", "duplicate selection"), commands));
  const now = useProjectStore.getState().project;
  itemSelection.getState().selectKinds({
    clip: clipIds.filter((id) => now?.clips[id]),
    automationPoint: lanes.ids.filter((id) => now?.automation_points[id]),
  });
}

// ---- Moving both: a drag of either kind takes the other along (time only) ---------------

/** One undo step label for a drag that moved clips and points together. */
export const MOVE_SELECTION = keyedStep("undo.arrangement.move_selection", "move selection");

/**
 * The selected clips, following a drag of the selected points by `dt` beats (their time
 * only: they stay on their tracks). Null without selected clips. `preview` shows them moved
 * (the arrangement's drag preview), `command` moves them, `done` drops the preview.
 */
export interface ClipsFollowing {
  /** The earliest clip start (a drag keeps everything at time >= 0). */
  minStart: Beats;
  preview(dt: Beats): void;
  command(dt: Beats): Command | null;
  done(): void;
}

export function clipsFollowing(): ClipsFollowing | null {
  const clips = selectedClips();
  if (clips.length === 0) return null;
  const bounds = (dt: Beats) => new Map(clips.map((c) => [c.id, { ...boundsOf(c), start: startOf(c) + dt }]));
  return {
    minStart: Math.min(...clips.map(startOf)),
    preview: (dt) => useArrangementUi.getState().setPreview(dt === 0 ? null : { bounds: bounds(dt), copy: false }),
    command: (dt) => (dt === 0 ? null : moveCommand(clips, bounds(dt), false, newId)),
    done: () => useArrangementUi.getState().setPreview(null),
  };
}

/**
 * The selected track automation points, following a drag of the selected clips by `dt`
 * beats (their time only; each lane keeps its own rules: grid-free, values unchanged).
 * Null without selected points.
 */
export interface PointsFollowing {
  /** The earliest point's time (a drag keeps everything at time >= 0). */
  minTime: Beats;
  command(dt: Beats): Command | null;
}

export function pointsFollowing(): PointsFollowing | null {
  const groups = selectedLaneGroups();
  if (groups.length === 0) return null;
  const mods = { increments: false, lockTime: false, lockValue: true };
  return {
    minTime: Math.min(...groups.flatMap((g) => g.points.map((p) => p.time))),
    command: (dt) => editPointsCommand(groups.flatMap((g) => g.lane.dragEdits(g.points, g.points[0]!, dt, 0, mods))),
  };
}
