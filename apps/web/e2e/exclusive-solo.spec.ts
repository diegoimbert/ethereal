// Exclusive solo on the web build, through the UI, against the real engine.
//
// A plain click on a Solo button makes that item the only soloed one of its scope (soloed or not);
// the only soloed item unsolos on a plain click; Ctrl/Cmd-click is the additive toggle. Scopes:
// arranger tracks, the chains of an audio effect rack, the pads of a drum rack.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import {
  addDevice,
  addDrumRack,
  addRackChain,
  rackChains,
  rackPanel,
  createTrack,
  expandFolder,
  launch,
  newProject,
  openDeviceTab,
  openLibrary,
  padButton,
  playButton,
  headerButton,
} from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const MOD = ["Control"] as const;

test("exclusive solo: arranger tracks, audio effect rack chains, drum rack pads", async ({ page }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await page.setViewportSize({ width: 1600, height: 1400 });
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Exclusive Solo ${Date.now()}`);

  // --- arranger: three tracks ------------------------------------------------------------
  const a = await createTrack(page, "Midi");
  const b = await createTrack(page, "Midi");
  const c = await createTrack(page, "Midi");
  const solo = (name: string) => headerButton(page, name, `solo ${name}`);
  const soloedTracks = async () =>
    Object.values((await doc(page)).tracks)
      .filter((t) => t.mixer.solo)
      .map((t) => t.name)
      .sort();
  const names = [a.name, b.name, c.name];
  for (const n of names) await (await solo(n)).click({ modifiers: [...MOD] });
  await expect.poll(soloedTracks).toEqual([...names].sort());
  await (await solo(b.name)).click();
  await expect.poll(soloedTracks).toEqual([b.name]);
  await (await solo(c.name)).click();
  await expect.poll(soloedTracks).toEqual([c.name]);
  await (await solo(c.name)).click();
  await expect.poll(soloedTracks).toEqual([]);

  // --- audio effect rack: three chains ---------------------------------------------------
  await openDeviceTab(page, a.name);
  const before = new Set(Object.keys((await doc(page)).devices));
  await addDevice(page, "AudioEffectRack");
  let rackId = "";
  await expect
    .poll(async () => {
      const d = Object.values((await doc(page)).devices).find(
        (x) => !before.has(x.id) && x.kind.type === "Builtin" && x.kind.device.type === "AudioEffectRack",
      );
      rackId = d?.id ?? "";
      return rackId !== "";
    })
    .toBe(true);
  // A new effect rack comes with its first chain (4d8c128d); two more. Each is a row on the
  // rack's card with the track header's solo (RackPanel.tsx).
  const panel = rackPanel(page, rackId);
  await expect(rackChains(panel)).toHaveCount(1);
  for (let i = 0; i < 2; i++) await addRackChain(panel);
  const chainSolo = rackChains(panel).getByRole("button", { name: /^solo chain \d$/ });
  await expect(chainSolo).toHaveCount(3);
  const expanded = () =>
    rackChains(panel)
      .locator(":scope > .eth-rack__row")
      .evaluateAll((rows) => rows.map((r) => r.getAttribute("aria-expanded")));
  const expandedBefore = await expanded();
  const chainSoloed = async () =>
    Object.values((await doc(page)).rack_chains)
      .filter((ch) => ch.rack === rackId && ch.solo)
      .map((ch) => ch.name)
      .sort();
  const windowsBefore = await page.getByRole("dialog").count();
  for (let i = 0; i < 3; i++) await chainSolo.nth(i).click({ modifiers: [...MOD] });
  await expect.poll(chainSoloed).toEqual(["chain 1", "chain 2", "chain 3"]);
  await chainSolo.nth(1).click();
  await expect.poll(chainSoloed).toEqual(["chain 2"]);
  await chainSolo.nth(2).click();
  await expect.poll(chainSoloed).toEqual(["chain 3"]);
  await chainSolo.nth(2).click();
  await expect.poll(chainSoloed).toEqual([]);
  // Solo clicks neither expand / collapse a chain nor open a device window.
  expect(await expanded()).toEqual(expandedBefore);
  await expect(page.getByRole("dialog")).toHaveCount(windowsBefore);

  // --- drum rack: three pads (runtime solo: read the pressed state of the buttons) ---------
  const d = await createTrack(page, "Midi");
  // The library first: the rack's window (its pad editor) opens over the top of the page.
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const { win: view } = await addDrumRack(page, d.name);
  const grid = view.getByRole("grid", { name: "drum pads" });
  await expect(grid.getByRole("gridcell")).toHaveCount(16);
  const files = page.getByRole("tree", { name: "folders" });
  const cells: [string, RegExp][] = [
    ["Kick.wav", /^C1 /],
    ["Snare.wav", /^D1 /],
    ["Hat.wav", /^D#1 /],
  ];
  for (const [i, [file, cell]] of cells.entries()) {
    await files.getByRole("treeitem", { name: file, exact: true }).dragTo(grid.getByRole("gridcell", { name: cell }));
    await expect.poll(async () => Object.keys((await doc(page)).drum_pads).length, { timeout: 20_000 }).toBe(i + 1);
  }
  // Each pad's Solo button sits on the pad (2533f415), as on a track header; its pressed state
  // is the pad's (runtime) solo.
  const padNames = ["Kick", "Snare", "Hat"];
  const soloedPads = async () => {
    const out: string[] = [];
    for (const n of padNames) if ((await grid.getByRole("button", { name: `solo ${n}`, exact: true }).getAttribute("aria-pressed")) === "true") out.push(n);
    return out;
  };
  const clickPad = async (name: string, modifiers: "Control"[] = []) => (await padButton(grid, `solo ${name}`)).click({ modifiers });
  for (const n of padNames) await clickPad(n, ["Control"]);
  await expect.poll(soloedPads).toEqual(padNames);
  await clickPad("Snare");
  await expect.poll(soloedPads).toEqual(["Snare"]);
  await clickPad("Hat");
  await expect.poll(soloedPads).toEqual(["Hat"]);
  await clickPad("Hat");
  await expect.poll(soloedPads).toEqual([]);

  expect(errors).toEqual([]);
});
