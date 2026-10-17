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
import { doc, sendBatched, setViewport, ulid } from "./perfProject";
import { launch, newProject } from "./ui";
import { evaluatePoints } from "@/features/automation/curve";

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

test("an empty lane draws the current value dashed; the first point takes it", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page);
  await newProject(page, `Baseline ${Date.now()}`);
  const fx = await buildLanesFixture(page);
  // Pan 50 % left, and its lane without points (shown with the track's lanes).
  const panLane = ulid(0, 7);
  await sendBatched(page, [
    { domain: "Mixer", command: { type: "SetPan", track: fx.auto, pan: -0.5 } },
    {
      domain: "Automation",
      command: { type: "CreateLane", id: panLane, owner: { type: "Track", track: fx.auto }, target: { type: "TrackPan", track: fx.auto } },
    },
  ]);
  await openAutomation(page);
  await setViewport(page, 40, 0);
  const close = page.getByRole("button", { name: /^close / });
  while ((await close.count()) > 0) await close.first().click();
  await expectRenderer(page);

  const pan = page.getByRole("group", { name: "pan automation" });
  const svg = pan.getByTestId("automation-lane-svg");
  // Bipolar: 50 % left is a quarter of the way up.
  await expect(svg).toHaveAttribute("data-baseline", "0.25");
  // Dashes on the canvas: 4 px on, 4 px off from beat 0 (the view's left edge here).
  await page.mouse.move(2, 2);
  await expect
    .poll(async () => {
      // Measured each time: the lanes settle after opening.
      const box = (await svg.boundingBox())!;
      const lineY = box.y + 5 + 0.75 * (box.height - 10);
      const [dash, gap, bg] = await Promise.all([
        pixelAt(page, box.x + 200 + 2, lineY),
        pixelAt(page, box.x + 200 + 6, lineY),
        pixelAt(page, box.x + 200 + 2, lineY + 12),
      ]);
      const diff = (a: number[], b: number[]) => Math.max(...a.map((v, i) => Math.abs(v - b[i]!)));
      return diff(dash, bg) > 30 && diff(gap, bg) < 16;
    })
    .toBe(true);

  const box = (await svg.boundingBox())!;
  // Double-click far from the line: the point takes the current value, not the pointer's.
  await page.mouse.dblclick(box.x + 300, box.y + 8);
  await expect.poll(async () => (await pointsOf(page, panLane)).length).toBe(1);
  expect((await pointsOf(page, panLane))[0]!.value).toBeCloseTo(0.25, 9);
  // With a point, no baseline.
  await expect(svg).not.toHaveAttribute("data-baseline");
  expect(errors).toEqual([]);
});

test("a point added between two takes the lane's value there; held, it drags like a grabbed point (no value swap)", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page);
  await newProject(page, `Insert ${Date.now()}`);
  const fx = await buildLanesFixture(page);
  // A pan lane: 0.2 at beat 0, 0.8 at beat 8 (linear), 0.2 at beat 12.
  const panLane = ulid(0, 8);
  const ids = [ulid(3, 0), ulid(3, 1), ulid(3, 2)];
  await sendBatched(page, [
    {
      domain: "Automation",
      command: { type: "CreateLane", id: panLane, owner: { type: "Track", track: fx.auto }, target: { type: "TrackPan", track: fx.auto } },
    },
    {
      domain: "Automation",
      command: {
        type: "AddPoints",
        lane: panLane,
        points: [
          { id: ids[0]!, time: 0, value: 0.2, curve: { type: "Linear" } },
          { id: ids[1]!, time: 8, value: 0.8, curve: { type: "Linear" } },
          { id: ids[2]!, time: 12, value: 0.2, curve: { type: "Linear" } },
        ],
      },
    },
  ]);
  await openAutomation(page);
  await setViewport(page, 40, 0);
  const close = page.getByRole("button", { name: /^close / });
  while ((await close.count()) > 0) await close.first().click();
  await expectRenderer(page);
  const svg = page.getByRole("group", { name: "pan automation" }).getByTestId("automation-lane-svg");
  const original = await pointsOf(page, panLane);
  const neighboursUnchanged = async () => {
    const now = await pointsOf(page, panLane);
    for (const o of original) expect(now.find((p) => p.id === o.id)).toMatchObject({ time: o.time, value: o.value, curve: o.curve });
  };
  const added = async () => (await pointsOf(page, panLane)).find((p) => !ids.includes(p.id));
  const undo = () => page.keyboard.press("ControlOrMeta+z");

  // Screen x of a beat, from the points (40 px per beat).
  const p0 = await automationPoint(page, ids[0]!);
  const xAt = (beat: number) => p0.x + beat * 40;
  const box = (await svg.boundingBox())!;
  // Double-click mid-segment (beat 4) far from the line: the point lands on it (0.5).
  await page.mouse.dblclick(xAt(4), box.y + 8);
  await expect.poll(async () => (await pointsOf(page, panLane)).length).toBe(4);
  const p = (await added())!;
  expect(p.time).toBeCloseTo(4, 1);
  expect(p.value).toBeCloseTo(0.2 + 0.075 * p.time, 9);
  if (p.time === 4) expect(p.value).toBeCloseTo(0.5, 9);
  await neighboursUnchanged();
  await undo();
  await expect.poll(async () => (await pointsOf(page, panLane)).length).toBe(3);

  // Double-click at beat 2 and keep holding: drag right onto the neighbour's beat 8, then
  // past it to beat 10. The neighbours keep their values (no swap).
  const at = { x: xAt(2), y: box.y + box.height / 2 };
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await page.mouse.up();
  await page.mouse.down({ clickCount: 2 });
  await expect.poll(async () => (await pointsOf(page, panLane)).length).toBe(4);
  const createdValue = (await added())!.value;
  expect(createdValue).toBeCloseTo(0.2 + 0.075 * (await added())!.time, 9);
  await page.mouse.move(xAt(8), at.y, { steps: 6 });
  await expect.poll(async () => (await added())!.time).toBeCloseTo(8, 4);
  expect((await added())!.time).toBeLessThan(8);
  await neighboursUnchanged();
  await page.mouse.move(xAt(10), at.y, { steps: 4 });
  await page.mouse.up();
  await expect.poll(async () => (await added())!.time).toBeCloseTo(10, 1);
  // A level drag keeps its exact value.
  expect((await added())!.value).toBe(createdValue);
  await neighboursUnchanged();
  const held = (await pointsOf(page, panLane)).map((q) => [q.time, q.value]);

  // Release then grab: the same moves give the same lane.
  await undo();
  await expect.poll(async () => (await pointsOf(page, panLane)).length).toBe(3);
  await page.mouse.dblclick(at.x, at.y);
  await expect.poll(async () => (await pointsOf(page, panLane)).length).toBe(4);
  const grab = await automationPoint(page, (await added())!.id);
  await page.mouse.move(grab.x, grab.y);
  await page.mouse.down();
  await page.mouse.move(xAt(8), grab.y, { steps: 6 });
  await page.mouse.move(xAt(10), grab.y, { steps: 4 });
  await page.mouse.up();
  await expect.poll(async () => (await added())!.time).toBeCloseTo(10, 1);
  expect((await pointsOf(page, panLane)).map((q) => [q.time, q.value])).toEqual(held);
  await neighboursUnchanged();
  expect(errors).toEqual([]);
});

