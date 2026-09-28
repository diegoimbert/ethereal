// Bugfix `instrument-replace`: a new MIDI track comes with the built-in Synth; adding another
// instrument from "Add device" replaces it (Ableton behaviour) instead of putting it in front
// of the Synth, where the trailing Synth (no notes in) cleared its audio: a silent track.
// New track → Add Poly Synth → the chain is just the Poly Synth → a clip plays → meter moves.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { addDevice, createTrack, newProject, openDeviceTab, playButton } from "./ui";

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

const chainOf = async (page: Page, track: string) =>
  Object.values((await doc(page)).devices)
    .filter((d) => d.track === track && d.pad === null && !d.chain)
    .sort((a, b) => (a.order < b.order ? -1 : 1))
    .map((d) => (d.kind.type === "Builtin" ? d.kind.device.type : d.kind.type));

test("adding an instrument to a new MIDI track replaces its Synth, and the track plays", async ({ page }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, `Instrument replace ${Date.now()}`);

  const midi = await createTrack(page, "Midi");
  await expect.poll(() => chainOf(page, midi.id)).toEqual(["Synth"]);
  await openDeviceTab(page, midi.name);
  await addDevice(page, "PolySynth");
  await expect.poll(() => chainOf(page, midi.id)).toEqual(["PolySynth"]);

  // A one-bar clip with a few notes, drawn in the piano roll.
  await page.locator(`[data-lane="${midi.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(1);
  await openClip(page, Object.values((await doc(page)).clips)[0]!.id);
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  const gridBox = (await grid.boundingBox())!;
  const clipWidth = (await page.getByTestId("piano-roll-clip-end").boundingBox())!.x - gridBox.x;
  for (const fx of [0.02, 0.27, 0.52, 0.77]) {
    await grid.dblclick({ position: { x: clipWidth * fx + 2, y: gridBox.height * 0.45 } });
  }
  await expect.poll(async () => Object.keys((await doc(page)).notes).length).toBe(4);

  await playButton(page).click();
  await expect.poll(() => peakOf(page, midi.id), { timeout: 10_000 }).toBeGreaterThan(0.02);
  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop" }).first().click();

  expect(errors).toEqual([]);
});
