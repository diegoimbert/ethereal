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

/** Where an action's key handler listens. `global` overlaps every other scope. */
export type KeymapScope = "global" | "arrangement" | "pianoRoll" | "automation";

export const SCOPE_LABELS: Record<KeymapScope, string> = {
  global: "Everywhere",
  arrangement: "Arrangement",
  pianoRoll: "Piano roll",
  automation: "Automation lane",
};

export interface KeymapAction {
  /** Stable id, stored in the user's overrides (`"transport.play"`, `"edit.duplicate"`). */
  id: string;
  label: string;
  /** Section in the editor and the cheat sheet. */
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
  { id: "transport.play", label: "Play / Stop", group: "Transport", scopes: G, chords: ["Space", "Shift+Space"], palette: "transport:play", handled: true },
  { id: "transport.record", label: "Record", group: "Transport", scopes: G, chords: [], ableton: ["F9"], palette: "transport:record" },
  { id: "transport.loop", label: "Loop on / off", group: "Transport", scopes: G, chords: [], ableton: ["Mod+L"], palette: "transport:loop" },
  { id: "transport.metronome", label: "Metronome on / off", group: "Transport", scopes: G, chords: [], palette: "transport:metronome" },
  // Edit (global)
  { id: "edit.undo", label: "Undo", group: "Edit", scopes: G, chords: ["Mod+Z"], palette: "edit:undo", handled: true },
  { id: "edit.redo", label: "Redo", group: "Edit", scopes: G, chords: ["Mod+Shift+Z", "Ctrl+Y"], palette: "edit:redo", handled: true },
  // Edit (the focused editor: arrangement, piano roll, automation lane)
  { id: "edit.copy", label: "Copy", group: "Edit", scopes: E, chords: ["Mod+C"], handled: true },
  { id: "edit.cut", label: "Cut", group: "Edit", scopes: E, chords: ["Mod+X"], handled: true },
  { id: "edit.paste", label: "Paste at the playhead", group: "Edit", scopes: E, chords: ["Mod+V"], handled: true },
  { id: "edit.duplicate", label: "Duplicate", group: "Edit", scopes: E, chords: ["Mod+D"], handled: true },
  { id: "edit.delete", label: "Delete", group: "Edit", scopes: E, chords: ["Backspace", "Delete"], handled: true, keywords: "remove" },
  // Aliases the old handlers accepted (Shift or Alt didn't change Delete). Their own action so
  // that Delete stays within MAX_CHORDS_PER_ACTION and stays editable on its own.
  {
    id: "edit.deleteModified",
    label: "Delete (with Shift / Alt)",
    group: "Edit",
    scopes: E,
    chords: ["Shift+Backspace", "Shift+Delete", "Alt+Backspace", "Alt+Delete"],
    handled: true,
    keywords: "remove",
  },
  { id: "edit.selectAll", label: "Select all", group: "Edit", scopes: E, chords: ["Mod+A"], handled: true },
  {
    id: "edit.deselect",
    label: "Deselect / close the piano roll",
    group: "Edit",
    scopes: G,
    chords: ["Escape"],
    handled: true,
    keywords: "escape clear dismiss",
  },
  { id: "edit.split", label: "Split at the playhead / time selection", group: "Edit", scopes: ["arrangement"], chords: ["Mod+E"], handled: true },
  // Arrangement
  { id: "clip.toggleLoop", label: "Clip loop on / off", group: "Arrangement", scopes: ["arrangement"], chords: ["Mod+Shift+L"], handled: true },
  { id: "track.group", label: "Group selected tracks", group: "Arrangement", scopes: ["arrangement"], chords: ["Mod+G"], handled: true },
  { id: "track.ungroup", label: "Ungroup", group: "Arrangement", scopes: ["arrangement"], chords: ["Mod+Shift+G"], handled: true },
  { id: "track.addAudio", label: "Add audio track", group: "Arrangement", scopes: G, chords: [], ableton: ["Mod+T"], palette: "track:audio" },
  { id: "track.addMidi", label: "Add MIDI track", group: "Arrangement", scopes: G, chords: [], ableton: ["Mod+Shift+T"], palette: "track:midi" },
  { id: "take.previous", label: "Previous take (comp region)", group: "Arrangement", scopes: ["arrangement"], chords: ["ArrowUp"], handled: true, keywords: "comping" },
  { id: "take.next", label: "Next take (comp region)", group: "Arrangement", scopes: ["arrangement"], chords: ["ArrowDown"], handled: true, keywords: "comping" },
  // Time selection (arrangement)
  { id: "time.cut", label: "Cut time", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+X"], handled: true },
  { id: "time.copy", label: "Copy time", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+C"], handled: true },
  { id: "time.paste", label: "Paste time (insert)", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+V"], handled: true },
  { id: "time.duplicate", label: "Duplicate time", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+D"], handled: true },
  { id: "time.insertSilence", label: "Insert silence", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+I"], handled: true },
  { id: "time.delete", label: "Delete time", group: "Time selection", scopes: ["arrangement"], chords: ["Mod+Shift+Backspace", "Mod+Shift+Delete"], handled: true },
  // Piano roll
  { id: "pianoRoll.quantize", label: "Quantize", group: "Piano roll", scopes: ["pianoRoll"], chords: ["Mod+U"], handled: true },
  { id: "pianoRoll.drawMode", label: "Draw mode", group: "Piano roll", scopes: ["pianoRoll"], chords: ["B"], handled: true },
  { id: "nudge.up", label: "Nudge up", group: "Piano roll", scopes: ["pianoRoll", "automation"], chords: ["ArrowUp", "Alt+ArrowUp"], handled: true, keywords: "move pitch value" },
  { id: "nudge.down", label: "Nudge down", group: "Piano roll", scopes: ["pianoRoll", "automation"], chords: ["ArrowDown", "Alt+ArrowDown"], handled: true, keywords: "move pitch value" },
  { id: "nudge.left", label: "Nudge left", group: "Piano roll", scopes: ["pianoRoll", "automation"], chords: ["ArrowLeft", "Alt+ArrowLeft"], handled: true, keywords: "move earlier" },
  { id: "nudge.right", label: "Nudge right", group: "Piano roll", scopes: ["pianoRoll", "automation"], chords: ["ArrowRight", "Alt+ArrowRight"], handled: true, keywords: "move later" },
  { id: "pianoRoll.octaveUp", label: "Octave up", group: "Piano roll", scopes: ["pianoRoll"], chords: ["Shift+ArrowUp"], handled: true },
  { id: "pianoRoll.octaveDown", label: "Octave down", group: "Piano roll", scopes: ["pianoRoll"], chords: ["Shift+ArrowDown"], handled: true },
  // Aliases the piano roll always accepted (Shift doesn't change a horizontal nudge):
  // piano-roll-only, so they don't clash with the automation lane's fine nudge.
  { id: "pianoRoll.nudgeLeftShift", label: "Nudge left (with Shift)", group: "Piano roll", scopes: ["pianoRoll"], chords: ["Shift+ArrowLeft"], handled: true },
  { id: "pianoRoll.nudgeRightShift", label: "Nudge right (with Shift)", group: "Piano roll", scopes: ["pianoRoll"], chords: ["Shift+ArrowRight"], handled: true },
  // Automation lane
  { id: "automation.fineUp", label: "Nudge up (fine)", group: "Automation", scopes: ["automation"], chords: ["Shift+ArrowUp", "Alt+Shift+ArrowUp"], handled: true },
  { id: "automation.fineDown", label: "Nudge down (fine)", group: "Automation", scopes: ["automation"], chords: ["Shift+ArrowDown", "Alt+Shift+ArrowDown"], handled: true },
  { id: "automation.fineLeft", label: "Nudge left (fine)", group: "Automation", scopes: ["automation"], chords: ["Shift+ArrowLeft", "Alt+Shift+ArrowLeft"], handled: true },
  { id: "automation.fineRight", label: "Nudge right (fine)", group: "Automation", scopes: ["automation"], chords: ["Shift+ArrowRight", "Alt+Shift+ArrowRight"], handled: true },
  // App
  { id: "palette.open", label: "Command palette", group: "App", scopes: G, chords: ["Mod+K"], handled: true, keywords: "search commands" },
  { id: "keymap.open", label: "Keyboard shortcuts…", group: "App", scopes: G, chords: [], palette: "keymap:open", keywords: "keymap bindings hotkeys" },
  { id: "project.save", label: "Save project", group: "App", scopes: G, chords: ["Mod+S"], handled: true },
  { id: "file.import", label: "Import audio…", group: "App", scopes: G, chords: ["Mod+I"], palette: "import:audio", handled: true },
  { id: "view.editor", label: "Show / hide the editor drawer", group: "View", scopes: G, chords: ["Mod+J"], ableton: ["Mod+Alt+L"], palette: "panel:drawer", handled: true },
  { id: "view.browser", label: "Open the browser", group: "View", scopes: G, chords: [], ableton: ["Mod+Alt+B"], palette: "panel:browser" },
  { id: "view.zoomToFit", label: "Zoom to fit", group: "View", scopes: G, chords: [], palette: "view:fit" },
  { id: "collab.focusChat", label: "Chat: focus input", group: "Collaboration", scopes: G, chords: ["Mod+Shift+M"], palette: "chat:focus", handled: true },
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
