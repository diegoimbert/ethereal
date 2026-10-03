/**
 * Chords (CONTRACTS.md §13.12): modifiers in the order `Mod` (Cmd on macOS, Ctrl elsewhere),
 * `Ctrl` (the real Control key, macOS only), `Alt`, `Shift`, then one key, joined by `+`
 * (`Mod+Shift+D`). Keys: `A`-`Z`, `0`-`9`, `F1`-`F24`, named keys, unshifted punctuation.
 *
 * Off macOS `Ctrl` and `Mod` are the same key: chords are normalized to `Mod` for matching
 * and display. On macOS a Control chord that no action binds falls back to the `Mod` form,
 * like the hard-coded handlers did (they took Control or Cmd everywhere).
 */

export const MODIFIERS = ["Mod", "Ctrl", "Alt", "Shift"] as const;
export type Modifier = (typeof MODIFIERS)[number];

export const NAMED_KEYS = [
  "Space",
  "Enter",
  "Escape",
  "Backspace",
  "Delete",
  "Tab",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "Home",
  "End",
  "PageUp",
  "PageDown",
] as const;

export const PUNCTUATION = [",", ".", "/", ";", "'", "[", "]", "\\", "-", "=", "`"] as const;

export const MAX_CHORDS_PER_ACTION = 4;
export const MAX_KEYMAP_OVERRIDES = 1024;

export function isMacPlatform(): boolean {
  return typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
}

/** A valid chord (the engine checks the same rules on `Keymap::Set`). */
export function isValidChord(chord: string): boolean {
  const parts = chord.split("+");
  const key = parts.pop();
  if (key === undefined) return false;
  let next = 0;
  for (const m of parts) {
    const i = MODIFIERS.indexOf(m as Modifier, next);
    if (i < 0) return false;
    next = i + 1;
  }
  return isValidKey(key);
}

function isValidKey(key: string): boolean {
  if (key.length === 1) return /^[A-Z0-9]$/.test(key) || (PUNCTUATION as readonly string[]).includes(key);
  if ((NAMED_KEYS as readonly string[]).includes(key)) return true;
  const f = /^F([1-9]|1[0-9]|2[0-4])$/.exec(key);
  return f !== null;
}

/** `{ mods, key }` of a valid chord. */
export function splitChord(chord: string): { mods: Modifier[]; key: string } {
  const parts = chord.split("+");
  const key = parts.pop() ?? "";
  return { mods: parts as Modifier[], key };
}

function joinChord(mods: ReadonlyArray<Modifier>, key: string): string {
  return [...MODIFIERS.filter((m) => mods.includes(m)), key].join("+");
}

/** Off macOS, `Ctrl` is `Mod` (one key): the matching / display form. */
export function normalizeChord(chord: string, mac = isMacPlatform()): string {
  if (mac) return chord;
  const { mods, key } = splitChord(chord);
  if (!mods.includes("Ctrl")) return chord;
  return joinChord([...mods.filter((m) => m !== "Ctrl"), "Mod"], key);
}

/** `KeyboardEvent.code` → chord key, for keys whose `key` changes with Shift / Alt / layout. */
const CODE_KEYS: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
};

const KEY_ALIASES: Record<string, string> = { " ": "Space", Esc: "Escape", Del: "Delete", Up: "ArrowUp", Down: "ArrowDown", Left: "ArrowLeft", Right: "ArrowRight" };

/** The chord key of a key event, or null (a lone modifier, an unknown key). */
export function eventKey(e: { key: string; code?: string }): string | null {
  const k = KEY_ALIASES[e.key] ?? e.key;
  if (k.length === 1) {
    const up = k.toUpperCase();
    if (/^[A-Z0-9]$/.test(up)) return up;
    if ((PUNCTUATION as readonly string[]).includes(k)) return k;
  } else if (isValidKey(k)) {
    return k;
  }
  // Shifted punctuation / digits, Alt-composed characters (macOS): the physical key.
  const code = e.code ?? "";
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  return CODE_KEYS[code] ?? null;
}

/** What chord matching reads from a key event (DOM or React; `code` when known). */
export type ChordEvent = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey"> & { code?: string };

/**
 * The chords a key event stands for, most specific first: `[exact]`, plus on macOS with
 * Control (and not Cmd) held the `Mod` form as a fallback. Empty for a lone modifier.
 */
