// ⌥ (Option) is a precision mode for automation point drags: off the grid, and the point
// moves a tenth as far as the pointer. Against the real engine; the UI mirror is `window.__ether`.
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { AutomationPoint, Project } from "@/generated";
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
  return lane ? Object.values(p.automation_points).filter((x) => x.lane === lane.id) : [];
};

test("⌥-dragging an automation point moves it a tenth as far as the pointer", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page, `Automation precision ${Date.now()}`);
  const midi = await createTrack(page, "Midi");
  await (await headerButton(page, midi.name, `Show automation of ${midi.name}`)).click();
  const volume = page.getByRole("group", { name: "volume automation" });
  const svg = volume.getByTestId("automation-lane-svg");
  await expect(svg).toBeVisible();
  // Let the open animation settle (the lane's box is where presses land).
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

  // One point, double-clicked in off the grid (⌥), so it sits right under the pointer.
  const x = box.x + 300;
  const y = box.y + box.height * 0.2;
  await svg.dblclick({ position: { x: 300, y: box.height * 0.2 }, modifiers: ["Alt"] });
  await expect.poll(async () => (await volumePoints(page)).length).toBe(1);
  const p0 = (await volumePoints(page))[0]!;
  const v0 = p0.value;
  // The view starts at beat 0: the lane's px per beat, to find the point after a snapped move.
  const pxPerBeat = 300 / p0.time;

  // A plain 10 px drag down.
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + 5, { steps: 2 });
  await page.mouse.move(x, y + 10, { steps: 2 });
  await page.mouse.up();
  await expect.poll(async () => (await volumePoints(page))[0]!.value).toBeLessThan(v0);
  const p1 = (await volumePoints(page))[0]!;
  const plain = v0 - p1.value;

  // ⌥ held, 100 px down: the value moves as far as the plain 10 px (a tenth of the pointer).
  // (The plain drag snapped the time to the grid: press where the point is now.)
  const x1 = box.x + p1.time * pxPerBeat;
  await page.keyboard.down("Alt");
  await page.mouse.move(x1, y + 10);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(x1, y + 10 + i * 10);
  const tip = volume.getByTestId("automation-drag-tip");
  await expect(tip).toContainText("fine, off grid");
  await page.mouse.up();
  await page.keyboard.up("Alt");
  await expect.poll(async () => (await volumePoints(page))[0]!.value).toBeLessThan(p1.value);
  const p2 = (await volumePoints(page))[0]!;
  const fine = p1.value - p2.value;
  expect(fine / plain).toBeGreaterThan(0.8);
  expect(fine / plain).toBeLessThan(1.2);
  expect(p2.time).toBe(p1.time);
  expect(errors).toEqual([]);
});
