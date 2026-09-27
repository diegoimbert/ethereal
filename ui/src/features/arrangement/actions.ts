/** Edit actions on the selected clips, shared by the toolbar and keyboard shortcuts. */

import type { EngineTransport } from "@/transport";
import { cmd, newId } from "@/transport";
import type { Beats, Clip, Command, Track, TrackId } from "@/generated";
import { MOD_KEY, type ContextMenuEntry } from "@/kit";
import { useEditorStore, useProjectStore, useSelectionStore } from "@/state";
import { itemSelection, playheadBeats } from "@/timeline";
import { copyClips, cutClips, hasClipboard, pasteClips } from "./clipboard";
import { isArrangementClip } from "./clipTime";
import { selectedClips, sendEdit } from "./context";
import { useArrangementUi } from "./uiStore";
import { asOneStep, deleteCommand, duplicateCommand, splitCommand, toggleLoopCommand } from "./editMath";

export type NewTrackKind = "Midi" | "Audio";

/**
 * Commands adding a track at the end of the list; a MIDI track comes with the built-in
 * synth, so it plays as soon as it has notes. One undo step.
 */
export function addTrackCommand(kind: NewTrackKind, id: TrackId, deviceId: string): Command {
  const commands: Command[] = [
    cmd("Track", { type: "Create", id, kind, name: null, color: null, parent: null, before: null }),
  ];
  if (kind === "Midi") {
    commands.push(
      cmd("Device", { type: "Insert", id: deviceId, track: id, device: { type: "Builtin", device: { type: "Synth" } }, before: null }),
    );
  }
  return asOneStep(kind === "Midi" ? "Add MIDI Track" : "Add Audio Track", commands)!;
}

/**
 * Move the playhead to `beats`, unless the transport is playing (clicking a clip or empty
 * space while stopped sets where playback will start). Reads the controller's transport
 * state, which knows about Play/Stop as soon as they are sent.
 */
export function locateIfStopped(transport: EngineTransport, beats: Beats): void {
  if (useProjectStore.getState().transport?.playing) return;
  transport
    .send(cmd("Transport", { type: "Locate", position: Math.max(0, beats) }))
    .catch((err: unknown) => console.warn("locate failed", err));
}

/**
 * Select `track` as the arrangement's entity (header click): it becomes the selected
 * track and the clip and automation point selections are cleared.
 */
export function selectTrackEntity(track: TrackId): void {
  itemSelection.getState().clear("clip");
  itemSelection.getState().clear("automationPoint");
  useSelectionStore.getState().selectTrack(track);
  useArrangementUi.getState().setTrackFocus(track);
}

/**
 * Keep a single kind of selected entity in the arrangement: selecting clips clears the
 * automation points and the track focus, and the other way around. Returns the unsubscribe.
 */
export function bindSingleSelection(): () => void {
  return itemSelection.subscribe((s, prev) => {
    const clips = s.selected.clip !== prev.selected.clip && s.selected.clip.size > 0;
    const points = s.selected.automationPoint !== prev.selected.automationPoint && s.selected.automationPoint.size > 0;
    if (!clips && !points) return;
    useArrangementUi.getState().setTrackFocus(null);
    itemSelection.getState().clear(clips ? "automationPoint" : "clip");
  });
}

/** Delete the focused track (Delete key with a track selected), if it can be deleted. */
export function deleteFocusedTrack(transport: EngineTransport): Promise<void> {
  const ui = useArrangementUi.getState();
  const track = ui.trackFocus ? useProjectStore.getState().project?.tracks[ui.trackFocus] : undefined;
  ui.setTrackFocus(null);
  if (!track || track.kind === "Master") return Promise.resolve();
  return sendEdit(transport, cmd("Track", { type: "Delete", id: track.id }));
}

/** Add a track (see `addTrackCommand`) and select it. */
export async function addTrack(transport: EngineTransport, kind: NewTrackKind): Promise<TrackId> {
  const id = newId();
  await sendEdit(transport, addTrackCommand(kind, id, newId()));
  if (useProjectStore.getState().project?.tracks[id]) useSelectionStore.getState().selectTrack(id);
  return id;
}

export type ClipAction = "split" | "duplicate" | "delete" | "loop" | "select-all" | "deselect" | "copy" | "cut" | "paste";

