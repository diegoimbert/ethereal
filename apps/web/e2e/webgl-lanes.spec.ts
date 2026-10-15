// The arranger's lane content besides clips (automation lanes, group summaries), on the
// backend the project runs (`webgl`, or the `canvas2d` fallback): lane layers draw on the
// arranger's canvas, no element per point. Gestures aim through the arranger test API
// (`window.__ether.arrangement.automationPoint`). A last test checks that the Canvas2D
// fallback's pixels are close to WebGL's on the same lanes.
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { AutomationPoint } from "@/generated";
import { pixelAt, setArrangerRenderer } from "./clips";
import { automationPoint, buildLanesFixture, expectRenderer, openAutomation, type LanesFixture } from "./lanesFixture";
import { doc, setViewport } from "./perfProject";
import { launch, newProject } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const pointsOf = async (page: Page, lane: string): Promise<AutomationPoint[]> =>
  Object.values((await doc(page)).automation_points)
    .filter((p) => p.lane === lane)
    .sort((a, b) => a.time - b.time);

async function setup(page: Page): Promise<LanesFixture> {
  await launch(page);
  await newProject(page, `Lanes ${Date.now()}`);
  const fx = await buildLanesFixture(page);
  await openAutomation(page);
  await setViewport(page, 40, 0);
  // The floating panes (editor, inspector) would cover the arranger.
  const close = page.getByRole("button", { name: /^close / });
  while ((await close.count()) > 0) await close.first().click();
  return fx;
}

test("automation points: select, multi-select, drag, bend, menu, delete; group summary", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const fx = await setup(page);
  await expectRenderer(page);
  const volume = page.getByRole("group", { name: "volume automation" });
  const curveSelect = page.getByRole("combobox", { name: "curve of selected points" }).first();

  // Click selects (the lane's curve menu enables), shift-click adds.
  const before = await pointsOf(page, fx.volumeLane);
  let p0 = await automationPoint(page, fx.points[0]!);
  await page.mouse.click(p0.x, p0.y);
  await expect(curveSelect).toBeEnabled();
  const p1 = await automationPoint(page, fx.points[1]!);
  await page.keyboard.down("Shift");
  await page.mouse.click(p1.x, p1.y);
  await page.keyboard.up("Shift");

  // Drag both one beat right (40 px per beat, on the grid).
  p0 = await automationPoint(page, fx.points[0]!);
  await page.mouse.move(p0.x, p0.y);
  await page.mouse.down();
  await page.mouse.move(p0.x + 20, p0.y, { steps: 3 });
  await page.mouse.move(p0.x + 40, p0.y, { steps: 3 });
  await expect(volume.getByTestId("automation-drag-tip")).toBeVisible();
  await page.mouse.up();
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).find((p) => p.id === fx.points[0])!.time).toBeCloseTo(before[0]!.time + 1, 6);
  expect((await pointsOf(page, fx.volumeLane)).find((p) => p.id === fx.points[1])!.time).toBeCloseTo(before[1]!.time + 1, 6);

  // Alt-drag the segment after the last point but one (linear) bends it.
  const a = await automationPoint(page, fx.points[4]!);
  const b = await automationPoint(page, fx.points[5]!);
  await page.keyboard.down("Alt");
  await page.mouse.move((a.x + b.x) / 2, (a.y + b.y) / 2);
  await page.mouse.down();
  await page.mouse.move((a.x + b.x) / 2, (a.y + b.y) / 2 - 20, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up("Alt");
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).find((p) => p.id === fx.points[4])!.curve.type).toBe("Curve");

  // A point's menu, then double-click deletes.
  const p3 = await automationPoint(page, fx.points[3]!);
  await page.mouse.click(p3.x, p3.y, { button: "right" });
  await expect(page.getByRole("menuitem", { name: "delete point" })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.mouse.dblclick(p3.x, p3.y);
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).some((p) => p.id === fx.points[3])).toBe(false);
  // Double-click on empty lane space adds one. (Left of the inspector: the first press of a
  // lane click selects the track, which shows it in the inspector over the lanes' right part.)
  const box = (await volume.getByTestId("automation-lane-svg").boundingBox())!;
  await page.mouse.dblclick(box.x + 16 * 40 + 10, box.y + box.height * 0.85);
  await expect.poll(async () => (await pointsOf(page, fx.volumeLane)).length).toBe(before.length);

  // The stepped DeviceEnabled lane (a switch, shown with the track's lanes): its points
  // are there to grab too.
  await expect(page.getByRole("group", { name: "synth: Device On automation" })).toBeVisible();
  const on = (await pointsOf(page, fx.enabledLane))[1]!;
  const onAt = await automationPoint(page, on.id);
  const hit = await page.evaluate(
    ([x, y]) => (window as unknown as { __ether: { arrangement: { hitTest(x: number, y: number): unknown } } }).__ether.arrangement.hitTest(x!, y!),
    [onAt.x, onAt.y],
  );
  expect(hit).toMatchObject({ part: "layer", layer: "automation", data: { point: on.id } });

  // Group summary: three merged bars (0–8, 12–15, 20–21).
  await expect
    .poll(() =>
      page.evaluate(
        (g) => (window as unknown as { __ether: { arrangement: { groupSummaryBars(t: string): number } } }).__ether.arrangement.groupSummaryBars(g),
        fx.group,
      ),
    )
    .toBe(3);

  expect(errors).toEqual([]);
});

test("automation and group lanes: Canvas2D pixels close to WebGL's", async ({ page }) => {
  test.skip(test.info().project.name === "canvas2d", "compares both backends in one run");
  const fx = await setup(page);
  const samples: Array<{ name: string; at: () => Promise<{ x: number; y: number }> }> = [
    { name: "point rim", at: async () => ({ ...(await automationPoint(page, fx.points[0]!)), x: (await automationPoint(page, fx.points[0]!)).x + 4 }) },
    { name: "point fill", at: () => automationPoint(page, fx.points[0]!) },
    { name: "hold line", at: async () => ({ ...(await automationPoint(page, fx.points[1]!)), x: (await automationPoint(page, fx.points[1]!)).x + 20 }) },
    { name: "lane background", at: async () => ({ ...(await automationPoint(page, fx.points[1]!)), y: (await automationPoint(page, fx.points[1]!)).y + 20 }) },
  ];
  const read = async () => {
    const out: Record<string, [number, number, number]> = {};
    for (const s of samples) {
      const p = await s.at();
      out[s.name] = await pixelAt(page, p.x, p.y);
    }
    return out;
  };
  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    const got: Record<string, Record<string, [number, number, number]>> = {};
    for (const renderer of ["webgl", "canvas2d"] as const) {
      await setArrangerRenderer(page, renderer);
      await page.mouse.move(2, 2);
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
      got[renderer] = await read();
    }
    await setArrangerRenderer(page, "webgl");
    for (const s of samples) {
      const [c, w] = [got.canvas2d![s.name]!, got.webgl![s.name]!];
      const d = Math.max(...c.map((v, i) => Math.abs(v - w[i]!)));
      expect(d, `${theme} ${s.name}: canvas2d ${c} vs webgl ${w}`).toBeLessThanOrEqual(24);
    }
  }
});
