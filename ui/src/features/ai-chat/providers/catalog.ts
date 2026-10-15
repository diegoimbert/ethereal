// The providers the AI chat offers. Two wire formats: Anthropic's Messages API (SDK) and the
// OpenAI-compatible Chat Completions API (fetch + SSE), which covers everything else. Every
// preset's base URL and model can be changed in the settings; "custom" is any other
// OpenAI-compatible server.

import { t, type MessageKey } from "@/i18n";

export type ProviderKind = "anthropic" | "openai";

export const PROVIDER_IDS = ["anthropic", "openai", "deepseek", "gemini", "mistral", "groq", "openrouter", "together", "ollama", "lmstudio", "custom"] as const;
export type ProviderId = (typeof PROVIDER_IDS)[number];

export interface ProviderPreset {
  id: ProviderId;
  /** The provider's name (a brand, as it writes it); the UI shows `providerName()`. */
  label: string;
  kind: ProviderKind;
  group: "cloud" | "local" | "other";
  baseUrl: string;
  /** Suggested models (the field is free text); the first is the default. */
  models: readonly string[];
  /** Cloud APIs need a key; local servers usually don't. */
  keyRequired: boolean;
  /** The key's format, the key field's placeholder ("sk-ant-…"); `""`: none (`keyPlaceholder()`). */
  keyHint: string;
  /** Local servers: the key field says "optional" rather than "not needed". */
  keyOptional?: boolean;
  /** Where to get a key: the provider's site and its own labels (`site`), or a message of ours. */
  keyHelp: { site: string } | { text: () => string };
  /** What to do when the browser blocks the request (CORS), in the current language. */
  corsHelp?: () => string;
  /**
   * `max_tokens` sent with each request, where the server's default is too small for long
   * batches of tool calls. Unset = the server's default.
   */
  maxTokens?: number;
}

const PROXY_HELP = () => t("ai_chat.provider.proxy_help", { example: "OpenRouter" });

