/**
 * Time-selection actions (keyboard shortcuts and the selection's context menu), Ableton-like.
 * Default keys below; the user's keymap decides (`time.*`, `edit.split`; keymap node):
 *
 * | action          | key     | what                                                     |
 * |-----------------|---------|----------------------------------------------------------|
 * | split           | ⌘E      | split every selected track at the selection edges        |
 * | split-tracks    | ⌘E      | no time selection, tracks selected: split them at the playhead |
 * | cut             | ⇧⌘X     | copy the selection, then delete its time                 |
 * | copy            | ⇧⌘C     | copy the selection to the (engine) time clipboard        |
 * | paste           | menu    | paste over the range at the selection start / playhead   |
 * | paste-insert    | ⇧⌘V     | paste, shifting what follows right                       |
 * | duplicate       | ⇧⌘D     | insert a copy of the selection right after it            |
 * | delete          | ⇧⌘⌫     | remove the selected time (later material moves left)     |
 * | insert-silence  | ⇧⌘I     | insert the selection's length of silence at its start    |
 *
 * Each is one undo step (split at both edges: two commands under one gesture). Refused
 * edits (a frozen track, ...) show their engine message as a notice.
 */

import type { Command, TrackId } from "@/generated";
import { firstMatch, shortcutLabel, type ChordEvent } from "@/features/keymap";
import type { ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { itemSelection, playheadBeats } from "@/timeline";
import { cmd, newId, nextGestureId, type EngineTransport } from "@/transport";
import { expandTracks, pasteTracks, timeSelection } from "./commands";
import { useTimeSelection, type TimeRangeSelection } from "./store";

export type TimeAction = "split" | "split-tracks" | "cut" | "copy" | "paste" | "paste-insert" | "duplicate" | "delete" | "insert-silence";

function errorText(err: unknown): string {
  let message = String(err);
  if (err && typeof err === "object" && "message" in err && typeof err.message === "string") message = err.message;
  else if (err && typeof err === "object" && "error" in err) message = String((err as { error: { message: string } }).error.message);
  // Transport errors read "<Code>: <message>"; the notice shows the message.
  return message.replace(/^[A-Z][A-Za-z]+: /, "");
}

/** Send `commands` as one undo gesture; a refusal becomes the notice. `true` on success. */
async function send(transport: EngineTransport, commands: Command[]): Promise<boolean> {
  const gesture = nextGestureId();
  try {
    for (const c of commands) await transport.send(c, { gesture });
    useTimeSelection.getState().notify(null);
    return true;
  } catch (err) {
    useTimeSelection.getState().notify(errorText(err));
    return false;
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
}

const timeEdit = (c: Extract<Command, { domain: "TimeEdit" }>["command"]): Command => cmd("TimeEdit", c);

/**
 * The selected tracks for "split at the playhead" when there is no time selection (nor
 * selected clips: then the clip split applies). Supplied by the arrangement.
 */
export interface TimeActionContext {
  selectedTracks: ReadonlySet<TrackId>;
  hasSelectedClips: boolean;
}

/** Whether `action` can run now. */
export function canRun(action: TimeAction, ctx?: TimeActionContext): boolean {
  const { selection, clipboard } = useTimeSelection.getState();
  switch (action) {
    case "split-tracks":
      return !selection && !!ctx && !ctx.hasSelectedClips && ctx.selectedTracks.size > 0;
    case "paste":
    case "paste-insert":
      return clipboard !== null;
    default:
      return selection !== null;
  }
}

export async function runTimeAction(transport: EngineTransport, action: TimeAction, ctx?: TimeActionContext): Promise<void> {
  const project = useProjectStore.getState().project;
  const store = useTimeSelection.getState();
  const sel = store.selection;
  if (!project || !canRun(action, ctx)) return;
  const selected = sel ? timeSelection(project, sel) : null;
  const select = (next: TimeRangeSelection | null) => useTimeSelection.getState().setSelection(next);
  switch (action) {
    case "split":
      await send(transport, [
        timeEdit({
          type: "Split",
          tracks: selected!.tracks,
          at: selected!.start,
          seed: newId(),
        }),
        timeEdit({
          type: "Split",
          tracks: selected!.tracks,
          at: selected!.end,
          seed: newId(),
        }),
      ]);
      return;
    case "split-tracks":
      await send(transport, [
        timeEdit({
          type: "Split",
          tracks: [...ctx!.selectedTracks],
          at: playheadBeats(),
          seed: newId(),
        }),
      ]);
      return;
    case "copy":
    case "cut": {
      const ok = await send(transport, [
        action === "copy"
          ? timeEdit({ type: "Copy", selection: selected! })
          : timeEdit({ type: "Cut", selection: selected!, seed: newId() }),
      ]);
      if (!ok) return;
      store.setClipboard({
        tracks: expandTracks(project, sel!.tracks).length,
        length: sel!.end - sel!.start,
      });
      if (action === "cut") select(null);
      return;
    }
    case "paste":
    case "paste-insert": {
      const at = sel ? sel.start : playheadBeats();
      const tracks = pasteTracks(project, sel?.tracks[0] ?? null);
      const insert = action === "paste-insert";
      const ok = await send(transport, [timeEdit({ type: "Paste", at, tracks, insert, seed: newId() })]);
      // The pasted range becomes the selection.
      const cb = store.clipboard!;
      if (ok && tracks.length)
        select({
          start: at,
          end: at + cb.length,
          tracks: tracks.slice(0, cb.tracks),
        });
      return;
    }
    case "duplicate": {
      const ok = await send(transport, [
        timeEdit({
          type: "DuplicateTime",
          selection: selected!,
          seed: newId(),
        }),
      ]);
      // The copy becomes the selection, so repeating extends the pattern.
      if (ok) select({ ...sel!, start: sel!.end, end: 2 * sel!.end - sel!.start });
      return;
    }
    case "delete":
      if (await send(transport, [timeEdit({ type: "DeleteTime", selection: selected!, seed: newId() })])) select(null);
      return;
    case "insert-silence":
      await send(transport, [
        timeEdit({
          type: "InsertSilence",
          tracks: selected!.tracks,
          at: selected!.start,
          length: selected!.end - selected!.start,
          global: selected!.global,
          seed: newId(),
        }),
      ]);
      return;
  }
}

/** Keymap action id -> time action (the arrangement's time-selection shortcuts). */
const TIME_KEY_ACTIONS: Record<string, TimeAction> = {
  "time.cut": "cut",
  "time.copy": "copy",
  "time.paste": "paste-insert",
  "time.duplicate": "duplicate",
  "time.insertSilence": "insert-silence",
  "time.delete": "delete",
};

/** The time action for a key press in the arrangement, or null (then the clip actions apply). */
export function timeActionForKey(e: ChordEvent, ctx: TimeActionContext): TimeAction | null {
  // keymap: chords from the user's keymap; `edit.split` (Mod+E) splits the time selection,
  // or the selected tracks at the playhead. Plain Mod+I is "Import audio..." (never here).
  const id = firstMatch([...Object.keys(TIME_KEY_ACTIONS), "edit.split"], e);
  if (!id) return null;
  const action = id === "edit.split" ? (useTimeSelection.getState().selection ? "split" : "split-tracks") : TIME_KEY_ACTIONS[id]!;
  return canRun(action, ctx) ? action : null;
}

/** Right-click menu inside the time selection. */
export function timeSelectionMenu(transport: EngineTransport): ContextMenuEntry[] {
  const run = (a: TimeAction) => () => void runTimeAction(transport, a);
  const hasClipboard = useTimeSelection.getState().clipboard !== null;
  return [
    { label: "Cut Time", shortcut: shortcutLabel("time.cut"), onSelect: run("cut") },
    { label: "Copy Time", shortcut: shortcutLabel("time.copy"), onSelect: run("copy") },
    {
      label: "Paste Over Selection",
      disabled: !hasClipboard,
      onSelect: run("paste"),
    },
    {
      label: "Paste Time",
      shortcut: shortcutLabel("time.paste"),
      disabled: !hasClipboard,
      onSelect: run("paste-insert"),
    },
    "separator",
    {
      label: "Split at Selection",
      shortcut: shortcutLabel("edit.split"),
      onSelect: run("split"),
    },
    {
      label: "Duplicate Time",
      shortcut: shortcutLabel("time.duplicate"),
      onSelect: run("duplicate"),
    },
    {
      label: "Insert Silence",
      shortcut: shortcutLabel("time.insertSilence"),
      onSelect: run("insert-silence"),
    },
    "separator",
    {
      label: "Delete Time",
      shortcut: shortcutLabel("time.delete"),
      danger: true,
      onSelect: run("delete"),
    },
  ];
}

/** `true` if content point (`x`, `y`) is inside the time selection's drawn area. */
export function inTimeSelection(
  x: number,
  y: number,
  rows: ReadonlyArray<{ track: { id: TrackId }; y: number; height: number }>,
  toBeats: (lanePx: number) => number,
  headerWidth: number,
): boolean {
  const sel = useTimeSelection.getState().selection;
  if (!sel || x < headerWidth) return false;
  const b = toBeats(x - headerWidth);
  if (b < sel.start || b > sel.end) return false;
  return rows.some((r) => sel.tracks.includes(r.track.id) && y >= r.y && y < r.y + r.height);
}

/**
 * Keep the time selection consistent with the other selections: it goes away when clips
 * are selected by other means than the drag that made it (a click, select-all) or tracks
 * are selected from their headers. Returns the unsubscribe.
 */
export function bindTimeSelection(selectedTracks: { subscribe(fn: (tracks: ReadonlySet<TrackId>) => void): () => void }): () => void {
  const offClips = itemSelection.subscribe((s, prev) => {
    if (s.selected.clip === prev.selected.clip) return;
    // Pruning (clips deleted by an edit) only removes ids: keep the selection then.
    const added = [...s.selected.clip].some((id) => !prev.selected.clip.has(id));
    if (added) useTimeSelection.getState().setSelection(null);
  });
  const offTracks = selectedTracks.subscribe((tracks) => {
    if (tracks.size) useTimeSelection.getState().setSelection(null);
  });
  return () => {
    offClips();
    offTracks();
  };
}
