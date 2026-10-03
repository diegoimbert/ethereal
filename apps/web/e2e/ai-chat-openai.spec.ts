// AI chat on a non-Anthropic provider (the OpenAI-compatible adapter), web build, real engine.
// api.openai.com/v1/chat/completions is stubbed with Playwright routing: the same scripted,
// tool-using conversation as ai-chat.spec.ts ("create a track called Bass and add a C minor
// arpeggio"), streamed as Chat Completions chunks with tool-call deltas split across chunks.
// Asserts the track, clip and notes appear, that the request carries the same system prompt
// and tools, and that only api.openai.com sees the key.
//
// No sleeps: every step waits on UI or engine state (mirror via `window.__ether`).
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test, type Page, type Route } from "@playwright/test";
import type { Project } from "@/generated";
import { newProject } from "./ui";

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

const KEY = "sk-proj-e2e-test-key-00000000000000000000";
const ARP = [48, 51, 55, 60, 63, 60, 55, 51]; // C minor, up and down
const ORIGIN = "https://api.openai.com/";

type Block = { text: string } | { tool: string; id: string; input: unknown };

/** One streamed Chat Completions response, as SSE (tool arguments split in two chunks). */
function sse(blocks: Block[], n: number): string {
  const chunks: object[] = [];
  const chunk = (delta: object, finish: string | null = null) =>
    chunks.push({ id: `chatcmpl_${n}`, object: "chat.completion.chunk", model: "gpt-5", choices: [{ index: 0, delta, finish_reason: finish }] });
  chunk({ role: "assistant", content: "" });
  let call = 0;
  for (const b of blocks) {
    if ("text" in b) chunk({ content: b.text });
    else {
      const index = call++;
      const args = JSON.stringify(b.input);
      chunk({ tool_calls: [{ index, id: b.id, type: "function", function: { name: b.tool, arguments: args.slice(0, 5) } }] });
      chunk({ tool_calls: [{ index, function: { arguments: args.slice(5) } }] });
    }
  }
  chunk({}, call > 0 ? "tool_calls" : "stop");
  return `${chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("")}data: [DONE]\n\n`;
}

interface Request {
  model: string;
  stream: boolean;
  tools: Array<{ type: string; function: { name: string; parameters: { type: string } } }>;
  messages: Array<{ role: string; content: string | null; tool_call_id?: string }>;
}

/** The JSON result of the last tool call (the request's last message). */
function lastResult(req: Request): Record<string, unknown> {
  const m = req.messages.at(-1)!;
  if (m.role !== "tool") throw new Error("no tool result");
  if (m.content!.startsWith("Error: ")) throw new Error(`tool failed: ${m.content}`);
  return JSON.parse(m.content!) as Record<string, unknown>;
}

