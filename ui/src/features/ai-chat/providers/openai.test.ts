// The OpenAI-compatible adapter under the provider-agnostic tool loop: mocked streamed Chat
// Completions responses (SSE) through `http.fetch`.
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ToolOutcome } from "../agentApi";
import { cappedResult, runTurn, STOPPED_RESULT, type LoopEvent, type TurnOptions } from "../loop";
import { errorResponse, fakeOpenAi, type FakeOpenAi } from "../testing";
import { createSession, ProviderError, testConnection, toolDefs } from ".";
import { http, noToolSupport, OpenAiSession, parseArgs, sseData, toOpenAiTools, type OaiMessage } from "./openai";

const TOOLS = toolDefs([
  {
    name: "create_track",
    description: "Create a track",
    input_schema: JSON.stringify({ type: "object", properties: { kind: { type: "string", enum: ["midi", "audio"] } }, required: ["kind"] }),
  },
  { name: "get_project_overview", description: "Overview", input_schema: "{}" },
]);
const KEY = "sk-test-openai-000000000000000000";

afterEach(() => vi.restoreAllMocks());

function setup(api: FakeOpenAi, callTool: TurnOptions["callTool"], extra: Partial<TurnOptions> = {}) {
  vi.spyOn(http, "fetch").mockImplementation(api.fetch);
  const events: LoopEvent[] = [];
  const session = createSession("openai", ["system prompt", "overview"], TOOLS) as OpenAiSession;
  session.addUser(["Make a bass track"]);
  const abort = new AbortController();
  const opts: TurnOptions = {
    session,
    conn: { baseUrl: "https://api.openai.com/v1", apiKey: KEY, model: "gpt-5" },
    maxIterations: 25,
    signal: abort.signal,
    callTool,
    onEvent: (e) => events.push(e),
    ...extra,
  };
  return { opts, events, history: session.history, abort };
}

const msgs = (api: FakeOpenAi, n: number) => api.requests[n]!.messages as OaiMessage[];

