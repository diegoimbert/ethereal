/**
 * The computer MIDI keyboard (Ableton's "Computer MIDI Keyboard"): while it is on, the
 * home row plays notes (`notes.ts`), Z/X shift the octave and C/V the velocity.
 *
 * Notes go to the engine as `Recording::LiveMidi` (CONTRACTS.md §13.16): exactly like a
 * hardware MIDI input, every monitored MIDI track plays them and armed MIDI tracks record
 * them; MIDI capture keeps them too. When no MIDI track would hear them, turning the keyboard
 * on (or playing) arms the selected MIDI track (Ableton's auto-arm).
 *
 * Keys:
 * - one `keydown`/`keyup` listener pair on `window` in the capture phase, installed when this
 *   module loads (before the keymap's dispatcher), so a note key never reaches another
 *   shortcut while the keyboard is on; keys with Ctrl/Cmd/Alt still do (Cmd+Z, Ctrl+C…);
 *   Space stays play/stop. Typing in a text field is left alone.
 * - auto-repeat is ignored; every key up sends its note off (the note it started, even if
 *   the octave changed meanwhile); window blur, the page being hidden, turning the keyboard
 *   off and a transport change release every held note (no stuck notes).
 * - native plugin editor windows forward their keys (`handleForwardedKey`, from the
 *   `transport-keys` channel) while the keyboard is on, so it keeps playing inside a plugin.
 */
import { create } from "zustand";
import type { Project, Track, TrackId } from "@/generated";
import { captureTargetTrack } from "@/features/capture/actions";
import { chordsFor, matchesAction, registerActions, useKeymapStore } from "@/features/keymap";
import { KEYBOARD_MIDI_ACTION } from "./action";
import { chordCodes, onPluginWindowKey, setEditorKeyCapture, type PluginWindowKey } from "@/features/plugin-keys";
import { isTextEntry } from "@/features/transport-bar/engine";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { clampOctave, clampVelocity, CONTROL_KEYS, DEFAULT_OCTAVE, DEFAULT_VELOCITY, NOTE_KEYS, NOTE_OFF, NOTE_ON, noteFor, stepVelocity } from "./notes";

/** Keymap action of the on/off toggle (default `M`, like Ableton). */
export const TOGGLE_ACTION = KEYBOARD_MIDI_ACTION.id;

registerActions([KEYBOARD_MIDI_ACTION]);

export interface KeyboardMidiState {
  enabled: boolean;
  /** Ableton naming: octave 3 starts at C3 = MIDI 60. */
  octave: number;
  velocity: number;
  /** Held keys (`KeyboardEvent.code`) → the note each one started. */
  held: Readonly<Record<string, number>>;
}

const STORAGE_KEY = "eth-keyboard-midi";

function load(): KeyboardMidiState {
  const state: KeyboardMidiState = { enabled: false, octave: DEFAULT_OCTAVE, velocity: DEFAULT_VELOCITY, held: {} };
  try {
    const raw = typeof localStorage === "undefined" ? null : localStorage.getItem(STORAGE_KEY);
    const saved = raw ? (JSON.parse(raw) as Partial<KeyboardMidiState>) : {};
    if (typeof saved.enabled === "boolean") state.enabled = saved.enabled;
    if (typeof saved.octave === "number") state.octave = clampOctave(saved.octave);
    if (typeof saved.velocity === "number") state.velocity = clampVelocity(saved.velocity);
  } catch {
    /* defaults */
  }
  return state;
}

export const useKeyboardMidi = create<KeyboardMidiState>()(() => load());

useKeyboardMidi.subscribe((s, prev) => {
  if (s.enabled === prev.enabled && s.octave === prev.octave && s.velocity === prev.velocity) return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ enabled: s.enabled, octave: s.octave, velocity: s.velocity }));
  } catch {
    /* not persisted */
  }
});

// ---- Engine ----------------------------------------------------------------------------------

let transport: EngineTransport | null = null;
let unsubscribePluginKeys: (() => void) | null = null;

/** `setEditorKeyCapture` owner of the keys plugin windows forward while the keyboard is on. */
const CAPTURE_OWNER = "keyboard-midi";
const KEYBOARD_CODES = [...Object.keys(NOTE_KEYS), ...Object.keys(CONTROL_KEYS)];

