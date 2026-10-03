// Recording from a track input tap (`tap-recording`, resampling).
//
// Native: the web UI connected to a local `ether-server` (real native engine + controller,
// null audio backend). A synth track plays a MIDI clip; an audio track takes its signal
// ("From <synth>" in the inspector), is armed from its header, and loop recording makes one
// take lane per pass of what the synth played (no audio input device involved).
//
// Browser build: nothing can be captured there, so arming a tapped track is disabled with
// the reason in its tooltip (hardware-input tracks stay armable).
//
// `TAP_RECORDING_SHOTS=<dir>` also saves PR screenshots (armed tapped track, recorded takes;
// dark and light). No sleeps except the recording itself.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Project, TakeLane } from "@/generated";
import { createTrack, newProject, openEngineServer, pickOption, playButton, selectTrack } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const shots = process.env.TAP_RECORDING_SHOTS;
const ulid = (kind: number, n: number) => `01J${kind}${String(n).padStart(22, "0")}`;

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

const shot = async (page: Page, name: string) => {
  if (!shots) return;
  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.screenshot({ path: `${shots}/${name}-${theme}.png` });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "dark"));
};

test("browser build: a tapped track can't be armed, with the reason shown", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => state(page).then((s) => s.project !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Tap arm ${Date.now()}`);
  const source = await createTrack(page, "Audio");
  const rec = await createTrack(page, "Audio");

  await selectTrack(page, rec.name);
  await pickOption(page, `${rec.name} input`, `From ${source.name}`);
  await expect.poll(async () => (await project(page)).tracks[rec.id]?.input.type).toBe("Track");

  const arm = page.getByRole("button", { name: `Arm ${rec.name}` }).first();
  await expect(arm).toHaveAttribute("aria-disabled", "true");
  await expect(arm).toHaveAttribute("title", /needs the desktop app/);
  await arm.click({ force: true }); // aria-disabled: Playwright would wait for "enabled"
  expect((await state(page)).armedTracks).not.toContain(rec.id);

  // A track on a hardware input stays armable (recording itself is disabled in the browser).
  const hw = page.getByRole("button", { name: `Arm ${source.name}` }).first();
  await expect(hw).not.toHaveAttribute("aria-disabled", "true");
  await hw.click();
  await expect.poll(() => state(page).then((s) => s.armedTracks)).toContain(source.id);

  expect(errors).toEqual([]);
});

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

test.describe("native engine", () => {
  let server: ChildProcess | null = null;
  let serverUrl = "";
  let dataDir = "";

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    const bin = buildServer();
    dataDir = mkdtempSync(join(tmpdir(), "ether-tap-e2e-"));
    const child = spawn(
      bin,
      ["--port", "0", "--token", TOKEN, "--name", "e2e-tap", "--data-dir", dataDir, "--library", join(dataDir, "library")],
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

  /** A second protocol client (sets up what the test isn't about). */
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

  test("arm a track taking a synth's signal; loop recording makes one take per pass", async ({ page }) => {
    test.setTimeout(180_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    await page.goto("/");
    await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
    await expect.poll(() => state(page).then((s) => s.project !== null), { timeout: 30_000 }).toBe(true);
    const localId = (await project(page)).id;

    // --- Remote engine; loop the first bar, no count-in.
    await openEngineServer(page);
    await page.getByLabel("Server address").fill(serverUrl);
    await page.getByLabel("Token").fill(TOKEN);
    await page.getByRole("button", { name: "Connect" }).click();
    await expect(page.getByTestId("remote-button")).toHaveText(/e2e-tap/);
    await expect.poll(async () => (await project(page)).id).not.toBe(localId);
    await peerSend("Transport", { type: "SetLoopRegion", region: { start: 0, end: 4 } });
    await peerSend("Transport", { type: "SetLoopEnabled", enabled: true });
    await peerSend("Recording", { type: "SetCountIn", bars: 0 });

    // --- A synth playing a one-bar clip (the source).
    const synth = ulid(1, 1);
    const clip = ulid(3, 1);
    await peerSend("Track", { type: "Create", id: synth, kind: "Midi", name: "Synth", color: null, parent: null, before: null });
    await peerSend("Device", {
      type: "Insert",
      id: ulid(2, 1),
      track: synth,
      device: { type: "Builtin", device: { type: "Synth" } },
      before: null,
    });
    await peerSend("Clip", { type: "CreateMidi", id: clip, track: synth, start: 0, length: 4, name: null });
    await peerSend("Note", {
      type: "Add",
      clip,
      notes: [0, 1, 2, 3].map((k) => ({ id: ulid(4, k), pitch: 60 + k * 4, velocity: 0.8, start: k, duration: 0.75 })),
    });

    // --- An audio track taking the synth's signal, armed from its header.
    const rec = await createTrack(page, "Audio");
    await selectTrack(page, rec.name);
    await pickOption(page, `${rec.name} input`, "From Synth");
    await expect.poll(async () => (await project(page)).tracks[rec.id]?.input).toEqual({ type: "Track", track: synth, tap: "PostFader" });
    const arm = page.getByRole("button", { name: `Arm ${rec.name}` }).first();
    await expect(arm).not.toHaveAttribute("aria-disabled", "true");
    await arm.click();
    await expect.poll(() => state(page).then((s) => s.armedTracks)).toContain(rec.id);
    await expect(arm).toHaveAttribute("aria-pressed", "true");
    // No input device needed: arming a tapped track doesn't ask for one.
    await expect(page.getByRole("dialog", { name: /Audio settings/i })).toHaveCount(0);
    await shot(page, "tap-armed");

    // --- Record ~2.5 passes of the 1-bar loop (2 s each at 120 bpm), then stop.
    const record = page.getByRole("button", { name: "Record armed tracks" });
    await expect(record).toBeEnabled({ timeout: 15_000 });
    await record.click();
    await expect(record).toHaveAttribute("title", "Stop recording");
    await page.waitForTimeout(5_000);
    await record.click();
    await page.getByTitle("Stop (press again to return to start)").click();
    await expect.poll(async () => lanesOf(await project(page), rec.id).length, { timeout: 15_000 }).toBeGreaterThanOrEqual(2);
    const p = await project(page);
    const lanes = lanesOf(p, rec.id);
    // One take clip per pass on the tapping track (stereo), the source untouched.
    for (const l of lanes) {
      const takes = Object.values(p.clips).filter((c) => c.lane === l.id);
      expect(takes).toHaveLength(1);
      const content = takes[0]!.content;
      expect(content.type).toBe("Audio");
      if (content.type === "Audio") expect(p.media[content.media]?.channels).toBe(2);
    }
    expect(Object.values(p.clips).filter((c) => c.track === synth)).toHaveLength(1);
    expect(Object.values(p.take_lanes).filter((l) => l.track === synth)).toHaveLength(0);

    await page.getByRole("button", { name: `Show takes of ${rec.name}` }).click();
    await expect(page.locator(`[data-lane-take="${lanes[0]!.id}"]`)).toBeVisible();
    await shot(page, "tap-takes");

    expect(errors).toEqual([]);
  });
});
