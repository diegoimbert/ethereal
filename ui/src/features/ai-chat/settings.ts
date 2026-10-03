// AI chat settings, kept on this machine only (localStorage; the desktop app has no Tauri
// store plugin, and its webview's localStorage lives in the app data dir):
// - the user's Anthropic API key (BYOK). It is read only to build the request to
//   api.anthropic.com, never logged, never sent to the engine or a collab peer;
// - the model and the tool-iteration cap per turn.
import { create } from "zustand";

export const MODELS = [
  { value: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" },
  { value: "claude-opus-5-5", label: "Claude Opus 5.5" },
  { value: "claude-haiku-4-5-20251001", label: "Claude Haiku 4.5" },
] as const;
export type ModelId = (typeof MODELS)[number]["value"];
export const DEFAULT_MODEL: ModelId = "claude-sonnet-5-5";

export const DEFAULT_MAX_ITERATIONS = 25;
export const MAX_ITERATIONS_LIMIT = 200;

const KEY_STORAGE = "eth.ai.apiKey";
const SETTINGS_STORAGE = "eth.ai.settings";

interface Saved {
  model?: string;
  maxIterations?: number;
}

export interface AiSettingsState {
  apiKey: string | null;
  model: ModelId;
  maxIterations: number;
  setApiKey(key: string | null): void;
  setModel(model: ModelId): void;
  setMaxIterations(n: number): void;
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string | null): void {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* not persisted (private mode, quota): kept for this session only */
  }
}

/** The key as entered, trimmed; `null` when empty. */
export function normalizeKey(key: string | null | undefined): string | null {
  const k = key?.trim() ?? "";
  return k ? k : null;
}

/** Plausible Anthropic key (the API has the last word). */
export function looksLikeApiKey(key: string): boolean {
  return /^sk-ant-[A-Za-z0-9_-]{16,}$/.test(key.trim());
}

/** `sk-ant-…a1b2` for display. */
export function maskKey(key: string): string {
  return key.length <= 12 ? "••••" : `${key.slice(0, 7)}…${key.slice(-4)}`;
}

export function clampIterations(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_MAX_ITERATIONS;
  return Math.min(MAX_ITERATIONS_LIMIT, Math.max(1, Math.round(n)));
}

function initial(): Pick<AiSettingsState, "apiKey" | "model" | "maxIterations"> {
  let saved: Saved = {};
  try {
    saved = JSON.parse(read(SETTINGS_STORAGE) ?? "{}") as Saved;
  } catch {
    /* defaults */
  }
  const model = MODELS.some((m) => m.value === saved.model) ? (saved.model as ModelId) : DEFAULT_MODEL;
  return {
    apiKey: normalizeKey(read(KEY_STORAGE)),
    model,
    maxIterations: saved.maxIterations !== undefined ? clampIterations(saved.maxIterations) : DEFAULT_MAX_ITERATIONS,
  };
}

export const useAiSettings = create<AiSettingsState>()((set, get) => {
  const saveSettings = () => {
    const { model, maxIterations } = get();
    write(SETTINGS_STORAGE, JSON.stringify({ model, maxIterations }));
  };
  return {
    ...initial(),
    setApiKey: (key) => {
      const k = normalizeKey(key);
      write(KEY_STORAGE, k);
      set({ apiKey: k });
    },
    setModel: (model) => {
      set({ model });
      saveSettings();
    },
    setMaxIterations: (n) => {
      set({ maxIterations: clampIterations(n) });
      saveSettings();
    },
  };
});

/** Tests: reload from storage. */
export function reloadAiSettings(): void {
  useAiSettings.setState(initial());
}
