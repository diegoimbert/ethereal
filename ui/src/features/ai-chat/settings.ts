// AI chat settings, kept on this machine only (localStorage; the desktop app has no Tauri
// store plugin, and its webview's localStorage lives in the app data dir):
// - the provider, and per provider its base URL, model and API key (BYOK). A key is read only
//   to build requests to its own provider's base URL, never logged, never sent to the engine
//   or a collab peer, and stored apart from the other settings;
// - the tool-iteration cap per turn.
import { create } from "zustand";
import { DEFAULT_PROVIDER, isProviderId, PROVIDER_IDS, PROVIDERS, type ProviderId } from "./providers/catalog";
import type { Connection } from "./providers/types";

/** The Anthropic models (suggestions of the default provider). */
export const MODELS = PROVIDERS.anthropic.models.map((value) => ({ value, label: modelLabel(value) }));
export const DEFAULT_MODEL = PROVIDERS.anthropic.models[0]!;

export const DEFAULT_MAX_ITERATIONS = 25;
export const MAX_ITERATIONS_LIMIT = 200;

/** The Anthropic key keeps its original slot; the others get `eth.ai.apiKey.<provider>`. */
export const keyStorage = (p: ProviderId) => (p === "anthropic" ? "eth.ai.apiKey" : `eth.ai.apiKey.${p}`);
const SETTINGS_STORAGE = "eth.ai.settings";

export interface ProviderConfig {
  baseUrl: string;
  model: string;
}

interface Saved {
  provider?: string;
  /** Before providers: the Anthropic model. */
  model?: string;
  maxIterations?: number;
  providers?: Partial<Record<string, Partial<ProviderConfig>>>;
}

export interface AiSettingsState {
  provider: ProviderId;
  configs: Record<ProviderId, ProviderConfig>;
  keys: Record<ProviderId, string | null>;
  /** The active provider's key and model (mirrors of `keys` / `configs`). */
  apiKey: string | null;
  model: string;
  maxIterations: number;
  setProvider(provider: ProviderId): void;
  /** Sets the key of `provider` (default: the active one); `null` removes it. */
  setApiKey(key: string | null, provider?: ProviderId): void;
  setModel(model: string): void;
  setBaseUrl(url: string): void;
  setMaxIterations(n: number): void;
}

/** `claude-sonnet-5-5` → `Claude Sonnet 5.5`; other ids as they are. */
export function modelLabel(id: string): string {
  const m = /^claude-(\w+)-(\d+)-(\d+)/.exec(id);
  return m ? `Claude ${m[1]![0]!.toUpperCase()}${m[1]!.slice(1)} ${m[2]}.${m[3]}` : id;
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

/** Plausible key for `provider` (the API has the last word). */
export function looksLikeApiKey(key: string, provider: ProviderId = "anthropic"): boolean {
  const k = key.trim();
  if (provider === "anthropic") return /^sk-ant-[A-Za-z0-9_-]{16,}$/.test(k);
  if (provider === "openai" || provider === "deepseek") return /^sk-[A-Za-z0-9_-]{16,}$/.test(k);
  return /^\S{8,}$/.test(k);
}

/** `sk-ant-…a1b2` for display. */
export function maskKey(key: string): string {
  return key.length <= 12 ? "••••" : `${key.slice(0, 7)}…${key.slice(-4)}`;
}

export function clampIterations(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_MAX_ITERATIONS;
  return Math.min(MAX_ITERATIONS_LIMIT, Math.max(1, Math.round(n)));
}

/** A base URL as entered: trimmed, without trailing slashes. */
export function normalizeBaseUrl(url: string): string {
  return url.trim().replace(/\/+$/, "");
}

export const defaultConfig = (p: ProviderId): ProviderConfig => ({ baseUrl: PROVIDERS[p].baseUrl, model: PROVIDERS[p].models[0] ?? "" });

function initial(): Pick<AiSettingsState, "provider" | "configs" | "keys" | "apiKey" | "model" | "maxIterations"> {
  let saved: Saved = {};
  try {
    saved = JSON.parse(read(SETTINGS_STORAGE) ?? "{}") as Saved;
  } catch {
    /* defaults */
  }
  const configs = {} as Record<ProviderId, ProviderConfig>;
  const keys = {} as Record<ProviderId, string | null>;
  for (const p of PROVIDER_IDS) {
    const s = saved.providers?.[p] ?? {};
    const d = defaultConfig(p);
    configs[p] = {
      baseUrl: typeof s.baseUrl === "string" && s.baseUrl.trim() ? normalizeBaseUrl(s.baseUrl) : d.baseUrl,
      model: typeof s.model === "string" && s.model.trim() ? s.model.trim() : d.model,
    };
    keys[p] = normalizeKey(read(keyStorage(p)));
  }
  // Settings saved before providers existed: `model` was an Anthropic model.
  if (!saved.providers?.anthropic?.model && typeof saved.model === "string" && PROVIDERS.anthropic.models.includes(saved.model)) {
    configs.anthropic.model = saved.model;
  }
  const provider = isProviderId(saved.provider) ? saved.provider : DEFAULT_PROVIDER;
  return {
    provider,
    configs,
    keys,
    apiKey: keys[provider],
    model: configs[provider].model,
    maxIterations: saved.maxIterations !== undefined ? clampIterations(saved.maxIterations) : DEFAULT_MAX_ITERATIONS,
  };
}

export const useAiSettings = create<AiSettingsState>()((set, get) => {
  const saveSettings = () => {
    const { provider, configs, maxIterations } = get();
    write(SETTINGS_STORAGE, JSON.stringify({ provider, maxIterations, providers: configs }));
  };
  const patchConfig = (patch: Partial<ProviderConfig>) => {
    const { provider, configs } = get();
    const next = { ...configs, [provider]: { ...configs[provider], ...patch } };
    set({ configs: next, model: next[provider].model });
    saveSettings();
  };
  return {
    ...initial(),
    setProvider: (provider) => {
      const { keys, configs } = get();
      set({ provider, apiKey: keys[provider], model: configs[provider].model });
      saveSettings();
    },
    setApiKey: (key, provider = get().provider) => {
      const k = normalizeKey(key);
      write(keyStorage(provider), k);
      const keys = { ...get().keys, [provider]: k };
      set({ keys, apiKey: keys[get().provider] });
    },
    setModel: (model) => patchConfig({ model: model.trim() }),
    setBaseUrl: (url) => patchConfig({ baseUrl: normalizeBaseUrl(url) || PROVIDERS[get().provider].baseUrl }),
    setMaxIterations: (n) => {
      set({ maxIterations: clampIterations(n) });
      saveSettings();
    },
  };
});

/** Where requests of the active provider go. */
export function activeConnection(s: Pick<AiSettingsState, "provider" | "configs" | "keys"> = useAiSettings.getState()): Connection {
  const c = s.configs[s.provider];
  return { baseUrl: c.baseUrl, model: c.model, apiKey: s.keys[s.provider] };
}

/** What the active provider still lacks before a message can be sent (`null` = ready). */
export function missingSetup(s: Pick<AiSettingsState, "provider" | "configs" | "keys"> = useAiSettings.getState()): "key" | "model" | null {
  if (PROVIDERS[s.provider].keyRequired && !s.keys[s.provider]) return "key";
  if (!s.configs[s.provider].model) return "model";
  return null;
}

/** Tests: reload from storage. */
export function reloadAiSettings(): void {
  useAiSettings.setState(initial());
}
