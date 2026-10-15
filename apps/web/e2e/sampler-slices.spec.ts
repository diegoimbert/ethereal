// Sampler slicing (#760) on the web build, through the UI, against the real engine:
// a demo loop dropped in a Sampler → its window's "slice" mode finds the transients (markers
// on the waveform) → pressing a slice plays it (the track meter moves) → "slice to drum
// rack" turns it into one pad per slice (one undo step) → a sample dropped on the rack
// window's "slice to pads" adds a pad per slice above them (one undo step).
//
// No sleeps: every step waits on UI or engine state (the mirror through `window.__ether`).
import { expect, test, type Page } from "@playwright/test";
import type { Device, Project, SliceSettings } from "@/generated";
import { addDevice, createTrack, expandFolder, launch, newProject, openDeviceTab, openDeviceWindow, openLibrary, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

const onTrack = async (page: Page, track: string, type: string): Promise<Device[]> =>
  Object.values((await doc(page)).devices).filter((d) => d.track === track && d.pad === null && d.kind.type === "Builtin" && d.kind.device.type === type);

const slicesOf = (d: Device | undefined): SliceSettings | null =>
  d?.kind.type === "Builtin" && d.kind.device.type === "Sampler" ? d.kind.device.slices : null;

const padNotes = async (page: Page, rack: string): Promise<number[]> =>
  Object.values((await doc(page)).drum_pads)
    .filter((p) => p.rack === rack)
    .map((p) => p.note)
    .sort((a, b) => a - b);

test.use({ viewport: { width: 1600, height: 1000 } });

test("sampler slices: transients, play a slice, slice to drum rack, slice to pads", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, `Sampler slices ${Date.now()}`);

  // --- A Sampler playing the demo bass loop ---------------------------------------------
  const midi = await createTrack(page, "Midi");
  await openDeviceTab(page, midi.name);
  await addDevice(page, "Sampler");
  await expect.poll(async () => (await onTrack(page, midi.id, "Sampler")).length).toBe(1);
  const samplerId = (await onTrack(page, midi.id, "Sampler"))[0]!.id;
  await page
    .getByRole("list", { name: `${midi.name} devices` })
    .getByRole("button", { name: "open Sampler window" })
    .click();
  const win = page.locator(`[data-device-window="${samplerId}"]`);
  await expect(win.getByText("loading…")).toHaveCount(0);
  await openLibrary(page);
  const files = page.getByRole("tree", { name: "folders" });
  await expandFolder(page, "Demo Samples");
  await files.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true }).dragTo(win.getByTestId("sample-slot"));
  await expect(win.getByTestId("sample-slot")).toContainText("Bass Loop", { timeout: 20_000 });

  // --- Slice mode: the transients appear as markers --------------------------------------
  await win.getByRole("tab", { name: "slice", exact: true }).click();
  const sampler = async () => (await doc(page)).devices[samplerId];
  await expect.poll(async () => slicesOf(await sampler())?.enabled, { timeout: 20_000 }).toBe(true);
  await expect.poll(async () => slicesOf(await sampler())?.markers.length ?? 0).toBeGreaterThan(1);
  const markers = slicesOf(await sampler())!.markers.length;
  const editor = win.getByTestId("slice-editor");
  await expect(editor.getByRole("slider", { name: /^slice \d+$/ })).toHaveCount(markers);
  await expect(editor.getByTestId("slice-count")).toHaveText(`${markers} slices`);
  // `SAMPLER_SLICES_SHOT=<png>`: the Sampler window in slice mode.
  if (process.env.SAMPLER_SLICES_SHOT) await win.screenshot({ path: process.env.SAMPLER_SLICES_SHOT, timeout: 15_000 });

  // --- Pressing a slice plays its key through the track ----------------------------------
  const first = editor.getByRole("button", { name: "play slice 1", exact: true });
  const box = (await first.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height * 0.75);
  await page.mouse.down();
  await expect.poll(() => peakOf(page, midi.id), { timeout: 10_000, intervals: [50] }).toBeGreaterThan(0.01);
  await page.mouse.up();

  // --- Slice to drum rack: one pad per slice, one undo step ------------------------------
  await editor.getByRole("button", { name: "slice to drum rack" }).click();
  await expect.poll(async () => (await onTrack(page, midi.id, "DrumRack")).length).toBe(1);
  expect(await sampler()).toBeUndefined();
  const rack = (await onTrack(page, midi.id, "DrumRack"))[0]!.id;
  const slicePads = Array.from({ length: Math.min(markers, 128 - 36) }, (_, i) => 36 + i);
  await expect.poll(() => padNotes(page, rack)).toEqual(slicePads);
  // Its window opens on the pads.
  const rackWin = page.locator(`[data-device-window="${rack}"]`);
  await expect(rackWin.getByRole("grid", { name: "drum pads" })).toBeVisible();

  const undo = page.getByRole("button", { name: "undo" });
  await undo.click();
  await expect.poll(async () => slicesOf(await sampler())?.markers.length).toBe(markers);
  await expect.poll(async () => (await onTrack(page, midi.id, "DrumRack")).length).toBe(0);
  await page.getByRole("button", { name: "redo" }).click();
  await expect.poll(() => padNotes(page, rack)).toEqual(slicePads);

  // --- Slice to pads: a sample dropped on the rack window's button ------------------------
  // (The undo closed its window, as for any removed device.)
  await openDeviceWindow(page, rack);
  await expect(rackWin.getByRole("grid", { name: "drum pads" })).toBeVisible();
  await files.getByRole("treeitem", { name: "Chords Loop 120.wav", exact: true }).dragTo(rackWin.getByTestId("slice-to-pads"));
  await expect.poll(async () => (await padNotes(page, rack)).length, { timeout: 20_000 }).toBeGreaterThan(slicePads.length);
  const all = await padNotes(page, rack);
  // The new pads start right above the first ones.
  expect(all.slice(0, slicePads.length)).toEqual(slicePads);
  expect(all[slicePads.length]).toBe(slicePads.at(-1)! + 1);
  await undo.click();
  await expect.poll(() => padNotes(page, rack)).toEqual(slicePads);

  expect(errors).toEqual([]);
});
