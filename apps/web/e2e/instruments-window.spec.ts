// devices-pass-instruments: every built-in instrument in its own window (base-153/154) on the
// real wasm engine. Each opens at its own default width with nothing clipped or scrolling
// sideways, and keeps fitting when the window is made narrow. The Poly Synth's unison
// controls (voices, detune in cents, blend) and drift drive the engine and the mirror.
// `INSTRUMENT_SHOTS=<dir>` also writes every instrument window in the dark and light themes.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import {
  addDevice,
  createTrack,
  expandFolder,
  launch,
  libraryTree,
  openDeviceTab,
  openDeviceWindow,
  openLibrary,
  padButton,
  pickOption,
  playButton,
  setNumberField,
} from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const shots = process.env.INSTRUMENT_SHOTS;
if (shots) test.use({ deviceScaleFactor: 2 });

const project = (page: Page): Promise<Project> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

/** `ether_devices::poly_synth::poly_synth`. */
const POLY = { UNISON_VOICES: 61, UNISON_DETUNE: 62, UNISON_BLEND: 63, DRIFT: 64 } as const;

/** The built-in instruments with a window (`BuiltinDeviceType`); an Instrument Rack has none, its chains and macros are on its card (b8ad9f79). */
const INSTRUMENTS = ["Synth", "PolySynth", "Sampler", "MultiSampler", "DrumRack"] as const;

async function open(page: Page, theme: string) {
  await page.setViewportSize({ width: 1600, height: shots ? 1400 : 1000 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

/** A MIDI track with `type` as its instrument; its window, open. */
async function insert(page: Page, type: string): Promise<{ device: string; win: Locator }> {
  const track = await createTrack(page, "Midi");
  await openDeviceTab(page, track.name);
  const before = new Set(Object.keys((await project(page)).devices));
  await addDevice(page, type);
  let device = "";
  await expect
    .poll(async () => {
      const p = await project(page);
      device = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
      return device !== "";
    })
    .toBe(true);
  const win = await openDeviceWindow(page, device);
  return { device, win };
}

/** Nothing in the window's body is wider than the body (no clipped or sideways-scrolling control). */
async function overflow(win: Locator): Promise<string[]> {
  return win.locator(".eth-fwin__body").evaluate((body) => {
    const box = body.getBoundingClientRect();
    const out: string[] = [];
    if (body.scrollWidth > body.clientWidth + 1) out.push(`body scrolls: ${body.scrollWidth} > ${body.clientWidth}`);
    for (const el of body.querySelectorAll<HTMLElement>(".eth-param, .eth-widget, .eth-device__group")) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && (r.left < box.left - 1 || r.right > box.right + 1)) {
        out.push(
          `${el.className.split(" ")[0]} "${el.textContent?.slice(0, 24)}" ${Math.round(r.left)}..${Math.round(r.right)} outside ${Math.round(box.left)}..${Math.round(box.right)}`,
        );
      }
    }
    return out;
  });
}

/** Drags the window's right edge so it is `width` px wide. */
async function resizeTo(page: Page, win: Locator, width: number) {
  const handle = win.locator(':scope > [data-resize="e"]');
  const h = (await handle.boundingBox())!;
  const box = (await win.boundingBox())!;
  const x = h.x + h.width / 2;
  const y = h.y + h.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x - (box.width - width), y, { steps: 6 });
  await page.mouse.up();
  await expect.poll(async () => Math.round((await win.boundingBox())!.width)).toBeLessThanOrEqual(width + 2);
}

