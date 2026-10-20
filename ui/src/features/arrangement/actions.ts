/** Edit actions on the selected clips, shared by the toolbar and keyboard shortcuts. */

import type { EngineTransport } from "@/transport";
import { cmd, newId } from "@/transport";
import type { Clip, Command, Project, Track, TrackId } from "@/generated";
import { firstMatch, shortcutLabel, type ChordEvent } from "@/features/keymap";
import type { ContextMenuEntry } from "@/kit";
import { t } from "@/i18n";
import { tracksOrdered, useEditorStore, useProjectStore, useSelectionStore } from "@/state";
import { itemSelection, type SelectMode } from "@/timeline";
import { insertPoint, pasteTarget } from "@/features/time-edits/marker";
import { copyClips, cutClips, hasClipboard, pasteClips } from "./clipboard";
import { renameClipInPlace } from "./clipRename";
import { isArrangementClip } from "./clipTime";
import { arrangementTracks } from "./layout";
import { selectedClips, sendEdit } from "./context";
import { useArrangementUi } from "./uiStore";
import { reverseClipsCommand } from "@/features/clip-editing/clipEditing";
import { consolidateIfAllowed } from "@/features/freeze";
import { exportMidiClips, midiClipsOfTracks } from "@/features/midi-file/exportMidi";
import { asOneStep, deleteCommand, duplicateCommand, splitCommand, toggleLoopCommand } from "./editMath";
import { getTrackClipboard } from "./clipboard";
import {
  canPasteTracks,
  selectedTrackIds,
  copySelectedTracks,
  cutSelectedTracks,
  deleteSelectedTracks,
  duplicateSelectedTracks,
  pasteTracks,
} from "./trackEdit";

export { deleteSelectedTracks } from "./trackEdit";

export type NewTrackKind = "Midi" | "Audio" | "Return";

const ADD_LABELS: Record<NewTrackKind, string> = { Midi: "add midi track", Audio: "add audio track", Return: "add return track" };
/** The undo label's message (`undo.track.create`, docs/LOCALIZATION.md "Text from the engine"). */
const ADD_KINDS: Record<NewTrackKind, string> = { Midi: "midi", Audio: "audio", Return: "return" };

/**
 * Commands adding a track at the end of the list (or at `at`); a MIDI track comes with the
 * built-in synth, so it plays as soon as it has notes. A return track is always top-level,
 * among the returns (`at.before` another return, or null: after the returns). One undo step.
 */
export function addTrackCommand(
  kind: NewTrackKind,
  id: TrackId,
  deviceId: string,
  at: { parent: TrackId | null; before: TrackId | null } = { parent: null, before: null },
): Command {
  const place = kind === "Return" ? { parent: null, before: at.before } : at;
  const commands: Command[] = [cmd("Track", { type: "Create", id, kind, name: null, color: null, parent: place.parent, before: place.before })];
  if (kind === "Midi") {
    commands.push(cmd("Device", { type: "Insert", id: deviceId, track: id, device: { type: "Builtin", device: { type: "Synth" } }, before: null }));
  }
  return asOneStep(ADD_LABELS[kind], commands, { id: "undo.track.create", args: { kind: ADD_KINDS[kind] } })!;
}

/**
 * Select tracks as the arrangement's entity (header click); the clip and automation point
 * selections are cleared. `replace` selects just `track`; `toggle` (cmd/ctrl-click) adds
 * or removes it; `add` (shift-click) selects the range from the last clicked track to it,
 * in display order.
 */
