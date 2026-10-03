// Test helpers: scripted Messages API streams (SSE) behind a fake `fetch`.
import Anthropic from "@anthropic-ai/sdk";

export type ScriptBlock = { text: string } | { tool: string; id: string; input: unknown };
export interface ScriptedResponse {
  blocks: ScriptBlock[];
  stop?: "end_turn" | "tool_use" | "max_tokens" | "refusal";
}

/** The SSE body of one streamed response. */
export function sseBody(r: ScriptedResponse, msgId = "msg_test"): string {
  const events: object[] = [
    {
      type: "message_start",
      message: {
        id: msgId,
        type: "message",
        role: "assistant",
        model: "claude-test",
        content: [],
        stop_reason: null,
        stop_sequence: null,
        usage: { input_tokens: 10, output_tokens: 0 },
      },
    },
  ];
  r.blocks.forEach((b, index) => {
    if ("text" in b) {
      events.push({ type: "content_block_start", index, content_block: { type: "text", text: "" } });
      // Two deltas, to exercise streaming.
      const half = Math.ceil(b.text.length / 2);
      for (const part of [b.text.slice(0, half), b.text.slice(half)]) {
        if (part) events.push({ type: "content_block_delta", index, delta: { type: "text_delta", text: part } });
      }
    } else {
      events.push({ type: "content_block_start", index, content_block: { type: "tool_use", id: b.id, name: b.tool, input: {} } });
      events.push({ type: "content_block_delta", index, delta: { type: "input_json_delta", partial_json: JSON.stringify(b.input) } });
    }
    events.push({ type: "content_block_stop", index });
  });
  const stop = r.stop ?? (r.blocks.some((b) => "tool" in b) ? "tool_use" : "end_turn");
  events.push({ type: "message_delta", delta: { stop_reason: stop, stop_sequence: null }, usage: { output_tokens: 5 } });
  events.push({ type: "message_stop" });
  return events.map((e) => `event: ${(e as { type: string }).type}\ndata: ${JSON.stringify(e)}\n\n`).join("");
}

export interface FakeApi {
  fetch: typeof fetch;
  /** Parsed JSON bodies of the requests made so far. */
  requests: Array<Record<string, unknown>>;
  /** Request headers (lower-case names). */
  headers: Array<Record<string, string>>;
}

/**
 * A fake api.anthropic.com: answers each request with the next scripted response (or the
 * result of a function of the request number). `hang` = never finish the body (until aborted).
 */
export function fakeApi(script: Array<ScriptedResponse | "hang"> | ((n: number) => ScriptedResponse | "hang")): FakeApi {
  const requests: Array<Record<string, unknown>> = [];
  const headers: Array<Record<string, string>> = [];
  const f = (async (_url: unknown, init?: RequestInit) => {
    const n = requests.length;
    requests.push(JSON.parse(String(init?.body ?? "{}")) as Record<string, unknown>);
    const h: Record<string, string> = {};
    new Headers(init?.headers).forEach((v, k) => (h[k] = v));
    headers.push(h);
    const next = typeof script === "function" ? script(n) : script[n];
    if (!next) throw new Error(`unexpected request #${n + 1}`);
    const signal = init?.signal;
    const enc = new TextEncoder();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        if (next === "hang") {
          // The start of a text answer, then nothing.
          const partial = sseBody({ blocks: [{ text: "Let me" }] }, `msg_${n}`).split("event: content_block_stop")[0]!;
          controller.enqueue(enc.encode(partial));
          const fail = () => controller.error(Object.assign(new Error("aborted"), { name: "AbortError" }));
          if (signal?.aborted) fail();
          signal?.addEventListener("abort", fail);
          return;
        }
        controller.enqueue(enc.encode(sseBody(next, `msg_${n}`)));
        controller.close();
      },
    });
    return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
  }) as typeof fetch;
  return { fetch: f, requests, headers };
}

/** A client over the fake API (no retries). */
export function fakeClient(api: FakeApi, apiKey = "sk-ant-test-0000000000000000"): Anthropic {
  return new Anthropic({ apiKey, dangerouslyAllowBrowser: true, fetch: api.fetch, maxRetries: 0 });
}

// ─── OpenAI-compatible (Chat Completions) ─────────────────────────────────────────────────

