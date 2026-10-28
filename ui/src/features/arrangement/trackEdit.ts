/**
 * Edit actions on the selected tracks (Ableton semantics): duplicate, copy, cut,
 * paste and delete, for the keyboard (⌘D / ⌘C / ⌘X / ⌘V / ⌫) and the track header's
 * right-click menu. Each edit is one undo step.
 *
 * - **Which tracks**: the arrangement's track selection (`selectedTracks`), top-most only
 *   (a track inside a selected group goes with the group), in display order, never the
 *   master.
 * - **Duplicate**: `Track::Duplicate` per track (devices, clips, automation, sends,
 *   routing; a group with its children), then `Track::Move` so the copies sit right after
 *   the last selected track, in the same order, in one `Edit::Batch`. The copies become
 *   the selection.
 * - **Copy** remembers the tracks (the clipboard slot is in `clipboard.ts`: the most
 *   recent copy of clips, time or tracks wins ⌘V). **Paste** duplicates them after the
 *   last selected track (else at the end), like Duplicate.
 * - **Cut** = copy, then delete. The deleted tracks can't be duplicated, so pasting
 *   restores them: while the cut is still the latest undo step, paste undoes it and moves
 *   the tracks to the paste position (net effect: one "Move" step, everything kept,
 *   plugin states included). After another edit, the cut can no longer be pasted.
 *
 * Copies of kinds a group can't hold (return, VCA) stay right after their source.
 */