export function eventChords(e: ChordEvent, mac = isMacPlatform()): string[] {
  const key = eventKey(e);
  if (!key) return [];
  const tail: Modifier[] = [];
  if (e.altKey) tail.push("Alt");
  if (e.shiftKey) tail.push("Shift");
  if (!mac) return [joinChord([...(e.metaKey || e.ctrlKey ? (["Mod"] as const) : []), ...tail], key)];
  const head: Modifier[] = [];
  if (e.metaKey) head.push("Mod");
  if (e.ctrlKey) head.push("Ctrl");
  const exact = joinChord([...head, ...tail], key);
  return e.ctrlKey && !e.metaKey ? [exact, joinChord(["Mod", ...tail], key)] : [exact];
}

const MAC_GLYPHS: Record<Modifier, string> = { Mod: "⌘", Ctrl: "⌃", Alt: "⌥", Shift: "⇧" };
const PC_NAMES: Record<Modifier, string> = { Mod: "Ctrl", Ctrl: "Ctrl", Alt: "Alt", Shift: "Shift" };
const KEY_LABELS: Record<string, { mac: string; pc: string }> = {
  Space: { mac: "Space", pc: "Space" },
  Enter: { mac: "↩", pc: "Enter" },
  Escape: { mac: "Esc", pc: "Esc" },
  Backspace: { mac: "⌫", pc: "⌫" },
  Delete: { mac: "⌦", pc: "Del" },
  Tab: { mac: "⇥", pc: "Tab" },
  ArrowUp: { mac: "↑", pc: "↑" },
  ArrowDown: { mac: "↓", pc: "↓" },
  ArrowLeft: { mac: "←", pc: "←" },
  ArrowRight: { mac: "→", pc: "→" },
  PageUp: { mac: "PgUp", pc: "PgUp" },
  PageDown: { mac: "PgDn", pc: "PgDn" },
};

/**
 * How a chord reads on this platform: macOS glyphs in the system order (⌃⌥⇧⌘, `⇧⌘D`),
 * elsewhere the app's existing hint style (`⇧Ctrl+D`, `Alt+Ctrl+K`).
 */
export function formatChord(chord: string, mac = isMacPlatform()): string {
  const { mods, key } = splitChord(normalizeChord(chord, mac));
  const label = KEY_LABELS[key]?.[mac ? "mac" : "pc"] ?? key;
  const order: Modifier[] = ["Ctrl", "Alt", "Shift", "Mod"];
  const used = order.filter((m) => mods.includes(m));
  if (mac) return used.map((m) => MAC_GLYPHS[m]).join("") + label;
  // The app's existing hint style off macOS (`⇧Ctrl+L`, from `⇧${MOD_KEY}L`).
  return used.map((m) => (m === "Shift" ? "⇧" : `${PC_NAMES[m]}+`)).join("") + label;
}

/**
 * A palette / menu shortcut hint (`⇧⌘M`, `Ctrl+Shift+M`, `⌘I`, `Space`) as a chord, or
 * null if it does not read as one. Used for palette commands that only carry a display hint.
 */
export function parseShortcutHint(hint: string): string | null {
  let rest = hint.trim();
  const mods = new Set<Modifier>();
  const glyphs: Record<string, Modifier> = { "⌘": "Mod", "⌃": "Ctrl", "⌥": "Alt", "⇧": "Shift" };
  const words: Record<string, Modifier> = { cmd: "Mod", command: "Mod", mod: "Mod", ctrl: "Mod", control: "Mod", alt: "Alt", option: "Alt", opt: "Alt", shift: "Shift" };
  for (;;) {
    const g = glyphs[rest[0] ?? ""];
    if (g && rest.length > 1) {
      mods.add(g);
      rest = rest.slice(1);
      continue;
    }
    const w = /^([A-Za-z]+)\s*\+\s*/.exec(rest);
    if (w && words[w[1]!.toLowerCase()] && rest.length > w[0].length) {
      mods.add(words[w[1]!.toLowerCase()]!);
      rest = rest.slice(w[0].length);
      continue;
    }
    break;
  }
  const glyphKeys: Record<string, string> = { "⌫": "Backspace", "⌦": "Delete", "↩": "Enter", "⏎": "Enter", "⎋": "Escape", Esc: "Escape", Del: "Delete", "↑": "ArrowUp", "↓": "ArrowDown", "←": "ArrowLeft", "→": "ArrowRight", "⇥": "Tab" };
  let key = glyphKeys[rest] ?? rest;
  if (key.length === 1) key = key.toUpperCase();
  const chord = joinChord([...mods], key);
  return isValidChord(chord) ? chord : null;
}
