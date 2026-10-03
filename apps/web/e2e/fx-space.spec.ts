// fx-space: the Convolution Reverb against the real wasm engine. A kick clip on an audio
// track, the reverb inserted after it: pick a factory IR in the IR widget (the live node
// swaps IRs in place), turn the mix up, and hear the tail on the track meter long after the
// 0.45 s kick has ended; with the IR removed, the same moment is silent (dry pass-through).
// Then drag the Decay marker on the IR plot (one undo step).
// `FX_SPACE_SHOTS=<dir>` also captures the panel in the dark (1440×900) and light themes.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, newProject, openDeviceTab, openLibrary, pickOption, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

const shots = process.env.FX_SPACE_SHOTS;

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

const irOf = async (page: Page, device: string) => {
  const k = (await doc(page)).devices[device]?.kind;
  return k?.type === "Builtin" && k.device.type === "ConvolutionReverb" ? k.device.ir : undefined;
};

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

async function open(page: Page, theme: string, size = { width: 1440, height: 900 }) {
  await page.setViewportSize(size);
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

/** An audio track with the demo kick at the start, and a Convolution Reverb on it. */
async function setup(page: Page) {
  await newProject(page, `FX Space ${Date.now()}`);
  const audio = await createTrack(page, "Audio");
  await openLibrary(page);
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  await page
    .getByRole("button", { name: "Kick.wav", exact: true })
    .dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 12, y: 20 } });
  await expect
    .poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length, { timeout: 20_000 })
    .toBe(1);
  await openDeviceTab(page, audio.name);
  await addDevice(page, "ConvolutionReverb");
  let device = "";
  await expect
    .poll(async () => {
      device = Object.values((await doc(page)).devices).find((d) => d.track === audio.id)?.id ?? "";
      return device !== "";
    })
    .toBe(true);
  const panel = page.locator(`[data-device="${device}"]`);
  await expect(panel.getByTestId("ir-widget")).toBeVisible();
  return { audio, device, panel };
}

/** Play from the start; the loudest meter reading between 1.0 s and 1.8 s after. */
async function tailPeak(page: Page, track: string): Promise<number> {
  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop" }).first().click();
  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop" }).first().click();
  await playButton(page).click();
  await page.waitForTimeout(1000);
  let peak = 0;
  for (let i = 0; i < 16; i++) {
    peak = Math.max(peak, await peakOf(page, track));
    await page.waitForTimeout(50);
  }
  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop" }).first().click();
  return peak;
}

test("convolution reverb: pick an IR, hear the tail, shape it", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await open(page, "dark");
  const { audio, device, panel } = await setup(page);
  const widget = panel.getByTestId("ir-widget");
  await expect(widget.getByRole("img", { name: "No impulse response" })).toBeVisible();

  // Pick the Cathedral (5 s) from the IR picker; the plot draws it.
  await pickOption(widget, "Impulse response", "Cathedral");
  await expect.poll(() => irOf(page, device)).toEqual({ type: "Factory", id: "cathedral" });
  await expect(widget.getByTestId("ir-meta")).toHaveText("5.0 s · Stereo");
  await expect(widget.getByTestId("ir-envelope")).toHaveCount(1);

  // Mix all the way up.
  await dragKnob(page, panel.getByRole("slider", { name: "Mix", exact: true }), -600);
  await expect.poll(async () => (await doc(page)).devices[device]?.params[0], { timeout: 5_000 }).toBe(100);

  // The reverb tail is still sounding long after the kick.
  expect(await tailPeak(page, audio.id)).toBeGreaterThan(0.002);

  // No IR: the dry signal passes through, and the same moment is silent.
  await widget.getByRole("button", { name: "Remove impulse response" }).click();
  await expect.poll(() => irOf(page, device)).toBeNull();
  expect(await tailPeak(page, audio.id)).toBeLessThan(0.001);

  // Back to an IR with the arrows, then shorten it with the Decay marker (one undo step).
  await widget.getByRole("button", { name: "Next impulse response" }).click();
  await expect.poll(() => irOf(page, device)).toEqual({ type: "Factory", id: "room" });
  const plot = widget.getByTestId("widget-ir");
  const box = (await plot.boundingBox())!;
  // Room: 0.6 s on a 0.6·1.5 + 0.25 s axis; the Decay marker sits at the IR's end.
  const xEnd = box.x + (0.6 / (0.6 * 1.5 + 0.25)) * box.width;
  await page.mouse.move(xEnd, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(xEnd - box.width * 0.2, box.y + box.height / 2, { steps: 8 });
  await page.mouse.up();
  await expect.poll(async () => (await doc(page)).devices[device]?.params[2] ?? 100).toBeLessThan(80);
  await expect(widget.getByTestId("ir-meta")).toContainText("of 0.60 s");
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await doc(page)).devices[device]?.params[2] ?? 100).toBe(100);

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

for (const theme of ["dark", "light"]) {
  test(`convolution reverb panel screenshot (${theme})`, async ({ page }) => {
    test.skip(!shots, "set FX_SPACE_SHOTS=<dir> to capture the panel");
    test.setTimeout(120_000);
    await open(page, theme);
    const { device, panel } = await setup(page);
    const widget = panel.getByTestId("ir-widget");
    await pickOption(widget, "Impulse response", "Concert Hall");
    await expect.poll(() => irOf(page, device)).toEqual({ type: "Factory", id: "hall" });
    // A shaped IR: some pre-delay and a shortened tail.
    await dragKnob(page, panel.getByRole("slider", { name: "Pre-delay", exact: true }), -60);
    await dragKnob(page, panel.getByRole("slider", { name: "Decay", exact: true }), 80);
    await panel.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/fx-space-app-${theme}.png` });
    await panel.screenshot({ path: `${shots}/fx-space-panel-${theme}.png`, timeout: 15_000 });
  });
}