import type { Command, Project, Track, TrackId } from "@/generated";
import { childTracks, tracksOrdered, useProjectStore, useSelectionStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";
import { getTrackClipboard, setTrackClipboard } from "./clipboard";
import { sendEdit } from "./context";
import { asCountedStep, countedStep, type StepLabel } from "./editMath";
import { useArrangementUi } from "./uiStore";

/** `ids` reduced to the top-most tracks (not inside another of them), in display order, no master. */
export function topMostTracks(project: Project, ids: Iterable<TrackId>): TrackId[] {
  const set = new Set(ids);
  const inside = (id: TrackId) => {
    let p = project.tracks[id]?.parent ?? null;
    for (let guard = 0; p !== null && guard < 64; guard++) {
      if (set.has(p)) return true;
      p = project.tracks[p]?.parent ?? null;
    }
    return false;
  };
  return tracksOrdered(project)
    .filter((t) => set.has(t.id) && t.kind !== "Master" && !inside(t.id))
    .map((t) => t.id);
}

/** The selected tracks the track edits act on (see the module doc). */
export function selectedTrackIds(): TrackId[] {
  const project = useProjectStore.getState().project;
  return project ? topMostTracks(project, useArrangementUi.getState().selectedTracks) : [];
}

/** Where copies go: under `parent`, before `before` (`null` = the end, like `Track::Create`). */
interface Placement {
  parent: TrackId | null;
  before: TrackId | null;
}

/**
 * Right after `anchor` (at its level), or at the end of the top level without one.
 * `moving`: tracks about to be moved there, skipped as the `before` reference.
 */
export function placementAfter(project: Project, anchor: TrackId | null, moving: ReadonlySet<TrackId> = new Set()): Placement {
  const a = anchor ? project.tracks[anchor] : undefined;
  if (!a || a.kind === "Master") return { parent: null, before: null };
  const sibs = childTracks(project, a.parent).filter((t) => t.id === a.id || !moving.has(t.id));
  const next = sibs[sibs.findIndex((t) => t.id === a.id) + 1];
  return { parent: a.parent, before: next?.id ?? null };
}

/** Whether a track of `kind` can be moved under `parent` (groups hold audio, MIDI and groups). */
function fits(track: Track, parent: TrackId | null): boolean {
  if (parent === null) return true;
  return track.kind === "Audio" || track.kind === "Midi" || track.kind === "Group";
}

/** Moves putting `ids` (existing, or created by the same batch) at `at`, in order. */
function moveCommands(project: Project, sources: TrackId[], ids: TrackId[], at: Placement): Command[] {
  const out: Command[] = [];
  sources.forEach((src, i) => {
    const t = project.tracks[src];
    if (t && fits(t, at.parent)) out.push(cmd("Track", { type: "Move", id: ids[i]!, parent: at.parent, before: at.before }));
  });
  return out;
}

/**
 * The batch duplicating `sources` (top-most, display order) to `at`, and the copies' ids.
 * `null` when there is nothing to duplicate.
 */
export function duplicateTracksCommand(
  project: Project,
  sources: TrackId[],
  at: Placement,
  label: StepLabel,
  makeId: () => TrackId = newId,
  snapshot: Project | null = null,
): { command: Command; copies: TrackId[] } | null {
  const live = sources.filter((id) => project.tracks[id] && project.tracks[id]!.kind !== "Master");
  if (live.length === 0) return null;
  const copies = live.map(() => makeId());
  const commands: Command[] = [...live.map((id, i) => cmd("Track", { type: "Duplicate", id, new_id: copies[i]! })), ...moveCommands(project, live, copies, at)];
  // A paste: each copy gets its source's name, color, volume and pan as they were at the copy.
  live.forEach((id, i) => {
    const then = snapshot?.tracks[id];
    const now = project.tracks[id]!;
    const copy = copies[i]!;
    if (!then || then === now) return;
    if (then.name !== now.name) commands.push(cmd("Track", { type: "Rename", id: copy, name: then.name }));
    if (then.color !== now.color) commands.push(cmd("Track", { type: "SetColor", id: copy, color: then.color }));
    if (then.mixer.volume !== now.mixer.volume) commands.push(cmd("Mixer", { type: "SetVolume", track: copy, volume: then.mixer.volume }));
    if (then.mixer.pan !== now.mixer.pan) commands.push(cmd("Mixer", { type: "SetPan", track: copy, pan: then.mixer.pan }));
  });
  return { command: cmd("Edit", { type: "Batch", label: label.text, key: label.key, commands }), copies };
}

/** Select `ids` as the arrangement's tracks (the first one focused), those that exist. */
function selectTracks(ids: TrackId[]): void {
  const project = useProjectStore.getState().project;
  const live = ids.filter((id) => project?.tracks[id]);
  if (live.length === 0) return;
  useArrangementUi.getState().setTrackSelection(live, live[0]!);
  useSelectionStore.getState().selectTrack(live[0]!);
}

/** Duplicate the selected tracks right after the last one (one undo step); the copies get selected. */
export async function duplicateSelectedTracks(transport: EngineTransport): Promise<TrackId[]> {
  const project = useProjectStore.getState().project;
  const ids = selectedTrackIds();
  if (!project || ids.length === 0) return [];
  const edit = duplicateTracksCommand(
    project,
    ids,
    placementAfter(project, ids.at(-1)!),
    countedStep("undo.tracks.duplicate", ids.length, "duplicate track", "duplicate tracks"),
  );
  if (!edit) return [];
  await sendEdit(transport, edit.command);
  selectTracks(edit.copies);
  return edit.copies;
}

/** Copy the selected tracks to the track clipboard. Returns how many were copied. */
export function copySelectedTracks(): number {
  const ids = selectedTrackIds();
  if (ids.length) setTrackClipboard({ tracks: ids, cutUndoId: null, snapshot: useProjectStore.getState().project });
  return ids.length;
}

/**
 * Delete the selected tracks (Delete key, menu, cut) as one undo step. The master track
 * stays; a track inside a selected group goes with the group.
 */
export function deleteSelectedTracks(transport: EngineTransport, label?: StepLabel): Promise<void> {
  const ids = selectedTrackIds();
  useArrangementUi.getState().setTrackFocus(null);
  const commands = ids.map((id) => cmd("Track", { type: "Delete", id }));
  return sendEdit(transport, asCountedStep(label ?? countedStep("undo.tracks.delete", ids.length, "delete track", "delete tracks"), commands));
}

/** Copy the selected tracks, then delete them (one undo step). */
export async function cutSelectedTracks(transport: EngineTransport): Promise<void> {
  const ids = selectedTrackIds();
  if (ids.length === 0) return;
  await deleteSelectedTracks(transport, countedStep("undo.tracks.cut", ids.length, "cut track", "cut tracks"));
  const project = useProjectStore.getState().project;
  const gone = !!project && ids.every((id) => !project.tracks[id]);
  // The cut's undo step, so paste can restore exactly these tracks (see the module doc).
  const undoId = gone ? useProjectStore.getState().history.undo_id : null;
  setTrackClipboard({ tracks: ids, cutUndoId: gone ? undoId : null });
}

/** Whether the track clipboard can be pasted now. */
export function canPasteTracks(): boolean {
  const cb = getTrackClipboard();
  const project = useProjectStore.getState().project;
  if (!cb || !project) return false;
  if (cb.tracks.some((id) => project.tracks[id])) return true;
  return cb.cutUndoId !== null && useProjectStore.getState().history.undo_id === cb.cutUndoId;
}

/**
 * Paste the track clipboard after the last selected track (else at the end), one undo
 * step; the pasted tracks get selected. Returns them.
 */
export async function pasteTracks(transport: EngineTransport): Promise<TrackId[]> {
  const cb = getTrackClipboard();
  if (!cb || !canPasteTracks()) return [];
  const before = useProjectStore.getState().project!;
  const sel = selectedTrackIds();
  const at = placementAfter(before, sel.at(-1) ?? null);
  if (cb.tracks.some((id) => before.tracks[id])) {
    const edit = duplicateTracksCommand(
      before,
      cb.tracks,
      at,
      countedStep("undo.tracks.paste", cb.tracks.length, "paste track", "paste tracks"),
      newId,
      cb.snapshot ?? null,
    );
    if (!edit) return [];
    await sendEdit(transport, edit.command);
    selectTracks(edit.copies);
    return edit.copies;
  }
  // After a cut: undo it (the tracks come back with everything), then move them.
  try {
    await transport.send(cmd("Edit", { type: "Undo" }));
  } catch (err) {
    console.warn("track paste failed", err);
    return [];
  }
  setTrackClipboard({ tracks: cb.tracks, cutUndoId: null, snapshot: useProjectStore.getState().project });
  const project = useProjectStore.getState().project;
  if (!project) return [];
  // The paste position was computed before the restore; recompute it on the restored tracks.
  const place = sel.length ? placementAfter(project, sel.at(-1)!, new Set(cb.tracks)) : at;
  const moves = moveCommands(project, cb.tracks, cb.tracks, place);
  await sendEdit(transport, asCountedStep(countedStep("undo.tracks.paste", cb.tracks.length, "paste track", "paste tracks"), moves));
  selectTracks(cb.tracks);
  return cb.tracks.filter((id) => useProjectStore.getState().project?.tracks[id]);
}
