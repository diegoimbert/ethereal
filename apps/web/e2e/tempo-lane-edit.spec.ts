// The master's Tempo lane edits like the automation lanes and the arranger (#621), against the
// real engine: a drag on empty space selects tempo points in a box, Delete removes them (one
// undo step), copy then paste at the cursor (a click on the lane) adds the copy there.
//
// No sleeps: every step waits on UI or engine state. The mirror is `window.__ether`.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { headerButton, launch, newProject, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const tempoPoints = async (page: Page) => Object.values((await doc(page)).tempo_points).sort((a, b) => a.time - b.time);

test("tempo lane: box select, delete, copy and paste at the cursor", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, `Tempo lane ${Date.now()}`);
  await expect.poll(async () => (await tempoPoints(page)).length, { timeout: 30_000 }).toBe(1);

  await (await headerButton(page, "master", "Show automation of master")).click();
  const svg = page.getByTestId("tempo-automation-lane").getByTestId("tempo-lane-svg");
  await expect(svg).toBeVisible();
  const box = (await svg.boundingBox())!;
  // The arrangement's default zoom: 24 px per beat from the lane's left edge (beat 0).
  const px = 24;
  const at = (beats: number, y = 0.25) => ({ x: px * beats + 3, y: box.height * y });

  // Two points (double-click), at beats 8 and 16.
  await svg.dblclick({ position: at(8) });
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(2);
  await svg.dblclick({ position: at(16, 0.6) });
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(3);
  const [, p8, p16] = await tempoPoints(page);

  // A drag on empty space from beat 6 (low) to 18 (high): both points are in the box.
  await page.mouse.move(box.x + px * 6, box.y + box.height - 2);
  await page.mouse.down();
  await page.mouse.move(box.x + px * 12, box.y + box.height / 2, { steps: 4 });
  await page.mouse.move(box.x + px * 18, box.y + 2, { steps: 4 });
  await page.mouse.up();
  for (const p of [p8!, p16!]) await expect(svg.locator(`[data-point="${p.id}"]`)).toHaveAttribute("aria-selected", "true");

  // Delete removes both (the point at beat 0 stays); one undo brings both back.
  await page.keyboard.press("Delete");
  await expect.poll(async () => (await tempoPoints(page)).map((p) => p.time)).toEqual([0]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await tempoPoints(page)).map((p) => p.time)).toEqual([0, 8, 16]);

  // Copy the point at 8, click the lane at beat 24 (the cursor), paste: a copy there.
  await svg.locator(`[data-point="${p8!.id}"]`).click();
  await page.keyboard.press("ControlOrMeta+c");
  await svg.click({ position: at(24, 0.9) });
  await page.keyboard.press("ControlOrMeta+v");
  await expect.poll(async () => (await tempoPoints(page)).map((p) => p.time)).toEqual([0, 8, 16, 24]);
  expect((await tempoPoints(page))[3]!.bpm).toBe(p8!.bpm);
  expect(errors).toEqual([]);
});