test("every instrument opens in a window that fits, wide and narrow", async ({ page }) => {
  test.setTimeout(240_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const widths: Record<string, number> = {};
  for (const type of INSTRUMENTS) {
    const { win } = await insert(page, type);
    widths[type] = Math.round((await win.boundingBox())!.width);
    expect(await overflow(win), `${type} at its default width`).toEqual([]);
    await resizeTo(page, win, 360);
    await page.waitForTimeout(100);
    expect(await overflow(win), `${type} at 360 px`).toEqual([]);
    await win
      .locator(":scope > header")
      .getByRole("button", { name: /^close .* window$/ })
      .click();
  }
  // The big instruments open wider than the simple ones.
  expect(widths.PolySynth).toBeGreaterThan(widths.Synth!);
  expect(widths.MultiSampler).toBeGreaterThan(widths.Synth!);
  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

test("poly synth: unison voices, detune, blend and drift reach the engine", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const { device, win } = await insert(page, "PolySynth");
  const param = async (id: number) => (await project(page)).devices[device]?.params[id];

  const unison = win.locator('[data-section="unison"]');
  await expect(unison).toBeVisible();
  await expect(unison.locator(".eth-device__group-name")).toHaveText("unison");
  // Voices: a number field, 1 → 7.
  await setNumberField(unison.getByRole("spinbutton", { name: "voices" }), 7);
  await expect.poll(() => param(POLY.UNISON_VOICES)).toBe(7);
  // Detune: a knob in cents.
  const detune = unison.getByRole("slider", { name: "detune" });
  await expect(detune).toHaveAttribute("aria-valuetext", /ct$/);
  await detune.focus();
  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await param(POLY.UNISON_DETUNE)) ?? 20).toBeGreaterThan(20);
  // Blend and drift: knobs, by keyboard.
  const blend = unison.getByRole("slider", { name: "blend" });
  await blend.focus();
  await page.keyboard.press("ArrowDown");
  await expect.poll(async () => (await param(POLY.UNISON_BLEND)) ?? 100).toBeLessThan(100);
  const drift = win.locator('[data-section="voice"]').getByRole("slider", { name: "drift" });
  await drift.focus();
  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await param(POLY.DRIFT)) ?? 0).toBeGreaterThan(0);
  // The legacy unison controls of old documents are not on the panel.
  await expect(win.getByRole("spinbutton", { name: "unison (legacy)" })).toHaveCount(0);
  await expect(win.getByRole("button", { name: /more .* controls/ })).toHaveCount(0);
  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

test("synth and sampler: the new controls reach the engine", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const paramOf = async (device: string, id: number) => (await project(page)).devices[device]?.params[id];

  // Synth (`ether_devices::synth::params`): Fine (9, cents) and Env Amount (12).
  const synth = await insert(page, "Synth");
  const fine = synth.win.locator('[data-section="oscillator"]').getByRole("slider", { name: "fine" });
  await expect(fine).toHaveAttribute("aria-valuetext", /ct$/);
  await fine.focus();
  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await paramOf(synth.device, 9)) ?? 0).toBeGreaterThan(0);
  const env = synth.win.locator('[data-section="filter"]').getByRole("slider", { name: "env amount" });
  await env.focus();
  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await paramOf(synth.device, 12)) ?? 0).toBeGreaterThan(0);

  await synth.win
    .locator(":scope > header")
    .getByRole("button", { name: /^close .* window$/ })
    .click();

  // Sampler (`ether_devices::sampler::params`): Loop (13), Reverse (12), Filter (17).
  const sampler = await insert(page, "Sampler");
  const loop = sampler.win.locator('[data-section="loop"]');
  // A segmented control when wide enough, else a select.
  const forward = loop.getByRole("button", { name: "forward" });
  if (await forward.count()) await forward.click();
  else await pickOption(loop, "loop", "forward");
  await expect.poll(() => paramOf(sampler.device, 13)).toBe(1);
  await sampler.win.locator('[data-section="playback"]').getByRole("button", { name: "reverse" }).click();
  await expect.poll(() => paramOf(sampler.device, 12)).toBe(1);
  await expect(sampler.win.locator('[data-section="envelope"] [data-widget="Envelope"]')).toBeVisible();
  await expect(sampler.win.locator('[data-section="filter"] [data-widget="FilterCurve"]')).toBeVisible();
  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