export function runClipAction(transport: EngineTransport, action: ClipAction): Promise<void> {
  const clips = selectedClips();
  switch (action) {
    case "split":
      return sendEdit(transport, splitCommand(clips, playheadBeats(), newId));
    case "duplicate": {
      // Select the copies, so repeated duplicates keep extending the pattern.
      const ids: string[] = [];
      const command = duplicateCommand(clips, () => {
        const id = newId();
        ids.push(id);
        return id;
      });
      return sendEdit(transport, command).then(() => {
        const project = useProjectStore.getState().project;
        const created = ids.filter((id) => project?.clips[id]);
        if (created.length) itemSelection.getState().select("clip", created, "replace");
      });
    }
    case "delete":
      // The selected entity: a track (header clicked) or the selected clips.
      if (useArrangementUi.getState().trackFocus) return deleteFocusedTrack(transport);
      return sendEdit(transport, deleteCommand(clips.map((c) => c.id)));
    case "loop":
      return sendEdit(transport, toggleLoopCommand(clips));
    case "select-all": {
      const project = useProjectStore.getState().project;
      const all = project ? Object.values(project.clips).filter(isArrangementClip).map((c) => c.id) : [];
      itemSelection.getState().select("clip", all, "replace");
      return Promise.resolve();
    }
    case "copy":
      copyClips();
      return Promise.resolve();
    case "cut":
      return cutClips(transport);
    case "paste":
      // At the playhead; onto the selected track when the clips all come from one track.
      return pasteClips(transport, playheadBeats(), useSelectionStore.getState().selectedTrack).then(() => {});
    case "deselect":
      itemSelection.getState().clear("clip");
      useArrangementUi.getState().setTrackFocus(null);
      return Promise.resolve();
  }
}

/** The action for a key press in the arrangement, or null. */
export function actionForKey(e: { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): ClipAction | null {
  const mod = e.metaKey || e.ctrlKey;
  const k = e.key.toLowerCase();
  if (!mod && !e.altKey && (e.key === "Delete" || e.key === "Backspace")) return "delete";
  if (!mod && e.key === "Escape") return "deselect";
  if (mod && !e.shiftKey && !e.altKey) {
    if (k === "e") return "split";
    if (k === "d") return "duplicate";
    if (k === "a") return "select-all";
    if (k === "c") return "copy";
    if (k === "x") return "cut";
    if (k === "v") return "paste";
  }
  if (mod && e.shiftKey && k === "l") return "loop";
  return null;
}

/**
 * Right-click menu of a clip. Right-clicking an unselected clip selects just it (and its
 * track) first, so the actions apply to what is highlighted.
 */
export function clipMenu(transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  if (!itemSelection.getState().isSelected("clip", clip.id)) itemSelection.getState().select("clip", [clip.id], "replace");
  useSelectionStore.getState().selectTrack(clip.track);
  const clips = selectedClips();
  const run = (a: ClipAction) => () => void runClipAction(transport, a);
  const allMuted = clips.length > 0 && clips.every((c) => c.muted);
  const allLooping = clips.length > 0 && clips.every((c) => c.looping.enabled);
  return [
    ...(clip.content.type === "Midi" && clips.length === 1
      ? [{ label: "Open in Piano Roll", onSelect: () => useEditorStore.getState().openClip(clip.id) }, "separator" as const]
      : []),
    { label: "Cut", shortcut: `${MOD_KEY}X`, onSelect: run("cut") },
    { label: "Copy", shortcut: `${MOD_KEY}C`, onSelect: run("copy") },
    { label: "Paste at Playhead", shortcut: `${MOD_KEY}V`, disabled: !hasClipboard(), onSelect: run("paste") },
    "separator",
    { label: "Split at Playhead", shortcut: `${MOD_KEY}E`, onSelect: run("split") },
    { label: "Duplicate", shortcut: `${MOD_KEY}D`, onSelect: run("duplicate") },
    { label: allLooping ? "Disable Loop" : "Enable Loop", shortcut: `⇧${MOD_KEY}L`, onSelect: run("loop") },
    {
      label: allMuted ? "Unmute" : "Mute",
      onSelect: () => void sendEdit(transport, cmd("Clip", { type: "SetMuted", ids: clips.map((c) => c.id), muted: !allMuted })),
    },
    "separator",
    { label: clips.length > 1 ? `Delete ${clips.length} Clips` : "Delete", shortcut: "⌫", danger: true, onSelect: run("delete") },
  ];
}

/** Right-click menu of a track header (selects the track). */
export function trackMenu(transport: EngineTransport, track: Track): ContextMenuEntry[] {
  selectTrackEntity(track.id);
  if (track.kind === "Master") return [];
  return [
    {
      label: "Duplicate Track",
      onSelect: () => {
        const id = newId();
        void sendEdit(transport, cmd("Track", { type: "Duplicate", id: track.id, new_id: id })).then(() => {
          if (useProjectStore.getState().project?.tracks[id]) useSelectionStore.getState().selectTrack(id);
        });
      },
    },
    "separator",
    {
      label: "Delete Track",
      shortcut: "⌫",
      danger: true,
      onSelect: () => {
        useArrangementUi.getState().setTrackFocus(null);
        void sendEdit(transport, cmd("Track", { type: "Delete", id: track.id }));
      },
    },
  ];
}
