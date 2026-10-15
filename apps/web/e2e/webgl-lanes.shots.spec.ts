// Screenshots of the lane layers for the PR (not part of the regular suite): automation lanes
// (linear, step, curves, a selected point, a stepped DeviceEnabled lane) and a group summary,
// WebGL vs the Canvas2D fallback, dark and light:
// `LANES_SHOTS=<dir> npx playwright test webgl-lanes.shots --project webgl`.
// Comping shots: `COMPING_SHOTS=<dir>` with comping.spec.ts (both projects).
import { test, type Page } from "@playwright/test";
import { setArrangerRenderer } from "./clips";
import { automationPoint, buildLanesFixture, openAutomation } from "./lanesFixture";
import { setViewport } from "./perfProject";
import { launch, newProject } from "./ui";

const dir = process.env.LANES_SHOTS;
test.skip(!dir, "set LANES_SHOTS=<dir> to capture");
test.use({ viewport: { width: 1280, height: 640 } });

const frames = (page: Page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

test("lane layers: WebGL vs Canvas2D, dark and light", async ({ page }) => {
  test.skip(test.info().project.name === "canvas2d", "shoots both backends in one run");
  test.setTimeout(120_000);
  await launch(page);
  await newProject(page, `Lanes ${Date.now()}`);
  const fx = await buildLanesFixture(page);
  await openAutomation(page);
  await setViewport(page, 40, 0);
  // Select a point.
  await frames(page);
  const p = await automationPoint(page, fx.points[2]!);
  await page.mouse.click(p.x, p.y);
  // The floating panes cover the arranger.
  const close = page.getByRole("button", { name: /^close / });
  while ((await close.count()) > 0) await close.first().click();
  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    for (const renderer of ["webgl", "canvas2d"] as const) {
      await setArrangerRenderer(page, renderer);
      await page.mouse.move(5, 5);
      await frames(page);
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${dir}/lanes-${theme}-${renderer}.png` });
    }
  }
});
