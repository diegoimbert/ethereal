// Take lanes + swipe comping (`comping`) end to end: the web UI connected to a local
// `ether-server` (real native engine and controller, null audio backend) whose audio input
// is the `loopback` test input, so loop recording captures real takes without a device.
//
// loop 1 bar → record ~2.5 passes → one take lane per pass, the newest selected → show the
// take lanes → swipe a range on an older take (one SetComp) → Up/Down pick another take →
// audition → flatten to main-lane clips (one undo step each).
//
// `COMPING_SHOTS=<dir>` also saves PR screenshots (expanded take lanes with a swiped comp,
// dark and light).
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { CompRegion, Project, TakeLane } from "@/generated";
import { openEngineServer } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const shots = process.env.COMPING_SHOTS;

test.use({ viewport: { width: 1440, height: 900 } });

interface Handle {
  state(): { project: Project | null; armedTracks: string[] };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state());
const project = async (page: Page): Promise<Project> => {
  const p = (await state(page)).project;
  if (!p) throw new Error("no project open");
  return p;
};
const lanesOf = (p: Project, track: string): TakeLane[] =>
  Object.values(p.take_lanes)
    .filter((l) => l.track === track)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
const compOf = (p: Project, track: string): CompRegion[] =>
  Object.values(p.comp_regions)
    .filter((r) => r.track === track)
    .sort((a, b) => a.start - b.start);

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
  dataDir = mkdtempSync(join(tmpdir(), "ether-comping-e2e-"));
  const child = spawn(
    bin,
    ["--port", "0", "--token", TOKEN, "--name", "e2e-takes", "--data-dir", dataDir, "--library", join(dataDir, "library")],
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

/** A second protocol client (engine settings the UI has no control for in this setup). */
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

const shot = async (page: Page, name: string) => {
  if (shots) await page.screenshot({ path: `${shots}/${name}.png` });
};

test("loop recording makes take lanes; swipe, next/previous take, audition, flatten", async ({ page }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => state(page).then((s) => s.project !== null), { timeout: 30_000 }).toBe(true);
  const localId = (await project(page)).id;

  // --- Remote engine with the loopback input; loop the first bar, no count-in.
  await openEngineServer(page);
  await page.getByLabel("Server address").fill(serverUrl);
  await page.getByLabel("Token").fill(TOKEN);
  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByTestId("remote-button")).toHaveText(/e2e-takes/);
  await expect.poll(async () => (await project(page)).id).not.toBe(localId);
  await peerSend("Engine", {
    type: "SetAudioConfig",
    config: { backend: null, host: null, output_device: null, input_device: "loopback:1024", sample_rate: null, buffer_size: null },
  });
  await peerSend("Transport", { type: "SetLoopRegion", region: { start: 0, end: 4 } });
  await peerSend("Transport", { type: "SetLoopEnabled", enabled: true });
  await peerSend("Recording", { type: "SetCountIn", bars: 0 });

  // --- An armed audio track recording input 1.
  await page.getByRole("button", { name: /New track/ }).click();
  await page.getByRole("button", { name: "Create audio track" }).click();
  await expect.poll(async () => Object.values((await project(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const track = Object.values((await project(page)).tracks).find((t) => t.kind === "Audio")!;
  await page.getByRole("button", { name: `Arm ${track.name}` }).first().click();
  await expect.poll(() => state(page).then((s) => s.armedTracks)).toContain(track.id);

  // --- Record ~2.5 passes of the 1-bar loop (2 s each at 120 bpm), then stop.
  const record = page.getByRole("button", { name: "Record armed tracks" });
  await expect(record).toBeEnabled({ timeout: 15_000 });
  await record.click();
  await expect(record).toHaveAttribute("title", "Stop recording");
  await page.waitForTimeout(5_000);
  await record.click();
  await page.getByTitle("Stop (press again to return to start)").click();
  await expect.poll(async () => lanesOf(await project(page), track.id).length, { timeout: 15_000 }).toBeGreaterThanOrEqual(2);
  let p = await project(page);
  const lanes = lanesOf(p, track.id);
  // Each pass is a take clip on its own lane; the newest take plays from the loop start.
  for (const l of lanes) expect(Object.values(p.clips).filter((c) => c.lane === l.id)).toHaveLength(1);
  expect(Object.values(p.clips).filter((c) => c.track === track.id && c.lane == null)).toHaveLength(0);
  expect(compOf(p, track.id)[0]).toMatchObject({ lane: lanes.at(-1)!.id, start: 0 });
  // The comp shows on the main lane.
  await expect(page.locator(`[data-comp="${track.id}"] [data-comp-region]`).first()).toBeVisible();

  // --- Show the take lanes; swipe beats 1..3 on the first take.
  await page.getByRole("button", { name: `Show takes of ${track.name}` }).click();
  const first = page.locator(`[data-lane-take="${lanes[0]!.id}"]`);
  await expect(first).toBeVisible();
  const box = (await first.boundingBox())!;
  const pxPerBeat = await page.evaluate(() => {
    // The lane layer's --ppb is the live zoom (px per beat).
    const layer = document.querySelector<HTMLElement>(".eth-takes__lane .eth-arr-lane__layer")!;
    return parseFloat(layer.style.getPropertyValue("--ppb"));
  });
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + 1 * pxPerBeat + 2, y);
  await page.mouse.down();
  await page.mouse.move(box.x + 2 * pxPerBeat, y, { steps: 4 });
  await page.mouse.move(box.x + 3 * pxPerBeat + 2, y, { steps: 4 });
  await expect(page.getByTestId("swipe-preview")).toBeVisible();
  await page.mouse.up();
  await expect
    .poll(async () => compOf(await project(page), track.id).map((r) => [r.lane, r.start, r.end]))
    .toContainEqual([lanes[0]!.id, 1, 3]);
  await shot(page, "comping-dark");
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
  await shot(page, "comping-light");
  await page.evaluate(() => (document.documentElement.dataset.theme = "dark"));

  // --- Down: the next take for the selected region (one SetComp).
  await page.keyboard.press("ArrowDown");
  await expect
    .poll(async () => compOf(await project(page), track.id).map((r) => [r.lane, r.start, r.end]))
    .toContainEqual([lanes[1]!.id, 1, 3]);
  await page.keyboard.press("ArrowUp");
  await expect
    .poll(async () => compOf(await project(page), track.id).map((r) => [r.lane, r.start, r.end]))
    .toContainEqual([lanes[0]!.id, 1, 3]);

  // --- Audition a take (runtime: no undo step, no document change).
  const before = await project(page);
  await page.getByRole("button", { name: `Audition ${lanes[1]!.name}` }).click();
  await expect(page.getByRole("button", { name: `Stop auditioning ${lanes[1]!.name}` })).toHaveAttribute("aria-pressed", "true");
  expect(compOf(await project(page), track.id)).toEqual(compOf(before, track.id));
  await page.getByRole("button", { name: `Stop auditioning ${lanes[1]!.name}` }).click();

  // --- Flatten: the comp becomes main-lane clips; undo brings the comp back.
  const pieces = compOf(await project(page), track.id).length;
  await page.getByRole("group", { name: `${lanes[0]!.name} take` }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Flatten Comp", exact: true }).click();
  await expect.poll(async () => compOf(await project(page), track.id).length).toBe(0);
  p = await project(page);
  expect(Object.values(p.clips).filter((c) => c.track === track.id && c.lane == null).length).toBeGreaterThanOrEqual(pieces);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect.poll(async () => compOf(await project(page), track.id).length).toBe(pieces);

  expect(errors).toEqual([]);
});