/**
 * The codes native plugin editor windows forward to the app: while the keyboard is on, its
 * keys and the toggle's chords (so it can be turned off from inside a plugin); none while
 * it is off, so plugins keep receiving their own typing.
 */
export function pluginCaptureCodes(enabled: boolean): string[] {
  return enabled ? [...new Set([...KEYBOARD_CODES, ...chordCodes(chordsFor(TOGGLE_ACTION))])].sort() : [];
}

function syncPluginCapture(): void {
  if (transport) setEditorKeyCapture(transport, CAPTURE_OWNER, pluginCaptureCodes(useKeyboardMidi.getState().enabled));
}

useKeyboardMidi.subscribe((s, prev) => {
  if (s.enabled !== prev.enabled) syncPluginCapture();
});
useKeymapStore.subscribe((s, prev) => {
  if (s.keymap !== prev.keymap) syncPluginCapture();
});

/**
 * Where notes go (the app's transport; the toggle binds it), and where keys from plugin
 * editor windows come from. Returns the unbind.
 */
export function bindTransport(t: EngineTransport | null): () => void {
  if (t !== transport) {
    releaseAll();
    if (transport) setEditorKeyCapture(transport, CAPTURE_OWNER, []);
    unsubscribePluginKeys?.();
    unsubscribePluginKeys = null;
    transport = t;
    if (t) {
      unsubscribePluginKeys = onPluginWindowKey(t, (key) => void handleForwardedKey(fromPluginWindow(key)));
      syncPluginCapture();
    }
  }
  return () => {
    if (transport === t) bindTransport(null);
  };
}

function fromPluginWindow(k: PluginWindowKey): KeyInput {
  return { code: k.code, key: k.key, down: k.down, repeat: k.repeat, shiftKey: k.shift, ctrlKey: k.ctrl, altKey: k.alt, metaKey: k.meta };
}

function send(data: [number, number, number]): void {
  // Fire and forget: a failure (no project, host without live MIDI) only loses the note.
  transport?.send(cmd("Recording", { type: "LiveMidi", data })).catch(() => undefined);
}

/** A MIDI track hearing live input (monitor In, or armed with monitor Auto: the engine's rule). */
function listening(t: Track, armed: ReadonlyArray<TrackId>): boolean {
  return t.kind === "Midi" && (t.monitor === "In" || (t.monitor === "Auto" && armed.includes(t.id)));
}

/** Whether some MIDI track hears live input. */
export function hasListeningTrack(project: Project, armed: ReadonlyArray<TrackId>): boolean {
  return Object.values(project.tracks).some((t) => listening(t, armed));
}

/**
 * The MIDI track the keyboard plays: the first one hearing live input, else the one it would
 * auto-arm (the selected MIDI track, else the first MIDI track).
 */
export function targetTrack(project: Project, armed: ReadonlyArray<TrackId>): Track | undefined {
  return Object.values(project.tracks).find((t) => listening(t, armed)) ?? captureTargetTrack(project, armed);
}

let armRequested: TrackId | null = null;

/** Arm the selected MIDI track (Ableton's auto-arm) when no MIDI track would hear notes. */
export function ensureListeningTrack(): void {
  const { project, armedTracks } = useProjectStore.getState();
  if (!transport || !project || hasListeningTrack(project, armedTracks)) return;
  const target = captureTargetTrack(project, armedTracks);
  if (!target || target.monitor === "Off" || armRequested === target.id) return;
  armRequested = target.id;
  transport
    .send(cmd("Recording", { type: "Arm", track: target.id, armed: true, exclusive: false }))
    .catch(() => undefined)
    .finally(() => {
      armRequested = null;
    });
}

// ---- Actions ---------------------------------------------------------------------------------

export function setEnabled(enabled: boolean): void {
  if (!enabled) releaseAll();
  useKeyboardMidi.setState({ enabled });
  if (enabled) ensureListeningTrack();
}

export function toggleKeyboardMidi(): void {
  setEnabled(!useKeyboardMidi.getState().enabled);
}

export function shiftOctave(dir: -1 | 1): void {
  useKeyboardMidi.setState((s) => ({ octave: clampOctave(s.octave + dir) }));
}

