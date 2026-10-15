// Screenshots of the arranger on WebGL and on its Canvas2D fallback (PR material): the same
// fixture project on both backends, dark and light, at the normal zoom, zoomed out (1k
// clips on screen) and zoomed in. `E2E_SHOTS=<dir>` saves them; without it the spec only
// checks that both backends draw the fixture.
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { arrangerApi, clipPoint, setArrangerRenderer } from "./clips";
import { buildMixedProject, importBassLoop, setViewport, ulid } from "./perfProject";
import { launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const dir = process.env.E2E_SHOTS;

async function shoot(page: Page, name: string): Promise<void> {
  // Peaks arrive and the lanes settle.
  await page.waitForTimeout(1000);
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  await page.locator(".eth-arr__scroll").screenshot({ path: join(dir, `${name}.png`) });
}

test("WebGL vs Canvas2D arranger: screenshots", async ({ page }) => {
  test.skip(test.info().project.name === "canvas2d", "compares both backends in one run");
  test.setTimeout(600_000);
  await launch(page, `WebGL shots ${Date.now()}`);
  await buildMixedProject(page, { tracks: 40, clips: 1000, importSample: (t) => importBassLoop(page, t) });
  // A selected clip (glow) in view.
  const selected = ulid(6, 100);

  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    for (const renderer of ["webgl", "canvas2d"] as const) {
      await setArrangerRenderer(page, renderer);
      await setViewport(page, 24, 0);
      const p = await clipPoint(page, selected);
      await page.mouse.click(p.x, p.y);
      await page.mouse.move(5, 5);
      await shoot(page, `${theme}-${renderer}-normal`);
      await setViewport(page, 1.5, 0);
      await shoot(page, `${theme}-${renderer}-zoomed-out`);
      expect((await arrangerApi(page, "visibleClipIds")).length).toBeGreaterThan(250);
      await setViewport(page, 160, 6);
      await shoot(page, `${theme}-${renderer}-zoomed-in`);
    }
  }
});
