import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { AiChatPanel } from "./AiChatPanel";
import { clientFactory, resetAiChat } from "./chatStore";
import { bridgeIpc, CLAUDE_MCP_ADD } from "./mcpBridge";
import { isAiShortcut, openAiChat } from "./open";
import { reloadAiSettings, useAiSettings } from "./settings";
import { http } from "./providers/openai";
import { errorResponse, fakeApi, fakeClient, fakeOpenAi, type FakeApi, type FakeOpenAi, type ScriptedResponse } from "./testing";
import { useShellStore, resetShell } from "@/app/shell/shellStore";

const KEY = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123";
const project = () => useProjectStore.getState().project!;

let api: FakeApi;
function script(responses: Array<ScriptedResponse | "hang"> | ((n: number) => ScriptedResponse | "hang")) {
  api = fakeApi(responses);
  vi.spyOn(clientFactory, "create").mockImplementation((key) => fakeClient(api, key));
}

beforeEach(() => {
  localStorage.clear();
  reloadAiSettings();
  resetAiChat();
});
afterEach(() => {
  resetStores();
  resetShell();
  vi.restoreAllMocks();
});

const toolResult = (n: number) => JSON.parse(String((((api.requests[n]!.messages as Array<{ content: Array<{ content: string }> }>).at(-1)!.content)[0]!).content));

describe("AiChatPanel", () => {
  it("asks for an API key first, then shows the composer", async () => {
    await renderWithMock(<AiChatPanel />);
    expect(screen.getByText("Ask AI to edit your project")).toBeInTheDocument();
    expect(screen.getByText(/local storage/)).toBeInTheDocument();
    expect(screen.queryByTestId("ai-input")).toBeNull();
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: KEY } });
    fireEvent.click(screen.getByRole("button", { name: "Save key" }));
    expect(await screen.findByTestId("ai-input")).toBeInTheDocument();
    expect(useAiSettings.getState().apiKey).toBe(KEY);
  });

  it("runs a tool-using conversation against the mock agent: edits land in the document", async () => {
    useAiSettings.getState().setApiKey(KEY);
    script((n) => {
      if (n === 0) return { blocks: [{ text: "Creating the track." }, { tool: "create_track", id: "t1", input: { kind: "midi", name: "Bass" } }] };
      if (n === 1) {
        const { track_id } = toolResult(1);
        return { blocks: [{ tool: "create_midi_clip", id: "t2", input: { track_id, start_beats: 0, length_beats: 4 } }] };
      }
      if (n === 2) {
        const { clip_id } = toolResult(2);
        const notes = [48, 51, 55, 60].map((pitch, i) => ({ pitch, start_beats: i, duration_beats: 1 }));
        return { blocks: [{ tool: "add_notes", id: "t3", input: { clip_id, notes } }] };
      }
      return { blocks: [{ text: "Added a Bass track with a C minor arpeggio." }] };
    });
    await renderWithMock(<AiChatPanel />);
    const tracksBefore = new Set(Object.keys(project().tracks));

    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "Create a track called Bass and add a C minor arpeggio" } });
    fireEvent.keyDown(screen.getByTestId("ai-input"), { key: "Enter" });

    expect(await screen.findByText("Added a Bass track with a C minor arpeggio.")).toBeInTheDocument();
    const added = Object.values(project().tracks).filter((t) => !tracksBefore.has(t.id));
    expect(added.map((t) => t.name)).toEqual(["Bass"]);
    const bass = added[0]!;
    const clip = Object.values(project().clips).find((c) => c.track === bass.id)!;
    expect(Object.values(project().notes).filter((n) => n.clip === clip.id).map((n) => n.pitch).sort()).toEqual([48, 51, 55, 60]);

    // Chips: name + short input, final state.
    const chips = screen.getAllByTestId("ai-tool");
    expect(chips.map((c) => c.getAttribute("data-tool"))).toEqual(["create_track", "create_midi_clip", "add_notes"]);
    expect(chips.every((c) => c.getAttribute("data-state") === "ok")).toBe(true);
    expect(within(chips[0]!).getByText('kind: "midi", name: "Bass"')).toBeInTheDocument();
    expect(within(chips[2]!).getByText(/notes: 4 items/)).toBeInTheDocument();

    // The system prompt carries the project overview; the tools come from ListTools.
    const system = api.requests[0]!.system as Array<{ text: string }>;
    expect(system[0]!.text).toMatch(/beats/);
    expect(system[1]!.text).toMatch(/get_project_overview/);
    expect((api.requests[0]!.tools as Array<{ name: string }>).map((t) => t.name)).toContain("add_notes");

    // New chat clears the conversation.
    fireEvent.click(screen.getByRole("button", { name: "New chat" }));
    expect(screen.queryAllByTestId("ai-tool")).toHaveLength(0);
  });

  it("shows tool errors on the chip", async () => {
    useAiSettings.getState().setApiKey(KEY);
    script([{ blocks: [{ tool: "create_track", id: "t1", input: { kind: "banjo" } }] }, { blocks: [{ text: "That kind doesn't exist." }] }]);
    await renderWithMock(<AiChatPanel />);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "banjo track" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    await screen.findByText("That kind doesn't exist.");
    const chip = screen.getByTestId("ai-tool");
    expect(chip.getAttribute("data-state")).toBe("error");
    expect(within(chip).getAllByText(/must be one of/).length).toBeGreaterThan(0);
  });

  it("Stop interrupts the stream", async () => {
    useAiSettings.getState().setApiKey(KEY);
    script(["hang"]);
    await renderWithMock(<AiChatPanel />);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "hello" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    await screen.findByText("Let me");
    fireEvent.click(screen.getByTestId("ai-stop"));
    expect(await screen.findByText("Stopped.")).toBeInTheDocument();
    expect(screen.getByTestId("ai-send")).toBeInTheDocument();
  });

  it("a rejected key brings back the key setup", async () => {
    useAiSettings.getState().setApiKey(KEY);
    api = { requests: [], headers: [], fetch: (async () => new Response(JSON.stringify({ type: "error", error: { type: "authentication_error", message: "invalid x-api-key" } }), { status: 401, headers: { "content-type": "application/json" } })) as typeof fetch };
    vi.spyOn(clientFactory, "create").mockImplementation((key) => fakeClient(api, key));
    await renderWithMock(<AiChatPanel />);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "hello" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    expect(await screen.findByRole("alert")).toHaveTextContent(/API key was rejected/);
    expect(screen.getByText("Your API key was rejected")).toBeInTheDocument();
  });

  it("settings: masked key, tool-round cap; no MCP section in the web build", async () => {
    useAiSettings.getState().setApiKey(KEY);
    await renderWithMock(<AiChatPanel />);
    fireEvent.click(screen.getByRole("button", { name: "AI settings" }));
    expect(screen.getByTestId("ai-key-masked")).toHaveTextContent("sk-ant-…0123");
    expect(screen.queryByTestId("ai-mcp")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(useAiSettings.getState().apiKey).toBeNull();
  });
});

