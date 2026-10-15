// OpenAI-compatible adapter: Chat Completions with function calling, streamed (SSE), over
// plain fetch (no SDK). One adapter for OpenAI, DeepSeek, Gemini's OpenAI endpoint, Mistral,
// Groq, OpenRouter, Together, Ollama, LM Studio and any other compatible server.
//
// Translation: tool specs → `tools: [{type: "function", function: {name, description,
// parameters}}]`; streamed `delta.tool_calls` fragments are assembled per `index`; each tool
// result is one `role: "tool"` message (`is_error` has no field there, so errors are sent as
// "Error: …" text).
import { t } from "@/i18n";
import {
  hostOf,
  isAbortError,
  ProviderError,
  type ChatSession,
  type Connection,
  type LoopEvent,
  type Step,
  type ToolCall,
  type ToolDef,
  type ToolResult,
} from "./types";
import { DesktopFetchError, providerFetch } from "./fetch";

/** fetch used by the adapter (replaced in tests): Rust-side on the desktop (`./fetch`). */
export const http = {
  fetch: ((input: RequestInfo | URL, init?: RequestInit) => providerFetch(input, init)) as typeof fetch,
};

export interface OaiToolCall {
  id: string;
  type: "function";
  function: { name: string; arguments: string };
}

export type OaiMessage =
  | { role: "system"; content: string }
  | { role: "user"; content: string }
  | { role: "assistant"; content: string | null; tool_calls?: OaiToolCall[]; reasoning_content?: string }
  | { role: "tool"; tool_call_id: string; content: string };

export interface OaiTool {
  type: "function";
  function: { name: string; description: string; parameters: Record<string, unknown> };
}

export function toOpenAiTools(tools: ReadonlyArray<ToolDef>): OaiTool[] {
  return tools.map((t) => ({ type: "function", function: { name: t.name, description: t.description, parameters: t.parameters } }));
}

/** `{base}/chat/completions` (a trailing slash on the base is fine). */
export const endpoint = (baseUrl: string, path: string) => `${baseUrl.replace(/\/+$/, "")}/${path}`;

/** Labels for messages ("Rejected by DeepSeek"). */
export interface OpenAiOptions {
  label: string;
  corsHelp?: string;
  /** `max_tokens` per response; unset = the server's default (which can be small: 8K on DeepSeek without thinking). */
  maxTokens?: number;
}

export class OpenAiSession implements ChatSession {
  readonly history: OaiMessage[];
  private readonly tools: OaiTool[];
  /** The server refused our `max_tokens` (a model with a lower ceiling): use its default from then on. */
  private serverMaxTokens = false;

  constructor(
    system: string[],
    tools: ReadonlyArray<ToolDef>,
    private readonly opts: OpenAiOptions,
  ) {
    this.history = [{ role: "system", content: system.join("\n\n") }];
    this.tools = toOpenAiTools(tools);
  }

  addUser(parts: string[]): void {
    // Reasoning from earlier turns is not sent back (DeepSeek only wants it within a tool loop).
    for (const m of this.history) if (m.role === "assistant") delete m.reasoning_content;
    this.history.push({ role: "user", content: parts.join("\n\n") });
  }

  /** The saved history without its system message (this session has a fresh one). */
  restore(history: ReadonlyArray<unknown>): void {
    this.history.push(...(history as OaiMessage[]).filter((m) => m.role !== "system"));
  }

