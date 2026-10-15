// `keyboard-midi` registers its own codes with `setEditorKeyCapture`.
/**
 * Keys from plugin editor windows (CONTRACTS.md §13.15). A plugin editor is a native window,
 * so the web view never sees keys pressed there; the host forwards the **captured** ones as
 * `PluginEvent::EditorKey` instead of giving them to the plugin:
 *
 * - `Space` is always captured (the transport handles it like a Space in the app window).
 * - A feature adds codes (`KeyboardEvent.code`: `"KeyA"`, `"Digit1"`) with
 *   `setEditorKeyCapture(transport, owner, codes)`; the module sends the union of every
 *   owner's codes as one `Plugin::SetEditorKeyCapture` (only when it changes; failures, e.g.
 *   on the web where plugins don't exist, are ignored).
 * - `onPluginWindowKey(transport, listener)` subscribes; `asKeyboardEvent(key)` turns one into
 *   what keymap matching (`matchesAction`) reads.
 *
 * A key is only captured without Ctrl / Alt / Cmd (plugin shortcuts keep working); a release
 * comes iff its press did (`down: false`).
 */

import type { DeviceId, PluginWindowKey } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";
import { splitChord, type ChordEvent } from "@/features/keymap/chords";

export type { PluginWindowKey };

/** Always captured by the host; never needs registering. */
export const ALWAYS_CAPTURED = "Space";

/** Subscribe to keys forwarded from plugin editor windows. Returns the unsubscribe. */
export function onPluginWindowKey(transport: EngineTransport, listener: (key: PluginWindowKey, device: DeviceId) => void): () => void {
  return transport.onEvent((e) => {
    if (e.type === "Plugin" && e.event.type === "EditorKey") listener(e.event.key, e.event.device);
  });
}

/** A forwarded key as the DOM-like event keymap matching reads. */
export function asKeyboardEvent(k: PluginWindowKey): ChordEvent & { repeat: boolean; type: "keydown" | "keyup" } {
  return {
    key: k.key,
    code: k.code,
    shiftKey: k.shift,
    ctrlKey: k.ctrl,
    altKey: k.alt,
    metaKey: k.meta,
    repeat: k.repeat,
    type: k.down ? "keydown" : "keyup",
  };
}

/**
 * The `KeyboardEvent.code`s a plugin window must capture for these chords: plain or
 * Shift chords only (the host never captures Ctrl / Alt / Cmd chords), letters, digits,
 * `Space` and the named / punctuation keys the host knows.
 */
export function chordCodes(chords: ReadonlyArray<string>): string[] {
  const out = new Set<string>();
  for (const chord of chords) {
    const { mods, key } = splitChord(chord);
    if (mods.some((m) => m !== "Shift")) continue;
    const code = keyCode(key);
    if (code) out.add(code);
  }
  return [...out].sort();
}

const PUNCT_CODES: Record<string, string> = {
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
};

function keyCode(key: string): string | null {
  if (/^[A-Z]$/.test(key)) return `Key${key}`; // eth-allow-case: KeyboardEvent.code values
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  if (PUNCT_CODES[key]) return PUNCT_CODES[key]!;
  if (["Space", "Enter", "Escape", "Backspace", "Tab", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(key)) return key;
  return null;
}

interface CaptureState {
  owners: Map<string, ReadonlyArray<string>>;
  /** The last set sent (sorted, joined), to send only changes. */
  sent: string;
}

const captures = new WeakMap<EngineTransport, CaptureState>();

/** The union of every owner's codes, without `Space` (always captured). */
function union(state: CaptureState): string[] {
  const all = new Set<string>();
  for (const codes of state.owners.values()) for (const c of codes) if (c !== ALWAYS_CAPTURED) all.add(c);
  return [...all].sort();
}

/**
 * Set the codes `owner` wants captured in plugin editor windows (replacing its previous
 * ones; `[]` removes them). Sends the union of all owners when it changed.
 */
export function setEditorKeyCapture(transport: EngineTransport, owner: string, codes: ReadonlyArray<string>): void {
  let state = captures.get(transport);
  if (!state) {
    state = { owners: new Map(), sent: "" };
    captures.set(transport, state);
  }
  if (codes.length) state.owners.set(owner, [...codes]);
  else state.owners.delete(owner);
  const next = union(state);
  const key = next.join(",");
  if (key === state.sent) return;
  state.sent = key;
  transport.send(cmd("Plugin", { type: "SetEditorKeyCapture", codes: next })).catch(() => {
    // No plugin host (web, mock): nothing to capture.
  });
}

/** The codes currently requested on `transport` besides Space (tests). */
export function requestedCaptureCodes(transport: EngineTransport): string[] {
  const state = captures.get(transport);
  return state ? union(state) : [];
}
