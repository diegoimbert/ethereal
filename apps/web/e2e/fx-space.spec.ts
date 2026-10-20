// fx-space: the Convolution Reverb against the real wasm engine. A kick clip on an audio
// track, the reverb inserted after it: pick a factory IR in the IR widget (the live node
// swaps IRs in place), turn the mix up, and hear the tail on the track meter long after the
// 0.45 s kick has ended; with the IR removed, the same moment is silent (dry pass-through).
// Then drag the Decay marker on the IR plot (one undo step).
// `FX_SPACE_SHOTS=<dir>` also captures the panel in the dark (1440×900) and light themes.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, newProject, openDeviceTab, openLibrary, pickOption, playButton, launch, expandFolder, openDeviceWindow } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
  playhead(): { transport: { seconds: number; playing: boolean } } | null;
}

const shots = process.env.FX_SPACE_SHOTS;

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

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
  await launch(page);
}

/** An audio track with the demo kick at the start, and a Convolution Reverb on it. */
async function setup(page: Page) {
  await newProject(page, `FX Space ${Date.now()}`);
  const audio = await createTrack(page, "Audio");
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  await page.getByRole("treeitem", { name: "Kick.wav", exact: true }).dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 12, y: 20 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length, { timeout: 20_000 }).toBe(1);
  await openDeviceTab(page, audio.name);
  await addDevice(page, "ConvolutionReverb");
  let device = "";
  await expect
    .poll(async () => {
      device = Object.values((await doc(page)).devices).find((d) => d.track === audio.id)?.id ?? "";
      return device !== "";
    })
    .toBe(true);
  // A factory device's UI is in its window.
  const panel = await openDeviceWindow(page, device);
  await expect(panel.getByTestId("ir-widget")).toBeVisible();
  return { audio, device, panel };
}

const playheadOf = (page: Page) =>
  page.evaluate(() => {
    const t = (window as unknown as { __ether: Handle }).__ether.playhead()?.transport;
    return t ? { seconds: t.seconds, playing: t.playing } : null;
  });

/**
 * Play from the start; the loudest meter reading between 1.0 s and 1.8 s of transport time
 * (the 0.45 s kick is over). Paced by the engine's playhead, not the wall clock: under load the
 * page reads meters late, and a meter frame holds the peak since the previous one, so a stale
 * or early frame would carry the dry kick. Only readings taken after two newer playhead frames
 * past 1.0 s count (they hold audio rendered after it).
 */
async function tailPeak(page: Page, track: string): Promise<number> {
  const stop = page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "stop" }).first();
  await stop.click();
  await stop.click();
  await expect.poll(() => playheadOf(page)).toEqual({ seconds: 0, playing: false });
  await playButton(page).click();
  const peak = await page.evaluate(async (t) => {
    const h = (window as unknown as { __ether: Handle }).__ether;
    let last = -1;
    let past = 0;
    let peak = 0;
    const deadline = performance.now() + 30_000;
    while (performance.now() < deadline) {
      const p = h.playhead()?.transport;
      if (p?.playing && p.seconds !== last) {
        last = p.seconds;
        if (last >= 1.0) past++;
      }
      if (past >= 3) {
        const m = h.meter(t);
        if (m) peak = Math.max(peak, m.peak[0], m.peak[1]);
        if (last >= 1.8) return peak;
      }
      await new Promise((r) => setTimeout(r, 10));
    }
    throw new Error(`the playhead never reached 1.8 s (at ${last} s)`);
  }, track);
  await stop.click();
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
  // A new convolution reverb starts on the factory hall.
  await expect.poll(() => irOf(page, device)).toEqual({ type: "Factory", id: "hall" });
  await expect(widget.getByTestId("ir-meta")).toHaveText("2.8 s · stereo");

  // Pick the Cathedral (5 s) from the IR picker; the plot draws it.
  await pickOption(widget, "impulse response", "Cathedral");
  await expect.poll(() => irOf(page, device)).toEqual({ type: "Factory", id: "cathedral" });
  await expect(widget.getByTestId("ir-meta")).toHaveText("5.0 s · stereo");
  await expect(widget.getByTestId("ir-envelope")).toHaveCount(1);

  // Mix all the way up.
  await dragKnob(page, panel.getByRole("slider", { name: "mix", exact: true }), -600);
  await expect.poll(async () => (await doc(page)).devices[device]?.params[0], { timeout: 5_000 }).toBe(100);

  // The reverb tail is still sounding long after the kick.
  expect(await tailPeak(page, audio.id)).toBeGreaterThan(0.002);

  // No IR: the dry signal passes through, and the same moment is silent.
  await widget.getByRole("button", { name: "remove impulse response" }).click();
  await expect.poll(() => irOf(page, device)).toBeNull();
  // The node takes the removal at its next block and fades the Cathedral's tail out over the
  // swap crossfade (`SWAP_MS`, 200 ms; ether-devices `removing_the_ir_silences_the_tail_*`):
  // the very first play after it is silent at that moment.
  expect(await tailPeak(page, audio.id)).toBeLessThan(0.001);

  // Back to an IR with the arrows, then shorten it with the Decay marker (one undo step).
  await widget.getByRole("button", { name: "next impulse response" }).click();
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

  // A file of the user's as the IR (prepared off the audio thread like the factory
  // ones): drop the 2 s chords loop on the widget, and its tail sounds after the kick.
  await page.getByRole("treeitem", { name: "Chords Loop 120.wav", exact: true }).dragTo(widget.getByTestId("widget-ir"));
  await expect.poll(async () => (await irOf(page, device))?.type, { timeout: 20_000 }).toBe("Media");
  await expect(widget.getByTestId("ir-meta")).toHaveText(/^\d+\.\d s · (mono|stereo)$/, { timeout: 20_000 });
  await expect.poll(() => tailPeak(page, audio.id), { timeout: 60_000 }).toBeGreaterThan(0.002);

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

for (const theme of ["dark", "light"]) {
  test(`convolution reverb panel screenshot (${theme})`, async ({ page }) => {
    test.skip(!shots, "set FX_SPACE_SHOTS=<dir> to capture the panel");
    test.setTimeout(120_000);
    await open(page, theme);
    const { device, panel } = await setup(page);
    const widget = panel.getByTestId("ir-widget");
    await pickOption(widget, "impulse response", "Concert Hall");
    await expect.poll(() => irOf(page, device)).toEqual({ type: "Factory", id: "hall" });
    // A shaped IR: some pre-delay and a shortened tail.
    await dragKnob(page, panel.getByRole("slider", { name: "pre-delay", exact: true }), -60);
    await dragKnob(page, panel.getByRole("slider", { name: "decay", exact: true }), 80);
    await panel.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/fx-space-app-${theme}.png` });
    await panel.screenshot({ path: `${shots}/fx-space-panel-${theme}.png`, timeout: 15_000 });
  });
}