  async step(conn: Connection, signal: AbortSignal, onEvent: (e: LoopEvent) => void): Promise<Step> {
    const url = endpoint(conn.baseUrl, "chat/completions");
    const send = () => {
      const max = this.opts.maxTokens !== undefined && !this.serverMaxTokens ? { max_tokens: this.opts.maxTokens } : {};
      const body = { model: conn.model, messages: this.history, tools: this.tools, stream: true, ...max };
      return request(url, conn, { method: "POST", body: JSON.stringify(body), signal }, this.opts);
    };
    let res = await send();
    if (!res.ok && res.status === 400 && this.opts.maxTokens !== undefined && !this.serverMaxTokens) {
      const err = await httpError(res, conn, this.opts);
      if (!/max_tokens/i.test(err.message)) throw err;
      // "valid range of max_tokens is [1, 8192]": this model has a lower ceiling; use its default.
      this.serverMaxTokens = true;
      res = await send();
    }
    if (!res.ok) throw await httpError(res, conn, this.opts);
    if (!res.body) throw new ProviderError("other", t("ai_chat.error.empty_response", { provider: this.opts.label }));

    let text = "";
    let reasoning = "";
    let finish: string | null = null;
    const parts = new Map<number, { id: string; name: string; args: string }>();
    try {
      for await (const data of sseData(res.body)) {
        if (data === "[DONE]") break;
        let chunk: OaiChunk;
        try {
          chunk = JSON.parse(data) as OaiChunk;
        } catch {
          continue; // keep-alives, comments
        }
        if (chunk.error) throw new ProviderError("other", `${this.opts.label}: ${errorMessage(chunk.error)}`);
        const choice = chunk.choices?.[0];
        if (!choice) continue;
        const d = choice.delta ?? {};
        if (typeof d.reasoning_content === "string") reasoning += d.reasoning_content;
        if (typeof d.content === "string" && d.content) {
          if (!text) onEvent({ type: "text_start" });
          text += d.content;
          onEvent({ type: "text", delta: d.content });
        }
        for (const [i, tc] of (d.tool_calls ?? []).entries()) {
          // Some servers omit `index` (one complete call per chunk): fall back to arrival order.
          const index = typeof tc.index === "number" ? tc.index : parts.size + i;
          const p = parts.get(index) ?? { id: "", name: "", args: "" };
          if (tc.id) p.id = tc.id;
          if (tc.function?.name) p.name += tc.function.name;
          if (tc.function?.arguments) p.args += tc.function.arguments;
          parts.set(index, p);
        }
        if (choice.finish_reason) finish = choice.finish_reason;
      }
    } catch (e) {
      if (isAbortError(e) || signal.aborted) throw abortError();
      if (e instanceof ProviderError) throw e;
      throw new ProviderError("connection", t("ai_chat.error.broke_off", { host: hostOf(conn.baseUrl) }));
    }

    const all: ToolCall[] = [...parts.entries()]
      .sort(([a], [b]) => a - b)
      .map(([index, p]) => {
        const id = p.id || `call_${index}_${Math.random().toString(36).slice(2, 10)}`;
        const parsed = parseArgs(p.args);
        return "error" in parsed ? { id, name: p.name, input: p.args, invalid: parsed.error } : { id, name: p.name, input: parsed.value };
      });
    for (const c of all) onEvent({ type: "tool_call", id: c.id, name: c.name, input: c.input });

    const truncated = finish === "length";
    // Cut off at the output limit: arguments that parse as one JSON object are complete (strict
    // JSON needs the closing brace); the ones that don't were cut off mid-way.
    const calls = truncated ? all.filter((c) => c.invalid === undefined) : all;
    const cut = truncated ? all.filter((c) => c.invalid !== undefined) : [];
    const stop = truncated ? "max_tokens" : finish === "content_filter" && calls.length === 0 && !text ? "refusal" : calls.length > 0 ? "tool_use" : "end_turn";
    const raw: OaiMessage | null =
      !text && calls.length === 0
        ? null
        : {
            role: "assistant",
            content: text || null,
            ...(calls.length > 0
              ? { tool_calls: calls.map((c) => ({ id: c.id, type: "function" as const, function: { name: c.name, arguments: argsText(c) } })) }
              : {}),
            // Within a tool loop (a cut-off response continues it), DeepSeek wants its reasoning back.
            ...(reasoning && (calls.length > 0 || truncated) ? { reasoning_content: reasoning } : {}),
          };
    return { calls, stop, raw, ...(cut.length > 0 ? { cut } : {}) };
  }

  commit(step: Step): void {
    if (step.raw) this.history.push(step.raw as OaiMessage);
  }

