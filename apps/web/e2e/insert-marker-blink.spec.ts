// The arrangement's insert markers blink in sync (#238): a click places the marker on one
// track, Shift-clicks at the same place add it on others; every marker's blink animation
// starts at the same timeline time, restarted by the last placement (so all show at once).
//
// No sleeps: every step waits on UI state.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, launch, newProject } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

/** The start time of each insert marker's blink animation (null when it has none). */
const starts = (page: Page): Promise<(number | null)[]> =>
  page.evaluate(() =>
    [...document.querySelectorAll('[data-testid="insert-marker"]')].map((el) => {
      const t = el.getAnimations()[0]?.startTime;
      return typeof t === "number" ? t : null;
    }),
  );

const now = (page: Page): Promise<number> => page.evaluate(() => document.timeline.currentTime as number);

test("insert markers on several tracks blink on one shared phase", async ({ page }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Marker blink ${Date.now()}`);

  const tracks = [await createTrack(page, "Midi"), await createTrack(page, "Midi"), await createTrack(page, "Midi")];
  const lanes = tracks.map((t) => page.locator(`[data-lane="${t.id}"]`));
  const markers = page.getByTestId("insert-marker");

  const box0 = (await lanes[0]!.boundingBox())!;
  const x = 200;
  await page.mouse.click(box0.x + x, box0.y + 20);
  await expect(markers).toHaveCount(1);
  const [first] = await starts(page);
  expect(first).not.toBeNull();

  for (const [i, lane] of lanes.slice(1).entries()) {
    // Let the shared phase run on (an unsynced new marker would start later).
    const before = await now(page);
    await expect.poll(() => now(page)).toBeGreaterThan(before + 300);
    const box = (await lane.boundingBox())!;
    await page.keyboard.down("Shift");
    await page.mouse.click(box0.x + x, box.y + 20);
    await page.keyboard.up("Shift");
    await expect(markers).toHaveCount(i + 2);
  }

  const all = await starts(page);
  expect(all).toHaveLength(3);
  expect(new Set(all).size).toBe(1);
  // Restarted by the last Shift-click: later than the first placement.
  expect(all[0]!).toBeGreaterThan(first!);

  // They also show and hide together.
  const visibility = () => markers.evaluateAll((els) => els.map((el) => getComputedStyle(el).visibility));
  const v = await visibility();
  expect(new Set(v).size).toBe(1);

  expect(errors).toEqual([]);
});
