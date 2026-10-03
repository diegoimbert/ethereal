// automation-ux: automation lanes on the web build, through the UI, against the real engine.
//
// - opening a track's automation animates the row height (layout, not a clip): it grows
//   monotonically and ends at the expected height, and a click on the lane then works;
// - a stepped param (the synth's Transpose, semitones) snaps to whole steps;
// - copy/paste of points (keyboard and the lane's "Paste Here" menu);
// - 64 tracks: the open/close animation keeps a steady frame rate (automation-ux.perf.spec.ts).
//
// No sleeps: every step waits on UI or engine state. The UI mirror is `window.__ether`.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, newProject, pickOption } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

test.use({ viewport: { width: 1440, height: 900 } });

/**
 * On-screen heights of a row (from its top to the top of what follows it: rows below slide
 * by a transform) sampled every frame until it stops changing (after `action`).
 */
async function sampleRowHeights(page: Page, track: string, action: () => Promise<void>): Promise<number[]> {
  await page.evaluate((id) => {
    const w = window as unknown as { __rowHeights: number[]; __sampling: boolean };
    w.__rowHeights = [];
    w.__sampling = true;
    const row = () => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${id}"]`);
    const next = () => row()?.nextElementSibling as HTMLElement | null;
    let still = 0;
    const tick = () => {
      const h = (next()?.getBoundingClientRect().top ?? 0) - (row()?.getBoundingClientRect().top ?? 0);
      const last = w.__rowHeights[w.__rowHeights.length - 1];
      w.__rowHeights.push(h);
      still = last === h ? still + 1 : 0;
      // Stop after ~0.5 s without change once it has moved.
      if (still > 30 && w.__rowHeights.some((x) => x !== w.__rowHeights[0])) w.__sampling = false;
      else requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, track);
  await action();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __sampling: boolean }).__sampling), { timeout: 10_000 }).toBe(false);
  return page.evaluate(() => (window as unknown as { __rowHeights: number[] }).__rowHeights);
}

test("automation lanes animate open, snap stepped params, and copy/paste points", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await newProject(page, `Automation ${Date.now()}`);
  const midi = await createTrack(page, "Midi");
  const synth = Object.values((await doc(page)).devices).find((d) => d.track === midi.id)!;

  // --- Open: the row grows smoothly to lane + bar + one lane -----------------------------
  const laneHeight = await page.locator(`.eth-arr-row[data-track="${midi.id}"] .eth-arr-row__main`).evaluate((el) => el.getBoundingClientRect().height);
  const heights = await sampleRowHeights(page, midi.id, () => page.getByRole("button", { name: `Show automation of ${midi.name}` }).click());
  const expected = laneHeight + 20 + 64;
  expect(heights[heights.length - 1]).toBeCloseTo(expected, 0);
  const moving = heights.filter((h) => h > laneHeight + 0.5 && h < expected - 0.5);
  expect(moving.length).toBeGreaterThanOrEqual(3); // in-between frames: animated, not a jump
  for (let i = 1; i < heights.length; i++) expect(heights[i]!).toBeGreaterThanOrEqual(heights[i - 1]! - 0.01);

  // A click on the lane works once open: double-click adds a point.
  const volume = page.getByRole("group", { name: "Volume automation" });
  const volumeSvg = volume.getByTestId("automation-lane-svg");
  const box = (await volumeSvg.boundingBox())!;
  expect(box.height).toBeCloseTo(64, 0);
  await volumeSvg.dblclick({ position: { x: 40, y: box.height * 0.3 } });
  await volumeSvg.dblclick({ position: { x: 160, y: box.height * 0.7 } });
  await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBe(2);

  // --- Copy both points, paste them at a later time (lane menu "Paste Here") ------------
  await volume.focus();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.press("ControlOrMeta+C");
  await volumeSvg.click({ button: "right", position: { x: 400, y: box.height / 2 } });
  await page.getByRole("menuitem", { name: "Paste Here" }).click();
  await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBe(4);
  const vol = Object.values((await doc(page)).automation_lanes).find((l) => l.target.type === "TrackVolume")!;
  const pts = Object.values((await doc(page)).automation_points)
    .filter((p) => p.lane === vol.id)
    .sort((a, b) => a.time - b.time);
  // Same shape, shifted in time.
  expect(pts[2]!.value).toBeCloseTo(pts[0]!.value, 9);
  expect(pts[3]!.value).toBeCloseTo(pts[1]!.value, 9);
  expect(pts[3]!.time - pts[2]!.time).toBeCloseTo(pts[1]!.time - pts[0]!.time, 9);

  // --- Transpose (semitones) snaps to whole steps -----------------------------------------
  await pickOption(page, "Show parameter", { value: `param:${synth.id}:1` });
  const transpose = page.getByRole("group", { name: /Transpose automation/ });
  const tSvg = transpose.getByTestId("automation-lane-svg");
  await expect(tSvg).toBeVisible();
  const tBox = (await tSvg.boundingBox())!;
  await expect(tSvg.locator(".eth-auto-lane__step").first()).toBeAttached();
  await tSvg.dblclick({ position: { x: 80, y: tBox.height * 0.37 } });
  const tLane = async () => Object.values((await doc(page)).automation_lanes).find((l) => l.target.type === "DeviceParam");
  await expect.poll(async () => (await tLane()) !== undefined).toBe(true);
  const semis = async () => {
    const lane = (await tLane())!;
    const p = Object.values((await doc(page)).automation_points).find((x) => x.lane === lane.id)!;
    return p.value * 48 - 24; // Transpose: -24..24 st, linear
  };
  await expect.poll(semis).toBeCloseTo(Math.round(await semis()), 9);
  const before = Math.round(await semis());
  // A coarse 24 px drag up moves exactly 3 st (8 px per step at the default lane height).
  const circle = transpose.locator("circle[data-point]").first();
  const c = (await circle.boundingBox())!;
  await page.mouse.move(c.x + c.width / 2, c.y + c.height / 2);
  await page.mouse.down();
  await page.mouse.move(c.x + c.width / 2 + 1, c.y + c.height / 2 - 24, { steps: 5 });
  await expect(transpose.getByTestId("automation-drag-tip")).toContainText("st");
  await page.mouse.up();
  await expect.poll(semis).toBeCloseTo(before + 3, 9);

  expect(errors).toEqual([]);
});