export function selectTrackEntity(track: TrackId, mode: SelectMode = "replace"): void {
  itemSelection.getState().clear("clip");
  itemSelection.getState().clear("automationPoint");
  const ui = useArrangementUi.getState();
  const anchor = ui.trackFocus;
  if (mode === "toggle") {
    const next = new Set(ui.selectedTracks);
    if (next.has(track)) {
      next.delete(track);
      ui.setTrackSelection(next, anchor === track ? ([...next].at(-1) ?? null) : anchor);
    } else {
      next.add(track);
      ui.setTrackSelection(next, track);
    }
  } else if (mode === "add" && anchor && anchor !== track) {
    const project = useProjectStore.getState().project;
    const order = project ? arrangementTracks(tracksOrdered(project), ui.folded).map((t) => t.id) : [];
    const [a, b] = [order.indexOf(anchor), order.indexOf(track)];
    if (a < 0 || b < 0) ui.setTrackFocus(track);
    else ui.setTrackSelection(order.slice(Math.min(a, b), Math.max(a, b) + 1), anchor);
  } else {
    ui.setTrackFocus(track);
  }
  const focus = useArrangementUi.getState().trackFocus;
  if (focus) useSelectionStore.getState().selectTrack(focus);
}

/**
 * The tracks Mod+A selects when a track is the edit target: the arrangement's rows in
 * display order, as a shift-click from the first row to the last would (groups and their
 * shown children, returns; children of folded groups stay out, like in a shift-range),
 * minus the master track.
 */
export function allSelectableTracks(project: Project, folded: ReadonlySet<TrackId>): TrackId[] {
  return arrangementTracks(tracksOrdered(project), folded)
    .filter((t) => t.kind !== "Master")
    .map((t) => t.id);
}

/** Select every track (`allSelectableTracks`) as the arrangement's entity; clips are cleared. */
export function selectAllTracks(): void {
  const project = useProjectStore.getState().project;
  if (!project) return;
  const ui = useArrangementUi.getState();
  const all = allSelectableTracks(project, ui.folded);
  if (all.length === 0) return;
  itemSelection.getState().clear("clip");
  itemSelection.getState().clear("automationPoint");
  // The focus (shift-click anchor, inspector) stays on the track it was on.
  const focus = ui.trackFocus && all.includes(ui.trackFocus) ? ui.trackFocus : all[0]!;
  ui.setTrackSelection(all, focus);
  useSelectionStore.getState().selectTrack(focus);
}

/**
 * A click (no drag) in the lanes, on `track`'s row (null: below the tracks) and on `clip`'s
 * body if there is one there: the clip is selected (the track focus cleared, the track
 * still becomes the "selected track"); on empty space the track is selected like a click
 * on its header (`selectTrackEntity`: the inspector shows it, modifiers extend/toggle).
 * Below the tracks, the track focus is cleared.
 */
export function selectAtLaneClick(track: TrackId | null, clip: Clip["id"] | null, mode: SelectMode): void {
  if (track && !clip) {
    selectTrackEntity(track, mode);
    return;
  }
  useArrangementUi.getState().setTrackFocus(null);
  if (track) useSelectionStore.getState().selectTrack(track);
  if (clip) itemSelection.getState().select("clip", [clip], mode);
}

/**
 * Keep a single kind of selected entity in the arrangement: selecting clips clears the
 * automation points and the track focus, and the other way around. Returns the unsubscribe.
 */
export function bindSingleSelection(): () => void {
  return itemSelection.subscribe((s, prev) => {
    const kinds = (["clip", "automationPoint", "tempoPoint"] as const).filter((k) => s.selected[k] !== prev.selected[k] && s.selected[k].size > 0);
    const chosen = kinds[0];
    if (!chosen) return;
    useArrangementUi.getState().setTrackFocus(null);
    // One kind at a time in the arrangement: clips, automation points or tempo points.
    for (const k of ["clip", "automationPoint", "tempoPoint"] as const) if (k !== chosen) itemSelection.getState().clear(k);
  });
}

/** Add a track (see `addTrackCommand`), at the end or at `at`, and select it. */
export async function addTrack(transport: EngineTransport, kind: NewTrackKind, at?: { parent: TrackId | null; before: TrackId | null }): Promise<TrackId> {
  const id = newId();
  const project = useProjectStore.getState().project;
  const place = at ?? (kind === "Return" && project ? newReturnPlace(project, useSelectionStore.getState().selectedTrack) : undefined);
  await sendEdit(transport, addTrackCommand(kind, id, newId(), place));
  if (useProjectStore.getState().project?.tracks[id]) useSelectionStore.getState().selectTrack(id);
  return id;
}

