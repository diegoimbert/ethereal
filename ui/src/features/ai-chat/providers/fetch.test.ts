// Provider requests on the desktop go through tauri-plugin-http (Rust-side, no CORS); on the
// web through the browser's fetch.
import { afterEach, beforeEach, describe, expect, it, vi, type Mock, type MockInstance } from "vitest";
import { createSession, ProviderError, testConnection, toolDefs, type LoopEvent } from ".";
import { clientFactory } from "./anthropic";
import { desktopHttp, resetDesktopFetch } from "./fetch";
import { OpenAiSession } from "./openai";

const KEY = "sk-test-desktop-0000000000000000";
const OPENAI = { baseUrl: "https://api.openai.com/v1", apiKey: KEY, model: "gpt-5" };
const enc = new TextEncoder();
const sse = (o: object) => enc.encode(`data: ${JSON.stringify(o)}\n\n`);
const delta = (content: string) => sse({ id: "c", object: "chat.completion.chunk", choices: [{ index: 0, delta: { content }, finish_reason: null }] });

const setTauri = (on: boolean) => {
  const w = window as unknown as Record<string, unknown>;
  if (on) w.__TAURI_INTERNALS__ = {};
  else delete w.__TAURI_INTERNALS__;
};

let invoke: Mock<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>;
let tauriFetch: Mock<(input: unknown, init?: RequestInit) => Promise<Response>>;
let browserFetch: MockInstance<typeof fetch>;

beforeEach(() => {
  resetDesktopFetch();
  invoke = vi.fn(async () => undefined);
  tauriFetch = vi.fn(async () => Response.json({ data: [{ id: "gpt-5" }] }));
  vi.spyOn(desktopHttp, "invoke").mockImplementation(invoke);
  vi.spyOn(desktopHttp, "fetch").mockImplementation(tauriFetch as unknown as typeof fetch);
  browserFetch = vi.spyOn(globalThis, "fetch").mockImplementation(async () => Response.json({ data: [{ id: "gpt-5" }] }));
});

afterEach(() => {
  setTauri(false);
  vi.restoreAllMocks();
});

const headersOf = (call: unknown[]) => new Headers((call[1] as RequestInit).headers);

