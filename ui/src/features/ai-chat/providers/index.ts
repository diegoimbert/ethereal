// Picks the adapter for a provider.
import { AnthropicSession, testAnthropic } from "./anthropic";
import { PROVIDERS, type ProviderId, type ProviderPreset } from "./catalog";
import { OpenAiSession, testOpenAi, type OpenAiOptions } from "./openai";
import type { ChatSession, Connection, ToolDef } from "./types";

export * from "./catalog";
export * from "./types";

/** "Ollama (local)" → "Ollama" in messages. */
const options = (p: ProviderPreset): OpenAiOptions => ({
  label: p.id === "custom" ? "the server" : p.label.replace(/ \((local|Claude)\)$/, ""),
  corsHelp: p.corsHelp,
});

/** A new conversation with `provider`: same system prompt and tools for every provider. */
export function createSession(provider: ProviderId, system: string[], tools: ReadonlyArray<ToolDef>): ChatSession {
  const p = PROVIDERS[provider];
  return p.kind === "anthropic" ? new AnthropicSession(system, tools) : new OpenAiSession(system, tools, options(p));
}

/** "Test connection": resolves a short success line, rejects with a `ProviderError`. */
export function testConnection(provider: ProviderId, conn: Connection, signal?: AbortSignal): Promise<string> {
  const p = PROVIDERS[provider];
  return p.kind === "anthropic" ? testAnthropic(conn, signal) : testOpenAi(conn, options(p), signal);
}
