/**
 * The action registry (CONTRACTS.md §13.12): every keyboard shortcut is an action with an id,
 * a label, where it applies (`scopes`) and its chords in the two built-in presets.
 *
 * - **Ethereal** (the default) is exactly the shortcuts the app had before the keymap.
 * - **Ableton-like** is Ethereal plus Ableton Live's chords for the same commands
 *   (`ableton`, added, never replacing one: switching presets keeps every default working).
 *
 * Two kinds of actions:
 * - **Handled**: a key handler asks the keymap (`matchesAction(id, e)`) instead of testing
 *   keys itself. The built-in list below.
 * - **Palette**: every command palette entry is an action too (the palette is the command
 *   registry, `ui/src/app/shell/commands.ts`). Entries linked to a built-in action (`action`
 *   field, or a hint that is a built-in's default chord) are that action; any other entry with
 *   a shortcut hint becomes an action whose default is the hint, so commands added to the
 *   palette by other features show up here, rebindable, with no list to keep in sync. Their
 *   chords are run by the keymap's dispatcher (`dispatcher.ts`).
 *
 * Other features can also add handled actions with `registerActions`.
 */

import type { MessageKey } from "@/i18n";

/** Where an action's key handler listens. `global` overlaps every other scope. */
export type KeymapScope = "global" | "arrangement" | "pianoRoll" | "automation";

/** Where an action works, as shown (message keys). */
export const SCOPE_LABELS: Record<KeymapScope, MessageKey> = {
  global: "shell.keymap.scope_global",
  arrangement: "shell.keymap.scope_arrangement",
  pianoRoll: "shell.keymap.scope_piano_roll",
  automation: "shell.keymap.scope_automation",
};

export interface KeymapAction {
  /** Stable id, stored in the user's overrides (`"transport.play"`, `"edit.duplicate"`). */
  id: string;
  /** The action's name, a message key (`actionLabel` shows it in the current language). */
  labelKey?: MessageKey;
  /** The name as text, for actions without a message (palette commands: already localized). */
  label?: string;
  /** Section in the editor and the cheat sheet: a stable id (`groupLabel` shows it). */
  group: string;
  scopes: ReadonlyArray<KeymapScope>;
  /** Default chords (the "Ethereal" preset). */
  chords: ReadonlyArray<string>;
  /** Chords the "Ableton-like" preset adds. */
  ableton?: ReadonlyArray<string>;
  /** Palette command run when the chord is pressed (actions without their own handler). */
  palette?: string;
  /**
   * Chords some hard-coded handler still answers (palette commands from features that test
   * keys themselves): the dispatcher only runs the command on the other chords, and blocks
   * these when the user unbinds them.
   */
  legacy?: ReadonlyArray<string>;
  /** True when a key handler looks the action up (the dispatcher leaves it alone). */
  handled?: boolean;
  /** Extra words the editor's search matches. */
  keywords?: string;
}

const E: ReadonlyArray<KeymapScope> = ["arrangement", "pianoRoll", "automation"];
const G: ReadonlyArray<KeymapScope> = ["global"];

