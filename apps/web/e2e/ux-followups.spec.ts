// ux-followups on the web build (real wasm engine + controller):
// - a VCA's inspector has no Devices section (a VCA carries no audio, the engine rejects
//   inserts);
// - a large knob's value fits its ring (the Compressor's "-18.0 dB" threshold);
// - through a local `ether-collab-relay`: peer avatars use readable ink, and a peer's
//   playhead shows in the piano roll of a clip it plays over.
//
// `UX_SHOTS=<dir>` also writes the PR screenshots (`UX_SHOTS_THEME=light` for light).
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { openClip } from "./clips";
import { addDevice, createTrack, openDeviceTab, playButton, selectTrack } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const shots = process.env.UX_SHOTS;
const theme = process.env.UX_SHOTS_THEME === "light" ? "light" : "dark";

test.use({ viewport: { width: 1440, height: 900 } });

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function open(page: Page): Promise<void> {
  if (shots) await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function shoot(page: Page, name: string) {
  if (!shots) return;
  await page.mouse.move(0, 0);
  await page.screenshot({ path: `${shots}/${name}-${theme}.png`, animations: "disabled" });
}

test("a VCA's inspector has no Devices section", async ({ page }) => {
  test.setTimeout(90_000);
  await open(page);
  const midi = await createTrack(page, "Midi");
  await page.getByRole("group", { name: `${midi.name} track` }).click({ button: "right", position: { x: 6, y: 10 } });
  await page.getByRole("menuitem", { name: "New VCA for Track" }).click();
  let vcaName = "";
  await expect
    .poll(async () => {
      vcaName = Object.values((await project(page))!.tracks).find((t) => t.kind === "Vca")?.name ?? "";
      return vcaName;
    })
    .not.toBe("");
  const inspector = page.getByTestId("inspector");

  // A regular track shows its devices…
  await selectTrack(page, midi.name);
  await expect(inspector.getByRole("heading", { name: "Devices", exact: true })).toBeVisible();
  // …a VCA doesn't (its mixer stays).
  await selectTrack(page, vcaName);
  await expect(inspector.getByRole("textbox", { name: "Track name" })).toHaveValue(vcaName);
  await expect(inspector.getByRole("heading", { name: "Mixer", exact: true })).toBeVisible();
  await expect(inspector.getByRole("heading", { name: "Devices", exact: true })).toHaveCount(0);
  await expect(page.getByRole("combobox", { name: "Add device" })).toHaveCount(0);
  await shoot(page, "ux-vca-inspector");
});

test("a large knob's value fits inside its ring", async ({ page }) => {
  test.setTimeout(90_000);
  await open(page);
  const audio = await createTrack(page, "Audio");
  await openDeviceTab(page, audio.name);
  await addDevice(page, "Compressor");
  const threshold = page.getByRole("slider", { name: "Threshold" }).first();
  await expect(threshold).toHaveAttribute("aria-valuetext", /dB$/);
  const value = threshold.locator(".eth-knob__center-value");
  const unit = threshold.locator(".eth-knob__center-unit");
  await expect(unit).toHaveText("dB");
  // Both stay inside the dial (after the fit scale).
  const dial = (await threshold.locator(".eth-knob__dial").boundingBox())!;
  for (const part of [value, unit]) {
    const b = (await part.boundingBox())!;
    expect(b.x).toBeGreaterThanOrEqual(dial.x - 0.5);
    expect(b.x + b.width).toBeLessThanOrEqual(dial.x + dial.width + 0.5);
  }
  if (shots) {
    const card = page.locator("section[data-device]").filter({ has: threshold });
    await card.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    await card.screenshot({ path: `${shots}/ux-knob-values-${theme}.png` });
  }
});

// ---- Collab: avatars and peer playheads in the piano roll ----------------------------------

const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const SESSION = `e2e-ux-${Math.random().toString(36).slice(2, 8)}`;

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

test.describe("collab", () => {
  let relay: ChildProcess | null = null;
  let relayUrl = "";

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    const child = spawn(buildRelay(), ["--port", "0", "--token", TOKEN], {
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

  async function join(page: Page, name: string) {
    await page.getByTestId("collab-button").click();
    await page.getByLabel("Relay address").fill(relayUrl);
    await page.getByLabel("Session").fill(SESSION);
    await page.getByLabel("Your name").fill(name);
    await page.getByLabel("Token").fill(TOKEN);
    await page.getByRole("button", { name: "Join" }).click();
    await expect(page.getByTestId("collab-button")).toHaveText(`● ${SESSION}`, { timeout: 20_000 });
    await page.keyboard.press("Escape");
  }

  test("peer avatars read in both themes; a peer's playhead shows in the piano roll", async ({ browser }) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    const a = await (await browser.newContext()).newPage();
    const b = await (await browser.newContext()).newPage();
    for (const p of [a, b]) p.on("pageerror", (e) => errors.push(e.message));
    await open(a);
    await open(b);

    // A: a MIDI track with a 4-bar-ish clip at the start; both join (B gets A's project).
    const track = await createTrack(a, "Midi");
    await a.locator(`[data-lane="${track.id}"]`).dblclick({ position: { x: 10, y: 20 } });
    await expect.poll(async () => Object.keys((await project(a))?.clips ?? {}).length).toBe(1);
    const clip: Clip = Object.values((await project(a))!.clips)[0]!;
    await join(a, "Ada Lovelace");
    await join(b, "Bob");
    await expect.poll(async () => Object.keys((await project(b))?.clips ?? {}).length, { timeout: 20_000 }).toBe(1);

    // B's avatar for Ada: initials in the ink chosen for the peer colour (never the theme bg).
    const avatar = b.getByTestId("collab-peers").locator(".eth-collab__avatar").first();
    await expect(avatar).toHaveText("AL", { timeout: 10_000 });
    const ink = await avatar.evaluate((el) => getComputedStyle(el).getPropertyValue("--eth-collab-peer-ink").trim());
    expect(ink).toMatch(/^#[0-9a-f]{6}$/i);
    const color = await avatar.evaluate((el) => getComputedStyle(el).color);
    const bg = await b.evaluate(() => getComputedStyle(document.body).backgroundColor);
    expect(color).not.toBe(bg);

    // B opens the clip; A plays from the start: B draws Ada's playhead over the notes grid.
    await openClip(b, clip.id);
    await expect(b.getByTestId("piano-roll-grid")).toBeVisible();
    await playButton(a).click();
    const head = b.locator('[data-testid="editor-peer-playhead"][data-peer="Ada Lovelace"]');
    await expect(head).toHaveAttribute("data-playing", "true", { timeout: 10_000 });
    await expect(head).toHaveText("AL");
    const first = Number(await head.getAttribute("data-beats"));
    await expect.poll(async () => Number(await head.getAttribute("data-beats")), { timeout: 10_000 }).toBeGreaterThan(first + 0.25);
    // A stops and locates into the middle of the clip (a press on a clip's body locates):
    // B's line holds still there, dimmed, on the clip's content axis.
    await a.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop", exact: true }).first().click();
    await expect(head).toHaveAttribute("data-playing", "false", { timeout: 10_000 });
    const cb = (await a.locator(`[data-clip-id="${clip.id}"]`).boundingBox())!;
    await a.mouse.click(cb.x + cb.width / 2, cb.y + cb.height - 4);
    const mid = clip.start + clip.length / 2;
    await expect.poll(async () => Math.abs(Number(await head.getAttribute("data-beats")) - mid), { timeout: 10_000 }).toBeLessThan(1);
    await expect(head).toHaveAttribute("data-hidden", "false");
    const gb = (await b.getByTestId("piano-roll-grid").boundingBox())!;
    const hb = (await head.boundingBox())!;
    expect(hb.x).toBeGreaterThan(gb.x);
    expect(hb.x).toBeLessThan(gb.x + gb.width);
    // The initials cap rides the top of the visible grid (the grid scrolls under the keys).
    const body = (await b.locator(".eth-pr__body").boundingBox())!;
    const cap = (await head.locator(".eth-collab-editor-playhead__cap").boundingBox())!;
    expect(Math.abs(cap.y - body.y)).toBeLessThan(2);
    await shoot(b, "ux-piano-roll-peer-playhead");

    // "Hide users and notes" hides it.
    await b.getByTestId("collab-button").click();
    await b.getByRole("switch", { name: "Hide users and notes" }).click();
    await expect(b.getByTestId("editor-peer-playheads")).toHaveCount(0);
    await b.getByRole("switch", { name: "Hide users and notes" }).click();
    await b.keyboard.press("Escape");

    expect(errors).toEqual([]);
  });
});
