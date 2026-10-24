// Automation draw mode against the real engine: B in a focused lane (or the bar's "draw")
// turns it on; a drag then draws steps over the lane, as one undo step.
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { AutomationPoint } from "@/generated";
import { automationPoint, buildLanesFixture, expectRenderer, openAutomation } from "./lanesFixture";
import { doc, setViewport } from "./perfProject";
import { launch, newProject } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const pointsOf = async (page: Page, lane: string): Promise<AutomationPoint[]> =>
  Object.values((await doc(page)).automation_points)
    .filter((p) => p.lane === lane)
    .sort((a, b) => a.time - b.time);

test("draw mode: a drag draws steps over the lane, one undo step", async ({ page }) => {
  await launch(page);
  await newProject(page, `Draw ${Date.now()}`);
  const fx = await buildLanesFixture(page);
  await openAutomation(page);
  await setViewport(page, 40, 0);
  await expectRenderer(page);
  const before = await pointsOf(page, fx.volumeLane);
  // Volume points at 1, 3, 5, 9, 12, 14 (40 px per beat): draw low over beats 6 .. 8.
  const p0 = await automationPoint(page, fx.points[0]!);
  const volume = page.getByRole("group", { name: "volume automation" });
  const v = (await volume.boundingBox())!;
  await volume.focus();
  await page.keyboard.press("b");
  await expect(page.getByTestId("automation-draw-mode").first()).toHaveAttribute("aria-pressed", "true");
  const x = (beat: number) => p0.x + (beat - 1) * 40;
  const low = v.y + v.height - 8;
  await page.mouse.move(x(6.1), low);
  await page.mouse.down();
  await page.mouse.move(x(7.9), low, { steps: 6 });
  await page.mouse.up();
  await expect
    .poll(async () => (await pointsOf(page, fx.volumeLane)).filter((p) => p.time >= 6 && p.time < 8 && p.curve.type === "Step").length)
    .toBeGreaterThan(0);
  const after = await pointsOf(page, fx.volumeLane);
  // The points outside the drawn cells stay.
  for (const t of [1, 3, 5, 12, 14]) expect(after.some((p) => p.time === t)).toBe(true);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).map((p) => [p.time, p.value])).toEqual(before.map((p) => [p.time, p.value]));
});
