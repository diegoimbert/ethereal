/**
 * Note preview (Ableton's "Preview" switch in the MIDI editor): pressing a note in the piano
 * roll plays it on its clip's track instrument for as long as the pointer is held.
 *
 * - Notes go to the engine as `Recording::PreviewNote` (CONTRACTS.md §13.17): only that
 *   track's instrument hears them, armed or not, playing or stopped; never recorded.
 * - One preview note at a time. `playPreview` again (a vertical drag reaching a new pitch)
 *   sends the previous note's off first, then the new note's on.
 * - No stuck notes: the note stops on pointer up / cancel anywhere in the window, window
 *   blur, the page being hidden, the toggle turned off, and the editor unmounting
 *   (`stopPreview`).
 * - The toolbar toggle (`usePreviewSetting`) is on by default and remembered per browser.
 */

import { create } from "zustand";
import type { TrackId } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";
import { midiVelocity } from "./noteEdits";

const STORAGE_KEY = "ethereal.pianoRoll.preview";
const NOTE_ON = 0x90;
const NOTE_OFF = 0x80;

function savedEnabled(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) !== "off";
  } catch {
    return true;
  }
}

export interface PreviewSetting {
  enabled: boolean;
  setEnabled(enabled: boolean): void;
}

/** The toolbar's preview toggle (default on, persisted). */
export const usePreviewSetting = create<PreviewSetting>((set) => ({
  enabled: savedEnabled(),
  setEnabled: (enabled) => {
    if (!enabled) stopPreview();
    set({ enabled });
    try {
      localStorage.setItem(STORAGE_KEY, enabled ? "on" : "off");
    } catch {
      /* not persisted */
    }
  },
}));

interface Held {
  transport: EngineTransport;
  track: TrackId;
  pitch: number;
}

let held: Held | null = null;

function send(transport: EngineTransport, track: TrackId, data: [number, number, number]): void {
  // Fire and forget: a failure (host without live MIDI, track gone) only loses the preview.
  transport.send(cmd("Recording", { type: "PreviewNote", track, data })).catch(() => undefined);
}

const onRelease = () => stopPreview();
const onHidden = () => {
  if (document.visibilityState === "hidden") stopPreview();
};

function listen(on: boolean): void {
  const method = on ? "addEventListener" : "removeEventListener";
  window[method]("pointerup", onRelease, true);
  window[method]("pointercancel", onRelease, true);
  window[method]("blur", onRelease);
  document[method]("visibilitychange", onHidden);
}

/**
 * Play `pitch` (0..127) at `velocity` (0..1, the note's) on `track` until the pointer is
 * released. No-op while the toggle is off, or if that very note is already sounding.
 */
export function playPreview(transport: EngineTransport, track: TrackId, pitch: number, velocity: number): void {
  if (!usePreviewSetting.getState().enabled) return;
  holdNote(transport, track, pitch, Math.max(1, midiVelocity(velocity)));
}

/**
 * Play `pitch` at MIDI `velocity` (1..127) on `track` until the pointer is released (or
 * `stopPreview`), whatever the piano roll's toggle says: drum pad audition (#435), whose
 * note goes through the track's chain to the pad like playback does.
 */
export function holdNote(transport: EngineTransport, track: TrackId, pitch: number, velocity: number): void {
  if (held && held.transport === transport && held.track === track && held.pitch === pitch) return;
  if (held) release(held);
  else listen(true);
  held = { transport, track, pitch };
  send(transport, track, [NOTE_ON, pitch, Math.min(127, Math.max(1, Math.round(velocity)))]);
}

/** Stop the sounding preview note, if any. */
export function stopPreview(): void {
  if (!held) return;
  release(held);
  held = null;
  listen(false);
}

function release(h: Held): void {
  send(h.transport, h.track, [NOTE_OFF, h.pitch, 0]);
}

/** The preview note sounding now (tests). */
export function heldPreview(): { track: TrackId; pitch: number } | null {
  return held && { track: held.track, pitch: held.pitch };
}
