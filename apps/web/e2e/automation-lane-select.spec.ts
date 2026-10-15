// Automation lanes take the arrangement's usual clicks (owner request), against the real
// engine, on both arranger renderers (listed in ARRANGER_SPECS):
//
// - a click on a lane's empty space selects the track and places the insert marker there;
//   so does a click on the "+ Parameter…" bar's empty space;
// - like the arranger (#621): a ⇧-drag selects a time range in the lane; ⌘/Ctrl+D duplicates
//   the range's automation right after it (one undo step);
// - a double-click creates a point on the SECOND PRESS: it exists before the release and
//   follows the pointer until then (⇧: whole dB), one undo step with the creation.
//
// No sleeps: every step waits on UI or engine state. The UI mirror is `window.__ether`.
import { expect, test, type Page } from "@playwright/test";
import type { AutomationPoint, Project } from "@/generated";
import { expectRenderer } from "./lanesFixture";
import { createTrack, headerButton, launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

const volumePoints = async (page: Page): Promise<AutomationPoint[]> => {
  const p = await doc(page);
  const lane = Object.values(p.automation_lanes).find((l) => l.target.type === "TrackVolume");
  return lane
    ? Object.values(p.automation_points)
        .filter((x) => x.lane === lane.id)
        .sort((a, b) => a.time - b.time || (a.id < b.id ? -1 : 1))
    : [];
};

test("automation lanes: click sets the cursor, ⇧-drag selects a range, ⌘D duplicates it, double-press-drag adds a point", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page, `Automation select ${Date.now()}`);
  const midi = await createTrack(page, "Midi");
  await expectRenderer(page);
  await (await headerButton(page, midi.name, `Show automation of ${midi.name}`)).click();
  const volume = page.getByRole("group", { name: "volume automation" });
  const svg = volume.getByTestId("automation-lane-svg");
  await expect(svg).toBeVisible();
  // Let the open animation settle (the lane's box is where clicks land): the same box twice
  // in a row, a few frames apart.
  let last = "";
  await expect
    .poll(async () => {
      await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
      const now = JSON.stringify(await svg.boundingBox());
      const still = now === last;
      last = now;
      return still;
    })
    .toBe(true);
  const box = (await svg.boundingBox())!;
  expect(box.height).toBeCloseTo(64, 0);
  const header = page.getByRole("group", { name: `${midi.name} track` });

  // Two points (a ramp up), double-clicked in.
  await svg.dblclick({ position: { x: 20, y: box.height * 0.8 } });
  await expect.poll(async () => (await volumePoints(page)).length).toBe(1);
  await svg.dblclick({ position: { x: 200, y: box.height * 0.2 } });
  await expect.poll(async () => (await volumePoints(page)).length).toBe(2);
  const [a, b] = await volumePoints(page);
  expect(b!.value).toBeGreaterThan(a!.value);

  // --- A click on empty space: the track is selected, the insert marker is there ----------
  await page.mouse.click(box.x + 600, box.y + box.height * 0.9);
  await expect(header).toHaveClass(/eth-arr-header--selected/);
  const marker = page.locator(`[data-testid="insert-marker"][data-track="${midi.id}"]`);
  await expect(marker).toBeVisible();
  const markerBox = (await marker.boundingBox())!;
  // The marker spans the row, automation lanes included, at the click (snapped to the grid).
  expect(Math.abs(markerBox.x - (box.x + 600))).toBeLessThan(40);
  expect(markerBox.y + markerBox.height).toBeGreaterThanOrEqual(box.y + box.height - 1);
  expect((await volumePoints(page)).length).toBe(2);
  const beatsAt600 = Number(await marker.getAttribute("data-beats"));

  // --- A click on the "+ Parameter…" bar's empty space: the same (the marker moves there) ---
  const bar = page.locator(`[data-automation-track="${midi.id}"] .eth-auto-bar`);
  const barBox = (await bar.boundingBox())!;
  await page.mouse.click(box.x + 300, barBox.y + barBox.height / 2);
  await expect(marker).toBeVisible();
  await expect.poll(async () => Number(await marker.getAttribute("data-beats"))).toBeLessThan(beatsAt600);
  await expect(header).toHaveClass(/eth-arr-header--selected/);

  // --- ⇧-drag: a time range in the lane, covering both points ---------------------------
  await page.mouse.move(box.x + 4, box.y + box.height * 0.95);
  await page.keyboard.down("Shift");
  await page.mouse.down();
  await page.mouse.move(box.x + 120, box.y + box.height * 0.95, { steps: 4 });
  await page.mouse.move(box.x + 260, box.y + box.height * 0.95, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  const range = volume.getByTestId("automation-range");
  await expect(range).toBeVisible();
  const start = Number(await range.getAttribute("data-start"));
  const end = Number(await range.getAttribute("data-end"));
  expect(start).toBeLessThanOrEqual(a!.time);
  expect(end).toBeGreaterThanOrEqual(b!.time);

  // --- ⌘D: the range's automation again right after it, one undo step -------------------
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(async () => (await volumePoints(page)).length).toBeGreaterThan(2);
  const len = end - start;
  const after = await volumePoints(page);
  // The copies of both points, shifted by the range's length.
  for (const p of [a!, b!]) expect(after.some((q) => Math.abs(q.time - (p.time + len)) < 1e-6 && Math.abs(q.value - p.value) < 1e-9)).toBe(true);
  // The originals stay.
  for (const p of [a!, b!]) expect(after.some((q) => q.id === p.id)).toBe(true);
  // The copy is the selection now.
  await expect(range).toHaveAttribute("data-start", String(end));
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await volumePoints(page)).map((p) => p.id)).toEqual([a!.id, b!.id]);

  // --- Double press: the point exists before the release and follows the drag (⇧: whole dB) ---
  const x0 = box.x + 400;
  const y0 = box.y + box.height * 0.7;
  await page.mouse.click(x0, y0);
  await page.mouse.move(x0, y0);
  await page.mouse.down();
  await expect.poll(async () => (await volumePoints(page)).length).toBe(3);
  const created = (await volumePoints(page)).find((p) => p.id !== a!.id && p.id !== b!.id)!;
  await page.keyboard.down("Shift");
  await page.mouse.move(x0 + 40, y0 - 10, { steps: 4 });
  await page.mouse.move(x0 + 80, y0 - 20, { steps: 4 });
  const tip = volume.getByTestId("automation-drag-tip");
  await expect(tip).toBeVisible();
  // Whole dB on the tooltip while ⇧ is held.
  await expect(tip.locator(".eth-auto-lane__tip-value")).toHaveText(/^-?\d+(\.0+)? dB$/);
  const moving = (await volumePoints(page)).find((p) => p.id === created.id)!;
  expect(moving.time).toBeGreaterThan(created.time);
  expect(moving.value).toBeGreaterThan(created.value);
  await page.mouse.up();
  await page.keyboard.up("Shift");
  await expect(tip).toHaveCount(0);
  // One undo step for the creation and the drag.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await volumePoints(page)).map((p) => p.id)).toEqual([a!.id, b!.id]);

  expect(errors).toEqual([]);
});