describe("OpenAI-compatible adapter", () => {
  it("tool call → CallTool → tool result → final text", async () => {
    const api = fakeOpenAi([
      { blocks: [{ text: "Creating it." }, { tool: "create_track", id: "call_1", input: { kind: "midi", name: "Bass" } }] },
      { blocks: [{ text: "Done: added Bass." }] },
    ]);
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: '{"track_id":"T1"}', is_error: false }));
    const { opts, events, history } = setup(api, callTool);

    await expect(runTurn(opts)).resolves.toEqual({ reason: "end_turn" });
    expect(callTool).toHaveBeenCalledWith("create_track", { kind: "midi", name: "Bass" });

    // Request: endpoint, auth header, model, streaming, tools as functions with the JSON Schema.
    expect(api.urls[0]).toBe("https://api.openai.com/v1/chat/completions");
    expect(api.headers[0]!.authorization).toBe(`Bearer ${KEY}`);
    const first = api.requests[0]!;
    expect(first).toMatchObject({ model: "gpt-5", stream: true });
    expect((first.tools as unknown[])[0]).toEqual({
      type: "function",
      function: {
        name: "create_track",
        description: "Create a track",
        parameters: { type: "object", properties: { kind: { type: "string", enum: ["midi", "audio"] } }, required: ["kind"] },
      },
    });
    // The same system prompt as every provider, as one system message.
    expect(msgs(api, 0)[0]).toEqual({ role: "system", content: "system prompt\n\noverview" });
    expect(msgs(api, 0)[1]).toEqual({ role: "user", content: "Make a bass track" });

    // The second request carries the assistant's tool call and the tool message.
    const second = msgs(api, 1);
    expect(second.slice(2)).toEqual([
      {
        role: "assistant",
        content: "Creating it.",
        tool_calls: [{ id: "call_1", type: "function", function: { name: "create_track", arguments: '{"kind":"midi","name":"Bass"}' } }],
      },
      { role: "tool", tool_call_id: "call_1", content: '{"track_id":"T1"}' },
    ]);
    expect(history.map((m) => m.role)).toEqual(["system", "user", "assistant", "tool", "assistant"]);
    expect(history.at(-1)).toEqual({ role: "assistant", content: "Done: added Bass." });

    // Events: streamed text, the call (once its arguments are complete), its result, more text.
    const kinds = events.map((e) => e.type);
    expect(kinds.filter((k) => k === "text_start")).toHaveLength(2);
    expect(events).toContainEqual({ type: "tool_call", id: "call_1", name: "create_track", input: { kind: "midi", name: "Bass" } });
    expect(kinds.indexOf("tool_call")).toBeLessThan(kinds.indexOf("tool_result"));
    expect(events.flatMap((e) => (e.type === "text" ? [e.delta] : [])).join("")).toBe("Creating it.Done: added Bass.");
  });

  it("is_error results go back as error text", async () => {
    const api = fakeOpenAi([{ blocks: [{ tool: "create_track", id: "c1", input: { kind: "banjo" } }] }, { blocks: [{ text: "Sorry." }] }]);
    const { opts, events } = setup(api, async () => ({ content: "`kind` must be one of midi, audio", is_error: true }));
    await expect(runTurn(opts)).resolves.toEqual({ reason: "end_turn" });
    expect(msgs(api, 1).at(-1)).toEqual({ role: "tool", tool_call_id: "c1", content: "Error: `kind` must be one of midi, audio" });
    expect(events).toContainEqual({ type: "tool_result", id: "c1", content: "`kind` must be one of midi, audio", is_error: true });
  });

  it("parallel calls: each gets its tool message, in order", async () => {
    const api = fakeOpenAi([
      {
        blocks: [
          { tool: "get_project_overview", id: "a", input: {} },
          { tool: "create_track", id: "b", input: { kind: "audio" } },
        ],
      },
      { blocks: [{ text: "ok" }] },
    ]);
    const { opts } = setup(api, async (name) => ({ content: name, is_error: false }));
    await runTurn(opts);
    expect(msgs(api, 1).slice(-2)).toEqual([
      { role: "tool", tool_call_id: "a", content: "get_project_overview" },
      { role: "tool", tool_call_id: "b", content: "create_track" },
    ]);
  });

  it("whole tool calls without index or id (Gemini / Mistral style)", async () => {
    const api = fakeOpenAi([
      { blocks: [{ tool: "create_track", id: "", input: { kind: "midi" } }, { tool: "create_track", id: "", input: { kind: "audio" } }], wholeCalls: true, finish: "stop" },
      { blocks: [{ text: "ok" }] },
    ]);
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: "ok", is_error: false }));
    const { opts } = setup(api, callTool);
    await expect(runTurn(opts)).resolves.toEqual({ reason: "end_turn" });
    expect(callTool.mock.calls).toEqual([
      ["create_track", { kind: "midi" }],
      ["create_track", { kind: "audio" }],
    ]);
    const [x, y] = (msgs(api, 1)[2] as Extract<OaiMessage, { role: "assistant" }>).tool_calls!;
    expect(x!.id).toBeTruthy();
    expect(x!.id).not.toBe(y!.id);
  });

  it("invalid JSON arguments are never run; the model gets an error", async () => {
    const api = fakeOpenAi([
      () =>
        new Response(
          `data: ${JSON.stringify({ choices: [{ delta: { tool_calls: [{ index: 0, id: "c1", function: { name: "create_track", arguments: '{"kind": "mi' } }] }, finish_reason: "tool_calls" }] })}\n\ndata: [DONE]\n\n`,
          { status: 200, headers: { "content-type": "text/event-stream" } },
        ),
      { blocks: [{ text: "Retrying later." }] },
    ]);
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: "", is_error: false }));
    const { opts } = setup(api, callTool);
    await expect(runTurn(opts)).resolves.toEqual({ reason: "end_turn" });
    expect(callTool).not.toHaveBeenCalled();
    expect(msgs(api, 1).at(-1)).toMatchObject({ role: "tool", tool_call_id: "c1", content: expect.stringMatching(/^Error: Not run: .*valid JSON/) });
  });

  it("Stop aborts the stream: no tool runs, resolves stopped, history untouched", async () => {
    const api = fakeOpenAi(["hang"]);
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: "", is_error: false }));
    const { opts, events, abort, history } = setup(api, callTool);
    const turn = runTurn(opts);
    await vi.waitFor(() => expect(events.some((e) => e.type === "text")).toBe(true));
    abort.abort();
    await expect(turn).resolves.toEqual({ reason: "stopped" });
    expect(callTool).not.toHaveBeenCalled();
    expect(history).toHaveLength(2);
  });

  it("Stop during a tool round: the rest are answered as not run", async () => {
    const api = fakeOpenAi([
      {
        blocks: [
          { tool: "create_track", id: "a", input: { kind: "midi" } },
          { tool: "create_track", id: "b", input: { kind: "audio" } },
        ],
      },
    ]);
    const held: { abort?: AbortController } = {};
    const callTool = vi.fn(async (): Promise<ToolOutcome> => {
      held.abort!.abort();
      return { content: "ok", is_error: false };
    });
    const s = setup(api, callTool);
    held.abort = s.abort;
    await expect(runTurn(s.opts)).resolves.toEqual({ reason: "stopped" });
    expect(callTool).toHaveBeenCalledTimes(1);
    expect(s.history.slice(-2)).toEqual([
      { role: "tool", tool_call_id: "a", content: "ok" },
      { role: "tool", tool_call_id: "b", content: `Error: ${STOPPED_RESULT}` },
    ]);
  });

  it("caps tool rounds per turn", async () => {
    const api = fakeOpenAi((n) => ({ blocks: [{ tool: "get_project_overview", id: `t${n}`, input: {} }] }));
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: "{}", is_error: false }));
    const { opts, history } = setup(api, callTool, { maxIterations: 2 });
    await expect(runTurn(opts)).resolves.toEqual({ reason: "capped", iterations: 2 });
    expect(callTool).toHaveBeenCalledTimes(2);
    expect(history.at(-1)).toEqual({ role: "tool", tool_call_id: "t2", content: `Error: ${cappedResult(2)}` });
  });

  it("never runs a tool cut off at the length limit", async () => {
    const api = fakeOpenAi([{ blocks: [{ tool: "create_track", id: "x", input: { kind: "midi" } }], finish: "length" }]);
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: "", is_error: false }));
    await expect(runTurn(setup(api, callTool).opts)).resolves.toEqual({ reason: "max_tokens" });
    expect(callTool).not.toHaveBeenCalled();
  });

  it("DeepSeek reasoning is echoed within a tool loop, dropped on the next user turn", async () => {
    const api = fakeOpenAi([{ blocks: [{ tool: "get_project_overview", id: "r1", input: {} }], reasoning: "Need the overview." }, { blocks: [{ text: "ok" }] }]);
    const { opts } = setup(api, async () => ({ content: "{}", is_error: false }));
    await runTurn(opts);
    expect(msgs(api, 1)[2]).toMatchObject({ role: "assistant", reasoning_content: "Need the overview." });
    opts.session.addUser(["next"]);
    expect(opts.session.history[2]).not.toHaveProperty("reasoning_content");
  });

  it("keyless local servers get no authorization header", async () => {
    const api = fakeOpenAi([{ blocks: [{ text: "hi" }] }]);
    const { opts } = setup(api, vi.fn(), {});
    await runTurn({ ...opts, conn: { baseUrl: "http://localhost:11434/v1/", apiKey: null, model: "qwen3" } });
    expect(api.urls[0]).toBe("http://localhost:11434/v1/chat/completions");
    expect(api.headers[0]!.authorization).toBeUndefined();
  });
});

