// OpenAI-compatible adapter: Chat Completions with function calling, streamed (SSE), over
// plain fetch (no SDK). One adapter for OpenAI, DeepSeek, Gemini's OpenAI endpoint, Mistral,
// Groq, OpenRouter, Together, Ollama, LM Studio and any other compatible server.
//
// Translation: tool specs → `tools: [{type: "function", function: {name, description,
// parameters}}]`; streamed `delta.tool_calls` fragments are assembled per `index`; each tool
// result is one `role: "tool"` message (`is_error` has no field there, so errors are sent as
// "Error: …" text).
import { hostOf, isAbortError, ProviderError, type ChatSession, type Connection, type LoopEvent, type Step, type ToolCall, type ToolDef, type ToolResult } from "./types";

/** fetch used by the adapter (replaced in tests). */
export const http = {
  fetch: ((input: RequestInfo | URL, init?: RequestInit) => globalThis.fetch(input, init)) as typeof fetch,
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
}

export class OpenAiSession implements ChatSession {
  readonly history: OaiMessage[];
  private readonly tools: OaiTool[];

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

  async step(conn: Connection, signal: AbortSignal, onEvent: (e: LoopEvent) => void): Promise<Step> {
    const url = endpoint(conn.baseUrl, "chat/completions");
    const body = { model: conn.model, messages: this.history, tools: this.tools, stream: true };
    const res = await request(url, conn, { method: "POST", body: JSON.stringify(body), signal }, this.opts);
    if (!res.ok) throw await httpError(res, conn, this.opts);
    if (!res.body) throw new ProviderError("other", `${this.opts.label} sent an empty response.`);

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
      throw new ProviderError("connection", `The connection to ${hostOf(conn.baseUrl)} broke off. Try again.`);
    }

    const calls: ToolCall[] = [...parts.entries()]
      .sort(([a], [b]) => a - b)
      .map(([index, p]) => {
        const id = p.id || `call_${index}_${Math.random().toString(36).slice(2, 10)}`;
        const parsed = parseArgs(p.args);
        return "error" in parsed ? { id, name: p.name, input: p.args, invalid: parsed.error } : { id, name: p.name, input: parsed.value };
      });
    for (const c of calls) onEvent({ type: "tool_call", id: c.id, name: c.name, input: c.input });

    const stop = finish === "length" ? "max_tokens" : finish === "content_filter" && calls.length === 0 && !text ? "refusal" : calls.length > 0 ? "tool_use" : "end_turn";
    const raw: OaiMessage = {
      role: "assistant",
      content: text || null,
      ...(calls.length > 0
        ? { tool_calls: calls.map((c) => ({ id: c.id, type: "function" as const, function: { name: c.name, arguments: argsText(c) } })) }
        : {}),
      ...(reasoning && calls.length > 0 ? { reasoning_content: reasoning } : {}),
    };
    return { calls, stop, raw };
  }

  commit(step: Step): void {
    this.history.push(step.raw as OaiMessage);
  }

  addToolResults(results: ToolResult[]): void {
    for (const r of results) this.history.push({ role: "tool", tool_call_id: r.id, content: r.is_error ? `Error: ${r.content}` : r.content });
  }
}

const argsText = (c: ToolCall) => (typeof c.input === "string" ? c.input : JSON.stringify(c.input ?? {}));

/** Tool arguments: JSON text of an object ("" = no arguments). */
export function parseArgs(args: string): { value: unknown } | { error: string } {
  if (!args.trim()) return { value: {} };
  try {
    const v = JSON.parse(args) as unknown;
    if (v && typeof v === "object" && !Array.isArray(v)) return { value: v };
    return { error: "Not run: the tool arguments must be a JSON object." };
  } catch {
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
  return { "content-type": "application/json", ...(conn.apiKey ? { authorization: `Bearer ${conn.apiKey}` } : {}) };
}

/** fetch, with network failures turned into connection / CORS errors. */
async function request(url: string, conn: Connection, init: RequestInit, opts: OpenAiOptions): Promise<Response> {
  try {
    return await http.fetch(url, { ...init, headers: headers(conn) });
  } catch (e) {
    if (isAbortError(e) || init.signal?.aborted) throw abortError();
    throw await networkError(url, conn, opts, init.signal ?? undefined);
  }
}

/**
 * A fetch that failed without a response is either "unreachable" or "blocked by CORS" (the
 * browser hides which). Probe the same URL in `no-cors` mode (no key, no body): if that gets
 * through, the server is up and the browser blocked the real request.
 */
export async function networkError(url: string, conn: Connection, opts: OpenAiOptions, signal?: AbortSignal): Promise<ProviderError> {
  const host = hostOf(conn.baseUrl);
  let reachable = false;
  try {
    await http.fetch(url, { method: "GET", mode: "no-cors", signal });
    reachable = true;
  } catch {
    /* unreachable */
  }
  if (reachable) {
    const help = opts.corsHelp ?? "Use a provider that accepts requests from web apps, or a proxy.";
    return new ProviderError("cors", `${host} blocked the request from this app (CORS). ${help}`);
  }
  const local = /^(localhost|127\.0\.0\.1|\[::1\])(:\d+)?$/.test(host);
  return new ProviderError(
    "connection",
    local ? `Couldn't reach ${host}. Is ${opts.label} running? Check the base URL in the AI settings.` : `Couldn't reach ${host}. Check your connection and the base URL.`,
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
  const label = opts.label;
  if (s === 401) return new ProviderError("auth", `Your API key was rejected by ${label}. Check it in the AI settings.`, s);
  if (s === 403) return new ProviderError("permission", `${label} refused the request (403)${detail ? `: ${detail}` : ""}.`, s);
  if (s === 429) return new ProviderError("rate_limit", `Rate limited by ${label}${detail ? ` (${detail})` : ""}. Wait a moment and try again.`, s);
  if (s >= 400 && s < 500 && noToolSupport(detail)) {
    return new ProviderError(
      "no_tools",
      `The model "${conn.model}" doesn't support tool calls, which the AI chat needs to edit your project. Pick a model with tool (function) calling.${detail ? ` (${label}: ${detail})` : ""}`,
      s,
    );
  }
  if (s === 404) return new ProviderError("not_found", `Not found at ${hostOf(conn.baseUrl)} (404)${detail ? `: ${detail}` : ""}. Check the base URL and the model name.`, s);
  if (s >= 500) return new ProviderError("server", `${label} had a problem (${s}). Try again.`, s);
  return new ProviderError("other", `Request failed (${s})${detail ? `: ${detail}` : ""}`, s);
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
  if (ids.length === 0 || ids.includes(conn.model)) return `Connected to ${hostOf(conn.baseUrl)}.`;
  return `Connected to ${hostOf(conn.baseUrl)}, but it doesn't list the model "${conn.model}" (it has ${ids.length} other${ids.length === 1 ? "" : "s"}, e.g. ${ids.slice(0, 3).join(", ")}).`;
}