/**
 * Where a new return goes without an explicit place: right after the selected return, else
 * after the other returns.
 */
export function newReturnPlace(project: Project, selected: TrackId | null): { parent: null; before: TrackId | null } {
  if (!selected || project.tracks[selected]?.kind !== "Return") return { parent: null, before: null };
  const returns = tracksOrdered(project).filter((t) => t.kind === "Return");
  const i = returns.findIndex((t) => t.id === selected);
  return { parent: null, before: returns[i + 1]?.id ?? null };
}

/**
 * Every way of adding a track from the UI (Add track menu click or drop, the context menu):
 * add it, select it, and for a MIDI track open its instrument picker (`InstrumentPick`).
 */
export async function addTrackAndSelect(
  transport: EngineTransport,
  kind: NewTrackKind,
  at?: { parent: TrackId | null; before: TrackId | null },
): Promise<TrackId> {
  const id = await addTrack(transport, kind, at);
  selectTrackEntity(id);
  if (kind === "Midi") useArrangementUi.getState().setInstrumentPick(id);
  return id;
}

/**
 * What the arrangement's edit keys (⌘C/X/V/D, ⌫) act on: the selected tracks or the
 * selected clips. Rule: the most recently selected kind wins. Selecting one
 * normally clears the other (`selectTrackEntity`, `bindSingleSelection`), so this only
 * decides when both are set (e.g. a selection made by code): the later one. Paste
 * follows the clipboard instead (tracks after a track copy, clips after a clip copy).
 */
export function editTarget(): "tracks" | "clips" {
  const tracks = useArrangementUi.getState().selectedTracks.size > 0;
  const clips = itemSelection.getState().selected.clip.size > 0;
  if (tracks && clips) return trackSelectedAt > clipSelectedAt ? "tracks" : "clips";
  return tracks ? "tracks" : "clips";
}

let selectionSeq = 0;
let trackSelectedAt = 0;
let clipSelectedAt = 0;
useArrangementUi.subscribe((s, prev) => {
  if (s.selectedTracks !== prev.selectedTracks && s.selectedTracks.size > 0) trackSelectedAt = ++selectionSeq;
});
itemSelection.subscribe((s, prev) => {
  if (s.selected.clip !== prev.selected.clip && s.selected.clip.size > 0) clipSelectedAt = ++selectionSeq;
});

export type ClipAction = "split" | "duplicate" | "delete" | "loop" | "select-all" | "deselect" | "copy" | "cut" | "paste" | "reverse" | "consolidate";

