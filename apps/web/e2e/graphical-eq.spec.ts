// graphical-eq: the interactive EQ curve on the real web engine (CONTRACTS.md §12.15).
// A handle drag edits frequency + gain as one undo step; the handle's context menu sets the
// band type. With `GRAPHICAL_EQ_SHOTS=<dir>` it also captures the PR screenshots (dark and
// light, mid-drag) with a synthetic pre/post spectrum fed through the controller's message
// path (web analysis forwarding is `fx-analysis`'s).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Device, EqShape, Project, ServerMessage } from "@/generated";
import { magnitudeDb } from "../../../ui/src/features/devices/layout/eq/eqResponse";
import { addDevice, createTrack, launch, openDeviceTab, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

const shots = process.env.GRAPHICAL_EQ_SHOTS;
// Crisp PR screenshots.
test.use({ deviceScaleFactor: 2 });

// `ether_devices::eq`: band b uses ids 5b..5b+4 (on, type, freq, gain, q).
const pid = (band: number, offset: number) => band * 5 + offset;
const [ON, TYPE, FREQ, GAIN] = [0, 1, 2, 3];

async function openEq(page: Page, theme = "dark"): Promise<{ card: Locator; plot: Locator; id: string }> {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
  const track = await createTrack(page, "Audio");
  await openDeviceTab(page, track.name);
  const before = new Set(Object.keys((await project(page))!.devices));
  await addDevice(page, "Eq");
  let id = "";
  await expect
    .poll(async () => {
      const p = (await project(page))!;
      id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
      return id !== "";
    })
    .toBe(true);
  const card = page.locator(`section[data-device="${id}"]`);
  await expect(card.getByText("Loading…")).toHaveCount(0);
  const plot = card.getByTestId("widget-eq-curve");
  await expect(plot.locator('[data-band="7"]')).toBeVisible();
  await card.scrollIntoViewIfNeeded();
  return { card, plot, id };
}

const device = async (page: Page, id: string): Promise<Device> => (await project(page))!.devices[id]!;
const param = async (page: Page, id: string, p: number, fallback: number) => (await device(page, id)).params[p] ?? fallback;

async function center(plot: Locator, band: number): Promise<{ x: number; y: number }> {
  const b = (await plot.locator(`[data-band="${band}"] circle`).boundingBox())!;
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
}

async function dragBand(page: Page, plot: Locator, band: number, dx: number, dy: number, release = true) {
  const c = await center(plot, band);
  await page.mouse.move(c.x, c.y);
  await page.mouse.down();
  await page.mouse.move(c.x + dx / 2, c.y + dy / 2, { steps: 4 });
  await page.mouse.move(c.x + dx, c.y + dy, { steps: 4 });
  if (release) await page.mouse.up();
}

test("dragging an EQ band edits frequency and gain in one undo step; the menu sets its type", async ({ page }) => {
  test.setTimeout(60_000);
  const { plot, id } = await openEq(page);
  await dragBand(page, plot, 3, 60, -40);
  await expect.poll(() => param(page, id, pid(3, FREQ), 1000)).toBeGreaterThan(1200);
  expect(await param(page, id, pid(3, GAIN), 0)).toBeGreaterThan(2);
  await expect(page.getByTestId("eq-readout")).toContainText("Band 4");

  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => param(page, id, pid(3, FREQ), 1000)).toBe(1000);
  expect(await param(page, id, pid(3, GAIN), 0)).toBe(0);

  const c = await center(plot, 3);
  await page.mouse.click(c.x, c.y, { button: "right" });
  await page.getByRole("menuitem", { name: "High Shelf" }).click();
  await expect.poll(() => param(page, id, pid(3, TYPE), 2)).toBe(4);

  // Double-click toggles the band off.
  await page.mouse.dblclick(c.x, c.y);
  await expect.poll(() => param(page, id, pid(3, ON), 1)).toBe(0);
});

