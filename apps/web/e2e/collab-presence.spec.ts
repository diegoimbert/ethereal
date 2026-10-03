// Presence v2 (docs/COLLAB.md §8): two browser contexts, each with its own in-browser engine,
// in one session on a local `ether-collab-relay`.
//
// A moves the mouse over a clip → B draws A's pointer over the same point of that clip in
// B's own layout (B is zoomed differently) and it follows A's moves; A drags the clip → B
// sees "Ada · dragging"; A's mouse leaves → the pointer goes away; B follows A (chip click)
// → B's view matches A's; B scrolls → following stops.
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { createTrack, launch, openRelayJoin } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const SESSION = `e2e-presence-${Math.random().toString(36).slice(2, 8)}`;

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
  await launch(page);
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function join(page: Page, name: string) {
  await openRelayJoin(page);
  await page.getByLabel("Relay address").fill(relayUrl);
  await page.getByLabel("Session").fill(SESSION);
  await page.getByLabel("Your name").fill(name);
  await page.getByLabel("Token").fill(TOKEN);
  await page.getByRole("button", { name: "Join" }).click();
  await expect(page.getByTestId("collab-button")).toHaveText(`● ${SESSION}`, { timeout: 20_000 });
}

async function box(page: Page, selector: string) {
  const b = await page.locator(selector).first().boundingBox();
  expect(b, selector).not.toBeNull();
  return b!;
}

test("a peer's live pointer, activity and follow mode through a relay", async ({ browser }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  for (const p of [a, b]) p.on("pageerror", (e) => errors.push(e.message));
  await open(a);
  await open(b);

  // --- A: a MIDI track with a one-bar clip; both join (B gets A's project).
  const track = await createTrack(a, "Midi");
  await a.locator(`[data-lane="${track.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await project(a))?.clips ?? {}).length).toBe(1);
  const clip: Clip = Object.values((await project(a))!.clips)[0]!;
  await join(a, "Ada");
  await join(b, "Bob");
  await expect.poll(async () => Object.keys((await project(b))?.clips ?? {}).length, { timeout: 20_000 }).toBe(1);
  const clipSel = `[data-clip-id="${clip.id}"]`;
  await expect(b.locator(clipSel)).toBeVisible();

  // B zooms in (its own layout differs from A's).
  const widthBefore = (await box(b, clipSel)).width;
  const lanesB = await box(b, `[data-lane="${track.id}"]`);
  await b.mouse.move(lanesB.x + 5, lanesB.y + 5);
  await b.keyboard.down("Control");
  await b.mouse.wheel(0, -300);
  await b.keyboard.up("Control");
  await expect.poll(async () => (await box(b, clipSel)).width).toBeGreaterThan(widthBefore * 1.2);
  await b.mouse.move(0, 0);

  // --- A hovers a point of the clip; B draws the pointer over the same point of its clip.
  const pointerB = b.locator('[data-testid="peer-pointer"][data-peer="Ada"]');
  for (const f of [0.25, 0.6]) {
    const ca = await box(a, clipSel);
    await a.mouse.move(ca.x + f * ca.width, ca.y + ca.height / 2, { steps: 4 });
    const beats = clip.start + f * clip.length;
    await expect
      .poll(async () => Number(await pointerB.getAttribute("data-beats")), { timeout: 10_000 })
      .toBeCloseTo(beats, 1);
    await expect(pointerB).toHaveAttribute("data-track", track.id);
    await expect(pointerB).toHaveAttribute("data-hidden", "false");
    const cb = await box(b, clipSel);
    const pb = (await pointerB.boundingBox())!;
    expect(Math.abs(pb.x - (cb.x + f * cb.width))).toBeLessThan(3);
    expect(pb.y).toBeGreaterThanOrEqual(cb.y - 3);
    expect(pb.y).toBeLessThanOrEqual(cb.y + cb.height + 3);
  }
  await expect(pointerB).toContainText("Ada");

  // --- A drags the clip: B sees the activity next to the pointer until the drop.
  const ca = await box(a, `${clipSel} .eth-clip__title`);
  await a.mouse.move(ca.x + ca.width / 2, ca.y + ca.height / 2);
  await a.mouse.down();
  await a.mouse.move(ca.x + ca.width / 2 + 40, ca.y + ca.height / 2, { steps: 5 });
  await expect(pointerB).toContainText("Ada · dragging", { timeout: 10_000 });
  await a.mouse.up();
  await expect(pointerB).not.toContainText("dragging", { timeout: 10_000 });

  // --- A's mouse leaves the arrangement: the pointer goes away on B.
  await a.mouse.move(2, 2);
  await expect(pointerB).toHaveCount(0, { timeout: 10_000 });

  // --- B follows A: B's zoom becomes A's (same window size → same clip width); a local
  // scroll stops following.
  await b.getByRole("button", { name: "Follow Ada" }).click();
  await expect(b.getByRole("button", { name: "Stop following Ada" })).toHaveAttribute("aria-pressed", "true");
  const wa = (await box(a, clipSel)).width;
  await expect.poll(async () => (await box(b, clipSel)).width, { timeout: 10_000 }).toBeCloseTo(wa, 0);
  await expect(a.locator('[data-testid="collab-peers"] [data-peer="Bob"]')).toHaveAttribute("title", /following you/, {
    timeout: 10_000,
  });
  const lanes = await box(b, `[data-lane="${track.id}"]`);
  await b.mouse.move(lanes.x + 50, lanes.y + 5);
  await b.mouse.wheel(0, 100);
  await expect(b.getByRole("button", { name: "Follow Ada" })).toHaveAttribute("aria-pressed", "false");

  await ctxA.close();
  await ctxB.close();
  expect(errors.filter((e) => /panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
