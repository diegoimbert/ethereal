// An unpinned (floating) editor drawer covers the bottom of the arrangement: the arrangement
// keeps empty room below its tracks, so the last track can still be scrolled above the drawer.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, launch } from "./ui";

const doc = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project } } }).__ether.state().project);

test("the last track scrolls above a floating piano roll", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1280, height: 720 });
  await launch(page, `Drawer space ${Date.now()}`);

  // Enough tracks to fill the arrangement; a clip on the last one, opened in the piano roll.
  let last = await createTrack(page, "Midi");
  for (let i = 0; i < 9; i++) last = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${last.id}"]`).scrollIntoViewIfNeeded();
  await page.locator(`[data-lane="${last.id}"]`).dblclick({ position: { x: 5, y: 20 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === last.id).length).toBe(1);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === last.id)!.id;
  await openClip(page, clip);
  // The section, not the collapsed bar (it may still be fading out).
  const drawer = page.locator('section[data-pane="bottom"]');
  await expect(drawer).toBeVisible();
  await expect(drawer).not.toHaveClass(/eth-float--pinned/);

  // Scrolled to the bottom, the last track ends above the drawer.
  await page.locator(".eth-arr__scroll").evaluate((el) => (el.scrollTop = el.scrollHeight));
  await expect
    .poll(async () => {
      const row = (await page.locator(`[data-lane="${last.id}"]`).boundingBox())!;
      const pane = (await drawer.boundingBox())!;
      return row.y + row.height <= pane.y;
    })
    .toBe(true);
});