export function runClipAction(transport: EngineTransport, action: ClipAction): Promise<void> {
  const clips = selectedClips();
  // The selected tracks, when they are the edit target (see `editTarget`).
  const onTracks = editTarget() === "tracks";
  switch (action) {
    case "split":
      // At the insert marker (else the range start, else the playhead): not the playhead
      // while you listen.
      return sendEdit(transport, splitCommand(clips, insertPoint().at, newId));
    case "duplicate": {
      if (onTracks) return duplicateSelectedTracks(transport).then(() => {});
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
      if (onTracks) return deleteSelectedTracks(transport);
      return sendEdit(transport, deleteCommand(clips.map((c) => c.id)));
    case "loop":
      return sendEdit(transport, toggleLoopCommand(clips));
    case "reverse": {
      // Audio clips: reversed unless all already are (then unreversed). MIDI clips: their
      // notes, lanes and envelopes are mirrored in time.
      return sendEdit(transport, reverseClipsCommand(clips));
    }
    case "consolidate":
      // Mod+J, like the clip menu's "consolidate" (frozen / rendering tracks: nothing).
      consolidateIfAllowed(transport, clips);
      return Promise.resolve();
    case "select-all": {
      // A track selected (header or empty lane clicked): every track; else every clip.
      if (onTracks) {
        selectAllTracks();
        return Promise.resolve();
      }
      const project = useProjectStore.getState().project;
      const all = project
        ? Object.values(project.clips)
            .filter(isArrangementClip)
            .map((c) => c.id)
        : [];
      itemSelection.getState().select("clip", all, "replace");
      return Promise.resolve();
    }
    case "copy":
      if (onTracks) copySelectedTracks();
      else copyClips();
      return Promise.resolve();
    case "cut":
      return onTracks ? cutSelectedTracks(transport) : cutClips(transport);
    case "paste":
      if (getTrackClipboard()) return pasteTracks(transport).then(() => {});
      // At the insert marker (after a range selection; the playhead only without either),
      // onto the marker's track (else the selected one) when the clips all come from one track.
      return pasteClips(transport, pasteTarget(), insertPoint().track ?? useSelectionStore.getState().selectedTrack).then(() => {});
    case "deselect":
      itemSelection.getState().clear("clip");
      useArrangementUi.getState().setTrackFocus(null);
      // Escape also closes an unpinned piano roll.
      useEditorStore.getState().dismiss();
      return Promise.resolve();
  }
}

/** The action for a key press in the arrangement, or null. */
export function actionForKey(e: ChordEvent): ClipAction | null {
  // keymap: the chords come from the user's keymap (registry ids → clip actions).
  const id = firstMatch(Object.keys(CLIP_KEY_ACTIONS), e);
  return id ? CLIP_KEY_ACTIONS[id]! : null;
}

/** Keymap action id → clip action (the arrangement's clip shortcuts). */
const CLIP_KEY_ACTIONS: Record<string, ClipAction> = {
  "edit.delete": "delete",
  "edit.deleteModified": "delete",
  "edit.deselect": "deselect",
  "edit.split": "split",
  "edit.duplicate": "duplicate",
  "edit.selectAll": "select-all",
  "edit.copy": "copy",
  "edit.cut": "cut",
  "edit.paste": "paste",
  "clip.toggleLoop": "loop",
  "clip.reverse": "reverse",
  "clip.consolidate": "consolidate",
};

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
  // Several MIDI clips selected: the piano roll edits them all at once.
  const midi = clips.filter((c) => c.content.type === "Midi").length;
  return [
    ...(clip.content.type === "Midi"
      ? [
          {
            label:
              clips.length > 1 && midi > 1 ? t("arrangement.clip_menu.edit_in_piano_roll", { count: midi }) : t("arrangement.clip_menu.open_in_piano_roll"),
            onSelect: () => useEditorStore.getState().openClip(clip.id),
          },
          "separator" as const,
        ]
      : // An audio clip opens in its editor (the drawer's audio tab), like a click on it.
        [{ label: t("arrangement.clip_menu.open_in_editor"), onSelect: () => useEditorStore.getState().openClip(clip.id) }, "separator" as const]),
    ...(clips.length <= 1 ? [{ label: t("arrangement.menu.rename"), onSelect: () => renameClipInPlace(clip.id) }, "separator" as const] : []),
    { label: t("arrangement.menu.cut"), shortcut: shortcutLabel("edit.cut"), onSelect: run("cut") },
    { label: t("arrangement.menu.copy"), shortcut: shortcutLabel("edit.copy"), onSelect: run("copy") },
    { label: t("arrangement.menu.paste"), shortcut: shortcutLabel("edit.paste"), disabled: !hasClipboard(), onSelect: run("paste") },
    "separator",
    { label: t("arrangement.clip_menu.split"), shortcut: shortcutLabel("edit.split"), onSelect: run("split") },
    { label: t("arrangement.menu.duplicate"), shortcut: shortcutLabel("edit.duplicate"), onSelect: run("duplicate") },
    {
      label: allLooping ? t("arrangement.clip_menu.disable_loop") : t("arrangement.clip_menu.enable_loop"),
      shortcut: shortcutLabel("clip.toggleLoop"),
      onSelect: run("loop"),
    },
    {
      label: allMuted ? t("arrangement.clip_menu.unmute") : t("arrangement.clip_menu.mute"),
      onSelect: () => void sendEdit(transport, cmd("Clip", { type: "SetMuted", ids: clips.map((c) => c.id), muted: !allMuted })),
    },
    // The selected MIDI clips as a Standard MIDI File (from the first clip).
    ...(clips.length > 0 && midi === clips.length
      ? [
          "separator" as const,
          {
            label: t("arrangement.menu.export_midi"),
            onSelect: () =>
              void exportMidiClips(
                transport,
                clips.map((c) => c.id),
                { name: clips.length === 1 ? clip.name : (useProjectStore.getState().project?.tracks[clip.track]?.name ?? clip.name), fromProjectStart: false },
              ),
          },
        ]
      : []),
    "separator",
    { label: t("arrangement.clip_menu.delete", { count: clips.length }), shortcut: shortcutLabel("edit.delete"), danger: true, onSelect: run("delete") },
  ];
}