/** A pink-ish mix spectrum (dBFS) at `hz`, with a little movement over `t`. */
function mixDb(hz: number, t: number): number {
  const tilt = -22 - 4.5 * Math.log2(hz / 100);
  const lowRoll = hz < 40 ? -18 * Math.log2(40 / hz) : 0;
  const body = 6 * Math.exp(-((Math.log2(hz / 120) / 0.8) ** 2)) + 4 * Math.exp(-((Math.log2(hz / 2500) / 1.1) ** 2));
  const wobble = 2.5 * Math.sin(hz * 0.013 + t * 3) * Math.cos(hz * 0.0021 - t);
  return Math.max(-110, tilt + lowRoll + body + wobble);
}

async function feedSpectrum(page: Page, deviceId: string, bands: ReadonlyArray<{ shape: EqShape; freq: number; gain: number; q: number; on: boolean }>) {
  const n = 192;
  const frames: string[] = [];
  for (let k = 0; k < 6; k++) {
    const pre: number[] = [];
    const post: number[] = [];
    for (let i = 0; i < n; i++) {
      const hz = 20 * Math.pow(1000, (i + 0.5) / n);
      const d = mixDb(hz, k * 0.3);
      pre.push(d);
      post.push(d + bands.reduce((s, b) => s + (b.on ? magnitudeDb(b.shape, b.freq, b.gain, b.q, hz, 48000) : 0), 0));
    }
    const msg = (stage: "Pre" | "Post", bins: number[]): ServerMessage => ({
      kind: "Event",
      body: { type: "Analysis", event: { type: "Frame", device: deviceId, data: { type: "Spectrum", min_hz: 20, max_hz: 20000, bins_db: bins, stage } } },
    });
    frames.push(JSON.stringify([msg("Pre", pre), msg("Post", post)]));
  }
  await page.evaluate((batches) => {
    const engine = (window as unknown as { __etherEngine: { handles(): { controller: Worker } | null } }).__etherEngine;
    const worker = engine.handles()!.controller;
    let i = 0;
    const tick = () => worker.dispatchEvent(new MessageEvent("message", { data: { type: "server", json: batches[i++ % batches.length] } }));
    tick();
    setInterval(tick, 33);
  }, frames);
}

for (const theme of ["dark", "light"]) {
  test(`EQ panel screenshots (${theme})`, async ({ page }) => {
    test.skip(!shots, "set GRAPHICAL_EQ_SHOTS=<dir> to capture the PR screenshots");
    test.setTimeout(60_000);
    const { card, plot, id } = await openEq(page, theme);
    // A typical mix move: low shelf lift, mud cut, presence boost, air roll-off on.
    await dragBand(page, plot, 1, 0, -12);
    await dragBand(page, plot, 2, 20, 22);
    await dragBand(page, plot, 4, 0, -26);
    const hc = await center(plot, 7);
    await page.mouse.dblclick(hc.x, hc.y);
    await expect.poll(() => param(page, id, pid(7, ON), 0)).toBe(1);
    const p = (await device(page, id)).params;
    const shapes: EqShape[] = ["LowCut", "LowShelf", "Bell", "Notch", "HighShelf", "HighCut"];
    const bands = [0, 1, 2, 3, 4, 5, 6, 7].map((b) => ({
      shape: shapes[p[pid(b, TYPE)] ?? [0, 1, 2, 2, 2, 2, 4, 5][b]!]!,
      freq: p[pid(b, FREQ)] ?? [30, 100, 250, 1000, 2500, 6000, 10000, 18000][b]!,
      gain: p[pid(b, GAIN)] ?? 0,
      q: p[pid(b, 4)] ?? Math.SQRT1_2,
      on: (p[pid(b, ON)] ?? (b === 0 || b === 7 ? 0 : 1)) >= 0.5,
    }));
    await feedSpectrum(page, id, bands);
    await expect(plot.getByTestId("eq-spectrum-post")).toBeVisible();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(300);
    await card.screenshot({ path: `${shots}/eq-${theme}.png`, timeout: 15_000 });
    await plot.screenshot({ path: `${shots}/eq-${theme}-curve.png`, timeout: 15_000 });
    if (theme === "dark") {
      await dragBand(page, plot, 4, 40, -8, false);
      await page.waitForTimeout(200);
      await plot.screenshot({ path: `${shots}/eq-${theme}-drag.png`, timeout: 15_000 });
      await page.mouse.up();
    }
  });
}
