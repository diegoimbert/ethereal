/**
 * The user's wheel/mouse settings (Settings > Input), kept on this device only
 * (localStorage; in memory where storage is unavailable). Defaults depend on the platform
 * (`defaultInputSettings`). Wheel handlers read `inputSettings()` at event time.
 *
 * Also tracks whether the Ctrl key is physically down, to tell a trackpad pinch (which
 * browsers report as ctrl + wheel) from a real Ctrl + wheel.
 */

import { create } from "zustand";
import {
  defaultInputSettings,
  detectPlatform,
  sanitizeInputSettings,
  type InputSettings,
  type Platform,
} from "./wheelInput";

const KEY = "eth-input-settings";

export const PLATFORM: Platform = detectPlatform();

function load(): InputSettings {
  const defaults = defaultInputSettings(PLATFORM);
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? sanitizeInputSettings(JSON.parse(raw), defaults) : defaults;
  } catch {
    return defaults;
  }
}

function save(s: InputSettings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* storage unavailable: keep it for this session */
  }
}

interface InputSettingsState {
  settings: InputSettings;
  set(patch: Partial<InputSettings>): void;
  /** Back to this platform's defaults. */
  reset(): void;
}

export const useInputSettings = create<InputSettingsState>()((set, get) => ({
  settings: load(),
  set: (patch) => {
    const settings = sanitizeInputSettings({ ...get().settings, ...patch }, defaultInputSettings(PLATFORM));
    save(settings);
    set({ settings });
  },
  reset: () => {
    const settings = defaultInputSettings(PLATFORM);
    try {
      localStorage.removeItem(KEY);
    } catch {
      /* ignore */
    }
    set({ settings });
  },
}));

/** Current settings (for event handlers). */
export function inputSettings(): InputSettings {
  return useInputSettings.getState().settings;
}

// ---- Physical Ctrl key -------------------------------------------------------------------

let ctrlDown = false;
let tracking = false;

function track(): void {
  if (tracking || typeof window === "undefined") return;
  tracking = true;
  const onKey = (e: KeyboardEvent) => {
    ctrlDown = e.ctrlKey;
  };
  window.addEventListener("keydown", onKey, { capture: true });
  window.addEventListener("keyup", onKey, { capture: true });
  window.addEventListener("blur", () => {
    ctrlDown = false;
  });
}
track();

/**
 * A trackpad pinch: browsers report it as a wheel event with `ctrlKey` set while the Ctrl
 * key isn't down (and no other modifier is: a gesture with Shift/Alt/Cmd held is a real
 * Ctrl + wheel). Pinch always zooms, unaffected by the zoom modifier or inversion.
 */
export function isPinch(e: { ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }): boolean {
  return e.ctrlKey && !ctrlDown && !e.shiftKey && !e.altKey && !e.metaKey;
}
