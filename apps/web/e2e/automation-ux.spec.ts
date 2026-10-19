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
import { automationPoint, expectRenderer } from "./lanesFixture";
import { createTrack, launch, newProject, pickOption, headerButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

test.use({ viewport: { width: 1440, height: 900 } });

interface RowSamples {
  /** On-screen heights, in order: every animation frame and every frame the tween drew. */
  heights: number[];
  /** A tween frame was drawn: the row laid out taller than shown (its slot clipped, the rows below shifted up). */
  tweened: boolean;
  /** Something still clipped / shifted when sampling stopped (the tween didn't clean up). */
  leftover: boolean;
}

/**
 * On-screen heights of a row (from its top to the top of what follows it: rows below slide
 * by a transform) sampled every frame until it stops changing (after `action`), and on every
 * style change the lane animation makes (`laneAnimation.ts` draws each tween frame by
 * styles), so the frames it drew are seen however slow the machine is.
 */
async function sampleRowHeights(page: Page, track: string, action: () => Promise<void>): Promise<RowSamples> {
  await page.evaluate((id) => {
    const w = window as unknown as { __rows: RowSamples; __sampling: boolean };
    const s: RowSamples = { heights: [], tweened: false, leftover: false };
    w.__rows = s;
    w.__sampling = true;
    const row = () => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${id}"]`);
    const next = () => row()?.nextElementSibling as HTMLElement | null;
    const slot = () => row()?.querySelector<HTMLElement>(":scope > .eth-arr-row__automation") ?? null;
    const drawn = () => Boolean(next()?.style.transform) || slot()?.style.overflow === "hidden";
    const sample = () => s.heights.push((next()?.getBoundingClientRect().top ?? 0) - (row()?.getBoundingClientRect().top ?? 0));
    const observer = new MutationObserver(() => {
      if (drawn()) s.tweened = true;
      sample();
    });
    observer.observe(row()!.parentElement!, { attributes: true, attributeFilter: ["style"], subtree: true });
    let still = 0;
    const tick = () => {
      const last = s.heights[s.heights.length - 1];
      sample();
      still = last === s.heights[s.heights.length - 1] ? still + 1 : 0;
      // Stop after ~0.5 s without change once it has moved.
      if (still > 30 && s.heights.some((x) => x !== s.heights[0])) {
        observer.disconnect();
        s.leftover = drawn();
        w.__sampling = false;
      } else requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, track);
  await action();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __sampling: boolean }).__sampling), { timeout: 10_000 }).toBe(false);
  return page.evaluate(() => (window as unknown as { __rows: RowSamples }).__rows);
}

test("automation lanes animate open, snap stepped params, and copy/paste points @frames", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await newProject(page, `Automation ${Date.now()}`);
  const midi = await createTrack(page, "Midi");
  const synth = Object.values((await doc(page)).devices).find((d) => d.track === midi.id)!;
  // Both arranger renderers (the `webgl` project: points drawn on the canvas, no elements).
  await expectRenderer(page);

  // --- Open: the row grows smoothly to lane + bar + one lane -----------------------------
  const laneHeight = await page.locator(`.eth-arr-row[data-track="${midi.id}"] .eth-arr-row__main`).evaluate((el) => el.getBoundingClientRect().height);
  const { heights, tweened, leftover } = await sampleRowHeights(page, midi.id, async () =>
    (await headerButton(page, midi.name, `Show automation of ${midi.name}`)).click(),
  );
  const expected = laneHeight + 20 + 64;
  // Animated, not a jump: the row was drawn by the tween (laid out at its new height, shown
  // shorter). How many in-between frames there are depends on the machine's frame rate (a
  // loaded machine may draw one or none in the 300 ms): the frame rate is the perf spec's
  // (automation-ux.perf.spec.ts), not this one's.
  expect(tweened).toBe(true);
  // ...grew monotonically from the closed height, ended at the open one, and cleaned up.
  expect(heights[0]).toBeCloseTo(laneHeight, 0);
  expect(heights[heights.length - 1]).toBeCloseTo(expected, 0);
  for (let i = 1; i < heights.length; i++) expect(heights[i]!).toBeGreaterThanOrEqual(heights[i - 1]! - 0.01);
  expect(leftover).toBe(false);

  // A click on the lane works once open: double-click adds a point.
  const volume = page.getByRole("group", { name: "volume automation" });
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
  await page.getByRole("menuitem", { name: "paste here" }).click();
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
  await pickOption(page, "show parameter", { value: `param:${synth.id}:1` });
  const transpose = page.getByRole("group", { name: /transpose automation/ });
  const tSvg = transpose.getByTestId("automation-lane-svg");
  await expect(tSvg).toBeVisible();
  const tBox = (await tSvg.boundingBox())!;
  // Step gridlines (counted on the lane: the WebGL renderer draws them on its canvas).
  await expect(tSvg).not.toHaveAttribute("data-steps", "0");
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
  const tLaneId = (await tLane())!.id;
  const tPoint = Object.values((await doc(page)).automation_points).find((x) => x.lane === tLaneId)!;
  const c = await automationPoint(page, tPoint.id);
  await page.mouse.move(c.x, c.y);
  await page.mouse.down();
  await page.mouse.move(c.x + 1, c.y - 24, { steps: 5 });
  await expect(transpose.getByTestId("automation-drag-tip")).toContainText("st");
  await page.mouse.up();
  await expect.poll(semis).toBeCloseTo(before + 3, 9);

  // --- Set Value…: a small popover at the point (not a modal); Enter sets, Esc cancels ---
  // On a volume point (its lane shows the whole range, so the point stays on screen).
  const vPoint = pts[0]!;
  const value = async () => (await doc(page)).automation_points[vPoint.id]!.value;
  const setValue = async () => {
    const at = await automationPoint(page, vPoint.id);
    await page.mouse.click(at.x, at.y, { button: "right" });
    await page.getByRole("menuitem", { name: "set value…" }).click();
    const pop = page.getByRole("dialog", { name: "set Volume" });
    await expect(pop).toBeVisible();
    await expect(pop).toHaveClass(/eth-popover/);
    const pb = (await pop.boundingBox())!;
    // Next to the point, inside the window.
    expect(Math.hypot(pb.x - at.x, pb.y - at.y)).toBeLessThan(40);
    expect(pb.x + pb.width).toBeLessThanOrEqual(1440);
    expect(pb.y + pb.height).toBeLessThanOrEqual(900);
    const field = pop.getByRole("textbox", { name: "volume value" });
    await expect(field).toBeFocused();
    return { pop, field };
  };
  {
    const { pop, field } = await setValue();
    await expect(field).toHaveValue(/^-?\d+(\.\d+)?$/);
    await expect(pop).toContainText("db");
    // The text is selected: typing replaces it.
    await page.keyboard.type("-3 db");
    await page.keyboard.press("Enter");
    await expect(pop).toBeHidden();
    await expect.poll(value).not.toBeCloseTo(vPoint.value, 6);
  }
  const set = await value();
  {
    const { pop, field } = await setValue();
    await expect(field).toHaveValue("-3");
    await page.keyboard.type("-20");
    await page.keyboard.press("Escape");
    await expect(pop).toBeHidden();
    expect(await value()).toBe(set);
  }

  expect(errors).toEqual([]);
});