/**
 * Right-click menu of a track header. Right-clicking an unselected track selects just it
 * first; on a selected one, every entry acts on the whole selection (one undo step each),
 * like the clip menu, and the labels say how many ("delete 3 tracks").
 */
export function trackMenu(transport: EngineTransport, track: Track): ContextMenuEntry[] {
  if (!useArrangementUi.getState().selectedTracks.has(track.id)) selectTrackEntity(track.id);
  if (track.kind === "Master") return [];
  const count = selectedTrackIds().length;
  const addReturn: ContextMenuEntry = { label: t("arrangement.menu.add_return"), onSelect: () => void addTrackAndSelect(transport, "Return") };
  // The selected tracks' MIDI clips as a Standard MIDI File, from the project start.
  const project = useProjectStore.getState().project;
  const midiClips = project ? midiClipsOfTracks(project, selectedTrackIds()) : [];
  const exportMidi: ContextMenuEntry = {
    label: t("arrangement.menu.export_midi"),
    disabled: midiClips.length === 0,
    onSelect: () =>
      void exportMidiClips(
        transport,
        midiClips.map((c) => c.id),
        { name: count > 1 ? (project?.settings.name ?? track.name) : track.name, fromProjectStart: true },
      ),
  };
  return [
    { label: t("arrangement.track_menu.cut", { count }), shortcut: shortcutLabel("edit.cut"), onSelect: () => void cutSelectedTracks(transport) },
    { label: t("arrangement.track_menu.copy", { count }), shortcut: shortcutLabel("edit.copy"), onSelect: () => void copySelectedTracks() },
    {
      label: t("arrangement.track_menu.paste", { count: getTrackClipboard()?.tracks.length ?? 1 }),
      shortcut: shortcutLabel("edit.paste"),
      disabled: !canPasteTracks(),
      onSelect: () => void pasteTracks(transport),
    },
    "separator",
    {
      label: t("arrangement.track_menu.duplicate", { count }),
      shortcut: shortcutLabel("edit.duplicate"),
      onSelect: () => void duplicateSelectedTracks(transport),
    },
    "separator",
    {
      label: t("arrangement.track_menu.delete", { count }),
      shortcut: shortcutLabel("edit.delete"),
      danger: true,
      onSelect: () => void deleteSelectedTracks(transport),
    },
    "separator",
    exportMidi,
    addReturn,
  ];
}

/** Context-menu items adding a track at the end of the list (right-click on empty space). */
export function newTrackMenu(transport: EngineTransport): ContextMenuEntry[] {
  const add = (kind: NewTrackKind) => () => void addTrackAndSelect(transport, kind);
  return [
    { label: t("arrangement.menu.add_audio"), onSelect: add("Audio") },
    { label: t("arrangement.menu.add_midi"), onSelect: add("Midi") },
    { label: t("arrangement.menu.add_return"), onSelect: add("Return") },
  ];
}
