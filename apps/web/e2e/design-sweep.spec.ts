// design-sweep on the web build (real wasm engine + controller):
// - theme switch smoke: dark → light → dark restyles the page from the tokens (canvas
//   drawing included) without errors;
// - MIDI learn hooks: transport, track header, inspector (mixer) and device param controls
//   carry `data-midi-target` with the right target (MIDI learn reads only this attribute).
import { expect, test, type Page } from "@playwright/test";
import type { MidiMapTarget, Project } from "@/generated";

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

const count = (o: object) => Object.keys(o).length;

/** The target an element's `data-midi-target` names (null if none). */
const targetOf = (page: Page, selector: string) =>
  page
    .locator(selector)
    .first()
    .getAttribute("data-midi-target")
    .then((v) => (v === null ? null : (JSON.parse(v) as MidiMapTarget)));

const token = (page: Page, name: string) =>
  page.evaluate((n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(), name);

test("theme switch and MIDI learn targets", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  // A fresh project with one MIDI track (the built-in synth comes with it).
  const name = `Design sweep ${Date.now()}`;
  await page.getByRole("button", { name: "Projects" }).click();
  await page.getByLabel("New project name").fill(name);
  await page.getByRole("dialog", { name: "Projects" }).getByRole("button", { name: "New" }).click();
  await expect(page.getByTestId("project-name")).toHaveText(name);
  const baseTracks = count((await doc(page)).tracks);
  await page.getByRole("button", { name: /New track/ }).click();
  await page.getByRole("button", { name: "Create MIDI track" }).click();
  await expect.poll(async () => count((await doc(page)).tracks)).toBe(baseTracks + 1);
  const midi = Object.values((await doc(page)).tracks).find((t) => t.kind === "Midi")!;
  const synth = Object.values((await doc(page)).devices).find((d) => d.track === midi.id)!;
  expect(synth).toBeTruthy();

  // --- MIDI learn hooks --------------------------------------------------------------------
  // Transport controls.
  expect(await targetOf(page, ".eth-tb__play")).toEqual({ type: "Transport", action: "TogglePlay" });
  expect(await targetOf(page, '.eth-tb [aria-label="Stop"]:not(.eth-tb__play)')).toEqual({ type: "Transport", action: "Stop" });
  expect(await targetOf(page, ".eth-tb__record")).toEqual({ type: "Transport", action: "ToggleRecord" });
  expect(await targetOf(page, '.eth-tb [aria-label="Loop"]')).toEqual({ type: "Transport", action: "ToggleLoop" });
  expect(await targetOf(page, '.eth-tb [aria-label="Metronome"]')).toEqual({ type: "Transport", action: "ToggleMetronome" });
  expect(await targetOf(page, ".eth-tb__tap")).toEqual({ type: "Transport", action: "TapTempo" });

  // Arrangement track header: volume, mute, solo and arm name the track.
  const header = `.eth-arr-row[data-track="${midi.id}"] .eth-arr-header`;
  expect(await targetOf(page, `${header} .eth-arr-vol`)).toEqual({
    type: "Param",
    target: { type: "TrackVolume", track: midi.id },
  });
  expect(await targetOf(page, `${header} .eth-arr-header__mute`)).toEqual({ type: "TrackMute", track: midi.id });
  expect(await targetOf(page, `${header} .eth-arr-header__solo`)).toEqual({ type: "TrackSolo", track: midi.id });
  expect(await targetOf(page, `${header} .eth-arr-header__arm`)).toEqual({ type: "TrackArm", track: midi.id });

  // Inspector (the track's mixer controls and its device params).
  await page.getByRole("group", { name: `${midi.name} track` }).click();
  const inspector = page.getByTestId("inspector");
  await expect(inspector).toBeVisible();
  expect(await targetOf(page, `[data-testid="inspector"] [aria-label="${midi.name} volume"]`)).toEqual({
    type: "Param",
    target: { type: "TrackVolume", track: midi.id },
  });
  expect(await targetOf(page, `[data-testid="inspector"] [aria-label="${midi.name} pan"]`)).toEqual({
    type: "Param",
    target: { type: "TrackPan", track: midi.id },
  });
  expect(await targetOf(page, '[data-testid="inspector"] .eth-inspector__mute')).toEqual({ type: "TrackMute", track: midi.id });
  const params = inspector.locator(`[data-device="${synth.id}"] [data-param]`);
  await expect(params.first()).toBeVisible();
  const n = await params.count();
  expect(n).toBeGreaterThan(0);
  for (let i = 0; i < n; i++) {
    const el = params.nth(i);
    const param = Number(await el.getAttribute("data-param"));
    const target = JSON.parse((await el.getAttribute("data-midi-target"))!) as MidiMapTarget;
    expect(target).toEqual({ type: "Param", target: { type: "DeviceParam", device: synth.id, param } });
  }

  // --- Theme switch smoke -----------------------------------------------------------------
  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-theme", "dark");
  const darkBg = await token(page, "--eth-color-bg");
  const darkSeam = await token(page, "--eth-color-warp-wave");
  await page.getByRole("button", { name: "Switch to light theme" }).click();
  await expect(html).toHaveAttribute("data-theme", "light");
  expect(await token(page, "--eth-color-bg")).not.toBe(darkBg);
  expect(await token(page, "--eth-color-warp-wave")).not.toBe(darkSeam);
  await expect(page.locator(".eth-arr")).toBeVisible();
  await page.getByRole("button", { name: "Switch to dark theme" }).click();
  await expect(html).toHaveAttribute("data-theme", "dark");
  expect(await token(page, "--eth-color-bg")).toBe(darkBg);

  expect(errors).toEqual([]);
});
