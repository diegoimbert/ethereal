import type { BetaMessageParam } from "@anthropic-ai/sdk/resources/beta/messages/messages";
import { describe, expect, it, vi } from "vitest";
import type { ToolOutcome } from "./agentApi";
import { cappedResult, runTurn, STOPPED_RESULT, toApiTools, type LoopEvent, type TurnOptions } from "./loop";
import { fakeApi, fakeClient, type FakeApi } from "./testing";

const TOOLS = toApiTools([
  { name: "create_track", description: "Create a track", input_schema: JSON.stringify({ type: "object", properties: { kind: { type: "string" } }, required: ["kind"] }) },
  { name: "get_project_overview", description: "Overview", input_schema: "{}" },
]);

function setup(api: FakeApi, callTool: TurnOptions["callTool"], extra: Partial<TurnOptions> = {}) {
  const events: LoopEvent[] = [];
  const messages: BetaMessageParam[] = [{ role: "user", content: "Make a bass track" }];
  const abort = new AbortController();
  const opts: TurnOptions = {
    client: fakeClient(api),
    model: "claude-sonnet-5-5",
    system: [{ type: "text", text: "system" }],
    tools: TOOLS,
    messages,
    maxIterations: 25,
    signal: abort.signal,
    callTool,
    onEvent: (e) => events.push(e),
    ...extra,
  };
  return { opts, events, messages, abort };
}

type Block = { type: string; [k: string]: unknown };
const blocksOf = (m: BetaMessageParam | undefined) => (m?.content ?? []) as Block[];

