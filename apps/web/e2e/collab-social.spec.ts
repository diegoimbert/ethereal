// collab-social (docs/COLLAB.md §12): two browser contexts, each with its own in-browser
// engine, in one session on a local `ether-collab-relay`.
//
// Chat: A focuses the chat with Mod+Shift+M and sends; B (chat closed) gets a toast, opens
// the chat from it and replies. Notes: A leaves a note on a track lane and one in the piano
// roll; B sees both, resolves the arranger one (A sees it dimmed). Playheads: A plays, B
// draws A's playhead moving. "Hide users and notes": B hides the notes and playheads.
//
// `SOCIAL_SHOTS=<dir>` also writes the PR screenshots (`SOCIAL_SHOTS_THEME=light` for light).
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, playButton } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const SESSION = `e2e-social-${Math.random().toString(36).slice(2, 8)}`;
const shots = process.env.SOCIAL_SHOTS;

test.use({ viewport: { width: 1440, height: 900 } });

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

/** Build `ether-collab-relay` and return the binary path (cargo tells where it is). */
function buildRelay(): string {
  const out = execFileSync("cargo", ["build", "-p", "ether-collab", "--bin", "ether-collab-relay", "--message-format=json"], {
    cwd: repoRoot,
    env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === "ether-collab-relay" && msg.executable) return msg.executable;
  }
  throw new Error("cargo did not report the ether-collab-relay binary");
}

let relay: ChildProcess | null = null;
let relayUrl = "";

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const bin = buildRelay();
  const child = spawn(bin, ["--port", "0", "--token", TOKEN], {
    env: { ...process.env, RUST_LOG: "warn" },
    stdio: ["ignore", "pipe", "inherit"],
  });
  relay = child;
  relayUrl = await new Promise<string>((resolveUrl, reject) => {
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /listening on (ws:\/\/\S+)/.exec(buf);
      if (m) resolveUrl(m[1]!);
    });
    child.on("exit", (code) => reject(new Error(`ether-collab-relay exited (${code})`)));
  });
});

test.afterAll(() => {
  relay?.kill();
});