  addToolResults(results: ToolResult[]): void {
    for (const r of results) this.history.push({ role: "tool", tool_call_id: r.id, content: r.is_error ? `Error: ${r.content}` : r.content });
  }

  addNote(text: string): void {
    this.history.push({ role: "user", content: text });
  }
}

const argsText = (c: ToolCall) => (typeof c.input === "string" ? c.input : JSON.stringify(c.input ?? {}));

/** Tool arguments: JSON text of an object ("" = no arguments). */
export function parseArgs(args: string): { value: unknown } | { error: string } {
  if (!args.trim()) return { value: {} };
  try {
    const v = JSON.parse(args) as unknown;
    if (v && typeof v === "object" && !Array.isArray(v)) return { value: v };
    // eth-allow-case: tool results for the model, not the UI
    return { error: "Not run: the tool arguments must be a JSON object." };
  } catch {
    // eth-allow-case
    return { error: "Not run: the tool arguments were not valid JSON. Send them again as one JSON object." };
  }
}

interface OaiChunk {
  error?: unknown;
  choices?: Array<{
    delta?: {
      content?: string | null;
      reasoning_content?: string | null;
      tool_calls?: Array<{ index?: number; id?: string; type?: string; function?: { name?: string; arguments?: string } }>;
    };
    finish_reason?: string | null;
  }>;
}

/** The `data:` payloads of an SSE stream (multi-line data joined; comments ignored). */
export async function* sseData(body: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const reader = body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  let data: string[] = [];
  try {
    for (;;) {
      const { done, value } = await reader.read();
      buf += done ? dec.decode() : dec.decode(value, { stream: true });
      let nl: number;
      while ((nl = buf.search(/\r\n|\r|\n/)) >= 0) {
        const line = buf.slice(0, nl);
        buf = buf.slice(nl + (buf.startsWith("\r\n", nl) ? 2 : 1));
        if (line === "") {
          if (data.length) yield data.join("\n");
          data = [];
        } else if (line.startsWith("data:")) {
          data.push(line.slice(line.startsWith("data: ") ? 6 : 5));
        }
      }
      if (done) {
        if (buf.startsWith("data:")) data.push(buf.slice(buf.startsWith("data: ") ? 6 : 5));
        if (data.length) yield data.join("\n");
        return;
      }
    }
  } finally {
    reader.releaseLock();
  }
}

const abortError = () => Object.assign(new Error("aborted"), { name: "AbortError" });

function headers(conn: Connection): Record<string, string> {
  // The key goes in this request only (to the provider's own base URL).
  return { "content-type": "application/json", ...(conn.apiKey ? { authorization: `Bearer ${conn.apiKey}` } : {}) }; // eth-allow-case: Bearer is the HTTP auth scheme
}

/** fetch, with network failures turned into connection / CORS errors. */
async function request(url: string, conn: Connection, init: RequestInit, opts: OpenAiOptions): Promise<Response> {
  try {
    return await http.fetch(url, { ...init, headers: headers(conn) });
  } catch (e) {
    if (isAbortError(e) || init.signal?.aborted) throw abortError();
    throw await networkError(url, conn, opts, init.signal ?? undefined, e);
  }
}

/**
 * A fetch that failed without a response is either "unreachable" or "blocked by CORS" (the
 * browser hides which). Probe the same URL in `no-cors` mode (no key, no body): if that gets
 * through, the server is up and the browser blocked the real request.
 */
