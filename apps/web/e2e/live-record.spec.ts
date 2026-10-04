// Live recording view (`live-record`): the web UI connected to a local `ether-server`
// (real native engine, null audio backend) whose audio input is the `loopback` test input,
// so recording captures real audio without a device.
//
// start ether-server → UI connects ("Remote") → a peer client sets the loopback input →
// UI: audio track, input 1, arm → record → a live "recording" clip appears in the lane and
// grows with the peaks received so far → stop → the committed clip lands exactly where the
// live one was, and the live clip goes away.
//
// The browser build itself has no inputs (its recording is `Unsupported`); the mock
// simulation (MockLiveRecord) is covered by vitest and shows in `just dev-ui`.
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createOnLaunch, launch, openEngineServer } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;

interface Handle {
  state(): { project: Project | null; armedTracks: string[] };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state());
const project = async (page: Page): Promise<Project> => {
  const p = (await state(page)).project;
  if (!p) throw new Error("no project open");
  return p;
};

function buildServer(): string {
  const out = execFileSync("cargo", ["build", "-p", "ether-server", "--bin", "ether-server", "--message-format=json"], {
    cwd: repoRoot,
    env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === "ether-server" && msg.executable) return msg.executable;
  }
  throw new Error("cargo did not report the ether-server binary");
}

let server: ChildProcess | null = null;
let serverUrl = "";
let dataDir = "";

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const bin = buildServer();
  dataDir = mkdtempSync(join(tmpdir(), "ether-live-record-e2e-"));
  const child = spawn(
    bin,
    ["--port", "0", "--token", TOKEN, "--name", "e2e-rec", "--data-dir", dataDir, "--library", join(dataDir, "library")],
    { env: { ...process.env, ETHER_AUDIO: "null", RUST_LOG: "warn" }, stdio: ["ignore", "pipe", "inherit"] },
  );
  server = child;
  serverUrl = await new Promise<string>((resolveUrl, reject) => {
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /listening on (ws:\/\/\S+)/.exec(buf);
      if (m) resolveUrl(m[1]!);
    });
    child.on("exit", (code) => reject(new Error(`ether-server exited (${code})`)));
  });
});

test.afterAll(() => {
  server?.kill();
  if (dataDir) rmSync(dataDir, { recursive: true, force: true });
});

/** A second protocol client (sets the server's audio input). */
async function peerSend(domain: string, command: object): Promise<void> {
  const ws = new WebSocket(serverUrl);
  await new Promise((r, j) => {
    ws.onopen = r;
    ws.onerror = j;
  });
  ws.send(JSON.stringify({ protocol_version: 1, token: TOKEN, client: "e2e peer" }));
  await new Promise<void>((done, fail) => {
    ws.onmessage = () => {
      ws.onmessage = (e) => {
        if (typeof e.data !== "string") return;
        const m = JSON.parse(e.data) as { kind: string; body: { id: number; result: { status: string; error?: unknown } } };
        if (m.kind !== "Reply" || m.body.id !== 1) return;
        if (m.body.result.status === "Ok") done();
        else fail(new Error(`${domain} failed: ${JSON.stringify(m.body.result.error)}`));
      };
      ws.send(JSON.stringify({ id: 1, gesture: null, command: { domain, command } }));
    };
  });
  ws.close();
}

test("live recording clip grows while recording, then becomes the real clip", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => state(page).then((s) => s.project !== null), { timeout: 30_000 }).toBe(true);
  const localId = (await project(page)).id;

  // --- Remote engine with the loopback input (engine output → input, 1024 samples late).
  await openEngineServer(page);
  await page.getByLabel("Server address").fill(serverUrl);
  await page.getByLabel("Token").fill(TOKEN);
  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByTestId("remote-button")).toHaveText(/e2e-rec/);
  // base-131: the server has no project open; the project screen creates one there.
  await createOnLaunch(page, "Remote song");
  await expect.poll(async () => (await project(page)).id).not.toBe(localId);
  await peerSend("Engine", {
    type: "SetAudioConfig",
    config: { backend: null, host: null, output_device: null, input_device: "loopback:1024", sample_rate: null, buffer_size: null },
  });

  // --- An armed audio track recording input 1.
  await page.getByRole("button", { name: /New track/ }).click();
  await page.getByRole("button", { name: "Create audio track" }).click();
  await expect.poll(async () => Object.values((await project(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const track = Object.values((await project(page)).tracks).find((t) => t.kind === "Audio")!;
  await page.getByRole("button", { name: `Arm ${track.name}` }).first().click();
  await expect.poll(() => state(page).then((s) => s.armedTracks)).toContain(track.id);
  const lane = page.locator(`[data-lane="${track.id}"]`);
  const live = lane.getByTestId("live-clip");

  // --- Record: the live clip appears and grows (peaks and width) while recording.
  const record = page.getByRole("button", { name: "Record armed tracks" });
  await expect(record).toBeEnabled({ timeout: 15_000 });
  await record.click();
  await expect(live).toHaveCount(1, { timeout: 15_000 });
  await expect(live).toHaveAttribute("data-live-phase", "recording");
  const peaks = async () => Number(await live.getAttribute("data-live-peaks"));
  const first = await peaks();
  await expect.poll(peaks).toBeGreaterThan(first + 50);
  const width0 = (await live.boundingBox())!.width;
  await expect.poll(async () => (await live.boundingBox())!.width).toBeGreaterThan(width0 + 5);
  // Wide enough to become a clip element (narrower clips are painted, see smallClips.ts).
  await expect.poll(async () => (await live.boundingBox())!.width, { timeout: 30_000 }).toBeGreaterThan(80);

  // --- Stop: the committed clip lands where the live one was; the live clip goes away.
  const liveBox = (await live.boundingBox())!;
  // The frozen live clip (handoff) may go away quickly: note its width when it freezes.
  await live.evaluate((el) => {
    const w = window as unknown as { __liveHandoffWidth?: number };
    new MutationObserver(() => {
      if (el.getAttribute("data-live-phase") === "handoff")
        requestAnimationFrame(() => (w.__liveHandoffWidth ??= el.getBoundingClientRect().width));
    }).observe(el, { attributes: true });
  });
  await expect(record).toHaveAttribute("title", "Stop recording");
  await record.click();
  await expect
    .poll(async () => Object.values((await project(page)).clips).filter((c) => c.track === track.id).length, { timeout: 15_000 })
    .toBe(1);
  const clip = lane.locator("[data-clip-id]");
  await expect(clip).toHaveCount(1);
  const clipBox = (await clip.boundingBox())!;
  expect(Math.abs(clipBox.x - liveBox.x)).toBeLessThan(1.5);
  // ... and ends where the live one ended (it got the last peaks before `Stopped`).
  const handoffWidth = await page.evaluate(() => (window as unknown as { __liveHandoffWidth?: number }).__liveHandoffWidth);
  expect(handoffWidth).toBeDefined();
  expect(Math.abs(clipBox.width - handoffWidth!)).toBeLessThan(2);
  await expect(live).toHaveCount(0, { timeout: 10_000 });
  const committed = (await project(page)).clips[(await clip.getAttribute("data-clip-id"))!]!;
  expect(committed.content.type).toBe("Audio");

  expect(errors).toEqual([]);
});