describe("provider fetch", () => {
  it("web: the browser's fetch, nothing through Tauri", async () => {
    await testConnection("openai", OPENAI);
    expect(browserFetch).toHaveBeenCalledTimes(1);
    expect(tauriFetch).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("desktop: OpenAI-compatible requests go through the Tauri fetch, after allowing the origin once", async () => {
    setTauri(true);
    await testConnection("openai", OPENAI);
    await testConnection("openai", OPENAI);
    expect(browserFetch).not.toHaveBeenCalled();
    expect(tauriFetch).toHaveBeenCalledTimes(2);
    expect(tauriFetch.mock.calls[0]![0]).toBe("https://api.openai.com/v1/models");
    const h = headersOf(tauriFetch.mock.calls[0]!);
    expect(h.get("authorization")).toBe(`Bearer ${KEY}`);
    // No Origin header (local servers refuse unknown origins).
    expect(h.get("origin")).toBe("");
    // The origin only (never the key), and once.
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("ai_http_allow", { baseUrl: "https://api.openai.com" });
  });

  it("desktop: the Anthropic SDK uses the Tauri fetch too", async () => {
    setTauri(true);
    tauriFetch.mockImplementation(async () => Response.json({ type: "model", id: "claude-sonnet-5-5", display_name: "Claude Sonnet 5.5" }));
    const client = clientFactory.create(KEY);
    await expect(client.models.retrieve("claude-sonnet-5-5")).resolves.toMatchObject({ id: "claude-sonnet-5-5" });
    expect(browserFetch).not.toHaveBeenCalled();
    expect(String(tauriFetch.mock.calls[0]![0])).toBe("https://api.anthropic.com/v1/models/claude-sonnet-5-5");
    expect(headersOf(tauriFetch.mock.calls[0]!).get("x-api-key")).toBe(KEY);
    expect(invoke).toHaveBeenCalledWith("ai_http_allow", { baseUrl: "https://api.anthropic.com" });
  });

  it("desktop: the response streams token by token", async () => {
    setTauri(true);
    let body!: ReadableStreamDefaultController<Uint8Array>;
    tauriFetch.mockImplementation(
      async () =>
        new Response(new ReadableStream<Uint8Array>({ start: (c) => void (body = c) }), { status: 200, headers: { "content-type": "text/event-stream" } }),
    );
    const session = createSession("openai", ["system"], toolDefs([])) as OpenAiSession;
    session.addUser(["hi"]);
    const texts: string[] = [];
    let seen: () => void = () => {};
    const onEvent = (e: LoopEvent) => {
      if (e.type === "text") {
        texts.push(e.delta);
        seen();
      }
    };
    const next = () => new Promise<void>((r) => (seen = r));
    const step = session.step(OPENAI, new AbortController().signal, onEvent);

    await vi.waitFor(() => expect(body).toBeDefined());
    let got = next();
    body.enqueue(delta("Hel"));
    await got;
    expect(texts).toEqual(["Hel"]); // before the rest of the response exists
    got = next();
    body.enqueue(delta("lo"));
    await got;
    expect(texts).toEqual(["Hel", "lo"]);
    body.enqueue(sse({ id: "c", object: "chat.completion.chunk", choices: [{ index: 0, delta: {}, finish_reason: "stop" }] }));
    body.enqueue(enc.encode("data: [DONE]\n\n"));
    body.close();
    await expect(step).resolves.toMatchObject({ stop: "end_turn", raw: { content: "Hello" } });
    expect(browserFetch).not.toHaveBeenCalled();
  });

  it("desktop: Stop mid-stream is an abort, not an error", async () => {
    setTauri(true);
    tauriFetch.mockImplementation(async (_url: unknown, init?: RequestInit) => {
      const stream = new ReadableStream<Uint8Array>({
        start(c) {
          c.enqueue(delta("Hel"));
          // As the plugin does: the body errors with a plain string on cancel.
          init?.signal?.addEventListener("abort", () => c.error("Request cancelled"));
        },
      });
      return new Response(stream, { status: 200 });
    });
    const session = createSession("openai", ["system"], toolDefs([])) as OpenAiSession;
    session.addUser(["hi"]);
    const abort = new AbortController();
    const step = session.step(OPENAI, abort.signal, (e) => e.type === "text" && abort.abort());
    await expect(step).rejects.toMatchObject({ name: "AbortError" });
  });

  it("desktop: a base URL the app refuses is a clear error, no CORS probe", async () => {
    setTauri(true);
    invoke.mockImplementation(async () => {
      throw "the base URL's host isn't a valid host name";
    });
    const err = await testConnection("custom", { baseUrl: "https://bad*host/v1", apiKey: "", model: "m" }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ProviderError);
    expect((err as ProviderError).message).toContain("the base URL's host isn't a valid host name");
    expect(tauriFetch).not.toHaveBeenCalled();
    expect(browserFetch).not.toHaveBeenCalled();
  });

  it("desktop: an unreachable server is a connection error, not CORS", async () => {
    setTauri(true);
    tauriFetch.mockImplementation(async () => {
      throw "error sending request for url (http://localhost:11434/v1/models)";
    });
    const err = await testConnection("ollama", { baseUrl: "http://localhost:11434/v1", apiKey: "", model: "qwen3" }).catch((e: unknown) => e);
    expect(err).toMatchObject({ kind: "connection" });
    expect((err as ProviderError).message).toContain("couldn’t reach localhost:11434");
    // No no-cors probe on the desktop: one request only.
    expect(tauriFetch).toHaveBeenCalledTimes(1);
  });
});
