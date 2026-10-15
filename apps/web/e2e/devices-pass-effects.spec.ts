// devices-pass-effects: every built-in audio and MIDI effect opens in its window with its
// visualisation and no clipped / missing control, and each new param, changed in the window,
// reaches the project mirror (the real wasm engine's document).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { ci, addDevice, createTrack, launch, openDeviceTab, openDeviceWindow, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

async function dragUp(page: Page, knob: Locator) {
  await knob.scrollIntoViewIfNeeded();
  const box = (await knob.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 20; i++) await page.mouse.move(x, y - (600 * i) / 20);
  await page.mouse.up();
}

/** How a case changes its new param in the window. */
type Action = { toggle: string } | { knob: string } | { segment: string; option: string } | { select: string; option: string };

interface Case {
  type: string;
  midi?: boolean;
  /** Test ids of the window's visualisations. */
  shows: string[];
  /** A new param, how to change it in the window and the plain value expected after. */
  change?: { param: number; action: Action; expect: (v: number) => boolean };
}

const CASES: Case[] = [
  { type: "Compressor", shows: ["widget-dynamics-curve", "widget-meter"], change: { param: 7, action: { knob: "Knee" }, expect: (v) => v === 24 } },
  { type: "Gate", shows: ["widget-dynamics-curve", "widget-meter"] },
  { type: "Limiter", shows: ["widget-dynamics-curve", "widget-meter"] },
  { type: "Delay", shows: ["widget-delay-taps"], change: { param: 10, action: { toggle: "Freeze" }, expect: (v) => v === 1 } },
  { type: "Reverb", shows: ["widget-reverb-decay"], change: { param: 9, action: { toggle: "Freeze" }, expect: (v) => v === 1 } },
  { type: "Utility", shows: ["widget-meter"], change: { param: 6, action: { select: "Channel", option: "Swap" }, expect: (v) => v === 3 } },
  { type: "Eq", shows: ["widget-eq-curve"], change: { param: 41, action: { select: "Channel", option: "Mid" }, expect: (v) => v === 3 } },
  { type: "Chorus", shows: ["widget-lfo"], change: { param: 10, action: { toggle: "Sync" }, expect: (v) => v === 1 } },
  { type: "Phaser", shows: ["widget-lfo"], change: { param: 10, action: { select: "LFO Shape", option: "Triangle" }, expect: (v) => v === 1 } },
  { type: "Flanger", shows: ["widget-lfo"], change: { param: 10, action: { select: "LFO Shape", option: "Square" }, expect: (v) => v === 2 } },
  { type: "Tremolo", shows: ["widget-lfo"] },
  { type: "Saturator", shows: ["widget-transfer-curve"] },
  { type: "AutoFilter", shows: ["widget-filter-curve", "widget-lfo"] },
  { type: "Bitcrusher", shows: [] },
  { type: "MultibandCompressor", shows: ["widget-crossover", "widget-meter"] },
  { type: "TransientShaper", shows: ["widget-meter"] },
  { type: "SpectrumAnalyzer", shows: ["widget-spectrum"] },
  { type: "Tuner", shows: [] },
  { type: "Arpeggiator", midi: true, shows: [], change: { param: 9, action: { knob: "Distance" }, expect: (v) => v === 24 } },
  { type: "Velocity", midi: true, shows: ["widget-velocity-curve"], change: { param: 1, action: { knob: "Drive" }, expect: (v) => v === 100 } },
  { type: "Chord", midi: true, shows: [] },
  { type: "ScaleQuantize", midi: true, shows: [] },
  { type: "NoteLength", midi: true, shows: [] },
  { type: "Randomizer", midi: true, shows: [] },
  { type: "Pitch", midi: true, shows: [], change: { param: 0, action: { knob: "Pitch" }, expect: (v) => v === 128 } },
];

async function act(page: Page, win: Locator, a: Action) {
  if ("toggle" in a) await win.getByRole("button", { name: ci(a.toggle) }).click();
  else if ("knob" in a) await dragUp(page, win.getByRole("slider", { name: ci(a.knob) }));
  else if ("segment" in a)
    await win
      .getByRole("group", { name: ci(a.segment) })
      .getByRole("button", { name: ci(a.option) })
      .click();
  else {
    await win.getByRole("combobox", { name: ci(a.select) }).click();
    await page.getByRole("option", { name: ci(a.option) }).click();
  }
}

for (const group of [false, true]) {
  test(`${group ? "MIDI" : "audio"} effects: windows, visualisations and new params`, async ({ page }) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.setViewportSize({ width: 1440, height: 900 });
    await launch(page);
    await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
    await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
    const track = await createTrack(page, group ? "Midi" : "Audio");
    await openDeviceTab(page, track.name);
    for (const c of CASES.filter((c) => !!c.midi === group)) {
      const before = new Set(Object.keys((await doc(page)).devices));
      await addDevice(page, c.type);
      let id = "";
      await expect
        .poll(async () => {
          const p = await doc(page);
          id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
          return id;
        })
        .not.toBe("");
      const win = await openDeviceWindow(page, id);
      for (const t of c.shows) await expect(win.getByTestId(t).first(), `${c.type}: ${t}`).toBeVisible();
      await expect(win.locator(".eth-widget--missing"), c.type).toHaveCount(0);
      // Nothing overflows the window horizontally at its default size.
      const overflow = await win.evaluate((el) => {
        const body = el.querySelector("[data-device-editor]") as HTMLElement | null;
        return body ? body.scrollWidth - body.clientWidth : 0;
      });
      expect(overflow, `${c.type} overflows its window by ${overflow}px`).toBeLessThanOrEqual(1);
      if (c.change) {
        const { param, action, expect: ok } = c.change;
        await act(page, win, action);
        await expect
          .poll(
            async () => {
              const v = (await doc(page)).devices[id]?.params[param];
              return typeof v === "number" && ok(v) ? "ok" : `value ${v}`;
            },
            { message: `${c.type} param ${param}` },
          )
          .toBe("ok");
      }
      await win
        .locator(":scope > header")
        .getByRole("button", { name: /^close .* window$/ })
        .click();
    }
    expect(errors).toEqual([]);
  });
}