async function open(page: Page): Promise<void> {
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function join(page: Page, name: string) {
  await page.getByTestId("collab-button").click();
  await page.getByLabel("Relay address").fill(relayUrl);
  await page.getByLabel("Session").fill(SESSION);
  await page.getByLabel("Your name").fill(name);
  await page.getByLabel("Token").fill(TOKEN);
  await page.getByRole("button", { name: "Join" }).click();
  await expect(page.getByTestId("collab-button")).toHaveText(`● ${SESSION}`, { timeout: 20_000 });
}

/** `SOCIAL_SHOTS_THEME=light` captures the light theme (one run per theme). */
const theme = process.env.SOCIAL_SHOTS_THEME === "light" ? "light" : "dark";

async function shoot(page: Page, name: string) {
  if (shots) await page.screenshot({ path: `${shots}/${name}-${theme}.png`, animations: "disabled" });
}

test("chat, pinned notes, peers' playheads and 'Hide users and notes' through a relay", async ({ browser }) => {
  test.setTimeout(240_000);
  const errors: string[] = [];
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  if (shots) {
    for (const ctx of [ctxA, ctxB]) await ctx.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  }
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  for (const p of [a, b]) p.on("pageerror", (e) => errors.push(e.message));
  await open(a);
  await open(b);

  // --- A: a MIDI track with a clip; both join (B gets A's project).
  const track = await createTrack(a, "Midi");
  await a.locator(`[data-lane="${track.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await project(a))?.clips ?? {}).length).toBe(1);
  const clip: Clip = Object.values((await project(a))!.clips)[0]!;
  await expect(a.getByRole("button", { name: "Chat", exact: true })).toHaveCount(0);
  await join(a, "Ada");
  await join(b, "Bob");
  await expect.poll(async () => Object.keys((await project(b))?.clips ?? {}).length, { timeout: 20_000 }).toBe(1);

  // --- Chat: A's shortcut opens the chat with the input focused; Enter sends.
  await a.locator(`[data-lane="${track.id}"]`).click({ position: { x: 400, y: 20 } });
  await a.keyboard.press("ControlOrMeta+Shift+M");
  const inputA = a.getByTestId("chat-input");
  await expect(inputA).toBeFocused();
  await inputA.fill("Hey Bob, the bass is late in bar 3");
  await inputA.press("Enter");
  await expect(a.getByTestId("chat-message")).toHaveCount(1);
  // B's chat is closed: a toast, which opens the chat.
  const toast = b.locator(".eth-toast");
  await expect(toast).toContainText("Ada", { timeout: 10_000 });
  await expect(toast).toContainText("the bass is late");
  await b.locator(".eth-toast__body").first().click();
  await expect(b.getByTestId("chat-message")).toHaveCount(1);
  await expect(b.getByTestId("chat-message").first()).toContainText("Ada");
  await b.getByTestId("chat-input").fill("On it, leaving you a note there");
  await b.getByTestId("chat-input").press("Enter");
  await expect(a.getByTestId("chat-message")).toHaveCount(2, { timeout: 10_000 });
  await expect(a.locator(".eth-toast")).toHaveCount(0);

  // --- Notes: A right-clicks the lane → "Leave a note".
  const lane = a.locator(`[data-lane="${track.id}"]`);
  const lb = (await lane.boundingBox())!;
  await a.mouse.click(lb.x + 600, lb.y + lb.height / 2, { button: "right" });
  await a.getByRole("menuitem", { name: "Leave a note" }).click();
  await a.getByTestId("note-input").fill("Nudge the bass 1/16 earlier here?");
  await a.getByTestId("note-input").press("Enter");
  await expect(a.getByTestId("pinned-note")).toHaveCount(1);
  const noteB = b.getByTestId("arranger-notes").getByTestId("pinned-note");
  await expect(noteB).toHaveCount(1, { timeout: 10_000 });
  await noteB.locator("button").click();
  await expect(b.getByTestId("note-card")).toContainText("Nudge the bass");
  await expect(b.getByTestId("note-card")).toContainText("Ada");
  await shoot(b, "social-note-card");
  await b.getByRole("button", { name: "Resolve" }).click();
  await expect(a.getByTestId("pinned-note")).toHaveAttribute("data-resolved", "true", { timeout: 10_000 });
  await b.keyboard.press("Escape");

  // --- A note in the piano roll (the open clip's content coordinates).
  // The chat pane floats over the lanes: close it first.
  await a.getByRole("button", { name: "Chat", exact: true }).click();
  await openClip(a, clip.id);
  const grid = a.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  const gb = (await a.locator(".eth-pr__body").boundingBox())!;
  await a.mouse.click(gb.x + gb.width / 2, gb.y + gb.height / 2, { button: "right" });
  await a.getByRole("menuitem", { name: "Leave a note" }).click();
  await a.getByTestId("note-input").fill("Ghost note on the 'and' of 2");
  await a.getByTestId("note-input").press("Enter");
  await expect(a.getByTestId("editor-notes").getByTestId("pinned-note")).toHaveCount(1);
  await b.getByRole("button", { name: "Chat", exact: true }).click();
  await openClip(b, clip.id);
  await expect(b.getByTestId("editor-notes").getByTestId("pinned-note")).toHaveCount(1, { timeout: 10_000 });
  const pinned = Object.values((await project(b))!.pinned_notes);
  expect(pinned.map((n) => n.position.editor?.clip ?? null).sort()).toEqual([clip.id, null].sort());

  // --- Playheads: A plays; B draws A's line moving (extrapolated between samples).
  await playButton(a).click();
  const head = b.locator('[data-testid="peer-playhead"][data-peer="Ada"]');
  await expect(head).toHaveAttribute("data-playing", "true", { timeout: 10_000 });
  const first = Number(await head.getAttribute("data-beats"));
  await expect.poll(async () => Number(await head.getAttribute("data-beats")), { timeout: 10_000 }).toBeGreaterThan(first + 0.5);
  await expect(head).toHaveAttribute("data-hidden", "false");
  // A toast on B (its chat is closed), with A's playhead and both notes in the shot.
  await a.keyboard.press("ControlOrMeta+Shift+M");
  await expect(a.getByTestId("chat-input")).toBeFocused();
  await a.getByTestId("chat-input").fill("Playing from the top, listen to the fill");
  await a.getByTestId("chat-input").press("Enter");
  await expect(b.locator(".eth-toast")).toContainText("Playing from the top", { timeout: 10_000 });
  await shoot(b, "social-arranger-toast-playhead");
  await b.getByRole("button", { name: "Chat", exact: true }).click();
  await expect(b.getByTestId("chat-message")).toHaveCount(3);
  await shoot(b, "social-chat");

  // --- "Hide users and notes" (B only): notes and playheads go; the chat stays.
  await b.getByTestId("collab-button").click();
  await b.getByRole("switch", { name: "Hide users and notes" }).click();
  await shoot(b, "social-hide-toggle");
  await b.getByRole("button", { name: "Close", exact: true }).click();
  await expect(b.getByTestId("pinned-note")).toHaveCount(0);
  await expect(b.locator('[data-testid="peer-playhead"]')).toHaveCount(0);
  await expect(b.getByTestId("chat-message")).toHaveCount(3);
  await expect(a.getByTestId("pinned-note")).toHaveCount(2);
  await b.getByTestId("collab-button").click();
  await b.getByRole("switch", { name: "Hide users and notes" }).click();
  await b.getByRole("button", { name: "Close", exact: true }).click();
  await expect(b.getByTestId("pinned-note")).toHaveCount(2);

  await ctxA.close();
  await ctxB.close();
  expect(errors.filter((e) => /panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
