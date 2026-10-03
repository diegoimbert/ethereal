// AI chat on the web build, against the real engine (WasmTransport -> controller Worker): the
// agent API (Command::Agent) is the controller's tool registry, the same one the MCP server
// uses. api.anthropic.com is stubbed with Playwright routing: a scripted, tool-using
// conversation ("create a track called Bass and add a C minor arpeggio") whose tool inputs
// use the ids the engine returned. Asserts the track, clip and notes appear in the
// arrangement, and that each tool call is one undo step.
//
// No sleeps: every step waits on UI or engine state (mirror via `window.__ether`).
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test, type Page, type Route } from "@playwright/test";
import type { Project } from "@/generated";
import { launch, newProject } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

/** Runs a command palette entry (its exact label). */
async function palette(page: Page, label: string): Promise<void> {
  await page.getByRole("button", { name: "Command palette" }).click();
  await page.getByRole("combobox", { name: "Search commands" }).fill(label);
  await page.getByRole("listbox", { name: "Commands" }).getByRole("option", { name: label }).first().click();
}

const KEY = "sk-ant-api03-e2e-test-key-0000000000000000";
const ARP = [48, 51, 55, 60, 63, 60, 55, 51]; // C minor, up and down

type Block = { text: string } | { tool: string; id: string; input: unknown };

/** One streamed Messages API response, as SSE. */
function sse(blocks: Block[], n: number): string {
  const events: object[] = [
    {
      type: "message_start",
      message: { id: `msg_${n}`, type: "message", role: "assistant", model: "claude-sonnet-5-5", content: [], stop_reason: null, stop_sequence: null, usage: { input_tokens: 1, output_tokens: 0 } },
    },
  ];
  blocks.forEach((b, index) => {
    if ("text" in b) {
      events.push({ type: "content_block_start", index, content_block: { type: "text", text: "" } });
      events.push({ type: "content_block_delta", index, delta: { type: "text_delta", text: b.text } });
    } else {
      events.push({ type: "content_block_start", index, content_block: { type: "tool_use", id: b.id, name: b.tool, input: {} } });
      events.push({ type: "content_block_delta", index, delta: { type: "input_json_delta", partial_json: JSON.stringify(b.input) } });
    }
    events.push({ type: "content_block_stop", index });
  });
  const stop = blocks.some((b) => "tool" in b) ? "tool_use" : "end_turn";
  events.push({ type: "message_delta", delta: { stop_reason: stop, stop_sequence: null }, usage: { output_tokens: 1 } });
  events.push({ type: "message_stop" });
  return events.map((e) => `event: ${(e as { type: string }).type}\ndata: ${JSON.stringify(e)}\n\n`).join("");
}

interface Request {
  model: string;
  tools: Array<{ name: string }>;
  system: Array<{ text: string }>;
  messages: Array<{ role: string; content: string | Array<{ type: string; content?: string; is_error?: boolean; tool_use_id?: string }> }>;
}

/** The JSON result of the last tool call (the request's last user message). */
function lastResult(req: Request): Record<string, unknown> {
  const content = req.messages.at(-1)!.content;
  if (typeof content === "string") throw new Error("no tool result");
  const r = content.find((b) => b.type === "tool_result")!;
  if (r.is_error) throw new Error(`tool failed: ${r.content}`);
  return JSON.parse(r.content!) as Record<string, unknown>;
}

/** Stubs api.anthropic.com with the scripted conversation; returns the requests it saw. */
async function stubClaude(page: Page): Promise<{ requests: Request[]; headers: Array<Record<string, string>> }> {
  const requests: Request[] = [];
  const headers: Array<Record<string, string>> = [];
  await page.route("https://api.anthropic.com/**", async (route: Route) => {
    const req = route.request();
    if (req.method() === "OPTIONS") {
      return route.fulfill({
        status: 204,
        headers: { "access-control-allow-origin": "*", "access-control-allow-headers": "*", "access-control-allow-methods": "POST" },
      });
    }
    const body = req.postDataJSON() as Request;
    const n = requests.length;
    requests.push(body);
    headers.push(req.headers());
    let blocks: Block[];
    switch (n) {
      case 0:
        blocks = [{ text: "I'll create the Bass track." }, { tool: "create_track", id: "toolu_1", input: { kind: "midi", name: "Bass" } }];
        break;
      case 1:
        blocks = [{ tool: "create_midi_clip", id: "toolu_2", input: { track_id: lastResult(body).track_id, start_beats: 0, length_beats: 4 } }];
        break;
      case 2:
        blocks = [
          {
            tool: "add_notes",
            id: "toolu_3",
            input: {
              clip_id: lastResult(body).clip_id,
              notes: ARP.map((pitch, i) => ({ pitch, start_beats: i * 0.5, duration_beats: 0.5, velocity: 100 })),
            },
          },
        ];
        break;
      default:
        blocks = [{ text: "Done: a Bass track with a one-bar C minor arpeggio." }];
    }
    await route.fulfill({
      status: 200,
      headers: { "content-type": "text/event-stream", "access-control-allow-origin": "*" },
      body: sse(blocks, n),
    });
  });
  return { requests, headers };
}

