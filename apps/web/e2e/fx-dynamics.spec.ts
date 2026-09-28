// fx-dynamics: Gate, Multiband Compressor and Transient Shaper on the web build, against the
// real wasm engine. Inserts all three on an audio track playing a demo loop, checks their
// declarative panels (crossover display, gain-reduction meters, typed controls), then turns
// the gate's threshold all the way up: the loop is gated to silence (the track meter stays
// at the floor), and back down it plays again.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, newProject, openDeviceTab, openLibrary, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

/** Drag a knob vertically by `dy` pixels (negative = up = increase). */
async function dragKnob(page: Page, knob: Locator, dy: number) {
  await knob.scrollIntoViewIfNeeded();
  const box = (await knob.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 20; i++) await page.mouse.move(x, y + (dy * i) / 20);
  await page.mouse.up();
}

test("gate, multiband compressor and transient shaper on an audio track", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `FX Dynamics ${Date.now()}`);

  // Audio track with a looping demo sample.
  const audio = await createTrack(page, "Audio");
  await openLibrary(page);
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  await page
    .getByRole("button", { name: "Bass Loop 120.wav", exact: true })
    .dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect
    .poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length, {
      timeout: 20_000,
    })
    .toBe(1);

  await openDeviceTab(page, audio.name);
  const chainOf = async () =>
    Object.values((await doc(page)).devices)
      .filter((d) => d.track === audio.id)
      .sort((a, b) => (a.order < b.order ? -1 : 1));
  for (const [i, type] of (["TransientShaper", "MultibandCompressor", "Gate"] as const).entries()) {
    await addDevice(page, type);
    await expect.poll(async () => (await chainOf()).length).toBe(i + 1);
  }
  const [shaper, mbc, gate] = (await chainOf()).map((d) => d.id) as [string, string, string];
  const panel = (id: string) => page.locator(`[data-device="${id}"]`);

  // Declarative panels: crossover display, one gain-reduction meter per band, gate meter.
  await expect(panel(mbc).getByTestId("widget-crossover")).toBeVisible();
  await expect(panel(mbc).getByTestId("widget-meter")).toHaveCount(3);
  await expect(panel(gate).getByTestId("widget-meter")).toHaveCount(1);
  await expect(panel(gate).getByRole("slider", { name: "Threshold" })).toBeVisible();
  for (const name of ["Attack", "Sustain", "Attack Time", "Release Time", "Mix"]) {
    await expect(panel(shaper).getByRole("slider", { name, exact: true })).toBeVisible();
  }

  // Plays through (gate open at -40 dB threshold).
  await playButton(page).click();
  await expect.poll(() => peakOf(page, audio.id), { timeout: 15_000 }).toBeGreaterThan(0.05);

  // Threshold all the way up (0 dB): the loop never reaches it, the gate closes (-80 dB
  // floor = silence).
  const threshold = panel(gate).getByRole("slider", { name: "Threshold" });
  await dragKnob(page, threshold, -600);
  await expect.poll(async () => (await doc(page)).devices[gate]?.params[0], { timeout: 5_000 }).toBe(0);
  await expect.poll(() => peakOf(page, audio.id), { timeout: 15_000 }).toBeLessThan(0.001);

  // Back down: it plays again.
  await dragKnob(page, threshold, 600);
  await expect.poll(async () => (await doc(page)).devices[gate]?.params[0], { timeout: 5_000 }).toBe(-80);
  await expect.poll(() => peakOf(page, audio.id), { timeout: 15_000 }).toBeGreaterThan(0.05);

  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop" }).first().click();
  expect(errors).toEqual([]);
});
