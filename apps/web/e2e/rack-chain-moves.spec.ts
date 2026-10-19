// Rack chain moves on the web build, through the UI, against the real engine
// (ui/src/features/racks/chainClipboard.ts): drag a chain row to reorder it in its rack, drag it
// onto another rack's chains (a move), and right-click copy then paste it into another rack.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, addRackChain, createTrack, launch, newProject, openDeviceTab, playButton, rackChain, rackChains, rackPanel } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

/** The chain names of `rack`, in order. */
async function chainNames(page: Page, rack: string): Promise<string[]> {
  return Object.values((await doc(page)).rack_chains)
    .filter((c) => c.rack === rack)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1))
    .map((c) => c.name);
}

/** Adds an audio effect rack to the shown track; resolves to its id. */
async function addEffectRack(page: Page): Promise<string> {
  const before = new Set(Object.keys((await doc(page)).devices));
  await addDevice(page, "AudioEffectRack");
  let id = "";
  await expect
    .poll(async () => {
      const d = Object.values((await doc(page)).devices).find(
        (x) => !before.has(x.id) && x.kind.type === "Builtin" && x.kind.device.type === "AudioEffectRack",
      );
      id = d?.id ?? "";
      return id !== "";
    })
    .toBe(true);
  return id;
}

test("rack chains: drag to reorder, drag into another rack, copy and paste", async ({ page }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1600, height: 1400 });
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Chain Moves ${Date.now()}`);
  const track = await createTrack(page, "Audio");
  await openDeviceTab(page, track.name);

  // --- an audio effect rack with three chains; the third dragged first ---------------------
  const a = await addEffectRack(page);
  const panelA = rackPanel(page, a);
  await expect(rackChains(panelA)).toHaveCount(1);
  for (let i = 0; i < 2; i++) await addRackChain(panelA);
  await expect.poll(() => chainNames(page, a)).toEqual(["chain 1", "chain 2", "chain 3"]);
  // Dragged by its name, dropped on the top half of the first row.
  const row = (panel: typeof panelA, name: string) => rackChain(panel, name).locator(":scope > .eth-rack__row");
  await row(panelA, "chain 3")
    .locator(".eth-rack__name")
    .dragTo(row(panelA, "chain 1"), { targetPosition: { x: 40, y: 2 } });
  await expect.poll(() => chainNames(page, a)).toEqual(["chain 3", "chain 1", "chain 2"]);

  // --- a second rack: "chain 1" dragged onto its chains (moved) -----------------------------
  const b = await addEffectRack(page);
  const panelB = rackPanel(page, b);
  await expect(rackChains(panelB)).toHaveCount(1);
  // Dropped on the bottom half of its only row: after it.
  const target = row(panelB, "chain 1");
  const height = (await target.boundingBox())!.height;
  await row(panelA, "chain 1")
    .locator(".eth-rack__name")
    .dragTo(target, { targetPosition: { x: 40, y: height - 3 } });
  await expect.poll(() => chainNames(page, b)).toEqual(["chain 1", "chain 1"]);
  await expect.poll(() => chainNames(page, a)).toEqual(["chain 3", "chain 2"]);

  // --- right-click copy "chain 2", paste after rack b's first chain --------------------------
  await row(panelA, "chain 2").click({ button: "right" });
  await page.getByRole("menuitem", { name: /^copy/ }).click();
  await rackChains(panelB).first().locator(":scope > .eth-rack__row").click({ button: "right" });
  await page.getByRole("menuitem", { name: /^paste after/ }).click();
  await expect.poll(() => chainNames(page, b)).toEqual(["chain 1", "chain 2", "chain 1"]);
  // The source keeps its chain.
  expect(await chainNames(page, a)).toEqual(["chain 3", "chain 2"]);
  expect(errors).toEqual([]);
});
