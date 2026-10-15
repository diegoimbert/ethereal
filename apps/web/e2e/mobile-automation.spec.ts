// Automation on touch screens (#522): the track sheet shows a track's automation lanes; on a
// lane a double-tap adds a point, long-press then drag moves it, a double-tap on it deletes it;
// a plain drag on the lane pans.
import { expect, test, type Page } from "@playwright/test";
import { automationPoint } from "./lanesFixture";
import { arrangementViewState, centerOf, IPHONE, launchMobile, settled, shot, touchscreen } from "./mobile";
import { createTrack } from "./ui";

test.use(IPHONE);

interface Pt {
  id: string;
  time: number;
  value: number;
}
const points = (page: Page) =>
  page.evaluate(() =>
    Object.values(
      (
        window as unknown as { __ether: { state(): { project: { automation_points: Record<string, { id: string; time: number; value: number }> } } } }
      ).__ether.state().project.automation_points,
    ).map((p) => ({ id: p.id, time: p.time, value: p.value })),
  ) as Promise<Pt[]>;

test("show a track's automation from its sheet; add, move and delete points by touch; a drag pans", async ({ page }) => {
  await launchMobile(page);
  const track = await createTrack(page, "Audio");
  const touch = await touchscreen(page);
  // The new track is selected: a tap on its header opens the sheet.
  await touch.tap(await centerOf(page.locator(`.eth-arr__scroll .eth-arr-header[data-track="${track.id}"] .eth-arr-header__name`)));
  const sheet = page.getByRole("dialog", { name: "track" });
  await expect(sheet).toBeVisible();
  await sheet.getByRole("button", { name: /automation of/i }).tap();
  await sheet.getByRole("button", { name: "close" }).tap();
  await expect(sheet).toHaveCount(0);
  const lane = page.getByRole("group", { name: /volume automation/i }).getByTestId("automation-lane-svg");
  await expect(lane).toBeVisible();
  await settled(page);
  const box = (await lane.boundingBox())!;
  // Double-tap: a point.
  const at = { x: box.x + 60, y: box.y + box.height * 0.3 };
  await touch.tap(at);
  await touch.sleep(120);
  await touch.tap(at);
  await expect.poll(async () => (await points(page)).length).toBe(1);
  const p0 = (await points(page))[0]!;
  await shot(page, "automation");
  // Long-press then drag the point down: its value drops.
  const p = await automationPoint(page, p0.id);
  await touch.drag(p, { x: p.x, y: p.y + box.height * 0.4 }, { hold: 600 });
  await expect.poll(async () => (await points(page))[0]!.value).toBeLessThan(p0.value);
  // A plain drag on the lane pans the timeline; the point stays.
  const before = await points(page);
  const s0 = (await arrangementViewState(page)).scrollBeats;
  await touch.drag({ x: box.x + 200, y: box.y + box.height / 2 }, { x: box.x + 100, y: box.y + box.height / 2 });
  expect((await arrangementViewState(page)).scrollBeats).toBeGreaterThan(s0);
  expect(await points(page)).toEqual(before);
  // Back where it was, then a double-tap on the point deletes it.
  await touch.drag({ x: box.x + 100, y: box.y + box.height / 2 }, { x: box.x + 220, y: box.y + box.height / 2 });
  const q = await automationPoint(page, p0.id);
  await touch.tap(q);
  await touch.sleep(120);
  await touch.tap(q);
  await expect.poll(async () => (await points(page)).length).toBe(0);
});
