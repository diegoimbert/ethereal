/**
 * Time-selection actions (keyboard shortcuts and the selection's context menu), Ableton-like.
 * Default keys below; the user's keymap decides (`time.*`, `edit.split`; keymap node):
 *
 * | action          | key        | what                                                     |
 * |-----------------|------------|----------------------------------------------------------|
 * | split           | ⌘E         | split every selected track at the selection edges        |
 * | split-tracks    | ⌘E         | no time range, tracks selected or an insert marker: split them at the marker (else the playhead) |
 * | cut             | ⌘X / ⇧⌘X   | copy the selection, then delete its time                 |
 * | copy            | ⌘C / ⇧⌘C   | copy the selection to the (engine) time clipboard        |
 * | paste           | ⌘V         | paste the copied section (its full length, over what is there) right after the selection, else at the insert marker, else the playhead |
 * | paste-insert    | ⇧⌘V        | paste at the selection start / insert marker / playhead, shifting what follows right |
 * | duplicate       | ⌘D / ⇧⌘D   | insert a copy of the selection right after it            |
 * | delete          | ⌫ / ⇧⌘⌫    | remove the selected time (later material moves left)     |
 * | insert-silence  | ⇧⌘I        | insert the selection's length of silence at its start    |
 *
 * section-edit (owner report): while a time selection exists, the PLAIN ⌘C/X/V/D and ⌫ act
 * on it (the section: clips are cut at its edges, only the span is copied), not on whole
 * clips; the ⇧⌘ forms stay as aliases. Without a time selection the clip shortcuts apply,
 * except ⌘V right after a time copy (the most recent copy wins, see `clipboard`).
 *
 * The insert marker (a zero-length selection, `marker.ts`) is not a range: copy, cut,
 * duplicate, delete and insert-silence need a real range; paste and split target the marker.
 *
 * Each is one undo step (split at both edges: two commands under one gesture). Refused
 * edits (a frozen track, ...) show their engine message as a notice.
 */

import type { Command, TrackId } from "@/generated";
import { firstMatch, shortcutLabel, type ChordEvent } from "@/features/keymap";
import type { ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, newId, nextGestureId, type EngineTransport } from "@/transport";
import { expandTracks, pasteTracks, timeSelection } from "./commands";
import { insertMarker, insertPoint, pasteTarget } from "./marker";
import { rangeOf, useTimeSelection, type TimeRangeSelection } from "./store";

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
  const range = rangeOf(selection);
  switch (action) {
    case "split-tracks":
      return !range && !!ctx && !ctx.hasSelectedClips && (ctx.selectedTracks.size > 0 || insertMarker() !== null);
    case "paste":
    case "paste-insert":
      return clipboard !== null;
    default:
      return range !== null;
  }
}

