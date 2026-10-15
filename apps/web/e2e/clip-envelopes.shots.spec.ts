// Screenshots of clip envelopes for the PR (not part of the regular suite):
// `CLIP_ENVELOPE_SHOTS=<dir> npx playwright test clip-envelopes.shots`.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, expandFolder, launch, newProject, openLibrary, pickOption } from "./ui";

const dir = process.env.CLIP_ENVELOPE_SHOTS;
test.skip(!dir, "set CLIP_ENVELOPE_SHOTS=<dir> to capture");
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}
const doc = async (page: Page): Promise<Project> => (await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project))!;

const shot = (page: Page, name: string) => page.screenshot({ path: `${dir}/${name}.png` });

/** Double-clicks points into a lane at (x, y) fractions of its box. */
async function draw(page: Page, lane: ReturnType<Page["getByTestId"]>, at: Array<[number, number]>) {
  const box = (await lane.boundingBox())!;
  for (const [fx, fy] of at) {
    const before = Object.keys((await doc(page)).automation_points).length;
    await lane.dblclick({ position: { x: box.width * fx, y: box.height * fy } });
    await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBe(before + 1);
  }
}

test("clip envelopes: piano roll and warp editor", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page);
  await newProject(page, "Clip envelopes");

  // --- MIDI clip: a Volume envelope under the notes --------------------------------------------
  const midi = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${midi.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  await openClip(page, clip.id);
  const roll = page.getByTestId("piano-roll");
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  for (const [x, y] of [
    [30, 180],
    [190, 200],
    [350, 160],
    [510, 220],
  ] as const)
    await grid.dblclick({ position: { x, y } });

  const picker = roll.getByRole("combobox", { name: "lane" });
  await picker.click();
  await shot(page, "01-piano-roll-picker");
  await page.keyboard.press("Escape");
  await pickOption(roll, "lane", { value: `env:volume:${midi.id}` });
  const svg = roll.getByTestId("clip-envelope").getByTestId("automation-lane-svg");
  await draw(page, svg, [
    [0.01, 0.2],
    [0.12, 0.8],
    [0.25, 0.3],
    [0.38, 0.6],
  ]);
  await shot(page, "02-piano-roll-volume-envelope");

  // --- Audio clip: the warp editor's envelope strip -------------------------------------------
  const audio = await createTrack(page, "Audio");
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  await page
    .getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true })
    .dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length, { timeout: 20_000 }).toBe(1);
  const audioClip = Object.values((await doc(page)).clips).find((c) => c.track === audio.id)!;
  await openClip(page, audioClip.id);
  const strip = page.getByTestId("warp-editor").getByTestId("clip-envelopes");
  await expect(strip).toBeVisible();
  await draw(page, strip.getByTestId("automation-lane-svg"), [
    [0.02, 0.1],
    [0.3, 0.9],
    [0.55, 0.4],
    [0.8, 0.15],
  ]);
  await shot(page, "03-warp-editor-envelope");

  // Transposition (the clip's own, like Live's sample envelope): a few semitone steps.
  await pickOption(strip, "clip envelope", { value: "env:clip-transpose" });
  await draw(page, strip.getByTestId("automation-lane-svg"), [
    [0.02, 0.5],
    [0.25, 0.2],
    [0.5, 0.8],
    [0.75, 0.5],
  ]);
  await shot(page, "04-warp-editor-transposition");
});
