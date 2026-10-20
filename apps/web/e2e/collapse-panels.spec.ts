// The inspector's and the bottom pane's x collapse them to a thin floating bar (not close
// them). The choice is remembered: a panel that opens again (a new selection, a clip) and a
// reload keep it, except a double-click on a clip, which expands both, or collapses both
// when both are expanded on that clip. `COLLAPSE_SHOTS=<dir>` also captures dark + light screenshots.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip, selectClip } from "./clips";
import { createTrack, launch, openOnLaunch } from "./ui";

const shots = process.env.COLLAPSE_SHOTS;

const doc = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project } } }).__ether.state().project);

// Waits for every running animation (the pane's fly-out / fly-in, the bar's fade) to finish.
const settled = (page: Page) =>
  expect
    .poll(() =>
      page.evaluate(
        () =>
          document.getAnimations().filter((a) => a.playState === "running" && (a.effect as KeyframeEffect).target?.closest(".eth-float, .eth-float-bar"))
            .length,
      ),
    )
    .toBe(0);

const header = (page: Page, name: string) => page.getByRole("group", { name: `${name} track` });

for (const theme of ["dark", "light"] as const) {
  test(`collapsing the inspector and the bottom pane (${theme})`, async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1280, height: 720 });
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    const name = `Collapse ${theme} ${Date.now()}`;
    await launch(page, name);

    const a = await createTrack(page, "Midi");
    const b = await createTrack(page, "Midi");
    await page.locator(`[data-lane="${a.id}"]`).dblclick({ position: { x: 5, y: 20 } });
    await expect.poll(async () => Object.values((await doc(page)).clips).length).toBe(1);

    // Inspector: x collapses it to a bar; another selection reopens it collapsed.
    await header(page, a.name).click({ position: { x: 6, y: 10 } });
    const inspector = page.getByTestId("inspector");
    await expect(inspector).toBeVisible();
    await page.getByRole("button", { name: "collapse the inspector panel" }).click();
    const bar = page.getByRole("button", { name: "expand the inspector panel" });
    await expect(bar).toBeVisible();
    await settled(page);
    await expect(inspector).toHaveCount(0);
    if (shots) await page.screenshot({ path: `${shots}/inspector-collapsed-${theme}.png` });
    await header(page, b.name).click({ position: { x: 6, y: 10 } });
    await expect(bar).toBeVisible();
    await expect(inspector).toHaveCount(0);

    // Reload: still collapsed. Enter on the focused bar expands it.
    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0);
    await page.reload();
    await openOnLaunch(page, name);
    await header(page, b.name).click({ position: { x: 6, y: 10 } });
    await expect(page.getByRole("button", { name: "expand the inspector panel" })).toBeVisible();
    await page.getByRole("button", { name: "expand the inspector panel" }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("inspector")).toBeVisible();
    await expect(page.getByRole("button", { name: "expand the inspector panel" })).toHaveCount(0);
    await settled(page);
    if (shots) await page.screenshot({ path: `${shots}/inspector-expanded-${theme}.png` });

    // Bottom pane: a clip opens the editor; x collapses it; the next clip opens it collapsed.
    const clip = Object.values((await doc(page)).clips)[0]!.id;
    await openClip(page, clip);
    await expect(page.locator('[data-pane="bottom"]')).toBeVisible();
    await page.getByRole("button", { name: "collapse the editor panel" }).click();
    const bottom = page.getByRole("button", { name: "expand the editor panel" });
    await expect(bottom).toBeVisible();
    await expect(page.locator(".eth-drawer")).toHaveCount(0);
    await settled(page);
    if (shots) await page.screenshot({ path: `${shots}/bottom-collapsed-${theme}.png` });
    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0);
    await page.reload();
    await openOnLaunch(page, name);
    // A click on a MIDI clip opens it in the editor: still collapsed.
    await selectClip(page, clip);
    await expect(page.getByRole("button", { name: "expand the editor panel" })).toBeVisible();
    await page.getByRole("button", { name: "expand the editor panel" }).click();
    await expect(page.locator(".eth-drawer")).toBeVisible();
    await expect(page.getByRole("button", { name: "expand the editor panel" })).toHaveCount(0);
    await settled(page);
    await expect(page.locator(".eth-float--closing")).toHaveCount(0);

    // A double-click on the clip expands both collapsed panels, at their last size.
    const height = (await page.locator('[data-pane="bottom"]').boundingBox())!.height;
    await page.getByRole("button", { name: "collapse the editor panel" }).click();
    await page.getByRole("button", { name: "collapse the inspector panel" }).click();
    await expect(page.getByRole("button", { name: "expand the editor panel" })).toBeVisible();
    await expect(page.getByRole("button", { name: "expand the inspector panel" })).toBeVisible();
    await settled(page);
    await openClip(page, clip);
    await expect(page.locator('[data-pane="bottom"]').getByTestId("piano-roll")).toBeVisible();
    await expect(page.getByTestId("inspector")).toBeVisible();
    await expect(page.getByRole("button", { name: "expand the editor panel" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "expand the inspector panel" })).toHaveCount(0);
    await settled(page);
    expect((await page.locator('[data-pane="bottom"]').boundingBox())!.height).toBeCloseTo(height, 0);

    // Both expanded: a double-click on the open clip collapses both; another expands them
    // back at the same size.
    await openClip(page, clip);
    await expect(page.getByRole("button", { name: "expand the editor panel" })).toBeVisible();
    await expect(page.getByRole("button", { name: "expand the inspector panel" })).toBeVisible();
    await settled(page);
    await expect(page.locator(".eth-drawer")).toHaveCount(0);
    await expect(page.getByTestId("inspector")).toHaveCount(0);
    await openClip(page, clip);
    await expect(page.locator('[data-pane="bottom"]').getByTestId("piano-roll")).toBeVisible();
    await expect(page.getByTestId("inspector")).toBeVisible();
    await settled(page);
    expect((await page.locator('[data-pane="bottom"]').boundingBox())!.height).toBeCloseTo(height, 0);
  });
}