export function shiftVelocity(dir: -1 | 1): void {
  useKeyboardMidi.setState((s) => ({ velocity: stepVelocity(s.velocity, dir) }));
}

/** Start the note of a note key (ignored if that key is already held). */
export function noteOn(code: string): void {
  const s = useKeyboardMidi.getState();
  if (s.held[code] !== undefined) return;
  const note = noteFor(code, s.octave);
  if (note === null) return;
  ensureListeningTrack();
  useKeyboardMidi.setState({ held: { ...s.held, [code]: note } });
  send([NOTE_ON, note, s.velocity]);
}

/** End the note a key started (if it is held). */
export function noteOff(code: string): void {
  const s = useKeyboardMidi.getState();
  const note = s.held[code];
  if (note === undefined) return;
  const held = { ...s.held };
  delete held[code];
  useKeyboardMidi.setState({ held });
  send([NOTE_OFF, note, 0]);
}

/** Note off for every held key. */
export function releaseAll(): void {
  for (const code of Object.keys(useKeyboardMidi.getState().held)) noteOff(code);
}

// ---- Keys ------------------------------------------------------------------------------------

/** A key press, from the DOM or forwarded from a native plugin window. */
export interface KeyInput {
  code: string;
  key: string;
  down: boolean;
  repeat?: boolean;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

/**
 * Handle one key; returns `true` when the keyboard used it (the caller then keeps it from
 * every other handler).
 */
export function handleKey(k: KeyInput): boolean {
  if (k.down && !k.repeat && matchesAction(TOGGLE_ACTION, k)) {
    toggleKeyboardMidi();
    return true;
  }
  const s = useKeyboardMidi.getState();
  if (!k.down) {
    // Always end a held note (even if a modifier went down meanwhile).
    const held = s.held[k.code] !== undefined;
    if (held) noteOff(k.code);
    return held || (s.enabled && plain(k) && (k.code in NOTE_KEYS || k.code in CONTROL_KEYS));
  }
  if (!s.enabled || !plain(k)) return false;
  if (k.code in NOTE_KEYS) {
    if (!k.repeat) noteOn(k.code);
    return true;
  }
  const control = CONTROL_KEYS[k.code];
  if (control === undefined) return false;
  if (!k.repeat) {
    if (control === "octaveDown") shiftOctave(-1);
    else if (control === "octaveUp") shiftOctave(1);
    else if (control === "velocityDown") shiftVelocity(-1);
    else shiftVelocity(1);
  }
  return true;
}

/** No Ctrl / Cmd / Alt / Shift: those keep going to the app's shortcuts. */
function plain(k: KeyInput): boolean {
  return !k.ctrlKey && !k.metaKey && !k.altKey && !k.shiftKey;
}

/** Elements whose own keys win (typing, menu type-ahead). */
function ownsKeys(target: EventTarget | null): boolean {
  if (isTextEntry(target)) return true;
  return target instanceof Element && target.closest('[role="menu"], [role="listbox"]') !== null;
}

function onDomKey(e: KeyboardEvent): void {
  if (e.type === "keydown" && ownsKeys(e.target)) return;
  if (e.type === "keydown" && e.defaultPrevented) return;
  const used = handleKey({
    code: e.code,
    key: e.key,
    down: e.type === "keydown",
    repeat: e.repeat,
    shiftKey: e.shiftKey,
    ctrlKey: e.ctrlKey,
    altKey: e.altKey,
    metaKey: e.metaKey,
  });
  if (used) {
    e.preventDefault();
    e.stopImmediatePropagation();
  }
}

/**
 * A key from a native plugin editor window (`transport-keys` forwarding). Same rules as an
 * app key; returns whether the keyboard used it.
 */
export function handleForwardedKey(k: KeyInput): boolean {
  return handleKey(k);
}

let installed = false;

/** Install the window listeners (idempotent; done when this module loads). */
export function installKeyboardMidi(): void {
  if (installed || typeof window === "undefined") return;
  installed = true;
  window.addEventListener("keydown", onDomKey, { capture: true });
  window.addEventListener("keyup", onDomKey, { capture: true });
  window.addEventListener("blur", releaseAll);
  window.addEventListener("pagehide", releaseAll);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") releaseAll();
  });
}

installKeyboardMidi();