/** The built-in, handled actions: every shortcut the app had, with its chord. */
export const BUILTIN_ACTIONS: ReadonlyArray<KeymapAction> = [
  // Transport
  // Space works everywhere but in text fields (transport-bar/keys.ts).
  { id: "transport.play", labelKey: "shell.action.transport_play", group: "Transport", scopes: G, chords: ["Space"], palette: "transport:play", handled: true },
  {
    // Id kept for saved keymaps; it now plays / stops in place.
    id: "transport.stopToSelection",
    labelKey: "shell.action.transport_stop_to_selection",
    group: "Transport",
    scopes: G,
    chords: ["Shift+Space"],
    palette: "transport:stopToSelection",
    handled: true,
    keywords: "play stop pause here stay",
  },
  {
    id: "transport.record",
    labelKey: "shell.action.transport_record",
    group: "Transport",
    scopes: G,
    chords: [],
    ableton: ["F9"],
    palette: "transport:record",
  },
  { id: "transport.loop", labelKey: "shell.action.transport_loop", group: "Transport", scopes: G, chords: [], ableton: ["Mod+L"], palette: "transport:loop" },
  { id: "transport.metronome", labelKey: "shell.action.transport_metronome", group: "Transport", scopes: G, chords: [], palette: "transport:metronome" },
  // Edit (global)
  { id: "edit.undo", labelKey: "shell.action.edit_undo", group: "Edit", scopes: G, chords: ["Mod+Z"], palette: "edit:undo", handled: true },
  { id: "edit.redo", labelKey: "shell.action.edit_redo", group: "Edit", scopes: G, chords: ["Mod+Shift+Z", "Ctrl+Y"], palette: "edit:redo", handled: true },
  // Edit (the focused editor: arrangement, piano roll, automation lane)
  { id: "edit.copy", labelKey: "shell.action.edit_copy", group: "Edit", scopes: E, chords: ["Mod+C"], handled: true },
  { id: "edit.cut", labelKey: "shell.action.edit_cut", group: "Edit", scopes: E, chords: ["Mod+X"], handled: true },
  { id: "edit.paste", labelKey: "shell.action.edit_paste", group: "Edit", scopes: E, chords: ["Mod+V"], handled: true },
  { id: "edit.duplicate", labelKey: "shell.action.edit_duplicate", group: "Edit", scopes: E, chords: ["Mod+D"], handled: true },
  { id: "edit.delete", labelKey: "shell.action.edit_delete", group: "Edit", scopes: E, chords: ["Backspace", "Delete"], handled: true, keywords: "remove" },
  // Aliases the old handlers accepted (Shift or Alt didn't change Delete). Their own action so
  // that Delete stays within MAX_CHORDS_PER_ACTION and stays editable on its own.
  {
    id: "edit.deleteModified",
    labelKey: "shell.action.edit_delete_modified",
    group: "Edit",
    scopes: E,
    chords: ["Shift+Backspace", "Shift+Delete", "Alt+Backspace", "Alt+Delete"],
    handled: true,
    keywords: "remove",
  },
  { id: "edit.selectAll", labelKey: "shell.action.edit_select_all", group: "Edit", scopes: E, chords: ["Mod+A"], handled: true },
  {
    id: "edit.deselect",
    labelKey: "shell.action.edit_deselect",
    group: "Edit",
    scopes: G,
    chords: ["Escape"],
    handled: true,
    keywords: "escape clear dismiss",
  },
  { id: "edit.split", labelKey: "shell.action.edit_split", group: "Edit", scopes: ["arrangement"], chords: ["Mod+E"], handled: true },
  // Arrangement
  { id: "clip.toggleLoop", labelKey: "shell.action.clip_toggle_loop", group: "Arrangement", scopes: ["arrangement"], chords: ["Mod+Shift+L"], handled: true },
  {
    id: "clip.reverse",
    labelKey: "shell.action.clip_reverse",
    group: "Arrangement",
    scopes: ["arrangement", "pianoRoll"],
    chords: ["R"],
    handled: true,
    keywords: "backwards mirror flip retrograde audio midi",
  },
  { id: "track.group", labelKey: "shell.action.track_group", group: "Arrangement", scopes: ["arrangement"], chords: ["Mod+G"], handled: true },
  { id: "track.ungroup", labelKey: "shell.action.track_ungroup", group: "Arrangement", scopes: ["arrangement"], chords: ["Mod+Shift+G"], handled: true },
  { id: "track.addAudio", labelKey: "shell.action.track_add_audio", group: "Arrangement", scopes: G, chords: [], ableton: ["Mod+T"], palette: "track:audio" },
  {
    id: "track.addMidi",
    labelKey: "shell.action.track_add_midi",
    group: "Arrangement",
    scopes: G,
    chords: [],
    ableton: ["Mod+Shift+T"],
    palette: "track:midi",
  },
  {
    id: "take.previous",
    labelKey: "shell.action.take_previous",
    group: "Arrangement",
    scopes: ["arrangement"],
    chords: ["ArrowUp"],
    handled: true,
    keywords: "comping",
  },
  {
    id: "take.next",
    labelKey: "shell.action.take_next",
    group: "Arrangement",
    scopes: ["arrangement"],
    chords: ["ArrowDown"],
    handled: true,
    keywords: "comping",
  },
  // Time selection (arrangement)
  { id: "time.cut", labelKey: "shell.action.time_cut", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+X"], handled: true },
  { id: "time.copy", labelKey: "shell.action.time_copy", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+C"], handled: true },
  { id: "time.paste", labelKey: "shell.action.time_paste", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+V"], handled: true },
  { id: "time.duplicate", labelKey: "shell.action.time_duplicate", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+D"], handled: true },
  {
    id: "time.insertSilence",
    labelKey: "shell.action.time_insert_silence",
    group: "Time selection",
    scopes: ["arrangement"],
    chords: ["Mod+Shift+I"],
    handled: true,
  },
  {
    id: "time.delete",
    labelKey: "shell.action.time_delete",
    group: "Time selection",
    scopes: ["arrangement"],
    chords: ["Mod+Shift+Backspace", "Mod+Shift+Delete"],
    handled: true,
  },
  // Piano roll
  { id: "pianoRoll.quantize", labelKey: "shell.action.piano_roll_quantize", group: "Piano roll", scopes: ["pianoRoll"], chords: ["Mod+U"], handled: true },
  { id: "pianoRoll.drawMode", labelKey: "shell.action.piano_roll_draw_mode", group: "Piano roll", scopes: ["pianoRoll"], chords: ["B"], handled: true },
  {
    id: "nudge.up",
    labelKey: "shell.action.nudge_up",
    group: "Piano roll",
    scopes: ["pianoRoll", "automation"],
    chords: ["ArrowUp"],
    handled: true,
    keywords: "move pitch value",
  },
  {
    id: "nudge.down",
    labelKey: "shell.action.nudge_down",
    group: "Piano roll",
    scopes: ["pianoRoll", "automation"],
    chords: ["ArrowDown"],
    handled: true,
    keywords: "move pitch value",
  },
  {
    id: "nudge.left",
    labelKey: "shell.action.nudge_left",
    group: "Piano roll",
    scopes: ["pianoRoll", "automation"],
    chords: ["ArrowLeft", "Alt+ArrowLeft"],
    handled: true,
    keywords: "move earlier",
  },
  {
    id: "nudge.right",
    labelKey: "shell.action.nudge_right",
    group: "Piano roll",
    scopes: ["pianoRoll", "automation"],
    chords: ["ArrowRight", "Alt+ArrowRight"],
    handled: true,
    keywords: "move later",
  },
  {
    id: "pianoRoll.octaveUp",
    labelKey: "shell.action.piano_roll_octave_up",
    group: "Piano roll",
    scopes: ["pianoRoll"],
    chords: ["Shift+ArrowUp"],
    handled: true,
  },
  {
    id: "pianoRoll.octaveDown",
    labelKey: "shell.action.piano_roll_octave_down",
    group: "Piano roll",
    scopes: ["pianoRoll"],
    chords: ["Shift+ArrowDown"],
    handled: true,
  },
  // Aliases the piano roll always accepted (Shift doesn't change a horizontal nudge; Alt
  // changes nothing): piano-roll-only, so they don't clash with the automation lane's fine
  // (⇧) and precise (⌥) nudges.
  {
    id: "pianoRoll.nudgeUpAlt",
    labelKey: "shell.action.piano_roll_nudge_up_alt",
    group: "Piano roll",
    scopes: ["pianoRoll"],
    chords: ["Alt+ArrowUp"],
    handled: true,
  },
  {
    id: "pianoRoll.nudgeDownAlt",
    labelKey: "shell.action.piano_roll_nudge_down_alt",
    group: "Piano roll",
    scopes: ["pianoRoll"],
    chords: ["Alt+ArrowDown"],
    handled: true,
  },
  {
    id: "pianoRoll.nudgeLeftShift",
    labelKey: "shell.action.piano_roll_nudge_left_shift",
    group: "Piano roll",
    scopes: ["pianoRoll"],
    chords: ["Shift+ArrowLeft"],
    handled: true,
  },
  {
    id: "pianoRoll.nudgeRightShift",
    labelKey: "shell.action.piano_roll_nudge_right_shift",
    group: "Piano roll",
    scopes: ["pianoRoll"],
    chords: ["Shift+ArrowRight"],
    handled: true,
  },
  // Automation lane
  {
    id: "automation.fineUp",
    labelKey: "shell.action.automation_fine_up",
    group: "Automation",
    scopes: ["automation"],
    chords: ["Shift+ArrowUp", "Alt+Shift+ArrowUp"],
    handled: true,
  },
  {
    id: "automation.fineDown",
    labelKey: "shell.action.automation_fine_down",
    group: "Automation",
    scopes: ["automation"],
    chords: ["Shift+ArrowDown", "Alt+Shift+ArrowDown"],
    handled: true,
  },
  {
    id: "automation.fineLeft",
    labelKey: "shell.action.automation_fine_left",
    group: "Automation",
    scopes: ["automation"],
    chords: ["Shift+ArrowLeft", "Alt+Shift+ArrowLeft"],
    handled: true,
  },
  {
    id: "automation.fineRight",
    labelKey: "shell.action.automation_fine_right",
    group: "Automation",
    scopes: ["automation"],
    chords: ["Shift+ArrowRight", "Alt+Shift+ArrowRight"],
    handled: true,
  },
  {
    id: "automation.preciseUp",
    labelKey: "shell.action.automation_precise_up",
    group: "Automation",
    scopes: ["automation"],
    chords: ["Alt+ArrowUp"],
    handled: true,
  },
  {
    id: "automation.preciseDown",
    labelKey: "shell.action.automation_precise_down",
    group: "Automation",
    scopes: ["automation"],
    chords: ["Alt+ArrowDown"],
    handled: true,
  },
  // App
  { id: "palette.open", labelKey: "shell.action.palette_open", group: "App", scopes: G, chords: ["Mod+K"], handled: true, keywords: "search commands" },
  { id: "keymap.open", labelKey: "shell.action.keymap_open", group: "App", scopes: G, chords: [], palette: "keymap:open", keywords: "keymap bindings hotkeys" },
  { id: "project.save", labelKey: "shell.action.project_save", group: "App", scopes: G, chords: ["Mod+S"], handled: true },
  {
    id: "project.snapshot",
    labelKey: "shell.action.project_snapshot",
    group: "App",
    scopes: G,
    chords: ["Mod+Alt+S"],
    palette: "project:snapshot",
    handled: true,
    keywords: "named version backup checkpoint",
  },
  { id: "file.import", labelKey: "shell.action.file_import", group: "App", scopes: G, chords: ["Mod+I"], palette: "import:audio", handled: true },
  {
    id: "view.editor",
    labelKey: "shell.action.view_editor",
    group: "View",
    scopes: G,
    chords: ["Mod+J"],
    ableton: ["Mod+Alt+L"],
    palette: "panel:drawer",
    handled: true,
  },
  { id: "view.browser", labelKey: "shell.action.view_browser", group: "View", scopes: G, chords: [], ableton: ["Mod+Alt+B"], palette: "panel:browser" },
  { id: "view.zoomToFit", labelKey: "shell.action.view_zoom_to_fit", group: "View", scopes: G, chords: [], palette: "view:fit" },
  {
    id: "collab.focusChat",
    labelKey: "shell.action.collab_focus_chat",
    group: "Collaboration",
    scopes: G,
    chords: ["Mod+Shift+M"],
    palette: "chat:focus",
    handled: true,
  },
  // ai-chat: its own handler answers Mod+Shift+A (`legacy`); the id is the palette command's
  // (saved bindings from when the palette derived it keep working).
  {
    id: "ai:ask",
    labelKey: "shell.action.ai_ask",
    group: "App",
    scopes: G,
    chords: ["Mod+Shift+A"],
    legacy: ["Mod+Shift+A"],
    palette: "ai:ask",
    keywords: "assistant claude chat",
  },
];

const extra = new Map<string, KeymapAction>();
const listeners = new Set<() => void>();
let version = 0;

/**
 * Add handled actions from a feature (ids unique; a later registration of the same id
 * replaces it). Returns an unregister function.
 */
export function registerActions(actions: ReadonlyArray<KeymapAction>): () => void {
  for (const a of actions) extra.set(a.id, a);
  bump();
  return () => {
    for (const a of actions) if (extra.get(a.id) === a) extra.delete(a.id);
    bump();
  };
}

function bump() {
  version++;
  for (const l of listeners) l();
}

/** Re-run when actions are registered (for `useSyncExternalStore`). */
export function subscribeRegistry(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function registryVersion(): number {
  return version;
}

/** The built-in and registered actions (not the palette-derived ones). */
export function registeredActions(): KeymapAction[] {
  const byId = new Map(BUILTIN_ACTIONS.map((a) => [a.id, a]));
  for (const [id, a] of extra) byId.set(id, a);
  return [...byId.values()];
}

export function registeredAction(id: string): KeymapAction | undefined {
  return extra.get(id) ?? BUILTIN_ACTIONS.find((a) => a.id === id);
}
