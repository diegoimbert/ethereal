// Screenshots/video of the automation lanes for the PR (not part of the regular suite):
// `AUTOMATION_SHOTS=<dir> npx playwright test automation-ux.shots`. Mid-animation frames
// step the page clock (Playwright's fake timers drive requestAnimationFrame and
// performance.now), so they are exact, not timing luck.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, newProject, pickOption } from "./ui";

const dir = process.env.AUTOMATION_SHOTS;
test.skip(!dir, "set AUTOMATION_SHOTS=<dir> to capture");
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark", video: dir ? { mode: "on", size: { width: 1440, height: 900 } } : "off" });

interface Handle {
  state(): { project: Project | null };
}
const doc = async (page: Page): Promise<Project> =>
  (await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project))!;

const shot = (page: Page, name: string) => page.screenshot({ path: `${dir}/${name}.png` });

test("automation lanes: animation frames, step grid, paste, tooltip", async ({ page }) => {
  test.setTimeout(120_000);
  await page.clock.install();
  await page.goto("/");
  await newProject(page, "Automation demo");
  const a = await createTrack(page, "Midi");
  const b = await createTrack(page, "Midi");
  await createTrack(page, "Midi");
  const synth = Object.values((await doc(page)).devices).find((d) => d.track === a.id)!;
  await shot(page, "01-closed");

  // Opening, frame by frame.
  await page.clock.pauseAt(Date.now() + 1000);
  await page.getByRole("button", { name: `Show automation of ${a.name}` }).click();
  for (const [i, ms] of [60, 60, 80].entries()) {
    await page.clock.runFor(ms);
    await shot(page, `02-opening-${i + 1}`);
  }
  await page.clock.runFor(400);
  await page.clock.resume();
  await shot(page, "03-open");

  // Points, then a Transpose lane with its step grid.
  const volume = page.getByRole("group", { name: "Volume automation" }).getByTestId("automation-lane-svg");
  const box = (await volume.boundingBox())!;
  for (const [x, f] of [
    [30, 0.4],
    [120, 0.15],
    [220, 0.6],
    [330, 0.3],
  ] as const)
    await volume.dblclick({ position: { x, y: box.height * f } });
  await pickOption(page, "Show parameter", { value: `param:${synth.id}:1` });
  const transpose = page.getByRole("group", { name: /Transpose automation/ });
  const tSvg = transpose.getByTestId("automation-lane-svg");
  await expect(tSvg).toBeVisible();

  // Resize the transpose lane taller (drag its header's bottom edge).
  const grip = page.getByRole("separator", { name: /Resize .*Transpose lane/ });
  const g = (await grip.boundingBox())!;
  await page.mouse.move(g.x + 20, g.y + g.height / 2);
  await page.mouse.down();
  await page.mouse.move(g.x + 20, g.y + g.height / 2 + 120, { steps: 6 });
  await shot(page, "04-resizing");
  await page.mouse.up();

  const t = (await tSvg.boundingBox())!;
  for (const [x, f] of [
    [40, 0.5],
    [140, 0.35],
    [240, 0.25],
    [340, 0.5],
  ] as const)
    await tSvg.dblclick({ position: { x, y: t.height * f } });
  await shot(page, "05-step-grid");

  // Drag tooltip (held mid-drag).
  const circle = transpose.locator("circle[data-point]").nth(1);
  const c = (await circle.boundingBox())!;
  await page.mouse.move(c.x + c.width / 2, c.y + c.height / 2);
  await page.mouse.down();
  await page.mouse.move(c.x + c.width / 2 + 6, c.y + c.height / 2 - 18, { steps: 4 });
  await shot(page, "06-drag-tooltip");
  await page.mouse.up();

  // Copy the volume points, paste them later in the lane.
  await page.getByRole("group", { name: "Volume automation" }).focus();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.press("ControlOrMeta+C");
  await volume.click({ button: "right", position: { x: 420, y: box.height / 2 } });
  await shot(page, "07-lane-menu");
  await page.getByRole("menuitem", { name: "Paste Here" }).click();
  await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBe(12);
  await shot(page, "08-pasted");

  // Point menu.
  await transpose.locator("circle[data-point]").nth(2).click({ button: "right" });
  await shot(page, "09-point-menu");
  await page.keyboard.press("Escape");

  // Closing a track below, frame by frame (rows under it slide up).
  await page.getByRole("button", { name: `Show automation of ${b.name}` }).click();
  await page.waitForFunction(() => document.querySelector(".eth-auto-track--animating") === null);
  await page.clock.pauseAt(Date.now() + 1000);
  await page.getByRole("button", { name: `Hide automation of ${a.name}` }).click();
  for (const [i, ms] of [80, 80].entries()) {
    await page.clock.runFor(ms);
    await shot(page, `10-closing-${i + 1}`);
  }
  await page.clock.runFor(400);
  await page.clock.resume();
  await shot(page, "11-closed-again");
});