describe("OpenAI-compatible errors", () => {
  const run = async (api: FakeOpenAi) => runTurn(setup(api, vi.fn()).opts);

  it("401: key rejected", async () => {
    const err = await run(fakeOpenAi([errorResponse(401, "Incorrect API key provided")])).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ProviderError);
    expect(err).toMatchObject({ kind: "auth", status: 401, keyRejected: true });
    expect((err as Error).message).toMatch(/rejected by OpenAI/);
    expect((err as Error).message).not.toContain(KEY);
  });

  it("models without tool support: a clear message", async () => {
    const err = await run(fakeOpenAi([errorResponse(400, "registry.ollama.ai/library/gemma3:latest does not support tools")])).catch((e: unknown) => e);
    expect(err).toMatchObject({ kind: "no_tools" });
    expect((err as Error).message).toMatch(/doesn't support tool calls/);
    expect(noToolSupport('"auto" tool choice requires --enable-auto-tool-choice and --tool-call-parser to be set')).toBe(true);
    expect(noToolSupport("Invalid schema for function 'add_notes'")).toBe(false);
  });

  it("429, 404, 5xx, and an error inside the stream", async () => {
    await expect(run(fakeOpenAi([errorResponse(429, "slow down")]))).rejects.toMatchObject({ kind: "rate_limit" });
    await expect(run(fakeOpenAi([errorResponse(404, "The model `gpt-9` does not exist")]))).rejects.toMatchObject({ kind: "not_found" });
    await expect(run(fakeOpenAi([errorResponse(503, "overloaded")]))).rejects.toMatchObject({ kind: "server" });
    const midStream = () => new Response(`data: ${JSON.stringify({ error: { message: "upstream died" } })}\n\n`, { status: 200 });
    await expect(run(fakeOpenAi([midStream]))).rejects.toMatchObject({ message: "OpenAI: upstream died" });
  });

  it("a blocked request (CORS) is told apart from an unreachable server", async () => {
    const session = createSession("deepseek", ["s"], TOOLS);
    session.addUser(["hi"]);
    const conn = { baseUrl: "https://api.deepseek.com", apiKey: KEY, model: "deepseek-chat" };
    // CORS: the real request fails, the no-cors probe gets an opaque answer.
    const fetchCors = vi.spyOn(http, "fetch").mockImplementation(async (_u, init) => {
      if (init?.mode === "no-cors") return new Response(null, { status: 200 });
      throw new TypeError("Failed to fetch");
    });
    const cors = await session.step(conn, new AbortController().signal, () => {}).catch((e: unknown) => e);
    expect(cors).toMatchObject({ kind: "cors" });
    expect((cors as Error).message).toMatch(/api\.deepseek\.com blocked the request .*CORS/);
    // The probe carries no key.
    const probe = fetchCors.mock.calls.find(([, init]) => init?.mode === "no-cors")!;
    expect(JSON.stringify(probe)).not.toContain(KEY);

    vi.spyOn(http, "fetch").mockRejectedValue(new TypeError("Failed to fetch"));
    const down = await session.step(conn, new AbortController().signal, () => {}).catch((e: unknown) => e);
    expect(down).toMatchObject({ kind: "connection" });

    const local = createSession("ollama", ["s"], TOOLS);
    local.addUser(["hi"]);
    const off = await local.step({ baseUrl: "http://localhost:11434/v1", apiKey: null, model: "qwen3" }, new AbortController().signal, () => {}).catch((e: unknown) => e);
    expect((off as Error).message).toMatch(/Is Ollama running/);
  });
});

