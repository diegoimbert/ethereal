// Anthropic adapter: the Messages API through @anthropic-ai/sdk (streaming, tool use).
//
// History is append-only (thinking blocks and fallback blocks are echoed back unchanged),
// so prompt caching and preserved thinking keep working across turns.
import { t } from "@/i18n";
import Anthropic from "@anthropic-ai/sdk";
import type {
  BetaContentBlock,
  BetaMessage,
  BetaMessageParam,
  BetaMessageStreamParams,
  BetaTextBlockParam,
  BetaTool,
  BetaToolResultBlockParam,
  BetaToolUseBlock,
} from "@anthropic-ai/sdk/resources/beta/messages/messages";
import { providerFetch } from "./fetch";
import { hostOf, isAbortError, ProviderError, type ChatSession, type Connection, type LoopEvent, type Step, type ToolDef, type ToolResult } from "./types";

/** Output budget per response (streamed, so no request timeout concerns). */
export const MAX_TOKENS = 32_000;

const DEFAULT_BASE = "https://api.anthropic.com";

/**
 * Builds the API client (replaced in tests). The key goes to the configured base URL only.
 * Requests go through `providerFetch` (Rust-side on the desktop, so no CORS).
 */
export const clientFactory = {
  create(apiKey: string, baseURL?: string): Anthropic {
    return new Anthropic({ apiKey, dangerouslyAllowBrowser: true, fetch: providerFetch, ...(baseURL && baseURL !== DEFAULT_BASE ? { baseURL } : {}) });
  },
};

/** Tool definitions in the Messages API shape. */
export function toApiTools(tools: ReadonlyArray<ToolDef>): BetaTool[] {
  return tools.map((t) => ({ name: t.name, description: t.description, input_schema: t.parameters as BetaTool["input_schema"] }));
}

/** Request parameters for `model` (shared by every request of a turn). */
export function requestParams(o: { model: string; system: BetaTextBlockParam[]; tools: BetaTool[]; messages: BetaMessageParam[] }): BetaMessageStreamParams {
  const params: BetaMessageStreamParams = {
    model: o.model,
    max_tokens: MAX_TOKENS,
    system: o.system,
    tools: o.tools,
    messages: o.messages,
    // Caches the longest stable prefix (tools, system, history) between requests.
    cache_control: { type: "ephemeral" },
  };
  // Server-side fallback when a safety classifier declines (Sonnet 5.5 / Opus 5.5).
  if (o.model === "claude-sonnet-5-5" || o.model === "claude-opus-5-5") {
    params.betas = ["server-side-fallback-2026-07-01"];
    params.fallbacks = "default";
  }
  return params;
}

export class AnthropicSession implements ChatSession {
  readonly history: BetaMessageParam[] = [];
  private readonly system: BetaTextBlockParam[];
  private readonly tools: BetaTool[];

  constructor(system: string[], tools: ReadonlyArray<ToolDef>) {
    this.system = system.map((text) => ({ type: "text", text }));
    this.tools = toApiTools(tools);
  }

  addUser(parts: string[]): void {
    this.history.push({ role: "user", content: parts.map((text) => ({ type: "text", text })) });
  }

  restore(history: ReadonlyArray<unknown>): void {
    this.history.push(...(history as BetaMessageParam[]));
  }

