// sidechain: set a compressor's sidechain source from its device header on the web build
// (real wasm engine + controller). Two audio tracks; a compressor on the second one; the
// header selector lists the first track (not master; its own track for its pre-FX signal),
// sets it, undo restores "No sidechain" in one step, and clearing works. Devices without a
// sidechain input (Delay) show no selector. v0.4: a compressor two racks deep keys from a
// rack chain of another track through the tap picker ("Audio Effect Rack | Chain 1 | ...").
//
// Screenshots for the PR (skipped unless `SIDECHAIN_SHOTS=<dir>`): the nested compressor's
// header with its source and chain tap, and the tap picker open.
import { expect, test, type Page } from "@playwright/test";
import type { Device, Project } from "@/generated";
import { addDevice, addToRackChain, createTrack, launch, newProject, openDeviceTab, playButton, rackChains, rackPanel } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const count = (o: object) => Object.keys(o).length;

test("set a compressor sidechain from the device header", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await launch(page);
  await expect(playButton(page)).toBeVisible({
    timeout: 30_000,
  });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  const name = `Sidechain ${Date.now()}`;
  await newProject(page, name);
  await expect(page.getByTestId("project-name")).toHaveText(name);
  const baseTracks = count((await doc(page)).tracks);

  // Two audio tracks: the first is the source, the second gets the compressor.
  for (let i = 1; i <= 2; i++) {
    await createTrack(page, "Audio");
    await expect.poll(async () => count((await doc(page)).tracks)).toBe(baseTracks + i);
  }
  const audio = Object.values((await doc(page)).tracks)
    .filter((t) => t.kind === "Audio")
    .sort((a, b) => (a.order < b.order ? -1 : 1));
  const [source, target] = audio as [(typeof audio)[0], (typeof audio)[0]];

  await openDeviceTab(page, target.name);
  const chainOf = async () => Object.values((await doc(page)).devices).filter((d) => d.track === target.id);
  for (const [i, type] of (["Compressor", "Delay"] as const).entries()) {
    await addDevice(page, type);
    await expect.poll(async () => (await chainOf()).length).toBe(i + 1);
  }
  const chain = await chainOf();
  const comp = chain.find((d) => d.kind.type === "Builtin" && d.kind.device.type === "Compressor")!;
  const delay = chain.find((d) => d.id !== comp.id)!;
  const panel = (id: string) => page.locator(`[data-device="${id}"]`);

  const selector = panel(comp.id).getByRole("button", {
    name: `sidechain source for ${comp.name}`,
  });
  await expect(selector).toBeVisible();
  await expect(selector).toHaveText("no sidechain");
  await expect(panel(delay.id).getByRole("button", { name: /sidechain source/ })).toHaveCount(0);

  // Only other non-master tracks are offered (one dropdown, with a search).
  await selector.click();
  const sources = page.getByRole("menu", { name: "sidechain sources" });
  const labels = (await sources.getByRole("menuitem").allTextContents()).map((l) => l.trim());
  await expect(page.getByRole("searchbox", { name: "search tracks" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(selector).toHaveAttribute("aria-expanded", "false");
  expect(labels).toContain(source.name);
  expect(labels).toContain(`${target.name} (this track)`);
  expect(labels).not.toContain("master");

  const sidechainOf = async () => (await doc(page)).devices[comp.id]?.sidechain;
  /** Opens the picker and clicks the source `name` (its default tap point). */
  const pick = async (name: string) => {
    await selector.click();
    await page.getByRole("menu", { name: "sidechain sources" }).getByRole("menuitem", { name, exact: true }).click();
  };
  await pick(source.name);
  await expect.poll(sidechainOf).toBe(source.id);
  await expect(selector).toHaveText(`${source.name} · post mixer`);

  // One undo step.
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(sidechainOf).toBeNull();
  await expect(selector).toHaveText("no sidechain");

  await pick(source.name);
  await expect.poll(sidechainOf).toBe(source.id);
  await pick("no sidechain");
  await expect.poll(sidechainOf).toBeNull();

  expect(errors).toEqual([]);
});

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

const isType = (d: Device, type: string) => d.kind.type === "Builtin" && d.kind.device.type === type;

/**
 * An Audio Effect Rack on `track`, which comes with its first chain (4d8c128d: a row on the
 * rack's card); returns the rack and its panel.
 */
async function rackWithChain(page: Page, trackName: string, before: Set<string>) {
  await openDeviceTab(page, trackName);
  await addDevice(page, "AudioEffectRack");
  const rack = await newDevice(page, (d) => isType(d, "AudioEffectRack") && d.chain == null, before);
  const panel = rackPanel(page, rack.id);
  await expect(rackChains(panel)).toHaveCount(1);
  return { rack, panel };
}

test("a compressor two racks deep keys from a rack chain of another track", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await page.setViewportSize({ width: 1600, height: 1400 });
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Nested sidechain ${Date.now()}`);

  const kick = await createTrack(page, "Audio");
  const bass = await createTrack(page, "Audio");
  const ids = async () => new Set(Object.keys((await doc(page)).devices));
  // Kick: Audio Effect Rack → Chain 1 (the tapped chain).
  const { rack: kickRack } = await rackWithChain(page, kick.name, await ids());
  // Bass: Audio Effect Rack → Chain 1 → Audio Effect Rack → Chain 1 → Compressor.
  const { panel: outer } = await rackWithChain(page, bass.name, await ids());
  let before = await ids();
  await addToRackChain(outer, "chain 1", "add audio effect", "Audio Effect Rack");
  const inner = await newDevice(page, (d) => d.chain != null && isType(d, "AudioEffectRack"), before);
  // Drawn in the outer rack's chain, with its own first chain.
  const innerPanel = rackPanel(page, inner.id);
  await expect(rackChains(innerPanel)).toHaveCount(1);
  before = await ids();
  await addToRackChain(innerPanel, "chain 1", "add audio effect", "Compressor");
  const comp = await newDevice(page, (d) => isType(d, "Compressor"), before);

  const card = page.locator(`section[data-device="${comp.id}"]`);
  await expect(card).toBeVisible();
  const source = card.getByRole("button", { name: `sidechain source for ${comp.name}` }).first();
  await expect(source).toHaveText("no sidechain");
  const sources = page.getByRole("menu", { name: "sidechain sources" });
  await source.click();
  await sources.getByRole("menuitem", { name: kick.name, exact: true }).click();
  const sc = async () => (await doc(page)).devices[comp.id]!;
  await expect.poll(async () => (await sc()).sidechain).toBe(kick.id);
  await expect(source).toHaveText(`${kick.name} · post mixer`);

  // The tap point: in the track's submenu (hover it).
  await source.click();
  await sources.getByRole("menuitem", { name: kick.name, exact: true }).hover();
  await page.getByRole("menu", { name: kick.name }).getByRole("menuitem", { name: "audio effect rack | chain 1 | post fx" }).click();
  const kickChain = Object.values((await doc(page)).rack_chains).find((c) => c.rack === kickRack.id)!;
  await expect.poll(async () => (await sc()).sidechain_tap).toEqual({ point: "PostFx", chain: { type: "Rack", chain: kickChain.id } });
  await expect(source).toHaveText(`${kick.name} · audio effect rack | chain 1 | post fx`);

  const shots = process.env.SIDECHAIN_SHOTS;
  if (shots) {
    await page.mouse.move(0, 0);
    await card.screenshot({
      path: `${shots}/nested-sidechain.png`,
      timeout: 15_000,
    });
    await source.click();
    await sources.getByRole("menuitem", { name: kick.name, exact: true }).hover();
    await expect(page.getByRole("menu", { name: kick.name })).toBeVisible();
    await page.screenshot({
      path: `${shots}/nested-sidechain-taps.png`,
      timeout: 15_000,
    });
    await page.keyboard.press("Escape");
  }

  // One undo step back to the track's post-fader output.
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(async () => (await sc()).sidechain_tap ?? null).toBeNull();
  await expect.poll(async () => (await sc()).sidechain).toBe(kick.id);
  expect(errors).toEqual([]);
});