describe("test connection", () => {
  it("lists the models with the key", async () => {
    const fetch = vi.spyOn(http, "fetch").mockImplementation(async () => Response.json({ data: [{ id: "gpt-5" }, { id: "gpt-4.1" }] }));
    await expect(testConnection("openai", { baseUrl: "https://api.openai.com/v1", apiKey: KEY, model: "gpt-5" })).resolves.toBe("Connected to api.openai.com.");
    expect(fetch.mock.calls[0]![0]).toBe("https://api.openai.com/v1/models");
    await expect(testConnection("openai", { baseUrl: "https://api.openai.com/v1", apiKey: KEY, model: "gpt-9" })).resolves.toMatch(/doesn't list the model "gpt-9"/);
    // Gemini lists `models/…` ids.
    fetch.mockImplementation(async () => Response.json({ data: [{ id: "models/gemini-2.5-flash" }] }));
    await expect(testConnection("gemini", { baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai", apiKey: KEY, model: "gemini-2.5-flash" })).resolves.toMatch(/^Connected/);
    fetch.mockResolvedValue(errorResponse(401, "bad key")());
    await expect(testConnection("openai", { baseUrl: "https://api.openai.com/v1", apiKey: "x", model: "gpt-5" })).rejects.toMatchObject({ kind: "auth" });
  });
});

describe("helpers", () => {
  it("tool specs become functions", () => {
    expect(toOpenAiTools(TOOLS)[1]).toEqual({ type: "function", function: { name: "get_project_overview", description: "Overview", parameters: { type: "object" } } });
  });

  it("parses tool arguments", () => {
    expect(parseArgs("")).toEqual({ value: {} });
    expect(parseArgs('{"a":1}')).toEqual({ value: { a: 1 } });
    expect(parseArgs("[1]")).toHaveProperty("error");
    expect(parseArgs("{")).toHaveProperty("error");
  });

  it("SSE: CRLF, split chunks, comments, multi-line data", async () => {
    const enc = new TextEncoder();
    const parts = [": hi\r\n\r\ndata: {\"a\"", ":1}\r\n\r\ndata: x\ndata: y\n\ndata: [DONE]"];
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        for (const p of parts) c.enqueue(enc.encode(p));
        c.close();
      },
    });
    const out: string[] = [];
    for await (const d of sseData(body)) out.push(d);
    expect(out).toEqual(['{"a":1}', "x\ny", "[DONE]"]);
  });
});
