// racks-modulation on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet).
//
// MIDI track → Instrument Rack → "+ Add instrument" twice (a chain with a Synth, one with a
// Poly Synth) → an LFO on the rack → drag the LFO onto a synth knob (a depth ring appears) →
// drag the depth chip → map Macro 1 (on the rack's card) onto another synth param (click its
// handle, then the param) → undo/redo (no page errors).
//
// Screenshots for the PR (skipped unless `RACKS_SHOTS=<dir>`; `RACKS_THEME=dark|light`):
// the rack with chains + macros, and a modulator mapped onto a knob with its depth ring.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Device, ModMapping, Project } from "@/generated";
import {
  ci,
  addDevice,
  addRackChain,
  addToRackChain,
  clickHeaderControl,
  createTrack,
  expandRackChain,
  launch,
  newProject,
  openDeviceTab,
  playButton,
  openDeviceWindow,
  rackChain,
  rackChains,
  rackPanel,
  showDeviceParams,
} from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const shots = process.env.RACKS_SHOTS;
const theme = process.env.RACKS_THEME ?? "dark";

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

async function newDevice(page: Page, pred: (d: Device) => boolean, before: Set<string>): Promise<Device> {
  let found: Device | undefined;
  await expect
    .poll(async () => {
      found = Object.values((await doc(page)).devices).find((d) => !before.has(d.id) && pred(d));
      return found !== undefined;
    })
    .toBe(true);
  return found!;
}

const mappings = async (page: Page): Promise<ModMapping[]> => Object.values((await doc(page)).mod_mappings);

/** A param widget of a device card by its param id. */
const param = (card: Locator, id: number): Locator => card.locator(`[data-param="${id}"]`).first();

