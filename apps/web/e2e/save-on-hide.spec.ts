// #565: an edit made right before the tab closes (inside the 1 s autosave wait) is saved: a
// hidden or closed tab saves at once.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, launch, openOnLaunch } from "./ui";

interface Handle {
  state(): { project: Project | null; dirty: boolean };
}

const state = (page: Page) =>
  page.evaluate(() => {
    const s = (window as unknown as { __ether?: Handle }).__ether?.state();
    return { tracks: s?.project ? Object.keys(s.project.tracks).length : -1, dirty: s?.dirty ?? false };
  });

test("an edit made right before the tab closes is there when the app opens again", async ({ context, page }) => {
  test.setTimeout(120_000);
  await launch(page, "Kept");
  await createTrack(page, "Midi");
  await expect.poll(async () => (await state(page)).dirty, { timeout: 10_000 }).toBe(false);

  // The edit, then the tab closes before the autosave would have run.
  await createTrack(page, "Audio");
  const before = await state(page);
  expect(before.dirty).toBe(true);
  await page.close();

  const again = await context.newPage();
  await again.goto("/");
  await openOnLaunch(again, "Kept");
  await expect.poll(async () => (await state(again)).tracks, { timeout: 10_000 }).toBe(before.tracks);
});

test("a hidden tab saves at once", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page, "Hidden");
  await createTrack(page, "Audio");
  expect((await state(page)).dirty).toBe(true);
  // The tab goes to the background (another tab, a minimized window).
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  // Saved well before the autosave wait would end.
  await expect.poll(async () => (await state(page)).dirty, { timeout: 500, intervals: [20] }).toBe(false);
});
