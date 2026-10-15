/**
 * Customizable keyboard shortcuts (CONTRACTS.md §13.12): the action registry, the presets
 * "Ethereal" (default) and "Ableton-like", conflicts, the editor and the printable cheat sheet.
 *
 * Key handlers ask `matchesAction("<action id>", e)` instead of testing keys. The app shell
 * mounts `KeymapRoot` once; the command palette registers its commands with
 * `setPaletteSource` (every palette command is an action).
 */
export { eventChords, formatChord, isValidChord, normalizeChord, parseShortcutHint, type ChordEvent } from "./chords";
export { findConflicts, scopesOverlap, type Conflict } from "./conflicts";
export { dispatchPaused, pauseDispatch } from "./dispatcher";
export { KeymapRoot } from "./KeymapRoot";
export { derivePaletteActions, paletteActionId, paletteEntries, paletteShortcut, setPaletteSource, type PaletteEntry } from "./palette";
export { printCheatSheet } from "./print";
export { ShortcutHint } from "./ShortcutHint";
export { installScopeTracker, resetScopeTracker, runAction } from "./run";
export { taggedAction } from "./tag";
export { actionLabel, groupLabel, scopesLabel } from "./groups";
export { BUILTIN_ACTIONS, registerActions, type KeymapAction, type KeymapScope } from "./registry";
export { chordsFor, closeKeymapEditor, firstMatch, matchesAction, openKeymapEditor, shortcutLabel, useKeymapStore, useShortcutLabel } from "./store";