test("racks: chains, a modulator and a macro mapped onto a synth", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await page.setViewportSize({ width: 1600, height: 1600 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Racks ${Date.now()}`);

  // --- MIDI track with an instrument rack ------------------------------------------------
  const midi = await createTrack(page, "Midi");
  await openDeviceTab(page, midi.name);
  let before = new Set(Object.keys((await doc(page)).devices));
  await addDevice(page, "InstrumentRack");
  const rack = await newDevice(page, (d) => d.kind.type === "Builtin" && d.kind.device.type === "InstrumentRack", before);
  const rackCard = page.locator(`section[data-device="${rack.id}"]`);
  await expect(rackCard.getByText("loading…")).toHaveCount(0);

  // --- two chains, a synth on the first ---------------------------------------------------
  // b8ad9f79: an instrument rack's "+ Add instrument" adds a chain with the picked instrument
  // (a collapsed row on the rack's card) and opens the instrument's window (closed again).
  const panel = rackPanel(page, rack.id);
  await expect(rackChains(panel)).toHaveCount(0);
  before = new Set(Object.keys((await doc(page)).devices));
  await addRackChain(panel, "Synth");
  const synth = await newDevice(page, (d) => d.chain != null, before);
  await addRackChain(panel, "Poly Synth");
  // The synth's card is in its expanded chain. base-153: a device's own UI (knobs) is in its
  // window; the rack's card keeps its chains, macros and modulators.
  await expandRackChain(panel, "chain 1");
  const synthCard = await openDeviceWindow(page, synth.id);
  // Chain devices are not on the track chain.
  expect(Object.values((await doc(page)).devices).some((d) => d.id === synth.id && d.chain == null)).toBe(false);
  await expect(
    page
      .getByRole("list", { name: `${midi.name} devices`, exact: true })
      .locator(`:scope > [role="group"] > .eth-devices__slot > section[data-device="${synth.id}"]`),
  ).toHaveCount(0);
  await expect(rackChain(panel, "chain 1").locator(`section[data-device="${synth.id}"]`)).toHaveCount(1);

  // --- an LFO on the rack, mapped onto the synth ------------------------------------------
  await clickHeaderControl(rackCard.getByRole("button", { name: `add modulator to ${rack.name}` }));
  await page.getByRole("menuitem", { name: "add LFO" }).click();
  const lfoCard = rackCard.locator("section[data-modulator]");
  await expect(lfoCard).toHaveCount(1);
  // Mapping mode: only params the LFO may reach light up (the synth's, the rack's selector).
  await lfoCard.getByRole("button", { name: "map LFO" }).click();
  await expect(synthCard.getByTestId("mod-target").first()).toBeVisible();
  await lfoCard.getByRole("button", { name: "stop mapping LFO" }).click();
  await expect(synthCard.getByTestId("mod-target")).toHaveCount(0);
  // Drag the LFO onto a synth knob.
  const paramId = async (name: string) =>
    Number(
      await synthCard
        .locator("[data-param]", { has: page.getByRole("slider", { name: ci(name) }) })
        .first()
        .getAttribute("data-param"),
    );
  const cutoff = await paramId("Resonance");
  // Both ends on screen (the inspector scrolls).
  const grip = lfoCard.getByRole("button", { name: "drag LFO onto a parameter" });
  await param(synthCard, cutoff).scrollIntoViewIfNeeded();
  const to = (await param(synthCard, cutoff).boundingBox())!;
  const from = (await grip.boundingBox())!;
  expect(from.y + from.height).toBeLessThan(page.viewportSize()!.height);
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 3, { steps: 10 });
  // The drop targets light up once the drag started.
  await expect(param(synthCard, cutoff).getByTestId("mod-target")).toBeVisible();
  await page.mouse.move(to.x + to.width / 2 + 1, to.y + to.height / 3, { steps: 2 });
  await page.mouse.up();
  await expect.poll(async () => (await mappings(page)).length).toBe(1);
  await expect(synthCard.getByTestId("mod-target")).toHaveCount(0);
  await expect(param(synthCard, cutoff).getByTestId("mod-ring")).toBeVisible();

  // Depth: drag the chip down (one undo step).
  const chip = param(synthCard, cutoff).getByTestId("mod-depth");
  const box = (await chip.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + 60, { steps: 6 });
  await page.mouse.up();
  await expect.poll(async () => (await mappings(page))[0]!.depth).toBeLessThan(0.5);

  // --- Macro 1 onto another synth param -------------------------------------------------------
  const other = await paramId("Decay");
  // b8ad9f79: a rack has no window; its macros are on its card, behind the parameters toggle.
  const rackParams = await showDeviceParams(page, rack.id);
  const handle = rackParams.getByRole("button", { name: "map Macro 1" });
  await handle.click();
  await param(synthCard, other).getByTestId("mod-target").click();
  await expect.poll(async () => (await mappings(page)).some((m) => m.source.type === "Macro" && m.param === other)).toBe(true);

  // Undo the macro mapping, redo it.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await mappings(page)).length).toBe(1);
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(async () => (await mappings(page)).length).toBe(2);

  if (shots) {
    await page.mouse.move(0, 0);
    await rackCard.scrollIntoViewIfNeeded();
    await rackCard.screenshot({ path: `${shots}/rack-chains-macros-${theme}.png`, timeout: 15_000 });
    await synthCard.scrollIntoViewIfNeeded();
    // No hover or focus on a knob: the kit turns its label into the value readout then.
    await page.mouse.move(0, 0);
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    await synthCard.screenshot({ path: `${shots}/modulated-knob-${theme}.png`, timeout: 15_000 });
  }
  expect(errors).toEqual([]);
});

// Nested racks: an Audio Effect Rack inside an Audio Effect Rack's chain, drawn one level in
// with its own chains; a Reverb on the inner chain; the outer rack's macro reaches it
// (modulation at any depth); undo/redo. Screenshot `nested-rack-<theme>.png` with RACKS_SHOTS.
test("racks: a rack nested in a rack chain", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await page.setViewportSize({ width: 1600, height: 1600 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Nested ${Date.now()}`);

  const audio = await createTrack(page, "Audio");
  await openDeviceTab(page, audio.name);
  let before = new Set(Object.keys((await doc(page)).devices));
  await addDevice(page, "AudioEffectRack");
  const outer = await newDevice(page, (d) => d.kind.type === "Builtin" && d.kind.device.type === "AudioEffectRack", before);
  const outerCard = page.locator(`section[data-device="${outer.id}"]`);
  // A new effect rack comes with its first chain (4d8c128d).
  const outerPanel = rackPanel(page, outer.id);
  await expect(rackChains(outerPanel)).toHaveCount(1);

  // An Audio Effect Rack on the outer rack's chain.
  before = new Set(Object.keys((await doc(page)).devices));
  await addToRackChain(outerPanel, "chain 1", "add audio effect", "Audio Effect Rack");
  const inner = await newDevice(page, (d) => d.chain != null && d.kind.type === "Builtin" && d.kind.device.type === "AudioEffectRack", before);
  const innerCard = rackChain(outerPanel, "chain 1").locator(`section[data-device="${inner.id}"]`);
  await expect(innerCard).toBeVisible();
  const innerPanel = rackPanel(page, inner.id);
  await expect(rackChains(innerPanel)).toHaveCount(1);
  await expect(innerPanel).toHaveClass(/eth-rack--nested/);
  await expect(innerPanel).toHaveAttribute("data-level", "2");

  // A Reverb on the inner rack's chain.
  before = new Set(Object.keys((await doc(page)).devices));
  await addToRackChain(innerPanel, "chain 1", "add audio effect", "Reverb");
  const reverb = await newDevice(page, (d) => d.kind.type === "Builtin" && d.kind.device.type === "Reverb", before);
  const reverbCard = await openDeviceWindow(page, reverb.id);
  const p = await doc(page);
  expect(p.rack_chains[p.devices[reverb.id]!.chain!]?.rack).toBe(inner.id);
  expect(p.rack_chains[p.devices[inner.id]!.chain!]?.rack).toBe(outer.id);

  // The outer rack's Macro 1 reaches the Reverb two levels down.
  // (A rack has no window: its macros are on its card, b8ad9f79.)
  await (await showDeviceParams(page, outer.id)).getByRole("button", { name: "map Macro 1" }).first().click();
  await reverbCard.getByTestId("mod-target").first().click();
  await expect
    .poll(async () => (await mappings(page)).some((m) => m.source.type === "Macro" && m.source.rack === outer.id && m.device === reverb.id))
    .toBe(true);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await mappings(page)).length).toBe(0);
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(async () => (await mappings(page)).length).toBe(1);

  if (shots) {
    await page.mouse.move(0, 0);
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    await outerCard.scrollIntoViewIfNeeded();
    await outerCard.screenshot({ path: `${shots}/nested-rack-${theme}.png`, timeout: 15_000 });
  }
  expect(errors).toEqual([]);
});