test("⇧-drag on the line inside a section moves only that section", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page);
  await newProject(page, `Section ${Date.now()}`);
  const fx = await buildLanesFixture(page);
  // A pan lane: 0.2 → 0.8 over 4 bars.
  const panLane = ulid(0, 9);
  const ids = [ulid(4, 0), ulid(4, 1)];
  await sendBatched(page, [
    {
      domain: "Automation",
      command: { type: "CreateLane", id: panLane, owner: { type: "Track", track: fx.auto }, target: { type: "TrackPan", track: fx.auto } },
    },
    {
      domain: "Automation",
      command: {
        type: "AddPoints",
        lane: panLane,
        points: [
          { id: ids[0]!, time: 0, value: 0.2, curve: { type: "Linear" } },
          { id: ids[1]!, time: 16, value: 0.8, curve: { type: "Linear" } },
        ],
      },
    },
  ]);
  await openAutomation(page);
  await setViewport(page, 40, 0);
  const close = page.getByRole("button", { name: /^close / });
  while ((await close.count()) > 0) await close.first().click();
  await expectRenderer(page);
  const pan = page.getByRole("group", { name: "pan automation" });
  const svg = pan.getByTestId("automation-lane-svg");
  const p0 = await automationPoint(page, ids[0]!);
  const xAt = (beat: number) => p0.x + beat * 40;
  const box = (await svg.boundingBox())!;
  const yOf = (v: number) => box.y + 5 + (1 - v) * (box.height - 10);
  /** The lane's value at `beat`, in engine order (time, then id). */
  const valueAt = async (beat: number) =>
    evaluatePoints(
      (await pointsOf(page, panLane)).sort((a, b) => a.time - b.time || (a.id < b.id ? -1 : 1)),
      beat,
    )!;

  // Select bars 2–3 (beats 4–12): a ⇧-drag away from the line.
  await page.keyboard.down("Shift");
  await page.mouse.move(xAt(4), yOf(0.97));
  await page.mouse.down();
  await page.mouse.move(xAt(8), yOf(0.97), { steps: 3 });
  await page.mouse.move(xAt(12), yOf(0.97), { steps: 3 });
  await page.mouse.up();
  await expect(pan.getByTestId("automation-range")).toHaveAttribute("data-start", "4");
  await expect(pan.getByTestId("automation-range")).toHaveAttribute("data-end", "12");
  const outside = [await valueAt(2), await valueAt(14)];

  // ⇧-drag the line at beat 8 (0.5) up by 12 px.
  await page.mouse.move(xAt(8), yOf(0.5));
  await page.mouse.down();
  await page.mouse.move(xAt(8), yOf(0.5) - 4, { steps: 2 });
  await page.mouse.move(xAt(8), yOf(0.5) - 12, { steps: 3 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  await expect.poll(async () => (await pointsOf(page, panLane)).length).toBe(6);
  const dv = 12 / (box.height - 10);
  expect(await valueAt(8)).toBeCloseTo(0.5 + dv, 2);
  expect(await valueAt(6)).toBeCloseTo(0.425 + dv, 2);
  // Bars 1 and 4 and the original points: unchanged.
  expect([await valueAt(2), await valueAt(14)]).toEqual(outside);
  const pts = await pointsOf(page, panLane);
  expect(pts.find((p) => p.id === ids[0])).toMatchObject({ time: 0, value: 0.2 });
  expect(pts.find((p) => p.id === ids[1])).toMatchObject({ time: 16, value: 0.8 });
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