export const PROVIDERS: Record<ProviderId, ProviderPreset> = {
  anthropic: {
    id: "anthropic",
    label: "Anthropic (Claude)", // i18n-ignore: a brand
    kind: "anthropic",
    group: "cloud",
    baseUrl: "https://api.anthropic.com",
    models: ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-4-5-20251001"],
    keyRequired: true,
    keyHint: "sk-ant-…",
    keyHelp: { site: "console.anthropic.com → API keys" },
  },
  openai: {
    id: "openai",
    label: "OpenAI", // i18n-ignore: a brand
    kind: "openai",
    group: "cloud",
    baseUrl: "https://api.openai.com/v1",
    models: ["gpt-5", "gpt-5-mini", "gpt-4.1", "gpt-4.1-mini"],
    keyRequired: true,
    keyHint: "sk-…",
    keyHelp: { site: "platform.openai.com → API keys" },
  },
  deepseek: {
    id: "deepseek",
    label: "DeepSeek", // i18n-ignore: a brand
    kind: "openai",
    group: "cloud",
    baseUrl: "https://api.deepseek.com",
    models: ["deepseek-chat", "deepseek-reasoner"],
    keyRequired: true,
    keyHint: "sk-…",
    keyHelp: { site: "platform.deepseek.com → API keys" },
    corsHelp: PROXY_HELP,
    // The default is 8K without thinking (64K with), and thinking counts against it; the
    // ceiling is 384K. A model with a lower ceiling refuses it: the adapter then retries with
    // the default.
    maxTokens: 65_536,
  },
  gemini: {
    id: "gemini",
    label: "Google Gemini", // i18n-ignore: a brand
    kind: "openai",
    group: "cloud",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    models: ["gemini-2.5-flash", "gemini-2.5-pro"],
    keyRequired: true,
    keyHint: "AIza…", // eth-allow-case: the key prefix
    keyHelp: { site: "aistudio.google.com → Get API key" }, // eth-allow-case: the site's own page names
    corsHelp: PROXY_HELP,
  },
  mistral: {
    id: "mistral",
    label: "Mistral", // i18n-ignore: a brand
    kind: "openai",
    group: "cloud",
    baseUrl: "https://api.mistral.ai/v1",
    models: ["mistral-large-latest", "mistral-medium-latest", "mistral-small-latest"],
    keyRequired: true,
    keyHint: "",
    keyHelp: { site: "console.mistral.ai → API keys" },
    corsHelp: PROXY_HELP,
  },
  groq: {
    id: "groq",
    label: "Groq", // i18n-ignore: a brand
    kind: "openai",
    group: "cloud",
    baseUrl: "https://api.groq.com/openai/v1",
    models: ["openai/gpt-oss-120b", "llama-3.3-70b-versatile", "moonshotai/kimi-k2-instruct"],
    keyRequired: true,
    keyHint: "gsk_…",
    keyHelp: { site: "console.groq.com → API keys" },
    corsHelp: PROXY_HELP,
  },
  openrouter: {
    id: "openrouter",
    label: "OpenRouter", // i18n-ignore: a brand
    kind: "openai",
    group: "cloud",
    baseUrl: "https://openrouter.ai/api/v1",
    models: ["openai/gpt-5", "anthropic/claude-sonnet-4.5", "deepseek/deepseek-chat", "google/gemini-2.5-pro"],
    keyRequired: true,
    keyHint: "sk-or-…",
    keyHelp: { site: "openrouter.ai → Keys" }, // eth-allow-case: the site's own page names
    corsHelp: PROXY_HELP,
  },
  together: {
    id: "together",
    label: "Together AI", // i18n-ignore: a brand; eth-allow-case: a brand
    kind: "openai",
    group: "cloud",
    baseUrl: "https://api.together.xyz/v1",
    models: ["meta-llama/Llama-3.3-70B-Instruct-Turbo", "Qwen/Qwen3-235B-A22B-Instruct-2507-tput", "deepseek-ai/DeepSeek-V3"],
    keyRequired: true,
    keyHint: "",
    keyHelp: { site: "api.together.ai → Settings → API keys" }, // eth-allow-case: the site's own page names
    corsHelp: PROXY_HELP,
  },
  ollama: {
    id: "ollama",
    label: "Ollama (local)", // i18n-ignore: a brand
    kind: "openai",
    group: "local",
    baseUrl: "http://localhost:11434/v1",
    models: ["qwen3", "llama3.1", "mistral-nemo"],
    keyRequired: false,
    keyHint: "",
    keyHelp: { text: () => t("ai_chat.provider.ollama_key_help", { name: "Ollama", command: "ollama pull qwen3" }) },
    corsHelp: () => t("ai_chat.provider.ollama_cors_help", { name: "Ollama", variable: "OLLAMA_ORIGINS" }),
  },
  lmstudio: {
    id: "lmstudio",
    label: "LM Studio (local)", // i18n-ignore: a brand
    kind: "openai",
    group: "local",
    baseUrl: "http://localhost:1234/v1",
    models: ["qwen/qwen3-8b", "openai/gpt-oss-20b"],
    keyRequired: false,
    keyHint: "",
    keyHelp: { text: () => t("ai_chat.provider.lmstudio_key_help", { name: "LM Studio", path: "Developer → Start server" }) }, // eth-allow-case: LM Studio's own menu names
    corsHelp: () => t("ai_chat.provider.lmstudio_cors_help", { name: "LM Studio", setting: "Enable CORS" }),
  },
  custom: {
    id: "custom",
    label: "other (OpenAI-compatible)", // i18n-ignore: English (the import resolver); the UI shows `providerName()`
    kind: "openai",
    group: "other",
    baseUrl: "http://localhost:8080/v1",
    models: [],
    keyRequired: false,
    keyHint: "",
    keyOptional: true,
    keyHelp: { text: () => t("ai_chat.provider.custom_key_help", { brand: "OpenAI", endpoint: "/chat/completions", examples: "vLLM, llama.cpp, LiteLLM" }) },
    corsHelp: () => t("ai_chat.provider.custom_cors_help"),
  },
};

export const DEFAULT_PROVIDER: ProviderId = "anthropic";

export const isProviderId = (v: unknown): v is ProviderId => typeof v === "string" && (PROVIDER_IDS as readonly string[]).includes(v);

const GROUPS: Record<ProviderPreset["group"], MessageKey> = {
  cloud: "ai_chat.provider.group_cloud",
  local: "ai_chat.provider.group_local",
  other: "ai_chat.provider.group_other",
};

/** The provider's name as the UI shows it ("Other (OpenAI-compatible)" in the current language). */
export const providerName = (id: ProviderId): string => (id === "custom" ? t("ai_chat.provider.custom", { brand: "OpenAI" }) : PROVIDERS[id].label);

/** The key field's placeholder: the key's format, else "API key", "not needed" or "optional". */
export function keyPlaceholder(p: ProviderPreset): string {
  if (p.keyHint) return p.keyHint;
  return t(p.keyRequired ? "ai_chat.key.label" : p.keyOptional ? "ai_chat.key.optional" : "ai_chat.key.not_needed");
}

/** Where to get a key, as the UI shows it. */
export const keyHelpText = (p: ProviderPreset): string => ("site" in p.keyHelp ? p.keyHelp.site : p.keyHelp.text());

/** The provider picker's options (grouped: cloud, local, other), in the current language; brands keep their case. */
export const providerOptions = () =>
  PROVIDER_IDS.map((id) => ({ value: id, label: providerName(id), group: t(GROUPS[PROVIDERS[id].group]), userText: id !== "custom" }));
