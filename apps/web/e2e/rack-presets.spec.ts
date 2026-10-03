// rack-presets on the web build (real wasm engine): a factory rack preset that stores chains
// loads from the rack's preset menu (chains, chain devices, the rack's LFO and the macro
// mappings appear), undo removes them in one step, and a user preset of the rack restores the
// same structure on a fresh rack.
//
// Screenshots for the PR (skipped unless `RACK_PRESETS_SHOTS=<dir>`;
// `RACK_PRESETS_THEME=dark|light`): the preset menu of the rack, and the rack after loading.
import { expect, test, type Page } from "@playwright/test";
import type { Device, Project } from "@/generated";
import { addDevice, createTrack, newProject, openDeviceTab, openLibrary, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const shots = process.env.RACK_PRESETS_SHOTS;
const theme = process.env.RACK_PRESETS_THEME ?? "dark";

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

/** A new MIDI track with an Instrument Rack; resolves with the rack. */
async function instrumentRack(page: Page): Promise<Device> {
  const track = await createTrack(page, "Midi");
  await openDeviceTab(page, track.name);
  const before = new Set(Object.keys((await doc(page)).devices));
  await addDevice(page, "InstrumentRack");
  let rack: Device | undefined;
  await expect
    .poll(async () => {
      rack = Object.values((await doc(page)).devices).find((d) => !before.has(d.id) && d.kind.type === "Builtin" && d.kind.device.type === "InstrumentRack");
      return rack !== undefined;
    })
    .toBe(true);
  await expect(page.locator(`section[data-device="${rack!.id}"]`).getByText("Loading…")).toHaveCount(0);
  return rack!;
}

/** Chain names of a rack, in order. */
async function chainNames(page: Page, rack: string): Promise<string[]> {
  return Object.values((await doc(page)).rack_chains)
    .filter((c) => c.rack === rack)
    .sort((a, b) => (a.order < b.order ? -1 : 1))
    .map((c) => c.name);
}

const structure = async (page: Page, rack: string) => {
  const p = await doc(page);
  const chains = Object.values(p.rack_chains).filter((c) => c.rack === rack);
  const devices = Object.values(p.devices).filter((d) => chains.some((c) => c.id === d.chain));
  return {
    chains: chains.length,
    devices: devices.length,
    modulators: Object.values(p.modulators).filter((m) => m.device === rack).length,
    mappings: Object.values(p.mod_mappings).filter((m) => m.device === rack || devices.some((d) => d.id === m.device)).length,
  };
};

test("rack presets: load a factory rack with chains, undo, save and reload it on another rack", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Rack presets ${Date.now()}`);

  const rack = await instrumentRack(page);
  const card = page.locator(`section[data-device="${rack.id}"]`);
  const trigger = card.getByRole("button", { name: `Presets for ${rack.name}` }).first();
  const panel = page.getByRole("dialog", { name: `Presets for ${rack.name}` });

  // --- Load "Layered Pad": two chains with a Poly Synth each, an LFO, four mappings -------
  await trigger.click();
  await expect(panel.getByRole("button", { name: /Layered Pad/ })).toBeVisible();
  if (shots) {
    await page.mouse.move(0, 0);
    await page.screenshot({ path: `${shots}/rack-presets-menu-${theme}.png` });
  }
  await panel.getByRole("button", { name: /Layered Pad/ }).click();
  await expect.poll(() => chainNames(page, rack.id)).toEqual(["Body", "Air"]);
  expect(await structure(page, rack.id)).toEqual({ chains: 2, devices: 2, modulators: 1, mappings: 4 });
  const chains = card.getByRole("listbox", { name: "Chains" });
  await expect(chains.getByRole("option")).toHaveCount(2);
  await expect(chains.getByRole("option", { name: "Body" })).toBeVisible();
  await expect(card.locator("section[data-modulator]")).toHaveCount(1);
  await expect(trigger).toHaveText(/Layered Pad/);
  if (shots) {
    await page.mouse.move(0, 0);
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    await page.screenshot({ path: `${shots}/rack-presets-loaded-${theme}.png` });
  }

  // --- One undo step back to the empty rack, and redo ------------------------------------
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => structure(page, rack.id)).toEqual({ chains: 0, devices: 0, modulators: 0, mappings: 0 });
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(() => chainNames(page, rack.id)).toEqual(["Body", "Air"]);

  // --- Save it as a user preset and load it onto a fresh rack ------------------------------
  await trigger.click();
  await panel.getByRole("button", { name: "Save preset…" }).click();
  const save = page.getByRole("dialog", { name: `Save preset for ${rack.name}` });
  await save.getByRole("textbox", { name: "Preset name" }).fill("E2E Layers");
  await save.getByRole("button", { name: "Save" }).click();
  await expect(save).toBeHidden();

  const rack2 = await instrumentRack(page);
  const card2 = page.locator(`section[data-device="${rack2.id}"]`);
  await card2.getByRole("button", { name: `Presets for ${rack2.name}` }).first().click();
  await panel.getByRole("region", { name: "User presets" }).getByRole("button", { name: /^E2E Layers/ }).click();
  await expect.poll(() => chainNames(page, rack2.id)).toEqual(["Body", "Air"]);
  expect(await structure(page, rack2.id)).toEqual({ chains: 2, devices: 2, modulators: 1, mappings: 4 });
  // New ids: the first rack keeps its own chains.
  expect(await chainNames(page, rack.id)).toEqual(["Body", "Air"]);

  // Clean up the user preset (the OPFS library outlives the test).
  await card2.getByRole("button", { name: `Presets for ${rack2.name}` }).first().click();
  const user = panel.getByRole("region", { name: "User presets" });
  await user.getByRole("button", { name: "More actions for E2E Layers" }).click();
  await page.getByRole("menuitem", { name: "Delete Preset…" }).click();
  await page.getByRole("dialog", { name: "Delete “E2E Layers”?" }).getByRole("button", { name: "Delete" }).click();
  expect(errors).toEqual([]);
});

test("rack presets: a factory rack preset loads from the library browser", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Rack presets browser ${Date.now()}`);
  // The new track is selected: the browser loads presets onto its matching device.
  const rack = await instrumentRack(page);

  await openLibrary(page, "Factory Presets");
  const browser = page.locator('[data-feature="browser"][data-browser="v2"]');
  await expect(browser).toBeVisible({ timeout: 20_000 });
  await browser.getByRole("searchbox", { name: "Search files" }).fill("Key Split");
  const row = browser.getByRole("list", { name: "Search results" }).getByRole("button", { name: "Key Split", exact: true });
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.dblclick();
  await expect.poll(() => chainNames(page, rack.id)).toEqual(["Bass", "Keys"]);
  expect(await structure(page, rack.id)).toEqual({ chains: 2, devices: 2, modulators: 0, mappings: 2 });
  expect(errors).toEqual([]);
});
