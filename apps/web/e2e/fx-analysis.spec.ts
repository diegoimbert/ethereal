// fx-analysis: Spectrum and Tuner on the web build, against the real wasm engine. An audio
// track plays a demo bass loop into both devices; their panels (shared renderer) watch the
// devices and draw the engine's analysis frames forwarded worklet → Worker → UI. Turning
// Peak Hold adds the held line, turning the Range knob moves the spectrum's dB axis.
//
// Screenshots for the PR: `FX_ANALYSIS_SHOTS=<dir> npx playwright test fx-analysis` also
// saves each panel in the dark and light themes.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, launch, newProject, openDeviceTab, openLibrary, playButton } from "./ui";

const shots = process.env.FX_ANALYSIS_SHOTS;

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

/** Drag a knob vertically by `dy` pixels (negative = up = increase). */
async function dragKnob(page: Page, knob: Locator, dy: number) {
  await knob.scrollIntoViewIfNeeded();
  const box = (await knob.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(x, y + (dy * i) / 10);
  await page.mouse.up();
}

for (const theme of shots ? ["dark", "light"] : ["dark"]) {
  test(`spectrum and tuner show the engine's analysis (${theme})`, async ({ page }) => {
    test.setTimeout(120_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    await launch(page);
    await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
    await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
    await newProject(page, `FX Analysis ${Date.now()}`);

    // Audio track with a looping demo bass.
    const audio = await createTrack(page, "Audio");
    await openLibrary(page);
    await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
    await page
      .getByRole("button", { name: "Bass Loop 120.wav", exact: true })
      .dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
    await expect
      .poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length, { timeout: 20_000 })
      .toBe(1);

    await openDeviceTab(page, audio.name);
    const chainOf = async () =>
      Object.values((await doc(page)).devices)
        .filter((d) => d.track === audio.id)
        .sort((a, b) => (a.order < b.order ? -1 : 1));
    await addDevice(page, "SpectrumAnalyzer");
    await expect.poll(async () => (await chainOf()).length).toBe(1);
    await addDevice(page, "Tuner");
    await expect.poll(async () => (await chainOf()).length).toBe(2);
    const chain = await chainOf();
    const spectrum = chain[0]!;
    const tuner = chain[1]!;
    const panel = (id: string) => page.locator(`section[data-device="${id}"]`);
    const plot = panel(spectrum.id).getByTestId("widget-spectrum");
    const tunerWidget = panel(tuner.id).getByTestId("widget-tuner");

    // Stopped: the engine analyses silence (spectrum at its floor, no pitch).
    await expect(plot.locator("polyline.eth-plot__line")).toHaveCount(1, { timeout: 15_000 });
    await expect(tunerWidget.getByText("No pitch")).toBeVisible();

    // Play: the bass loop shows up.
    await expect
      .poll(async () => ((await plot.locator("polyline.eth-plot__line").getAttribute("points")) ?? "").split(" ").length)
      .toBe(256);
    await playButton(page).click();
    // The bass loop has a pitch the tuner finds (a note name, not "–").
    await expect(tunerWidget.locator(".eth-tuner__note")).toHaveText(/^[A-G]♯?\d$/, { timeout: 15_000 });
    await expect(tunerWidget.locator(".eth-tuner__readout")).toHaveText(/ct · \d+\.\d Hz/);

    // Peak Hold on → the held line (UI-side) over the live spectrum.
    await panel(spectrum.id).getByRole("button", { name: "Peak Hold" }).click();
    await expect.poll(async () => (await doc(page)).devices[spectrum.id]!.params[5]).toBe(1);
    await expect(plot.getByTestId("spectrum-peak")).toHaveCount(1);
    if (shots) {
      await page.mouse.move(0, 0);
      await page.waitForTimeout(1000);
      await panel(spectrum.id).screenshot({ path: `${shots}/spectrum-${theme}.png`, timeout: 15_000 });
      await panel(tuner.id).screenshot({ path: `${shots}/tuner-${theme}.png`, timeout: 15_000 });
    }

    // Range knob (default -90 dB) up → a shallower dB axis.
    const axis = () => plot.locator('[data-axis="db"]').first().textContent();
    const before = await axis();
    await dragKnob(page, panel(spectrum.id).getByRole("slider", { name: "Range" }), -80);
    await expect.poll(async () => (await doc(page)).devices[spectrum.id]!.params[3]!).toBeGreaterThan(-90);
    await expect.poll(axis).not.toBe(before);

    // Stop; nothing errored.
    await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop" }).first().click();
    expect(errors).toEqual([]);
  });
}