export async function networkError(url: string, conn: Connection, opts: OpenAiOptions, signal?: AbortSignal, cause?: unknown): Promise<ProviderError> {
  const host = hostOf(conn.baseUrl);
  // The desktop fetches from Rust: no CORS, so nothing to probe.
  if (cause instanceof DesktopFetchError && cause.notAllowed) {
    // "the base URL can't contain a user name or password", or a redirect out of scope.
    return new ProviderError(
      "connection",
      /base URL/.test(cause.message)
        ? t("ai_chat.error.cannot_connect_because", { host, reason: cause.message })
        : t("ai_chat.error.cannot_connect_not_allowed", { host }),
    );
  }
  let reachable = false;
  if (!(cause instanceof DesktopFetchError)) {
    try {
      await http.fetch(url, { method: "GET", mode: "no-cors", signal });
      reachable = true;
    } catch {
      /* unreachable */
    }
  }
  if (reachable) {
    return new ProviderError("cors", t("ai_chat.error.cors", { host, help: opts.corsHelp ?? t("ai_chat.provider.cors_help") }));
  }
  const local = /^(localhost|127\.0\.0\.1|\[::1\])(:\d+)?$/.test(host);
  return new ProviderError(
    "connection",
    local ? t("ai_chat.error.unreachable_local", { host, provider: opts.label }) : t("ai_chat.error.unreachable", { host }),
  );
}

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object") {
    const o = err as { message?: unknown; error?: unknown; detail?: unknown };
    if (typeof o.message === "string") return o.message;
    if (o.error !== undefined) return errorMessage(o.error);
    if (typeof o.detail === "string") return o.detail;
  }
  return "";
}

/**
 * "This model can't do tools", as servers phrase it: Ollama "… does not support tools", vLLM
 * "… requires --enable-auto-tool-choice …", others "tool calling is not supported …".
 */
export function noToolSupport(detail: string): boolean {
  return /\btool|function/i.test(detail) && /(not|n't) (be )?support|unsupported|enable-auto-tool-choice|not (enabled|available)/i.test(detail);
}

/** An HTTP error response as a `ProviderError` with a user-facing message. */
export async function httpError(res: Response, conn: Connection, opts: OpenAiOptions): Promise<ProviderError> {
  let detail = "";
  try {
    const text = await res.text();
    try {
      detail = errorMessage(JSON.parse(text));
    } catch {
      detail = text.slice(0, 300);
    }
  } catch {
    /* no body */
  }
  const s = res.status;
  const provider = opts.label;
  if (s === 401) return new ProviderError("auth", t("ai_chat.error.key_rejected_by", { provider }), s);
  if (s === 403)
    return new ProviderError("permission", detail ? t("ai_chat.error.refused_detail", { provider, detail }) : t("ai_chat.error.refused", { provider }), s);
  if (s === 429)
    return new ProviderError(
      "rate_limit",
      detail ? t("ai_chat.error.rate_limited_detail", { provider, detail }) : t("ai_chat.error.rate_limited", { provider }),
      s,
    );
  if (s >= 400 && s < 500 && noToolSupport(detail)) {
    const model = conn.model;
    return new ProviderError("no_tools", detail ? t("ai_chat.error.no_tools_detail", { model, provider, detail }) : t("ai_chat.error.no_tools", { model }), s);
  }
  if (s === 404) {
    const host = hostOf(conn.baseUrl);
    return new ProviderError("not_found", detail ? t("ai_chat.error.not_found_at_detail", { host, detail }) : t("ai_chat.error.not_found_at", { host }), s);
  }
  if (s >= 500) return new ProviderError("server", t("ai_chat.error.server", { provider, status: s }), s);
  return new ProviderError("other", detail ? t("ai_chat.error.failed_detail", { status: s, detail }) : t("ai_chat.error.failed", { status: s }), s);
}

/** Test connection: list the models with the key (no tokens spent). */
export async function testOpenAi(conn: Connection, opts: OpenAiOptions, signal?: AbortSignal): Promise<string> {
  const url = endpoint(conn.baseUrl, "models");
  const res = await request(url, conn, { method: "GET", signal }, opts);
  if (!res.ok) throw await httpError(res, conn, opts);
  let ids: string[] = [];
  try {
    const body = (await res.json()) as { data?: Array<{ id?: unknown }> };
    ids = (body.data ?? []).flatMap((m) => (typeof m.id === "string" ? [m.id.replace(/^models\//, "")] : []));
  } catch {
    /* some servers return no list */
  }
  const host = hostOf(conn.baseUrl);
  if (ids.length === 0 || ids.includes(conn.model)) return t("ai_chat.test.connected_to", { host });
  return t("ai_chat.test.model_not_listed", { host, model: conn.model, count: ids.length, examples: ids.slice(0, 3).join(", ") });
}
