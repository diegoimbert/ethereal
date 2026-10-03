// fx-color: Saturator, Bitcrusher and Auto Filter on the web build, against the real wasm
// engine. Inserts all three on an audio track playing a demo loop, checks their declarative
// panels (transfer curve, filter curve, LFO preview, typed controls), turns the saturator's
// drive up (still plays, nothing blows up), then sweeps the auto filter's cutoff all the way
// down: the low-pass removes most of the loop (the track meter drops), and back up it
// plays at full level again.
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

/** The loudest meter peak seen over `ms` (the meter decays between loop hits). */
async function maxPeak(page: Page, track: string, ms = 2_500): Promise<number> {
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

test("saturator, bitcrusher and auto filter on an audio track", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  // Wide enough that the transport's Loop button isn't covered by the toolbar's end side.
  await page.setViewportSize({ width: 1440, height: 900 });
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `FX Color ${Date.now()}`);

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
  for (const [i, type] of (["Saturator", "Bitcrusher", "AutoFilter"] as const).entries()) {
    await addDevice(page, type);
    await expect.poll(async () => (await chainOf()).length).toBe(i + 1);
  }
  const [sat, crush, filter] = (await chainOf()).map((d) => d.id) as [string, string, string];
  const panel = (id: string) => page.locator(`[data-device="${id}"]`);

  // Declarative panels: typed widgets and the controls they own.
  await expect(panel(sat).getByTestId("widget-transfer-curve")).toBeVisible();
  for (const name of ["Drive", "Tone", "Output", "Mix"]) {
    await expect(panel(sat).getByRole("slider", { name, exact: true })).toBeVisible();
  }
  for (const name of ["Bits", "Rate", "Jitter", "Output", "Mix"]) {
    await expect(panel(crush).getByRole("slider", { name, exact: true })).toBeVisible();
  }
  await expect(panel(filter).getByTestId("widget-filter-curve")).toBeVisible();
  await expect(panel(filter).getByTestId("widget-lfo")).toBeVisible();
  await expect(panel(filter).getByRole("slider", { name: "Cutoff", exact: true })).toBeVisible();

  // Plays through, looping (the loop region covers the sample: 0..16 beats).
  const transportBar = page.getByRole("toolbar", { name: "Transport" });
  await transportBar.getByRole("button", { name: "Loop" }).click();
  await expect.poll(async () => (await doc(page)).settings.loop_enabled).toBe(true);
  await playButton(page).click();
  await expect.poll(() => peakOf(page, audio.id), { timeout: 15_000 }).toBeGreaterThan(0.05);

  // Drive up (auto gain keeps the level): still plays, finite.
  const drive = panel(sat).getByRole("slider", { name: "Drive", exact: true });
  await dragKnob(page, drive, -600);
  await expect.poll(async () => (await doc(page)).devices[sat]?.params[1], { timeout: 5_000 }).toBe(36);
  const open = await maxPeak(page, audio.id);
  expect(open).toBeGreaterThan(0.05);
  expect(Number.isFinite(open)).toBe(true);

  // Cutoff all the way down (20 Hz low-pass): most of the loop is gone.
  const cutoff = panel(filter).getByRole("slider", { name: "Cutoff", exact: true });
  await dragKnob(page, cutoff, 600);
  await expect.poll(async () => (await doc(page)).devices[filter]?.params[1], { timeout: 5_000 }).toBe(20);
  await page.waitForTimeout(500);
  expect(await maxPeak(page, audio.id)).toBeLessThan(open * 0.5);

  // Back up: full level again.
  await dragKnob(page, cutoff, -600);
  await expect.poll(async () => (await doc(page)).devices[filter]?.params[1], { timeout: 5_000 }).toBe(20000);
  await expect.poll(() => peakOf(page, audio.id), { timeout: 15_000 }).toBeGreaterThan(open * 0.5);

  await transportBar.getByRole("button", { name: "Stop" }).first().click();
  expect(errors).toEqual([]);
});
