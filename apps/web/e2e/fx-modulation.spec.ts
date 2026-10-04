// fx-modulation: Chorus, Phaser, Flanger and Tremolo on the web build, against the real wasm
// engine. Inserts all four on an audio track playing a demo loop, checks their declarative
// panels (LFO display, sync controls, through-zero toggle), then turns the tremolo's Output
// all the way down: the track meter drops by ~24 dB, and back up it plays at full level.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, launch, newProject, openDeviceTab, openLibrary, playButton } from "./ui";

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

/** Highest meter peak seen over `ms` milliseconds (the loop has quiet moments). */
async function maxPeak(page: Page, track: string, ms = 1_500): Promise<number> {
  let max = 0;
  for (let t = 0; t < ms; t += 100) {
    max = Math.max(max, await peakOf(page, track));
    await page.waitForTimeout(100);
  }
  return max;
}

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

test("chorus, phaser, flanger and tremolo on an audio track", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  // Wide enough that the transport toolbar never overlaps the Loop button.
  await page.setViewportSize({ width: 1600, height: 900 });
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `FX Modulation ${Date.now()}`);

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
  for (const [i, type] of (["Chorus", "Phaser", "Flanger", "Tremolo"] as const).entries()) {
    await addDevice(page, type);
    await expect.poll(async () => (await chainOf()).length).toBe(i + 1);
  }
  const [chorus, phaser, flanger, tremolo] = (await chainOf()).map((d) => d.id) as [string, string, string, string];
  const panel = (id: string) => page.locator(`[data-device="${id}"]`);

  // Declarative panels: hero knobs, sync controls, the tremolo's LFO display.
  for (const name of ["Rate", "Depth", "Delay", "Spread", "Feedback", "High Cut", "Mix"]) {
    await expect(panel(chorus).getByRole("slider", { name, exact: true })).toBeVisible();
  }
  for (const name of ["Rate", "Depth", "Center", "Feedback", "Stereo Phase", "Mix"]) {
    await expect(panel(phaser).getByRole("slider", { name, exact: true })).toBeVisible();
  }
  await expect(panel(flanger).getByRole("slider", { name: "Delay", exact: true })).toBeVisible();
  await expect(panel(tremolo).getByTestId("widget-lfo")).toBeVisible();

  // Through Zero (appended flanger param 9) toggles like any param.
  await panel(flanger).getByRole("button", { name: "Through Zero", exact: true }).click();
  await expect.poll(async () => (await doc(page)).devices[flanger]?.params[9], { timeout: 5_000 }).toBe(1);

  // Plays through the whole chain, looping (default loop region: the first 4 bars) so the
  // loop keeps sounding for the whole test.
  const loop = page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Loop", exact: true });
  await loop.click();
  await expect.poll(async () => (await doc(page)).settings.loop_enabled).toBe(true);
  await playButton(page).click();
  await expect.poll(() => peakOf(page, audio.id), { timeout: 15_000 }).toBeGreaterThan(0.05);
  const full = await maxPeak(page, audio.id);

  // Tremolo Output all the way down (-24 dB): the real DSP runs in the wasm engine.
  const output = panel(tremolo).getByRole("slider", { name: "Output", exact: true });
  await dragKnob(page, output, 600);
  await expect.poll(async () => (await doc(page)).devices[tremolo]?.params[7], { timeout: 5_000 }).toBe(-24);
  await expect.poll(() => maxPeak(page, audio.id), { timeout: 15_000 }).toBeLessThan(full / 6);

  // Back to 0 dB: full level again.
  await dragKnob(page, output, -300);
  await expect.poll(async () => (await doc(page)).devices[tremolo]?.params[7] ?? -24, { timeout: 5_000 }).toBeGreaterThan(0);
  await expect.poll(() => maxPeak(page, audio.id), { timeout: 15_000 }).toBeGreaterThan(full / 2);

  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop" }).first().click();
  expect(errors).toEqual([]);
});