/** `over`: "paste" goes over the selection (at its start), not after it (the menu item). */
export async function runTimeAction(
  transport: EngineTransport,
  action: TimeAction,
  ctx?: TimeActionContext,
  opts: { over?: boolean } = {},
): Promise<void> {
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
          tracks: ctx!.selectedTracks.size ? [...ctx!.selectedTracks] : [...insertMarker()!.tracks],
          at: insertPoint().at,
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
      const insert = action === "paste-insert";
      const at = insert || opts.over ? insertPoint().at : pasteAt(sel);
      const tracks = pasteTracks(project, sel?.tracks[0] ?? null);
      const ok = await send(transport, [timeEdit({ type: "Paste", at, tracks, insert, seed: newId() })]);
      // The pasted range becomes the selection (so the next ⌘V lands right after it).
      const cb = store.clipboard!;
      const target = tracks.length ? tracks.slice(0, cb.tracks) : sel?.tracks;
      // Never the playhead: edits stay dissociated from playback (insert marker, owner).
      if (ok && target?.length) select({ start: at, end: at + cb.length, tracks: target });
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

/** Keymap action id -> time action (the arrangement's ⇧⌘ time-selection shortcuts). */
const TIME_KEY_ACTIONS: Record<string, TimeAction> = {
  "time.cut": "cut",
  "time.copy": "copy",
  "time.paste": "paste-insert",
  "time.duplicate": "duplicate",
  "time.insertSilence": "insert-silence",
  "time.delete": "delete",
};

/** The plain edit actions that act on the section while a time range exists (section-edit). */
const SECTION_KEY_ACTIONS = ["edit.delete", "edit.copy", "edit.cut", "edit.paste", "edit.duplicate"] as const;

/**
 * Where ⌘V pastes: right after the time selection (the pasted range becomes the selection,
 * so repeated pastes tile the section, gaps included), else at the insert marker, else at
 * the playhead. "Paste Over Selection" (menu) pastes at the selection start instead.
 */
export function pasteAt(sel: TimeRangeSelection | null): number {
  return pasteTarget(sel);
}

/** The time action for a key press in the arrangement, or null (then the clip actions apply). */
export function timeActionForKey(e: ChordEvent, ctx: TimeActionContext): TimeAction | null {
  // keymap: chords from the user's keymap. Plain Mod+I is "Import audio..." (never here).
  // The insert marker is not a section: Copy/Cut/Duplicate/Delete then act on clips.
  const hasSelection = rangeOf(useTimeSelection.getState().selection) !== null;
  const id = firstMatch([...SECTION_KEY_ACTIONS, ...Object.keys(TIME_KEY_ACTIONS), "edit.split"], e);
  if (!id) return null;
  // Delete (plain ⌫) deletes the selected time while there is one; else clips/tracks.
  if (id === "edit.delete") return hasSelection && canRun("delete", ctx) ? "delete" : null;
  if (id === "edit.copy" || id === "edit.cut" || id === "edit.paste" || id === "edit.duplicate") {
    // The section shortcuts (see the module doc); Paste also pastes a time copy with no range.
    const action =
      id === "edit.duplicate" ? (hasSelection ? "duplicate" : null) : clipboardAction(id === "edit.copy" ? "copy" : id === "edit.cut" ? "cut" : "paste");
    return action && canRun(action, ctx) ? action : null;
  }
  // `edit.split` (Mod+E) splits the time selection, or the selected tracks at the marker/playhead.
  const action = id === "edit.split" ? (hasSelection ? "split" : "split-tracks") : TIME_KEY_ACTIONS[id]!;
  return canRun(action, ctx) ? action : null;
}

/**
 * The time action for a copy/cut/paste (key or the desktop Edit menu's clipboard event), or
 * null for the clip clipboard: copy/cut need a time selection; paste needs the time
 * clipboard to be the most recent copy.
 */
export function clipboardAction(kind: "copy" | "cut" | "paste" | null): TimeAction | null {
  const { selection, clipboard } = useTimeSelection.getState();
  if (kind === "paste") return clipboard ? "paste" : null;
  if (kind === "copy" || kind === "cut") return rangeOf(selection) ? kind : null;
  return null;
}

/** Right-click menu inside the time selection. */
export function timeSelectionMenu(transport: EngineTransport): ContextMenuEntry[] {
  const run = (a: TimeAction) => () => void runTimeAction(transport, a);
  const hasClipboard = useTimeSelection.getState().clipboard !== null;
  return [
    { label: "Cut Time", shortcut: shortcutLabel("edit.cut"), onSelect: run("cut") },
    { label: "Copy Time", shortcut: shortcutLabel("edit.copy"), onSelect: run("copy") },
    {
      label: "Paste After Selection",
      shortcut: shortcutLabel("edit.paste"),
      disabled: !hasClipboard,
      onSelect: run("paste"),
    },
    {
      label: "Paste Over Selection",
      disabled: !hasClipboard,
      onSelect: () => void runTimeAction(transport, "paste", undefined, { over: true }),
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
      shortcut: shortcutLabel("edit.duplicate"),
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
      shortcut: shortcutLabel("edit.delete"),
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
  const sel = rangeOf(useTimeSelection.getState().selection);
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