export interface OaiScriptedResponse {
  blocks: ScriptBlock[];
  finish?: "stop" | "tool_calls" | "length" | "content_filter";
  /** Streamed `reasoning_content` (DeepSeek reasoner). */
  reasoning?: string;
  /** Each tool call in one chunk without `index` (Gemini / Mistral style). */
  wholeCalls?: boolean;
}

/** The SSE chunks (`data:` payloads) of one streamed Chat Completions response. */
export function oaiChunks(r: OaiScriptedResponse, id = "chatcmpl_test"): object[] {
  const chunks: object[] = [];
  const chunk = (delta: object, finish: string | null = null) =>
    chunks.push({ id, object: "chat.completion.chunk", model: "test", choices: [{ index: 0, delta, finish_reason: finish }] });
  chunk({ role: "assistant", content: "" });
  if (r.reasoning) chunk({ reasoning_content: r.reasoning });
  let call = 0;
  for (const b of r.blocks) {
    if ("text" in b) {
      const half = Math.ceil(b.text.length / 2);
      for (const part of [b.text.slice(0, half), b.text.slice(half)]) if (part) chunk({ content: part });
    } else {
      const args = JSON.stringify(b.input);
      const index = call++;
      if (r.wholeCalls) {
        chunk({ tool_calls: [{ id: b.id, type: "function", function: { name: b.tool, arguments: args } }] });
      } else {
        chunk({ tool_calls: [{ index, id: b.id, type: "function", function: { name: b.tool, arguments: "" } }] });
        const half = Math.ceil(args.length / 2);
        chunk({ tool_calls: [{ index, function: { arguments: args.slice(0, half) } }] });
        chunk({ tool_calls: [{ index, function: { arguments: args.slice(half) } }] });
      }
    }
  }
  chunk({}, r.finish ?? (call > 0 ? "tool_calls" : "stop"));
  return chunks;
}

/** The SSE body of one streamed Chat Completions response (with a comment line and `[DONE]`). */
export function oaiSseBody(r: OaiScriptedResponse, id = "chatcmpl_test"): string {
  return `: keep-alive\n\n${oaiChunks(r, id)
    .map((c) => `data: ${JSON.stringify(c)}\n\n`)
    .join("")}data: [DONE]\n\n`;
}

export interface FakeOpenAi extends FakeApi {
  urls: string[];
}

type OaiStep = OaiScriptedResponse | "hang" | (() => Response);

/** A fake OpenAI-compatible server for `http.fetch` (same contract as `fakeApi`). */
export function fakeOpenAi(script: OaiStep[] | ((n: number) => OaiStep)): FakeOpenAi {
  const requests: Array<Record<string, unknown>> = [];
  const headers: Array<Record<string, string>> = [];
  const urls: string[] = [];
  const f = (async (url: unknown, init?: RequestInit) => {
    const n = requests.length;
    urls.push(String(url));
    requests.push(JSON.parse(String(init?.body ?? "{}")) as Record<string, unknown>);
    const h: Record<string, string> = {};
    new Headers(init?.headers).forEach((v, k) => (h[k] = v));
    headers.push(h);
    const next = typeof script === "function" ? script(n) : script[n];
    if (!next) throw new Error(`unexpected request #${n + 1}`);
    if (typeof next === "function") return next();
    const signal = init?.signal;
    const enc = new TextEncoder();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        if (next === "hang") {
          // The start of a text answer, then nothing.
          const first = oaiChunks({ blocks: [{ text: "Let me" }] }).slice(0, 2);
          controller.enqueue(enc.encode(first.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("")));
          const fail = () => controller.error(Object.assign(new Error("aborted"), { name: "AbortError" }));
          if (signal?.aborted) fail();
          signal?.addEventListener("abort", fail);
          return;
        }
        controller.enqueue(enc.encode(oaiSseBody(next, `chatcmpl_${n}`)));
        controller.close();
      },
    });
    return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
  }) as typeof fetch;
  return { fetch: f, requests, headers, urls };
}

/** A JSON error response, as providers send them. */
export const errorResponse = (status: number, message: string) => () =>
  new Response(JSON.stringify({ error: { message, type: "invalid_request_error" } }), { status, headers: { "content-type": "application/json" } });
