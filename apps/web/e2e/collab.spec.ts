// Real-time collaboration (docs/COLLAB.md): two browser contexts, each with its own
// in-browser engine (a "site"), join one session on a local `ether-collab-relay`.
//
// start the relay → A joins (creates the session with its project) → B joins (gets A's
// project) → edits on either side show up on the other → each sees the other in the
// presence bar, and A's selected track is outlined on B → B leaves.
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openRelayJoin } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const SESSION = `e2e-${Math.random().toString(36).slice(2, 8)}`;

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

const trackNames = async (page: Page): Promise<string[]> =>
  Object.values((await project(page))?.tracks ?? {})
    .map((t) => t.name)
    .sort();

/** Add a track through the arrangement's "New track" draft row. */
async function addTrack(page: Page, kind: "MIDI" | "audio"): Promise<void> {
  await page.getByRole("button", { name: /New track/ }).first().click();
  await page.getByRole("button", { name: `Create ${kind} track` }).click();
}

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

async function open(page: Page): Promise<string> {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  return (await project(page))!.id;
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

test("two browsers edit one project through a relay", async ({ browser }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  for (const p of [a, b]) p.on("pageerror", (e) => errors.push(e.message));

  const idA = await open(a);
  const idB = await open(b);
  expect(idB).not.toBe(idA);

  // --- A creates the session with its project; B joins and gets it.
  await addTrack(a, "MIDI");
  await join(a, "Ada");
  await join(b, "Bob");
  await expect.poll(async () => (await project(b))?.id, { timeout: 20_000 }).toBe(idA);
  await expect.poll(() => trackNames(b)).toEqual(await trackNames(a));

  // --- Edits flow both ways.
  const before = (await trackNames(a)).length;
  await addTrack(a, "MIDI");
  await expect.poll(async () => (await trackNames(b)).length, { timeout: 10_000 }).toBe(before + 1);
  await addTrack(b, "audio");
  await expect.poll(async () => (await trackNames(a)).length, { timeout: 10_000 }).toBe(before + 2);
  await expect.poll(() => trackNames(a)).toEqual(await trackNames(b));

  // --- Presence: each sees the other, and A's selected track is outlined on B.
  await expect(a.locator('[data-testid="collab-peers"] [data-peer="Bob"]')).toBeVisible({ timeout: 10_000 });
  await expect(b.locator('[data-testid="collab-peers"] [data-peer="Ada"]')).toBeVisible({ timeout: 10_000 });
  const row = a.locator(".eth-arr-row[data-track]:not(.eth-arr-row--draft)").first();
  const selected = await row.getAttribute("data-track");
  expect(selected).not.toBeNull();
  await row.locator(".eth-arr-header").first().click({ position: { x: 4, y: 4 } });
  await expect
    .poll(
      () =>
        b.evaluate(() => document.querySelector('[data-testid="collab-highlights"]')?.textContent ?? ""),
      { timeout: 10_000 },
    )
    .toContain(`data-track="${selected}"`);

  // --- B leaves: A's presence bar empties; B keeps a local copy of the project.
  await b.getByTestId("collab-button").click();
  await b.getByRole("button", { name: "Leave session" }).click();
  await expect(b.getByTestId("collab-button")).toHaveCount(0);
  await expect(a.getByTestId("collab-peers")).toHaveCount(0, { timeout: 10_000 });
  expect((await project(b))?.id).toBe(idA);

  await ctxA.close();
  await ctxB.close();
  expect(errors.filter((e) => /panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
