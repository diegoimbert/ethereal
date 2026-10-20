// The editor drawer's fullscreen button (piano roll / beat editor only): the drawer fills the
// workspace over the arrangement (the top bar stays put), the piano roll re-lays out at the new
// size, and Escape brings the drawer back at the height it had.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, launch } from "./ui";

const doc = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project } } }).__ether.state().project);

// Waits for the drawer's animations and transitions (fly-in, height) to finish.
const settled = (page: Page) =>
  expect
    .poll(() =>
      page.evaluate(
        () => document.getAnimations().filter((a) => a.playState === "running" && (a.effect as KeyframeEffect).target?.closest(".eth-float")).length,
      ),
    )
    .toBe(0);

test("the piano roll goes fullscreen and back", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1280, height: 720 });
  await launch(page, `Drawer fullscreen ${Date.now()}`);

  const track = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${track.id}"]`).dblclick({ position: { x: 5, y: 20 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).length).toBe(1);
  await openClip(page, Object.values((await doc(page)).clips)[0]!.id);
  const drawer = page.locator('[data-pane="bottom"]');
  await expect(page.getByTestId("piano-roll")).toBeVisible();
  await settled(page);
  const before = (await drawer.boundingBox())!;
  const rowsBefore = (await page.getByTestId("piano-roll-rows-canvas").boundingBox())!;

  // Fullscreen: the drawer spans the workspace (a float gap from its top and bottom edges).
  await drawer.getByRole("button", { name: "fullscreen editor" }).click();
  await expect(drawer).toHaveClass(/eth-float--fullscreen/);
  await settled(page);
  const workspace = (await page.locator(".eth-workspace").boundingBox())!;
  const full = (await drawer.boundingBox())!;
  expect(full.y - workspace.y).toBeLessThanOrEqual(16);
  expect(workspace.y + workspace.height - (full.y + full.height)).toBeLessThanOrEqual(16);
  expect(full.height).toBeGreaterThan(before.height + 100);
  // The top bar is still there, and the piano roll's canvas grew with the drawer.
  await expect(page.getByRole("toolbar", { name: "transport" })).toBeVisible();
  await expect.poll(async () => (await page.getByTestId("piano-roll-rows-canvas").boundingBox())!.height).toBeGreaterThan(rowsBefore.height + 100);

  // Escape: back at the height it had.
  await page.keyboard.press("Escape");
  await expect(drawer).not.toHaveClass(/eth-float--fullscreen/);
  await expect(drawer).toBeVisible();
  await settled(page);
  const after = (await drawer.boundingBox())!;
  expect(Math.abs(after.height - before.height)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.y - before.y)).toBeLessThanOrEqual(1);
  await expect(drawer.getByRole("button", { name: "fullscreen editor" })).toBeVisible();
});
