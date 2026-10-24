// One selection across automation lanes and tracks, against the real engine:
//
// - a box dragged from one automation lane into another selects the points of both; Delete
//   then deletes all of them, as one undo step;
// - a box dragged from a clip lane down into the track's automation lanes selects the clip
//   and the points; Delete deletes both, as one undo step.
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { AutomationPoint } from "@/generated";
import { clipPart, dragClip, isClipSelected } from "./clips";
import { automationPoint, buildLanesFixture, expectRenderer, openAutomation, type LanesFixture } from "./lanesFixture";
import { doc, setViewport, ulid } from "./perfProject";
import { launch, newProject } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const pointsOf = async (page: Page, lane: string): Promise<AutomationPoint[]> =>
  Object.values((await doc(page)).automation_points)
    .filter((p) => p.lane === lane)
    .sort((a, b) => a.time - b.time);

async function setup(page: Page): Promise<LanesFixture> {
  await launch(page);
  await newProject(page, `Multi-lane ${Date.now()}`);
  const fx = await buildLanesFixture(page);
  await openAutomation(page);
  await setViewport(page, 40, 0);
  const close = page.getByRole("button", { name: /^close / });
  while ((await close.count()) > 0) await close.first().click();
  await expectRenderer(page);
  return fx;
}

async function box(page: Page, from: { x: number; y: number }, to: { x: number; y: number }) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 4 });
  await page.mouse.move(to.x, to.y, { steps: 4 });
  await page.mouse.up();
}

test("a box from one automation lane into another selects both lanes' points; Delete takes them all", async ({ page }) => {
  const fx = await setup(page);
  const volume = page.getByRole("group", { name: "volume automation" });
  const enabled = page.locator(`[data-lane="${fx.enabledLane}"]`);
  // Volume points at 1 and 3, the on/off lane's point at 2 (40 px per beat). The lanes are
  // measured once the points hold still (the lanes open animated).
  const p0 = await automationPoint(page, fx.points[0]!);
  const v = (await volume.boundingBox())!;
  const e = (await enabled.boundingBox())!;
  const pxPerBeat = 40;
  await box(page, { x: p0.x - pxPerBeat / 2, y: v.y + 2 }, { x: p0.x + 2.5 * pxPerBeat, y: e.y + e.height - 2 });
  await page.keyboard.press("Delete");
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).map((p) => p.time)).toEqual([5, 9, 12, 14]);
  await expect.poll(async () => (await pointsOf(page, fx.enabledLane)).map((p) => p.time)).toEqual([6, 10]);
  // One undo step brings back both lanes' points.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).length).toBe(6);
  await expect.poll(async () => (await pointsOf(page, fx.enabledLane)).length).toBe(3);
});

test("a box from a clip lane into the automation lanes selects the clip and the points; Delete takes both", async ({ page }) => {
  const fx = await setup(page);
  const clip = ulid(9, 9);
  const c = await clipPart(page, clip, "box");
  const volume = page.getByRole("group", { name: "volume automation" });
  const v = (await volume.boundingBox())!;
  const p1 = await automationPoint(page, fx.points[1]!);
  // From the clip lane's empty space left of the clip (beat 0.5), down into the volume lane
  // past its point at 3, short of the one at 5.
  await box(page, { x: c.x - 1.5 * 40, y: c.y + c.height / 2 }, { x: p1.x + 20, y: v.y + v.height - 2 });
  await expect.poll(() => isClipSelected(page, clip)).toBe(true);
  await page.keyboard.press("Delete");
  await expect.poll(async () => clip in (await doc(page)).clips).toBe(false);
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).map((p) => p.time)).toEqual([5, 9, 12, 14]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => clip in (await doc(page)).clips).toBe(true);
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).length).toBe(6);
});

test("dragging a clip selected with automation points moves the points along, as one undo step", async ({ page }) => {
  const fx = await setup(page);
  const clip = ulid(9, 9);
  const c = await clipPart(page, clip, "box");
  const volume = page.getByRole("group", { name: "volume automation" });
  const v = (await volume.boundingBox())!;
  const p1 = await automationPoint(page, fx.points[1]!);
  await box(page, { x: c.x - 1.5 * 40, y: c.y + c.height / 2 }, { x: p1.x + 20, y: v.y + v.height - 2 });
  await expect.poll(() => isClipSelected(page, clip)).toBe(true);
  const start = (await doc(page)).clips[clip]!.start;
  // The clip's title bar (the body lets presses through to the lane), one beat to the right.
  await dragClip(page, clip, 40);
  await expect.poll(async () => (await doc(page)).clips[clip]!.start).toBe(start + 1);
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).map((p) => p.time)).toEqual([2, 4, 5, 9, 12, 14]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await doc(page)).clips[clip]!.start).toBe(start);
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).map((p) => p.time)).toEqual([1, 3, 5, 9, 12, 14]);
});
