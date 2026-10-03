// The providers the AI chat offers. Two wire formats: Anthropic's Messages API (SDK) and the
// OpenAI-compatible Chat Completions API (fetch + SSE), which covers everything else. Every
// preset's base URL and model can be changed in the settings; "custom" is any other
// OpenAI-compatible server.

export type ProviderKind = "anthropic" | "openai";

export const PROVIDER_IDS = [
  "anthropic",
  "openai",
  "deepseek",
  "gemini",
  "mistral",
  "groq",
  "openrouter",
  "together",
  "ollama",
  "lmstudio",
  "custom",
] as const;
export type ProviderId = (typeof PROVIDER_IDS)[number];

export interface ProviderPreset {
  id: ProviderId;
  label: string;
  kind: ProviderKind;
  group: "Cloud" | "Local" | "Other";
  baseUrl: string;
  /** Suggested models (the field is free text); the first is the default. */
  models: readonly string[];
  /** Cloud APIs need a key; local servers usually don't. */
  keyRequired: boolean;
  /** Placeholder of the key field. */
  keyHint: string;
  /** Where to get a key. */
  keyHelp: string;
  /** What to do when the browser blocks the request (CORS). */
  corsHelp?: string;
}

const PROXY_HELP = "Use a provider that accepts requests from web apps (OpenRouter does), or point the base URL at a proxy you run.";

export const PROVIDERS: Record<ProviderId, ProviderPreset> = {
  anthropic: {
    id: "anthropic",
    label: "Anthropic (Claude)",
    kind: "anthropic",
    group: "Cloud",
    baseUrl: "https://api.anthropic.com",
    models: ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-4-5-20251001"],
    keyRequired: true,
    keyHint: "sk-ant-…",
    keyHelp: "console.anthropic.com → API keys",
  },
  openai: {
    id: "openai",
    label: "OpenAI",
    kind: "openai",
    group: "Cloud",
    baseUrl: "https://api.openai.com/v1",
    models: ["gpt-5", "gpt-5-mini", "gpt-4.1", "gpt-4.1-mini"],
    keyRequired: true,
    keyHint: "sk-…",
    keyHelp: "platform.openai.com → API keys",
  },
  deepseek: {
    id: "deepseek",
    label: "DeepSeek",
    kind: "openai",
    group: "Cloud",
    baseUrl: "https://api.deepseek.com",
    models: ["deepseek-chat", "deepseek-reasoner"],
    keyRequired: true,
    keyHint: "sk-…",
    keyHelp: "platform.deepseek.com → API keys",
    corsHelp: PROXY_HELP,
  },
  gemini: {
    id: "gemini",
    label: "Google Gemini",
    kind: "openai",
    group: "Cloud",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    models: ["gemini-2.5-flash", "gemini-2.5-pro"],
    keyRequired: true,
    keyHint: "AIza…",
    keyHelp: "aistudio.google.com → Get API key",
    corsHelp: PROXY_HELP,
  },
  mistral: {
    id: "mistral",
    label: "Mistral",
    kind: "openai",
    group: "Cloud",
    baseUrl: "https://api.mistral.ai/v1",
    models: ["mistral-large-latest", "mistral-medium-latest", "mistral-small-latest"],
    keyRequired: true,
    keyHint: "API key",
    keyHelp: "console.mistral.ai → API keys",
    corsHelp: PROXY_HELP,
  },
  groq: {
    id: "groq",
    label: "Groq",
    kind: "openai",
    group: "Cloud",
    baseUrl: "https://api.groq.com/openai/v1",
    models: ["openai/gpt-oss-120b", "llama-3.3-70b-versatile", "moonshotai/kimi-k2-instruct"],
    keyRequired: true,
    keyHint: "gsk_…",
    keyHelp: "console.groq.com → API keys",
    corsHelp: PROXY_HELP,
  },
  openrouter: {
    id: "openrouter",
    label: "OpenRouter",
    kind: "openai",
    group: "Cloud",
    baseUrl: "https://openrouter.ai/api/v1",
    models: ["openai/gpt-5", "anthropic/claude-sonnet-4.5", "deepseek/deepseek-chat", "google/gemini-2.5-pro"],
    keyRequired: true,
    keyHint: "sk-or-…",
    keyHelp: "openrouter.ai → Keys",
    corsHelp: PROXY_HELP,
  },
  together: {
    id: "together",
    label: "Together AI",
    kind: "openai",
    group: "Cloud",
    baseUrl: "https://api.together.xyz/v1",
    models: ["meta-llama/Llama-3.3-70B-Instruct-Turbo", "Qwen/Qwen3-235B-A22B-Instruct-2507-tput", "deepseek-ai/DeepSeek-V3"],
    keyRequired: true,
    keyHint: "API key",
    keyHelp: "api.together.ai → Settings → API keys",
    corsHelp: PROXY_HELP,
  },
  ollama: {
    id: "ollama",
    label: "Ollama (local)",
    kind: "openai",
    group: "Local",
    baseUrl: "http://localhost:11434/v1",
    models: ["qwen3", "llama3.1", "mistral-nemo"],
    keyRequired: false,
    keyHint: "Not needed",
    keyHelp: "Ollama needs no key; pull a model that supports tools (ollama pull qwen3)",
    corsHelp: "Ollama refuses requests from this page's origin. Quit Ollama, set OLLAMA_ORIGINS to this origin (or *), and start it again.",
  },
  lmstudio: {
    id: "lmstudio",
    label: "LM Studio (local)",
    kind: "openai",
    group: "Local",
    baseUrl: "http://localhost:1234/v1",
    models: ["qwen/qwen3-8b", "openai/gpt-oss-20b"],
    keyRequired: false,
    keyHint: "Not needed",
    keyHelp: "LM Studio needs no key; start its server (Developer → Start server) with a model loaded",
    corsHelp: "LM Studio refuses requests from web pages by default: turn on \"Enable CORS\" in its server settings.",
  },
  custom: {
    id: "custom",
    label: "Other (OpenAI-compatible)",
    kind: "openai",
    group: "Other",
    baseUrl: "http://localhost:8080/v1",
    models: [],
    keyRequired: false,
    keyHint: "Optional",
    keyHelp: "Any server with an OpenAI-compatible /chat/completions endpoint (vLLM, llama.cpp, LiteLLM, …)",
    corsHelp: "The server refuses requests from this page's origin: allow it in the server's CORS settings.",
  },
};

export const DEFAULT_PROVIDER: ProviderId = "anthropic";

export const isProviderId = (v: unknown): v is ProviderId => typeof v === "string" && (PROVIDER_IDS as readonly string[]).includes(v);

/** The provider picker's options (grouped: cloud, local, other). */
export const PROVIDER_OPTIONS = PROVIDER_IDS.map((id) => ({ value: id, label: PROVIDERS[id].label, group: PROVIDERS[id].group }));
