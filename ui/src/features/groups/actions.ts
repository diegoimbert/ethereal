/**
 * Grouping, VCA and routing actions (v0.2, `groups-buses`), shared by the arrangement
 * (Cmd+G, context menu) and the mixer. Callers pass the selection; nothing here reads a
 * view's store.
 */

import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { create } from "zustand";
import type { Command, Track, TrackId } from "@/generated";
import { MOD_KEY, type ContextMenuEntry } from "@/kit";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, newId, nextGestureId, type EngineTransport } from "@/transport";
import { assignedTo, groupableSelection, groupCommand, ungroupLosses, vcaTargets } from "./model";

/** Send one edit as one undo gesture; failures are logged (the mirror is unchanged). */
export async function sendGroupsEdit(transport: EngineTransport, command: Command | null): Promise<boolean> {
  if (!command) return false;
  const gesture = nextGestureId();
  try {
    await transport.send(command, { gesture });
    return true;
  } catch (err) {
    console.warn("[groups] edit failed", err);
    return false;
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
}

const project = () => useProjectStore.getState().project;

/**
 * Cmd+G: group the selected tracks into a new group track (one undo step). Returns the new
 * group's id (selected), or `null` if nothing was groupable.
 */
export async function groupTracks(transport: EngineTransport, selected: Iterable<TrackId>): Promise<TrackId | null> {
  const p = project();
  if (!p) return null;
  const group = newId();
  const ok = await sendGroupsEdit(transport, groupCommand(p.tracks, selected, group));
  if (!ok || !project()?.tracks[group]) return null;
  useSelectionStore.getState().selectTrack(group);
  return group;
}

/** A pending ungroup that would lose the group's devices, lanes, sends or routings. */
export interface UngroupConfirmation {
  group: TrackId;
  name: string;
  losses: string[];
}

interface UngroupConfirmState {
  pending: UngroupConfirmation | null;
  transport: EngineTransport | null;
  ask(transport: EngineTransport, c: UngroupConfirmation): void;
  close(): void;
}

/** The "Ungroup anyway?" confirmation (rendered by `UngroupConfirmDialog`). */
export const useUngroupConfirm = create<UngroupConfirmState>((set) => ({
  pending: null,
  transport: null,
  ask: (transport, pending) => set({ transport, pending }),
  close: () => set({ pending: null, transport: null }),
}));

/**
 * Cmd+Shift+G: ungroup `group` (children move to its parent, in place). If the group has
 * devices, automation, sends or routings to it, asks first (they are lost).
 */
export async function ungroupTrack(transport: EngineTransport, group: TrackId, force = false): Promise<boolean> {
  const p = project();
  const g = p?.tracks[group];
  if (!p || !g || g.kind !== "Group") return false;
  const losses = ungroupLosses(p, group);
  if (losses.length > 0 && !force) {
    useUngroupConfirm.getState().ask(transport, { group, name: g.name, losses });
    return false;
  }
  return sendGroupsEdit(transport, cmd("Track", { type: "Ungroup", group, force: losses.length > 0 }));
}

/** Ungroup every selected group (topmost first; one undo step each). */
export async function ungroupSelected(transport: EngineTransport, selected: Iterable<TrackId>): Promise<void> {
  const p = project();
  if (!p) return;
  for (const t of groupableSelection(p.tracks, selected)) {
    if (t.kind === "Group") await ungroupTrack(transport, t.id);
  }
}

/** Assign `ids` to `vca` (`null` = unassign), one undo step. */
export function assignVca(transport: EngineTransport, ids: TrackId[], vca: TrackId | null): Promise<boolean> {
  const commands = ids.map((id) => cmd("Track", { type: "SetVca", id, vca }));
  const command =
    commands.length === 1 ? commands[0]! : cmd("Edit", { type: "Batch", label: vca ? "Assign to VCA" : "Remove from VCA", commands });
  return sendGroupsEdit(transport, commands.length ? command : null);
}

/** Create a VCA (named after the first track) and assign `ids` to it, one undo step. */
export async function newVcaFor(transport: EngineTransport, ids: TrackId[]): Promise<TrackId | null> {
  const p = project();
  if (!p || ids.length === 0) return null;
  const vca = newId();
  const first = p.tracks[ids[0]!];
  const commands: Command[] = [
    cmd("Track", { type: "Create", id: vca, kind: "Vca", name: null, color: first?.color ?? null, parent: null, before: null }),
    ...ids.map((id) => cmd("Track", { type: "SetVca", id, vca })),
  ];
  const ok = await sendGroupsEdit(transport, cmd("Edit", { type: "Batch", label: "New VCA", commands }));
  return ok ? vca : null;
}

/** Tracks a VCA can control among `ids` (not master, not the VCA itself). */
function vcaAssignable(tracks: Record<TrackId, Track>, ids: Iterable<TrackId>): TrackId[] {
  return [...ids].filter((id) => {
    const t = tracks[id];
    return t !== undefined && t.kind !== "Master";
  });
}

/** Keyboard shortcut of the groups actions (Cmd/Ctrl+G, Cmd/Ctrl+Shift+G), if any. */
export function groupShortcut(e: Pick<KeyboardEvent | ReactKeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">): "group" | "ungroup" | null {
  if (!(e.metaKey || e.ctrlKey) || e.altKey || e.key.toLowerCase() !== "g") return null;
  return e.shiftKey ? "ungroup" : "group";
}

/**
 * Track context-menu entries (appended after the arrangement's own): group/ungroup, VCA
 * assignment and "New VCA". `selected` = the tracks the menu applies to (the clicked
 * track is in it).
 */
export function groupsTrackMenu(transport: EngineTransport, track: Track, selected: ReadonlySet<TrackId>): ContextMenuEntry[] {
  const p = project();
  if (!p || track.kind === "Master") return [];
  const ids = selected.has(track.id) ? [...selected] : [track.id];
  const entries: ContextMenuEntry[] = ["separator"];
  const groupable = groupableSelection(p.tracks, ids);
  if (groupable.length > 0) {
    entries.push({
      label: groupable.length > 1 ? `Group ${groupable.length} Tracks` : "Group Track",
      shortcut: `${MOD_KEY}G`,
      onSelect: () => void groupTracks(transport, ids),
    });
  }
  if (track.kind === "Group") {
    entries.push({ label: "Ungroup", shortcut: `⇧${MOD_KEY}G`, onSelect: () => void ungroupTrack(transport, track.id) });
  }
  if (track.kind === "Vca") {
    const n = assignedTo(p.tracks, track.id).length;
    entries.push({ label: n === 1 ? "Controls 1 track" : `Controls ${n} tracks`, disabled: true, onSelect: () => {} });
  }
  const assignable = vcaAssignable(p.tracks, ids);
  if (assignable.length > 0) {
    for (const v of vcaTargets(p.tracks, track)) {
      if (v.id === track.vca) continue;
      entries.push({ label: `Assign to VCA “${v.name}”`, onSelect: () => void assignVca(transport, assignable, v.id) });
    }
    if (assignable.some((id) => p.tracks[id]?.vca)) {
      entries.push({ label: "Remove from VCA", onSelect: () => void assignVca(transport, assignable, null) });
    }
    if (track.kind !== "Vca") {
      entries.push({
        label: assignable.length > 1 ? `New VCA for ${assignable.length} Tracks` : "New VCA for Track",
        onSelect: () => void newVcaFor(transport, assignable),
      });
    }
  }
  return entries.length > 1 ? entries : [];
}