test("drum rack window: a pad's tune, preview and its Sampler's own panel", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const { device: rack, win } = await insert(page, "DrumRack");
  // Drop a demo sample from the library on pad C1 of the rack's window.
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const grid = win.getByRole("grid", { name: "drum pads" });
  await libraryTree(page)
    .getByRole("treeitem", { name: "Kick.wav", exact: true })
    .dragTo(grid.getByRole("gridcell", { name: /^C1 / }));
  let sampler = "";
  await expect
    .poll(
      async () => {
        const p = await project(page);
        const pad = Object.values(p.drum_pads).find((d) => d.rack === rack);
        sampler = pad ? (Object.values(p.devices).find((d) => d.pad === pad.id)?.id ?? "") : "";
        return sampler !== "";
      },
      { timeout: 20_000 },
    )
    .toBe(true);
  const settings = win.getByTestId("pad-settings");
  // A pad is its devices (2533f415): the sampler is a device card in the pad's chain, and its
  // declared panel (waveform, loop, envelope, filter) is its own window, not a knob list.
  await expect(settings.locator(`section[data-device="${sampler}"]`)).toBeVisible();
  // Tune (the sampler's transpose), at the top of the pad.
  const tune = settings.getByRole("slider", { name: "pad tune" });
  await tune.focus();
  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await project(page)).devices[sampler]?.params[2] ?? 0).toBe(1);
  // Preview, on the pad (shown on hover, with its mute and solo).
  await (await padButton(grid, "preview Kick")).click();
  // Nothing in the window overflows sideways.
  expect(await overflow(win)).toEqual([]);
  const panel = await openDeviceWindow(page, sampler);
  await expect(panel.locator('[data-widget="Envelope"]')).toBeVisible();
  await expect(panel.locator('[data-section="loop"]')).toBeVisible();
  if (shots) {
    await page.mouse.move(0, 0);
    await page.waitForTimeout(300);
    await win.screenshot({ path: `${shots}/DrumRack-pad-dark.png`, timeout: 15_000 });
  }
  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

test("multisampler window: zones dropped on the map, edited in the inspector", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const { device, win } = await insert(page, "MultiSampler");
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const map = win.locator('[data-widget="ZoneMap"]');
  for (const name of ["Kick.wav", "Snare.wav"]) {
    await libraryTree(page).getByRole("treeitem", { name, exact: true }).dragTo(map);
  }
  const zones = async () => {
    const d = (await project(page)).devices[device];
    return d?.kind.type === "Builtin" && d.kind.device.type === "MultiSampler" ? d.kind.device.zones : [];
  };
  await expect.poll(async () => (await zones()).length, { timeout: 20_000 }).toBe(2);
  expect(await overflow(win)).toEqual([]);
  if (shots) {
    await page.mouse.move(0, 0);
    await page.waitForTimeout(300);
    await win.screenshot({ path: `${shots}/MultiSampler-zones-dark.png`, timeout: 15_000 });
  }
  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

/** Effects whose windows open wider than the base (`window/sizes.ts`). */
const WIDE_EFFECTS = [
  "Eq",
  "Compressor",
  "Gate",
  "Limiter",
  "MultibandCompressor",
  "Delay",
  "Reverb",
  "ConvolutionReverb",
  "Utility",
  "SpectrumAnalyzer",
] as const;

test("effects with wide visuals open at their own width and fit", async ({ page }) => {
  test.setTimeout(240_000);
  await open(page, "dark");
  const track = await createTrack(page, "Audio");
  await openDeviceTab(page, track.name);
  for (const type of WIDE_EFFECTS) {
    const before = new Set(Object.keys((await project(page)).devices));
    await addDevice(page, type);
    let id = "";
    await expect
      .poll(async () => {
        const p = await project(page);
        id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
        return id !== "";
      })
      .toBe(true);
    const win = await openDeviceWindow(page, id);
    expect(Math.round((await win.boundingBox())!.width), type).toBeGreaterThan(600);
    expect(await overflow(win), type).toEqual([]);
    if (shots) {
      await page.mouse.move(0, 0);
      await page.waitForTimeout(200);
      await win.screenshot({ path: `${shots}/fx-${type}-dark.png`, timeout: 15_000 });
    }
    await win
      .locator(":scope > header")
      .getByRole("button", { name: /^close .* window$/ })
      .click();
  }
});

for (const theme of ["dark", "light"]) {
  test(`instrument window screenshots (${theme})`, async ({ page }) => {
    test.skip(!shots, "set INSTRUMENT_SHOTS=<dir> to capture the windows");
    test.setTimeout(240_000);
    await open(page, theme);
    for (const type of INSTRUMENTS) {
      const { win } = await insert(page, type);
      await page.mouse.move(0, 0);
      await page.waitForTimeout(300);
      await win.screenshot({ path: `${shots}/${type}-${theme}.png`, timeout: 15_000 });
      await win
        .locator(":scope > header")
        .getByRole("button", { name: /^close .* window$/ })
        .click();
    }
  });
}
