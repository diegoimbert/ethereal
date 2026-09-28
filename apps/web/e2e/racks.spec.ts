// racks-modulation on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet).
//
// MIDI track → Instrument Rack → "+ Chain" twice → a Synth on chain 1 → an LFO on the rack →
// drag the LFO onto a synth knob (a depth ring appears) → drag the depth chip → map Macro 1
// onto another synth param (click its handle, then the param) → undo/redo (no page errors).
//
// Screenshots for the PR (skipped unless `RACKS_SHOTS=<dir>`; `RACKS_THEME=dark|light`):
// the rack with chains + macros, and a modulator mapped onto a knob with its depth ring.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Device, ModMapping, Project } from "@/generated";
import { addDevice, createTrack, newProject, openDeviceTab, pickOption, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const shots = process.env.RACKS_SHOTS;
const theme = process.env.RACKS_THEME ?? "dark";

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

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
  await page.goto("/");
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
  await expect(rackCard.getByText("Loading…")).toHaveCount(0);

  // --- two chains, a synth on the first ---------------------------------------------------
  const chains = rackCard.getByRole("listbox", { name: "Chains" });
  await rackCard.getByRole("button", { name: "Chain", exact: true }).click();
  await expect(chains.getByRole("option")).toHaveCount(1);
  await rackCard.getByRole("button", { name: "Chain", exact: true }).click();
  await expect(chains.getByRole("option")).toHaveCount(2);
  await chains.getByRole("option", { name: "Chain 1" }).getByText("Chain 1").click();
  await expect(chains.getByRole("option", { name: "Chain 1" })).toHaveAttribute("aria-selected", "true");
  before = new Set(Object.keys((await doc(page)).devices));
  await pickOption(rackCard, "Add device to Chain 1", { value: "Synth" });
  const synth = await newDevice(page, (d) => d.chain != null, before);
  const synthCard = page.locator(`section[data-device="${synth.id}"]`);
  await expect(synthCard.getByText("Loading…")).toHaveCount(0);
  // Chain devices are not on the track chain.
  expect(Object.values((await doc(page)).devices).some((d) => d.id === synth.id && d.chain == null)).toBe(false);
  await expect(page.locator(`.eth-devices__slot > section[data-device="${synth.id}"]`)).toHaveCount(0);

  // --- an LFO on the rack, mapped onto the synth ------------------------------------------
  await rackCard.getByRole("button", { name: `Add modulator to ${rack.name}` }).click();
  await page.getByRole("menuitem", { name: "Add LFO" }).click();
  const lfoCard = rackCard.locator("section[data-modulator]");
  await expect(lfoCard).toHaveCount(1);
  // Mapping mode: only params the LFO may reach light up (the synth's, the rack's selector).
  await lfoCard.getByRole("button", { name: "Map LFO" }).click();
  await expect(synthCard.getByTestId("mod-target").first()).toBeVisible();
  await lfoCard.getByRole("button", { name: "Stop mapping LFO" }).click();
  await expect(synthCard.getByTestId("mod-target")).toHaveCount(0);
  // Drag the LFO onto a synth knob.
  const paramId = async (name: string) =>
    Number(await synthCard.locator("[data-param]", { has: page.getByRole("slider", { name, exact: true }) }).first().getAttribute("data-param"));
  const cutoff = await paramId("Resonance");
  // Both ends on screen (the inspector scrolls).
  const grip = lfoCard.getByRole("button", { name: "Drag LFO onto a parameter" });
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
  const handle = rackCard.getByRole("button", { name: "Map Macro 1" });
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
    await param(synthCard, cutoff).hover();
    await synthCard.screenshot({ path: `${shots}/modulated-knob-${theme}.png`, timeout: 15_000 });
  }
  expect(errors).toEqual([]);
});