describe("other providers (OpenAI-compatible)", () => {
  const OAI_KEY = "sk-proj-test-abcdefghijklmnopqrstuvwx";
  let oai: FakeOpenAi;
  function scriptOai(responses: Parameters<typeof fakeOpenAi>[0]) {
    oai = fakeOpenAi(responses);
    vi.spyOn(http, "fetch").mockImplementation(oai.fetch);
  }
  const lastTool = (n: number) => JSON.parse((oai.requests[n]!.messages as Array<{ content: string }>).at(-1)!.content) as Record<string, string>;

  it("first run: pick OpenAI, save its key, run a tool-using conversation", async () => {
    scriptOai((n) => {
      if (n === 0) return { blocks: [{ text: "Creating the track." }, { tool: "create_track", id: "c1", input: { kind: "midi", name: "Bass" } }] };
      if (n === 1) return { blocks: [{ tool: "create_midi_clip", id: "c2", input: { track_id: lastTool(1).track_id, start_beats: 0, length_beats: 4 } }] };
      if (n === 2) {
        const notes = [48, 51, 55, 60].map((pitch, i) => ({ pitch, start_beats: i, duration_beats: 1 }));
        return { blocks: [{ tool: "add_notes", id: "c3", input: { clip_id: lastTool(2).clip_id, notes } }] };
      }
      return { blocks: [{ text: "Added a Bass track with a C minor arpeggio." }] };
    });
    const anthropic = vi.spyOn(clientFactory, "create");
    await renderWithMock(<AiChatPanel />);
    fireEvent.click(screen.getByRole("combobox", { name: "Provider" }));
    fireEvent.click(await screen.findByRole("option", { name: "OpenAI" }));
    expect(screen.getByText(/only ever sent to api\.openai\.com/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: OAI_KEY } });
    fireEvent.click(screen.getByRole("button", { name: "Save key" }));
    expect(useAiSettings.getState()).toMatchObject({ provider: "openai", apiKey: OAI_KEY, model: "gpt-5" });
    expect(localStorage.getItem("eth.ai.apiKey.openai")).toBe(OAI_KEY);
    expect(localStorage.getItem("eth.ai.apiKey")).toBeNull();

    const tracksBefore = new Set(Object.keys(project().tracks));
    fireEvent.change(await screen.findByTestId("ai-input"), { target: { value: "Create a track called Bass and add a C minor arpeggio" } });
    fireEvent.keyDown(screen.getByTestId("ai-input"), { key: "Enter" });
    expect(await screen.findByText("Added a Bass track with a C minor arpeggio.")).toBeInTheDocument();

    const bass = Object.values(project().tracks).find((t) => !tracksBefore.has(t.id))!;
    expect(bass.name).toBe("Bass");
    const clip = Object.values(project().clips).find((c) => c.track === bass.id)!;
    expect(Object.values(project().notes).filter((n) => n.clip === clip.id).map((n) => n.pitch).sort()).toEqual([48, 51, 55, 60]);
    expect(screen.getAllByTestId("ai-tool").map((c) => c.getAttribute("data-state"))).toEqual(["ok", "ok", "ok"]);

    // Same system prompt and tools as with Anthropic; only OpenAI saw the key.
    const system = (oai.requests[0]!.messages as Array<{ role: string; content: string }>)[0]!;
    expect(system.role).toBe("system");
    expect(system.content).toMatch(/You are the assistant built into Ethereal/);
    expect(system.content).toMatch(/get_project_overview/);
    expect((oai.requests[0]!.tools as Array<{ function: { name: string } }>).map((t) => t.function.name)).toContain("add_notes");
    expect(oai.urls.every((u) => u.startsWith("https://api.openai.com/v1/"))).toBe(true);
    expect(oai.headers[0]!.authorization).toBe(`Bearer ${OAI_KEY}`);
    expect(anthropic).not.toHaveBeenCalled();
  });

  it("local servers need no key; a model without tools gets a clear error", async () => {
    useAiSettings.getState().setProvider("ollama");
    scriptOai([errorResponse(400, "registry.ollama.ai/library/gemma3:latest does not support tools")]);
    await renderWithMock(<AiChatPanel />);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "hello" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    expect(await screen.findByRole("alert")).toHaveTextContent(/"qwen3" doesn't support tool calls/);
    expect(oai.urls[0]).toBe("http://localhost:11434/v1/chat/completions");
    expect(screen.getByTestId("ai-send")).toBeInTheDocument();
  });

  it("a CORS block is explained in the panel", async () => {
    useAiSettings.getState().setProvider("lmstudio");
    vi.spyOn(http, "fetch").mockImplementation(async (_u, init) => {
      if (init?.mode === "no-cors") return new Response(null, { status: 200 });
      throw new TypeError("Failed to fetch");
    });
    await renderWithMock(<AiChatPanel />);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "hello" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    expect(await screen.findByRole("alert")).toHaveTextContent(/localhost:1234 blocked the request .*CORS.*Enable CORS/);
  });

  it("Stop interrupts the stream", async () => {
    useAiSettings.getState().setProvider("deepseek");
    useAiSettings.getState().setApiKey("sk-deepseek-0000000000000000");
    scriptOai(["hang"]);
    await renderWithMock(<AiChatPanel />);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "hello" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    await screen.findByText("Let");
    fireEvent.click(screen.getByTestId("ai-stop"));
    expect(await screen.findByText("Stopped.")).toBeInTheDocument();
  });

  it("a rejected key brings back the key setup for that provider", async () => {
    useAiSettings.getState().setProvider("groq");
    useAiSettings.getState().setApiKey("gsk_badbadbadbad");
    scriptOai([errorResponse(401, "Invalid API Key")]);
    await renderWithMock(<AiChatPanel />);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "hello" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    expect(await screen.findByRole("alert")).toHaveTextContent(/rejected by Groq/);
    expect(screen.getByText("Your API key was rejected")).toBeInTheDocument();
  });

  it("switching provider mid-conversation starts the model fresh, with a notice", async () => {
    useAiSettings.getState().setApiKey(KEY);
    script([{ blocks: [{ text: "Hi from Claude." }] }]);
    await renderWithMock(<AiChatPanel />);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "hello" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    await screen.findByText("Hi from Claude.");
    act(() => {
      useAiSettings.getState().setProvider("ollama");
    });
    scriptOai([{ blocks: [{ text: "Hi from Ollama." }] }]);
    fireEvent.change(screen.getByTestId("ai-input"), { target: { value: "hello again" } });
    fireEvent.click(screen.getByTestId("ai-send"));
    await screen.findByText("Hi from Ollama.");
    expect(screen.getByText(/Switched to Ollama \(local\): it starts fresh/)).toBeInTheDocument();
    expect((oai.requests[0]!.messages as unknown[]).length).toBe(2); // system + this message
  });

  it("settings: provider picker, base URL, model, per-provider key, test connection", async () => {
    useAiSettings.getState().setApiKey(KEY);
    await renderWithMock(<AiChatPanel />);
    fireEvent.click(screen.getByRole("button", { name: "AI settings" }));
    expect(screen.getByLabelText("Base URL")).toHaveValue("https://api.anthropic.com");
    fireEvent.click(screen.getByRole("combobox", { name: "Provider" }));
    fireEvent.click(await screen.findByRole("option", { name: "DeepSeek" }));
    expect(screen.getByLabelText("Base URL")).toHaveValue("https://api.deepseek.com");
    expect(screen.getByLabelText("Model name")).toHaveValue("deepseek-chat");
    // No DeepSeek key yet: the Anthropic one stays Anthropic's.
    expect(screen.queryByTestId("ai-key-masked")).toBeNull();
    expect(screen.getByTestId("ai-test-connection")).toBeDisabled();
    fireEvent.change(screen.getByLabelText("API key"), { target: { value: "sk-deepseek-0000000000000000" } });
    fireEvent.click(screen.getByRole("button", { name: "Save key" }));
    expect(screen.getByTestId("ai-key-masked")).toHaveTextContent("sk-deep…0000");
    expect(useAiSettings.getState().keys).toMatchObject({ anthropic: KEY, deepseek: "sk-deepseek-0000000000000000" });

    // Model: free text, committed on Enter.
    fireEvent.change(screen.getByLabelText("Model name"), { target: { value: "deepseek-reasoner" } });
    fireEvent.keyDown(screen.getByLabelText("Model name"), { key: "Enter" });
    expect(useAiSettings.getState().model).toBe("deepseek-reasoner");
    // Base URL: committed on blur, with a reset.
    fireEvent.change(screen.getByLabelText("Base URL"), { target: { value: "https://proxy.example/v1/" } });
    fireEvent.blur(screen.getByLabelText("Base URL"));
    expect(useAiSettings.getState().configs.deepseek.baseUrl).toBe("https://proxy.example/v1");
    expect(screen.getByText(/only ever sent to proxy\.example/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reset base URL" }));
    expect(useAiSettings.getState().configs.deepseek.baseUrl).toBe("https://api.deepseek.com");

    const fetch = vi.spyOn(http, "fetch").mockImplementation(async () => Response.json({ data: [{ id: "deepseek-chat" }, { id: "deepseek-reasoner" }] }));
    fireEvent.click(screen.getByTestId("ai-test-connection"));
    expect(await screen.findByTestId("ai-test-result")).toHaveTextContent("Connected to api.deepseek.com.");
    expect(fetch.mock.calls[0]![0]).toBe("https://api.deepseek.com/models");
  });
});

describe("MCP bridge toggle (desktop)", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("shows the toggle, calls the Tauri commands, then port, clients and the claude mcp add line", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    let status = { enabled: false, port: null as number | null, connected_clients: 0 };
    const invoke = vi.spyOn(bridgeIpc, "invoke").mockImplementation((async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "agent_bridge_set_enabled") status = args?.enabled ? { enabled: true, port: 43117, connected_clients: 1 } : { enabled: false, port: null, connected_clients: 0 };
      return status;
    }) as typeof bridgeIpc.invoke);
    useAiSettings.getState().setApiKey(KEY);
    await renderWithMock(<AiChatPanel />);
    fireEvent.click(screen.getByRole("button", { name: "AI settings" }));
    const toggle = await screen.findByRole("switch");
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    expect(await screen.findByTestId("ai-mcp-status")).toHaveTextContent("127.0.0.1:43117 · 1 connected");
    expect(screen.getByText(CLAUDE_MCP_ADD)).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("agent_bridge_set_enabled", { enabled: true });
    expect(invoke).toHaveBeenCalledWith("agent_bridge_status");
  });
});

describe("opening the AI chat", () => {
  it("palette/shortcut open the Ask AI tab", () => {
    expect(isAiShortcut({ metaKey: true, ctrlKey: false, shiftKey: true, altKey: false, key: "A", code: "KeyA" })).toBe(true);
    expect(isAiShortcut({ metaKey: true, ctrlKey: false, shiftKey: false, altKey: false, key: "a", code: "KeyA" })).toBe(false);
    act(() => openAiChat());
    expect(useShellStore.getState().left).toMatchObject({ open: true, tab: "ai" });
    // Opening again keeps it open (doesn't toggle it closed).
    act(() => openAiChat());
    expect(useShellStore.getState().left.open).toBe(true);
  });
});