describe("runTurn", () => {
  it("runs tool_use through CallTool, returns tool_result, loops until end_turn", async () => {
    const api = fakeApi([
      { blocks: [{ text: "Creating it." }, { tool: "create_track", id: "toolu_1", input: { kind: "midi", name: "Bass" } }] },
      { blocks: [{ text: "Done: added Bass." }] },
    ]);
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: '{"track_id":"T1"}', is_error: false }));
    const { opts, events, messages } = setup(api, callTool);

    await expect(runTurn(opts)).resolves.toEqual({ reason: "end_turn" });

    expect(callTool).toHaveBeenCalledWith("create_track", { kind: "midi", name: "Bass" });
    expect(api.requests).toHaveLength(2);
    // Tools sent with their parsed schemas; streaming; model; caching.
    const first = api.requests[0]!;
    expect(first.model).toBe("claude-sonnet-5-5");
    expect(first.stream).toBe(true);
    expect((first.tools as Array<{ name: string; input_schema: unknown }>)[0]).toMatchObject({
      name: "create_track",
      input_schema: { type: "object", properties: { kind: { type: "string" } } },
    });
    expect(first.cache_control).toEqual({ type: "ephemeral" });
    // Browser access header + key header (the key goes only to the API).
    expect(api.headers[0]!["anthropic-dangerous-direct-browser-access"]).toBe("true");
    expect(api.headers[0]!["x-api-key"]).toMatch(/^sk-ant-/);

    // The second request carries the assistant's tool_use and our tool_result.
    const second = api.requests[1]!.messages as BetaMessageParam[];
    expect(second).toHaveLength(3);
    expect(blocksOf(second[1]).map((b) => b.type)).toEqual(["text", "tool_use"]);
    expect(blocksOf(second[2])).toEqual([{ type: "tool_result", tool_use_id: "toolu_1", content: '{"track_id":"T1"}' }]);

    // History: user, assistant(tool_use), user(tool_result), assistant(final).
    expect(messages.map((m) => m.role)).toEqual(["user", "assistant", "user", "assistant"]);
    expect(blocksOf(messages[3])[0]).toMatchObject({ type: "text", text: "Done: added Bass." });

    // Events: streamed text, the call, its result, more text.
    const kinds = events.map((e) => e.type);
    expect(kinds.filter((k) => k === "text").length).toBeGreaterThanOrEqual(2);
    expect(events).toContainEqual({ type: "tool_call", id: "toolu_1", name: "create_track", input: { kind: "midi", name: "Bass" } });
    expect(events).toContainEqual({ type: "tool_result", id: "toolu_1", content: '{"track_id":"T1"}', is_error: false });
    expect(kinds.indexOf("tool_call")).toBeLessThan(kinds.indexOf("tool_result"));
    const text = events.flatMap((e) => (e.type === "text" ? [e.delta] : [])).join("");
    expect(text).toBe("Creating it.Done: added Bass.");
  });

  it("passes is_error through to the model", async () => {
    const api = fakeApi([
      { blocks: [{ tool: "create_track", id: "toolu_1", input: { kind: "banjo" } }] },
      { blocks: [{ text: "Sorry, that kind doesn't exist." }] },
    ]);
    const { opts, events } = setup(api, async () => ({ content: "`kind` must be one of midi, audio", is_error: true }));
    await expect(runTurn(opts)).resolves.toEqual({ reason: "end_turn" });
    const results = blocksOf((api.requests[1]!.messages as BetaMessageParam[])[2]);
    expect(results).toEqual([{ type: "tool_result", tool_use_id: "toolu_1", content: "`kind` must be one of midi, audio", is_error: true }]);
    expect(events).toContainEqual({ type: "tool_result", id: "toolu_1", content: "`kind` must be one of midi, audio", is_error: true });
  });

  it("answers parallel tool calls in one user message", async () => {
    const api = fakeApi([
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
    const sent = api.requests[1]!.messages as BetaMessageParam[];
    expect(blocksOf(sent[2]).map((b) => b.tool_use_id)).toEqual(["a", "b"]);
  });

  it("Stop aborts the stream: no tool runs, resolves stopped", async () => {
    const api = fakeApi(["hang"]);
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: "", is_error: false }));
    const { opts, events, abort, messages } = setup(api, callTool);
    const turn = runTurn(opts);
    await vi.waitFor(() => expect(events.some((e) => e.type === "text")).toBe(true));
    abort.abort();
    await expect(turn).resolves.toEqual({ reason: "stopped" });
    expect(callTool).not.toHaveBeenCalled();
    // The partial answer isn't added to the history (it stays append-only and valid).
    expect(messages).toHaveLength(1);
  });

  it("Stop during a tool round: the rest of the calls are not run and the loop ends", async () => {
    const api = fakeApi([
      {
        blocks: [
          { tool: "create_track", id: "a", input: { kind: "midi" } },
          { tool: "create_track", id: "b", input: { kind: "audio" } },
        ],
      },
    ]);
    const held: { abort?: AbortController } = {};
    const callTool = vi.fn(async (): Promise<ToolOutcome> => {
      held.abort!.abort(); // the user presses Stop while the first tool runs
      return { content: "ok", is_error: false };
    });
    const s = setup(api, callTool);
    held.abort = s.abort;
    await expect(runTurn(s.opts)).resolves.toEqual({ reason: "stopped" });
    expect(callTool).toHaveBeenCalledTimes(1);
    expect(api.requests).toHaveLength(1);
    // Every tool_use still gets its tool_result (the history stays valid for the next turn).
    expect(blocksOf(s.messages.at(-1))).toEqual([
      { type: "tool_result", tool_use_id: "a", content: "ok" },
      { type: "tool_result", tool_use_id: "b", content: STOPPED_RESULT, is_error: true },
    ]);
  });

  it("caps tool rounds per turn", async () => {
    const api = fakeApi((n) => ({ blocks: [{ tool: "get_project_overview", id: `t${n}`, input: {} }] }));
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: "{}", is_error: false }));
    const { opts, messages } = setup(api, callTool, { maxIterations: 3 });
    await expect(runTurn(opts)).resolves.toEqual({ reason: "capped", iterations: 3 });
    expect(callTool).toHaveBeenCalledTimes(3);
    expect(api.requests).toHaveLength(4);
    expect(blocksOf(messages.at(-1))).toEqual([{ type: "tool_result", tool_use_id: "t3", content: cappedResult(3), is_error: true }]);
  });

  it("never runs a tool cut off at max_tokens", async () => {
    const api = fakeApi([{ blocks: [{ tool: "create_track", id: "x", input: { kind: "midi" } }], stop: "max_tokens" }]);
    const callTool = vi.fn(async (): Promise<ToolOutcome> => ({ content: "", is_error: false }));
    const { opts } = setup(api, callTool);
    await expect(runTurn(opts)).resolves.toEqual({ reason: "max_tokens" });
    expect(callTool).not.toHaveBeenCalled();
  });

  it("sends the server-side fallback only for Sonnet/Opus 5.5", async () => {
    const api = fakeApi([{ blocks: [{ text: "hi" }] }, { blocks: [{ text: "hi" }] }]);
    await runTurn(setup(api, vi.fn()).opts);
    await runTurn(setup(api, vi.fn(), { model: "claude-haiku-4-5-20251001" }).opts);
    expect(api.requests[0]!.fallbacks).toBe("default");
    expect(api.headers[0]!["anthropic-beta"]).toContain("server-side-fallback-2026-07-01");
    expect(api.requests[1]!.fallbacks).toBeUndefined();
    expect(api.requests[1]!.model).toBe("claude-haiku-4-5-20251001");
  });

  it("rejects on API errors (e.g. a bad key)", async () => {
    const api: FakeApi = {
      requests: [],
      headers: [],
      fetch: (async () =>
        new Response(JSON.stringify({ type: "error", error: { type: "authentication_error", message: "invalid x-api-key" } }), {
          status: 401,
          headers: { "content-type": "application/json" },
        })) as typeof fetch,
    };
    await expect(runTurn(setup(api, vi.fn()).opts)).rejects.toMatchObject({ status: 401 });
  });
});

describe("toApiTools", () => {
  it("parses input_schema JSON text and tolerates bad schemas", () => {
    const [a, b] = toApiTools([
      { name: "a", description: "A", input_schema: '{"type":"object","properties":{"x":{"type":"number"}}}' },
      { name: "b", description: "B", input_schema: "not json" },
    ]);
    expect(a).toEqual({ name: "a", description: "A", input_schema: { type: "object", properties: { x: { type: "number" } } } });
    expect(b!.input_schema).toEqual({ type: "object" });
  });
});