// Guard kept for branches that predate agent-api (#186, merged); on dev the registry exists.
const registry = fileURLToPath(new URL("../../../crates/ether-controller/src/agent", import.meta.url));
test.skip(!existsSync(registry), "the controller's agent tool registry (node agent-api) is not merged yet");

test("ai-chat: a scripted tool-using conversation edits the arrangement", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // Nothing but api.anthropic.com may see the key.
  const leaks: string[] = [];
  page.on("request", (r) => {
    if (!r.url().startsWith("https://api.anthropic.com/") && JSON.stringify([r.headers(), r.postData()]).includes(KEY)) leaks.push(r.url());
  });
  const api = await stubClaude(page);

  await launch(page);
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  // The app may start on the project screen with nothing open: create the project directly.
  await newProject(page, `AI chat ${Date.now()}`);
  await expect.poll(async () => Object.keys((await project(page)).clips).length).toBe(0);
  const before = new Set(Object.keys((await project(page)).tracks));

  // Open from the palette ("Ask AI"), set the key inline.
  await palette(page, "Ask AI");
  const panel = page.getByTestId("ai-chat-panel");
  await expect(panel).toBeVisible();
  await panel.getByLabel("API key").fill(KEY);
  await panel.getByRole("button", { name: "Save key" }).click();

  await panel.getByTestId("ai-input").fill("Create a track called Bass and add a C minor arpeggio");
  await panel.getByTestId("ai-input").press("Enter");
  await expect(panel.getByText("Done: a Bass track with a one-bar C minor arpeggio.")).toBeVisible({ timeout: 30_000 });

  // Tool chips, all succeeded.
  const chips = panel.getByTestId("ai-tool");
  await expect(chips).toHaveCount(3);
  for (const name of ["create_track", "create_midi_clip", "add_notes"]) {
    await expect(panel.locator(`[data-testid="ai-tool"][data-tool="${name}"]`)).toHaveAttribute("data-state", "ok");
  }

  // The arrangement shows the track and its clip; the document has the notes.
  const doc = await project(page);
  const bass = Object.values(doc.tracks).find((t) => !before.has(t.id))!;
  expect(bass).toMatchObject({ name: "Bass", kind: "Midi" });
  await expect(page.getByRole("group", { name: "Bass track" })).toBeVisible();
  const clip = Object.values(doc.clips).find((c) => c.track === bass.id)!;
  await expect(page.locator(`[data-lane="${bass.id}"] [data-clip-id="${clip.id}"]`)).toBeVisible();
  const notes = Object.values(doc.notes).filter((n) => n.clip === clip.id).sort((a, b) => a.start - b.start);
  expect(notes.map((n) => n.pitch)).toEqual(ARP);

  // The model got the registry's tools, the overview in the system prompt, and the
  // direct-browser-access header; the model is the default.
  expect(api.requests).toHaveLength(4);
  expect(api.requests[0]!.model).toBe("claude-sonnet-5-5");
  expect(api.requests[0]!.tools.map((t) => t.name)).toEqual(expect.arrayContaining(["create_track", "create_midi_clip", "add_notes"]));
  expect(api.requests[0]!.system.map((s) => s.text).join("\n")).toMatch(/beats/);
  expect(api.headers[0]!["anthropic-dangerous-direct-browser-access"]).toBe("true");
  expect(leaks).toEqual([]);

  // Each tool call is one ordinary undo step: undo the notes, then the clip.
  await palette(page, "Undo");
  await expect.poll(async () => Object.values((await project(page)).notes).filter((n) => n.clip === clip.id).length).toBe(0);
  await palette(page, "Undo");
  await expect.poll(async () => (await project(page)).clips[clip.id]).toBeUndefined();

  expect(errors).toEqual([]);
});
