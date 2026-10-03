/**
 * The user's keymap (`Keymap { preset, overrides }`, stored by the engine in the user library)
 * and the lookups key handlers use.
 *
 * - `matchesAction(id, e)`: does this key event trigger action `id`? (the one line key
 *   handlers use instead of testing keys themselves)
 * - `chordsFor(id)` / `shortcutLabel(id)`: the action's current chords, for hints.
 * - `loadKeymap` / `saveKeymap` / `resetKeymap`: `Keymap::{Get, Set, Reset}`.
 */

import { create } from "zustand";
import type { KeyBinding, Keymap, KeymapPreset } from "@/generated";
import { cmd, isCommandFailed, type EngineTransport } from "@/transport";
import { eventChords, formatChord, isMacPlatform, MAX_KEYMAP_OVERRIDES, normalizeChord, type ChordEvent } from "./chords";
import { registeredAction, registeredActions, subscribeRegistry, type KeymapAction } from "./registry";

export const DEFAULT_KEYMAP: Keymap = { preset: "Ethereal", overrides: [] };

export const PRESET_LABELS: Record<KeymapPreset, string> = { Ethereal: "Ethereal", AbletonLike: "Ableton-like" };

export interface KeymapState {
  keymap: Keymap;
  /** The engine stores the keymap (false: `Unsupported` / no engine; changes last the session). */
  stored: boolean;
  /** Last save error, if any. */
  error: string | null;
  editorOpen: boolean;
  /** Bumped when the palette-derived actions change (see `setPaletteActions`). */
  paletteVersion: number;
}

export const useKeymapStore = create<KeymapState>()(() => ({
  keymap: DEFAULT_KEYMAP,
  stored: false,
  error: null,
  editorOpen: false,
  paletteVersion: 0,
}));

export function openKeymapEditor(): void {
  useKeymapStore.setState({ editorOpen: true });
}

export function closeKeymapEditor(): void {
  useKeymapStore.setState({ editorOpen: false });
}

/** An action's chords in a built-in preset (Ableton-like adds to Ethereal, never removes). */
export function presetChords(action: KeymapAction, preset: KeymapPreset): string[] {
  const base = [...action.chords];
  if (preset === "AbletonLike") for (const c of action.ableton ?? []) if (!base.includes(c)) base.push(c);
  return base;
}

/** An action's chords in `keymap`: the user's override, else the preset's. */
export function effectiveChords(action: KeymapAction, keymap: Keymap): string[] {
  const o = keymap.overrides.find((b) => b.action === action.id);
  return o ? [...o.chords] : presetChords(action, keymap.preset);
}

// ---- Palette-derived actions (kept here so lookups see them; see registry.ts) ----------

let paletteActions: ReadonlyArray<KeymapAction> = [];

/** The palette's own actions, as last derived from the command list (`palette.ts`). */
export function currentPaletteActions(): ReadonlyArray<KeymapAction> {
  return paletteActions;
}

export function setPaletteActions(actions: ReadonlyArray<KeymapAction>): void {
  const same =
    actions.length === paletteActions.length &&
    actions.every((a, i) => a.id === paletteActions[i]!.id && a.label === paletteActions[i]!.label && a.chords.join() === paletteActions[i]!.chords.join());
  paletteActions = actions;
  if (!same) {
    index = null;
    useKeymapStore.setState((s) => ({ paletteVersion: s.paletteVersion + 1 }));
  }
}

/** Every known action: built-in, registered and palette-derived. */
export function allActions(): KeymapAction[] {
  const out = registeredActions();
  const ids = new Set(out.map((a) => a.id));
  for (const a of paletteActions) if (!ids.has(a.id)) out.push(a);
  return out;
}

export function findAction(id: string): KeymapAction | undefined {
  return registeredAction(id) ?? paletteActions.find((a) => a.id === id);
}

// ---- Lookup index ------------------------------------------------------------------------

interface Index {
  keymap: Keymap;
  /** action id → normalized chords. */
  chords: Map<string, Set<string>>;
  /** Every normalized chord some action binds. */
  bound: Set<string>;
}

let index: Index | null = null;
subscribeRegistry(() => {
  index = null;
});

function currentIndex(): Index {
  const keymap = useKeymapStore.getState().keymap;
  if (index && index.keymap === keymap) return index;
  const mac = isMacPlatform();
  const chords = new Map<string, Set<string>>();
  const bound = new Set<string>();
  for (const a of allActions()) {
    const set = new Set(effectiveChords(a, keymap).map((c) => normalizeChord(c, mac)));
    chords.set(a.id, set);
    for (const c of set) bound.add(c);
  }
  // Overrides for actions not known right now (a palette command that is not available).
  for (const o of keymap.overrides) {
    if (chords.has(o.action)) continue;
    for (const c of o.chords) bound.add(normalizeChord(c, mac));
  }
  index = { keymap, chords, bound };
  return index;
}

