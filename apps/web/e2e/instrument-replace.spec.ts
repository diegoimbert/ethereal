// Bugfix `instrument-replace`: a new MIDI track comes with the built-in Synth; adding another
// instrument from "Add device" replaces it (Ableton behaviour) instead of putting it in front
// of the Synth, where the trailing Synth (no notes in) cleared its audio: a silent track.
// New track → Add Sampler (a demo sample dropped in) → the chain is just the Sampler → a clip
// plays → the track meter moves. (Poly Synth is still a silent placeholder on dev, pending
// synth-2, so the Sampler carries the audio proof.)
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { addDevice, createTrack, newProject, openDeviceTab, openLibrary, playButton } from "./ui";

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

// As in flow.spec.ts: at 1280 px the transport bar overflows onto its own buttons.
test.use({ viewport: { width: 1600, height: 900 } });

test("adding an instrument to a new MIDI track replaces its Synth, and the track plays", async ({ page }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  // `INSTRUMENT_REPLACE_SHOT=<png>`: also capture the chain after the replacement (dark).
  const shot = process.env.INSTRUMENT_REPLACE_SHOT;
  if (shot) await page.addInitScript(() => localStorage.setItem("eth-theme", "dark"));
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, `Instrument replace ${Date.now()}`);

  const midi = await createTrack(page, "Midi");
  await expect.poll(() => chainOf(page, midi.id)).toEqual(["Synth"]);
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

  await openDeviceTab(page, midi.name);
  await addDevice(page, "Sampler");
  await expect.poll(() => chainOf(page, midi.id)).toEqual(["Sampler"]);
  const chain = page.getByRole("list", { name: `${midi.name} devices` });
  await expect(chain.getByRole("listitem")).toHaveCount(1);

  // A sustained demo sample in the Sampler.
  await openLibrary(page);
  const files = page.getByRole("list", { name: "Files" });
  await files.getByRole("button", { name: "Demo Samples" }).click();
  await files.getByRole("button", { name: "Bass Loop 120.wav", exact: true }).dragTo(chain.getByTestId("sample-slot"));
  await expect(chain.getByTestId("sample-slot")).toContainText("Bass Loop", { timeout: 20_000 });
  if (shot) await chain.screenshot({ path: shot, timeout: 15_000 });

  const transport = page.getByRole("toolbar", { name: "Transport" });
  await transport.getByRole("button", { name: "Loop" }).click();
  await expect.poll(async () => (await doc(page)).settings.loop_enabled).toBe(true);
  await playButton(page).click();
  // Drawn notes are short: sample the meter often (as flow.spec.ts does).
  await expect.poll(() => peakOf(page, midi.id), { timeout: 10_000, intervals: [50] }).toBeGreaterThan(0.02);
  await transport.getByRole("button", { name: "Stop" }).first().click();

  expect(errors).toEqual([]);
});
