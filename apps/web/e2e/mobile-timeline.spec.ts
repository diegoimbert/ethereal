// Timeline navigation on touch screens (#522): one-finger pan (axis-locked, with momentum),
// pinch zoom of time and of track height, tap to select, a drag on the ruler scrubs, panning
// while playing stops following the playhead, long-press opens the context menu.
import { expect, test, type Page } from "@playwright/test";
import { createTrack } from "./ui";
import { arrangementViewState, centerOf, IPHONE, launchMobile, shot, touchscreen, transportState } from "./mobile";

test.use(IPHONE);

/** The arrangement's scroller and a point in the lanes (right of the header column). */
async function lanes(page: Page) {
  const scroller = page.locator(".eth-arr__scroll");
  const box = (await scroller.boundingBox())!;
  const headerWidth = await page.evaluate(() => parseFloat(getComputedStyle(document.querySelector(".eth-arr")!).getPropertyValue("--eth-arr-header-width")));
  return { scroller, box, left: box.x + headerWidth, at: (dx: number, dy: number) => ({ x: box.x + headerWidth + dx, y: box.y + dy }) };
}

const scrollTop = (page: Page) => page.locator(".eth-arr__scroll").evaluate((el) => el.scrollTop);

test("one-finger pan: time sideways (locked: no vertical drift), tracks up and down", async ({ page }) => {
  await launchMobile(page);
  for (let i = 0; i < 10; i++) await createTrack(page, "Audio");
  const touch = await touchscreen(page);
  const l = await lanes(page);
  const before = await arrangementViewState(page);
  // Mostly sideways with a little vertical drift: locked to time.
  await touch.drag(l.at(200, 200), l.at(40, 210));
  const after = await arrangementViewState(page);
  expect((after.scrollBeats - before.scrollBeats) * before.pxPerBeat).toBeCloseTo(160, -1);
  expect(await scrollTop(page)).toBe(0);
  // Upwards: the tracks scroll.
  await touch.drag(l.at(100, 300), l.at(102, 120));
  expect(await scrollTop(page)).toBeGreaterThan(150);
  expect((await arrangementViewState(page)).scrollBeats).toBeCloseTo(after.scrollBeats, 5);
});

test("a flick keeps scrolling after the finger lifts (momentum); a touch stops it", async ({ page }) => {
  await launchMobile(page);
  const touch = await touchscreen(page);
  const l = await lanes(page);
  const s0 = (await arrangementViewState(page)).scrollBeats;
  await touch.drag(l.at(220, 150), l.at(60, 150), { steps: 5, ms: 60, rest: 0 });
  const atRelease = (await arrangementViewState(page)).scrollBeats;
  expect(atRelease).toBeGreaterThan(s0);
  await expect.poll(async () => (await arrangementViewState(page)).scrollBeats).toBeGreaterThan(atRelease + 2);
  // A touch stops it where it is.
  await touch.tap(l.at(150, 300));
  const stopped = (await arrangementViewState(page)).scrollBeats;
  await page.waitForTimeout(300);
  expect((await arrangementViewState(page)).scrollBeats).toBe(stopped);
});

test("pinch: side by side zooms time around the fingers; one above the other zooms track height", async ({ page }) => {
  await launchMobile(page);
  const created = await createTrack(page, "Audio");
  const touch = await touchscreen(page);
  const l = await lanes(page);
  const v0 = await arrangementViewState(page);
  // The beat under the fingers' midpoint stays put.
  const mid = l.at(100, 200);
  const beatAt = (v: { pxPerBeat: number; scrollBeats: number }) => v.scrollBeats + (mid.x - l.left) / v.pxPerBeat;
  await touch.pinch({ x: mid.x - 40, y: mid.y }, { x: mid.x + 40, y: mid.y }, { x: mid.x - 80, y: mid.y }, { x: mid.x + 80, y: mid.y });
  const v1 = await arrangementViewState(page);
  expect(v1.pxPerBeat / v0.pxPerBeat).toBeCloseTo(2, 1);
  expect(beatAt(v1)).toBeCloseTo(beatAt(v0), 1);
  // Vertical pinch: the track lanes grow.
  const track = created;
  const header = page.getByRole("group", { name: `${track.name} track` });
  const h0 = (await header.boundingBox())!.height;
  await touch.pinch(l.at(100, 60), l.at(110, 140), l.at(100, 30), l.at(110, 190));
  await expect.poll(async () => (await header.boundingBox())!.height).toBeGreaterThan(h0 * 1.5);
  await shot(page, "pinched");
});

test("tap selects (the desktop's click), long-press opens the context menu", async ({ page }) => {
  await launchMobile(page);
  const created = await createTrack(page, "Audio");
  const track = created;
  const touch = await touchscreen(page);
  // A tap on the track's lane selects the track.
  const l = await lanes(page);
  const header = page.getByRole("group", { name: `${track.name} track` });
  const row = (await header.boundingBox())!;
  await page.evaluate(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await touch.tap({ x: l.left + 60, y: row.y + row.height / 2 });
  await expect(page.locator(".eth-arr-header--selected")).toHaveCount(1);
  // Long-press on the header: its context menu.
  await touch.longPress(await centerOf(header));
  await expect(page.getByRole("menu")).toBeVisible();
  await expect(page.getByRole("menuitem", { name: /delete/ })).toBeVisible();
});

test("a drag on the ruler scrubs the playhead; panning while playing stops following", async ({ page }) => {
  await launchMobile(page);
  const touch = await touchscreen(page);
  const ruler = page.getByRole("slider", { name: "timeline ruler" });
  const r = (await ruler.boundingBox())!;
  await touch.drag({ x: r.x + 20, y: r.y + r.height / 2 }, { x: r.x + 150, y: r.y + r.height / 2 });
  const position = await page.evaluate(
    () => (window as unknown as { __ether: { playhead(): { transport: { position: number } } | null } }).__ether.playhead()?.transport.position ?? 0,
  );
  expect(position).toBeGreaterThan(1);
  await page.getByTestId("mobile-transport").getByRole("button", { name: "play", exact: true }).tap();
  await expect.poll(async () => (await transportState(page)).playing).toBe(true);
  expect((await arrangementViewState(page)).followPlayhead).toBe(true);
  const l = await lanes(page);
  await touch.drag(l.at(200, 150), l.at(100, 150));
  expect((await arrangementViewState(page)).followPlayhead).toBe(false);
  await expect(page.getByRole("button", { name: "follow playhead" })).toHaveAttribute("aria-pressed", "false");
});

test("rotating keeps the beat at the centre of the lanes, the zoom and the selection", async ({ page }) => {
  await launchMobile(page);
  const created = await createTrack(page, "Audio");
  await page.getByRole("group", { name: `${created.name} track` }).click();
  const touch = await touchscreen(page);
  const l = await lanes(page);
  await touch.drag(l.at(220, 200), l.at(20, 200));
  const centre = async () => {
    const { box, left } = await lanes(page);
    const v = await arrangementViewState(page);
    return v.scrollBeats + (box.x + box.width - left) / 2 / v.pxPerBeat;
  };
  const before = await centre();
  const zoom = (await arrangementViewState(page)).pxPerBeat;
  await page.setViewportSize({ width: 664, height: 390 });
  await expect.poll(centre).toBeCloseTo(before, 0);
  expect((await arrangementViewState(page)).pxPerBeat).toBe(zoom);
  await expect(page.locator(".eth-arr-header--selected")).toHaveCount(1);
});