/**
 * Whether key event `e` triggers action `id` with the current keymap. On macOS a Control
 * chord nothing binds also tries its Cmd form (the old handlers took either).
 */
export function matchesAction(id: string, e: ChordEvent): boolean {
  const [exact, fallback] = eventChords(e);
  if (!exact) return false;
  const ix = currentIndex();
  const mine = ix.chords.get(id);
  if (!mine) return false;
  if (mine.has(exact)) return true;
  return fallback !== undefined && !ix.bound.has(exact) && mine.has(fallback);
}

/** The first of `ids` that `e` triggers, or null. */
export function firstMatch<T extends string>(ids: ReadonlyArray<T>, e: ChordEvent): T | null {
  for (const id of ids) if (matchesAction(id, e)) return id;
  return null;
}

/** Whether some action binds the chord of `e` (exactly). */
export function isBound(e: ChordEvent): boolean {
  const [exact] = eventChords(e);
  return exact !== undefined && currentIndex().bound.has(exact);
}

/** The action's current chords (raw, as stored). */
export function chordsFor(id: string): string[] {
  const a = findAction(id);
  if (a) return effectiveChords(a, useKeymapStore.getState().keymap);
  return useKeymapStore.getState().keymap.overrides.find((o) => o.action === id)?.chords.slice() ?? [];
}

/** The action's first chord as shown on this platform (`⇧⌘D`, `Ctrl+Shift+D`), or undefined. */
export function shortcutLabel(id: string): string | undefined {
  const [first] = chordsFor(id);
  return first === undefined ? undefined : formatChord(first);
}

/** Re-render on keymap changes and return `shortcutLabel(id)`. */
export function useShortcutLabel(id: string): string | undefined {
  useKeymapStore((s) => s.keymap);
  useKeymapStore((s) => s.paletteVersion);
  return shortcutLabel(id);
}

// ---- Editing -------------------------------------------------------------------------------

/**
 * `keymap` with action `id` bound to `chords` (`null`: back to the preset). An override equal
 * to the preset's chords is dropped; overrides stay sorted and unique.
 */
export function withBinding(keymap: Keymap, id: string, chords: ReadonlyArray<string> | null): Keymap {
  const rest = keymap.overrides.filter((o) => o.action !== id);
  const action = findAction(id);
  const preset = action ? presetChords(action, keymap.preset) : [];
  const unique = chords ? [...new Set(chords)] : null;
  const overrides: KeyBinding[] = unique && !(action && sameChords(unique, preset)) ? [...rest, { action: id, chords: unique }] : rest;
  return { preset: keymap.preset, overrides: sortOverrides(overrides) };
}

/** `keymap` on another preset; overrides that now equal the preset are dropped. */
export function withPreset(keymap: Keymap, preset: KeymapPreset): Keymap {
  const overrides = keymap.overrides.filter((o) => {
    const a = findAction(o.action);
    return !a || !sameChords(o.chords, presetChords(a, preset));
  });
  return { preset, overrides };
}

function sameChords(a: ReadonlyArray<string>, b: ReadonlyArray<string>): boolean {
  return a.length === b.length && a.every((c, i) => c === b[i]);
}

function sortOverrides(o: KeyBinding[]): KeyBinding[] {
  return o.sort((a, b) => (a.action < b.action ? -1 : a.action > b.action ? 1 : 0)).slice(0, MAX_KEYMAP_OVERRIDES);
}

// ---- Engine --------------------------------------------------------------------------------

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message || e.error.code;
  return e instanceof Error ? e.message : String(e);
}

/** `Keymap::Get` into the store (the default when the engine has none or can't say). */
export async function loadKeymap(transport: EngineTransport): Promise<void> {
  try {
    const reply = await transport.send(cmd("Keymap", { type: "Get" }));
    if (reply.type === "Keymap") useKeymapStore.setState({ keymap: reply.keymap, stored: true, error: null });
  } catch {
    useKeymapStore.setState({ stored: false });
  }
}

/** Apply `keymap` now and store it (`Keymap::Set`); without an engine it lasts the session. */
export async function saveKeymap(transport: EngineTransport | null, keymap: Keymap): Promise<void> {
  const before = useKeymapStore.getState().keymap;
  useKeymapStore.setState({ keymap, error: null });
  if (!transport || !useKeymapStore.getState().stored) return;
  try {
    await transport.send(cmd("Keymap", { type: "Set", keymap }));
  } catch (e) {
    useKeymapStore.setState({ keymap: before, error: errorText(e) });
  }
}

/** Back to the default keymap (`Keymap::Reset`). */
export async function resetKeymap(transport: EngineTransport | null): Promise<void> {
  const before = useKeymapStore.getState().keymap;
  useKeymapStore.setState({ keymap: DEFAULT_KEYMAP, error: null });
  if (!transport || !useKeymapStore.getState().stored) return;
  try {
    await transport.send(cmd("Keymap", { type: "Reset" }));
  } catch (e) {
    useKeymapStore.setState({ keymap: before, error: errorText(e) });
  }
}