/** Stubs api.openai.com with the scripted conversation; returns the requests it saw. */
async function stubOpenAi(page: Page): Promise<{ requests: Request[]; headers: Array<Record<string, string>> }> {
  const requests: Request[] = [];
  const headers: Array<Record<string, string>> = [];
  await page.route(`${ORIGIN}**`, async (route: Route) => {
    const req = route.request();
    if (req.method() === "OPTIONS") {
      return route.fulfill({
        status: 204,
        headers: { "access-control-allow-origin": "*", "access-control-allow-headers": "*", "access-control-allow-methods": "GET, POST" },
      });
    }
    if (!req.url().endsWith("/v1/chat/completions")) return route.fulfill({ status: 404, headers: { "access-control-allow-origin": "*" }, body: "{}" });
    const body = req.postDataJSON() as Request;
    const n = requests.length;
    requests.push(body);
    headers.push(req.headers());
    let blocks: Block[];
    switch (n) {
      case 0:
        blocks = [{ text: "I'll create the Bass track." }, { tool: "create_track", id: "call_1", input: { kind: "midi", name: "Bass" } }];
        break;
      case 1:
        blocks = [{ tool: "create_midi_clip", id: "call_2", input: { track_id: lastResult(body).track_id, start_beats: 0, length_beats: 4 } }];
        break;
      case 2:
        blocks = [
          {
            tool: "add_notes",
            id: "call_3",
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

const registry = fileURLToPath(new URL("../../../crates/ether-controller/src/agent", import.meta.url));
test.skip(!existsSync(registry), "the controller's agent tool registry (node agent-api) is not merged yet");

test("ai-chat (OpenAI-compatible): the scripted conversation edits the arrangement", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // Nothing but api.openai.com may see the key.
  const leaks: string[] = [];
  page.on("request", (r) => {
    if (!r.url().startsWith(ORIGIN) && JSON.stringify([r.headers(), r.postData()]).includes(KEY)) leaks.push(r.url());
  });
  const api = await stubOpenAi(page);

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await newProject(page, `AI chat OpenAI ${Date.now()}`);
  await expect.poll(async () => Object.keys((await project(page)).clips).length).toBe(0);
  const before = new Set(Object.keys((await project(page)).tracks));

  // Open from the palette, pick OpenAI in the first-run setup, enter its key.
  await palette(page, "Ask AI");
  const panel = page.getByTestId("ai-chat-panel");
  await expect(panel).toBeVisible();
  await panel.getByRole("combobox", { name: "Provider" }).click();
  await page.getByRole("option", { name: "OpenAI", exact: true }).click();
  await expect(panel.getByText(/only ever sent to api\.openai\.com/)).toBeVisible();
  await panel.getByLabel("API key").fill(KEY);
  await panel.getByRole("button", { name: "Save key" }).click();
  await expect(panel.getByTestId("ai-model")).toHaveText(/gpt-5/);

  await panel.getByTestId("ai-input").fill("Create a track called Bass and add a C minor arpeggio");
  await panel.getByTestId("ai-input").press("Enter");
  await expect(panel.getByText("Done: a Bass track with a one-bar C minor arpeggio.")).toBeVisible({ timeout: 30_000 });

  const chips = panel.getByTestId("ai-tool");
  await expect(chips).toHaveCount(3);
  for (const name of ["create_track", "create_midi_clip", "add_notes"]) {
    await expect(panel.locator(`[data-testid="ai-tool"][data-tool="${name}"]`)).toHaveAttribute("data-state", "ok");
  }

  const doc = await project(page);
  const bass = Object.values(doc.tracks).find((t) => !before.has(t.id))!;
  expect(bass).toMatchObject({ name: "Bass", kind: "Midi" });
  await expect(page.getByRole("group", { name: "Bass track" })).toBeVisible();
  const clip = Object.values(doc.clips).find((c) => c.track === bass.id)!;
  await expect(page.locator(`[data-lane="${bass.id}"] [data-clip-id="${clip.id}"]`)).toBeVisible();
  const notes = Object.values(doc.notes).filter((n) => n.clip === clip.id).sort((a, b) => a.start - b.start);
  expect(notes.map((n) => n.pitch)).toEqual(ARP);

  // Same system prompt and registry tools as the Anthropic path; bearer auth; default model.
  expect(api.requests).toHaveLength(4);
  const first = api.requests[0]!;
  expect(first).toMatchObject({ model: "gpt-5", stream: true });
  expect(first.messages[0]!.role).toBe("system");
  expect(first.messages[0]!.content).toMatch(/beats/);
  expect(first.tools.map((t) => t.function.name)).toEqual(expect.arrayContaining(["create_track", "create_midi_clip", "add_notes"]));
  expect(first.tools.every((t) => t.type === "function" && t.function.parameters.type === "object")).toBe(true);
  expect(api.headers[0]!.authorization).toBe(`Bearer ${KEY}`);
  expect(api.requests[3]!.messages.filter((m) => m.role === "tool").map((m) => m.tool_call_id)).toEqual(["call_1", "call_2", "call_3"]);
  expect(leaks).toEqual([]);

  // Each tool call is one ordinary undo step.
  await palette(page, "Undo");
  await expect.poll(async () => Object.values((await project(page)).notes).filter((n) => n.clip === clip.id).length).toBe(0);

  expect(errors).toEqual([]);
});

// Screenshots for PR review / the owner's UX pass (skipped unless `AI_CHAT_SHOTS=<dir>`):
//   AI_CHAT_SHOTS=/tmp/shots npx playwright test ai-chat-openai
const shots = process.env.AI_CHAT_SHOTS;
for (const theme of ["dark", "light"] as const) {
  test(`ai-chat shots: provider picker + OpenAI conversation (${theme})`, async ({ page }) => {
    test.skip(!shots, "set AI_CHAT_SHOTS=<dir> to capture screenshots");
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    await stubOpenAi(page);
    await page.goto("/");
    await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
    await newProject(page, `AI chat shots ${Date.now()}`);
    await palette(page, "Ask AI");
    const panel = page.getByTestId("ai-chat-panel");
    await panel.getByRole("combobox", { name: "Provider" }).click();
    await expect(page.getByRole("option", { name: "OpenAI", exact: true })).toBeVisible();
    await page.screenshot({ path: `${shots}/ai-provider-picker-${theme}.png` });
    await page.getByRole("option", { name: "OpenAI", exact: true }).click();
    await panel.getByLabel("API key").fill(KEY);
    await panel.getByRole("button", { name: "Save key" }).click();
    await panel.getByRole("button", { name: "AI settings" }).click();
    await expect(panel.getByTestId("ai-settings")).toBeVisible();
    await page.screenshot({ path: `${shots}/ai-provider-settings-${theme}.png` });
    await panel.getByRole("button", { name: "Done" }).click();
    await panel.getByTestId("ai-input").fill("Create a track called Bass and add a C minor arpeggio");
    await panel.getByTestId("ai-input").press("Enter");
    await expect(panel.getByText("Done: a Bass track with a one-bar C minor arpeggio.")).toBeVisible({ timeout: 30_000 });
    await page.screenshot({ path: `${shots}/ai-openai-conversation-${theme}.png` });
  });
}
