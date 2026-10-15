/**
 * The command palette as part of the registry: every palette command is an action, so a
 * command another feature adds to the palette (`buildCommands` in
 * `ui/src/app/shell/commands.ts`) appears in the keymap editor and the cheat sheet, and can
 * be bound, without being listed here.
 *
 * A palette command is the built-in action it links to (its `action` field; a built-in whose
 * `palette` is the command's id; or, for a command with a shortcut hint, the global built-in
 * whose default chord the hint is: e.g. a "Save" entry hinting ⌘S is `project.save`).
 * Otherwise it is its own action (id = the command id), whose default chord is its hint.
 * Such a hint is answered by the feature's own key handler (`legacy`); the dispatcher runs
 * the command on any other chord the user gives it.
 */

import { parseShortcutHint } from "./chords";
import { registeredAction, registeredActions, type KeymapAction } from "./registry";
import { currentPaletteActions, findAction, setPaletteActions, shortcutLabel } from "./store";

/** What the keymap needs from a palette command (`PaletteCommand` is one). */
export interface PaletteEntry {
  id: string;
  label: string;
  group: string;
  /** Display hint (`⇧⌘M`). */
  shortcut?: string;
  /** The registry action this command is. */
  action?: string;
  /** `false`: never bindable (per-project entries such as "Go to track …"). */
  bindable?: boolean;
  run(): void;
}

type Source = () => ReadonlyArray<PaletteEntry>;
let source: Source | null = null;

/** The palette registers how to list its commands (called by `commands.ts`). */
export function setPaletteSource(fn: Source | null): void {
  source = fn;
}

/** The palette commands available now ([] before the palette registered its source). */
export function paletteEntries(): ReadonlyArray<PaletteEntry> {
  try {
    return source?.() ?? [];
  } catch (e) {
    console.warn("[ethereal] keymap: listing palette commands failed:", e);
    return [];
  }
}

/** The built-in action a palette command is, if any. */
export function linkedAction(entry: PaletteEntry): KeymapAction | undefined {
  if (entry.action) return registeredAction(entry.action);
  const all = registeredActions();
  const byId = all.find((a) => a.id === entry.id || a.palette === entry.id);
  if (byId) return byId;
  const hint = entry.shortcut ? parseShortcutHint(entry.shortcut) : null;
  if (!hint) return undefined;
  return all.find((a) => a.handled && a.scopes.includes("global") && a.chords.includes(hint));
}

/** The keymap action id of a palette command (its link, or its own id). */
export function paletteActionId(entry: PaletteEntry): string {
  return linkedAction(entry)?.id ?? entry.id;
}

/**
 * Derive the palette's own actions from `entries` (the commands not linked to a built-in)
 * and publish them to the keymap (lookups, editor, cheat sheet). Returns them.
 */
export function derivePaletteActions(entries: ReadonlyArray<PaletteEntry> = paletteEntries()): KeymapAction[] {
  const known = new Map(currentPaletteActions().map((a) => [a.id, a]));
  const out: KeymapAction[] = [];
  const seen = new Set<string>();
  for (const e of entries) {
    if (e.bindable === false || seen.has(e.id) || linkedAction(e)) continue;
    seen.add(e.id);
    const hint = e.shortcut ? parseShortcutHint(e.shortcut) : null;
    const chords = hint ? [hint] : [];
    out.push({
      id: e.id,
      // Keep the first label seen (toggles like "Pin browser" / "Unpin browser" flip).
      label: known.get(e.id)?.label ?? e.label,
      group: e.group,
      scopes: ["global"],
      chords,
      legacy: chords,
      palette: e.id,
    });
  }
  // Keep actions seen earlier but not available now (e.g. chat outside a session), so their
  // bindings stay listed and their legacy chords stay known.
  for (const [id, a] of known) if (!seen.has(id)) out.push(a);
  setPaletteActions(out);
  return out;
}

/**
 * The shortcut a palette row shows: its action's current first chord (none if the user
 * unbound it), or the command's own hint while the keymap doesn't know the command yet.
 */
export function paletteShortcut(entry: PaletteEntry): string | undefined {
  const id = paletteActionId(entry);
  return findAction(id) ? shortcutLabel(id) : entry.shortcut;
}