  async step(conn: Connection, signal: AbortSignal, onEvent: (e: LoopEvent) => void): Promise<Step> {
    let message: BetaMessage;
    try {
      const client = clientFactory.create(conn.apiKey ?? "", conn.baseUrl);
      const stream = client.beta.messages.stream(requestParams({ model: conn.model, system: this.system, tools: this.tools, messages: this.history }), {
        signal,
      });
      for await (const event of stream) {
        if (event.type === "content_block_start") {
          if (event.content_block.type === "text") onEvent({ type: "text_start" });
        } else if (event.type === "content_block_delta") {
          if (event.delta.type === "text_delta") onEvent({ type: "text", delta: event.delta.text });
        } else if (event.type === "content_block_stop") {
          const block = stream.currentMessage?.content[event.index];
          if (block?.type === "tool_use") onEvent({ type: "tool_call", id: block.id, name: block.name, input: block.input });
        }
      }
      message = await stream.finalMessage();
    } catch (e) {
      throw toProviderError(e, conn.baseUrl);
    }
    const truncated = message.stop_reason === "max_tokens";
    let content = message.content;
    let cut: BetaToolUseBlock[] = [];
    if (truncated) {
      // The last block was being written when the limit hit: a tool_use there may hold a
      // partial input that still parses (the SDK parses partial JSON), so it is never run.
      const last = content.at(-1);
      if (last?.type === "tool_use") {
        cut = [last];
        content = content.slice(0, -1);
      }
      // A thinking block cut off before its signature can't be sent back.
      content = content.filter((b) => b.type !== "thinking" || !!b.signature);
    }
    const calls = content.filter((b): b is BetaToolUseBlock => b.type === "tool_use").map((b) => ({ id: b.id, name: b.name, input: b.input }));
    const stop = message.stop_reason === "refusal" ? "refusal" : truncated ? "max_tokens" : calls.length > 0 ? "tool_use" : "end_turn";
    const keep = content.some((b) => b.type === "tool_use" || (b.type === "text" && b.text));
    return {
      calls,
      stop,
      raw: !truncated || keep ? content : null,
      ...(cut.length > 0 ? { cut: cut.map((b) => ({ id: b.id, name: b.name, input: b.input })) } : {}),
    };
  }

  commit(step: Step): void {
    if (step.raw) this.history.push({ role: "assistant", content: step.raw as BetaContentBlock[] });
  }

  addNote(text: string): void {
    const last = this.history.at(-1);
    // Consecutive user turns are merged: append to the tool results' message when there is one.
    if (last?.role === "user" && Array.isArray(last.content)) last.content.push({ type: "text", text });
    else this.history.push({ role: "user", content: [{ type: "text", text }] });
  }

  addToolResults(results: ToolResult[]): void {
    // All results of one assistant message go back in ONE user message.
    const content: BetaToolResultBlockParam[] = results.map((r) => ({
      type: "tool_result",
      tool_use_id: r.id,
      content: r.content,
      ...(r.is_error ? { is_error: true } : {}),
    }));
    this.history.push({ role: "user", content });
  }
}

/** SDK errors as `ProviderError`s (aborts pass through). */
export function toProviderError(e: unknown, baseUrl = DEFAULT_BASE): unknown {
  if (isAbortError(e) || e instanceof Anthropic.APIUserAbortError) return Object.assign(new Error("aborted"), { name: "AbortError" });
  if (e instanceof ProviderError) return e;
  const host = hostOf(baseUrl);
  const provider = "Anthropic";
  if (e instanceof Anthropic.AuthenticationError) return new ProviderError("auth", t("ai_chat.error.key_rejected"), 401);
  if (e instanceof Anthropic.PermissionDeniedError) return new ProviderError("permission", t("ai_chat.error.model_not_allowed"), 403);
  if (e instanceof Anthropic.NotFoundError) return new ProviderError("not_found", t("ai_chat.error.model_not_found", { detail: apiMessage(e) }), 404);
  if (e instanceof Anthropic.RateLimitError) return new ProviderError("rate_limit", t("ai_chat.error.api_rate_limited", { provider }), 429);
  if (e instanceof Anthropic.APIConnectionError) return new ProviderError("connection", t("ai_chat.error.unreachable_check", { host }));
  if (e instanceof Anthropic.InternalServerError) return new ProviderError("server", t("ai_chat.error.api_server", { provider }), e.status);
  if (e instanceof Anthropic.APIError)
    return new ProviderError("other", t("ai_chat.error.failed_detail", { status: e.status ?? "?", detail: apiMessage(e) }), e.status);
  return e;
}

function apiMessage(e: InstanceType<typeof Anthropic.APIError>): string {
  const body = e.error as { error?: { message?: string } } | undefined;
  return body?.error?.message ?? e.message;
}

/** Test connection: the key works and the model exists (no tokens spent). */
export async function testAnthropic(conn: Connection, signal?: AbortSignal): Promise<string> {
  try {
    const m = await clientFactory.create(conn.apiKey ?? "", conn.baseUrl).models.retrieve(conn.model, {}, { signal });
    return t("ai_chat.test.model_available", { model: m.display_name ?? conn.model });
  } catch (e) {
    throw toProviderError(e, conn.baseUrl);
  }
}
