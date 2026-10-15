// Track header polish on the web build (real wasm engine), both arranger renderers:
// - the track name extends under the buttons hidden while the pointer is away, up to the
//   next button that stays visible (armed record); hovering brings the others back;
// - folding / unfolding a group updates the lanes at once (no click needed);
// - a selected clip gives its track's header the hover look.
//
// No sleeps. `E2E_SHOTS=<dir>` saves PR screenshots.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { arrangerApi } from "./clips";
import { createTrack, launch, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}
const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
const doc = async (page: Page): Promise<Project> => (await project(page))!;

test("track header: name room, group fold redraw, selected clip hover look", async ({ page }, info) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page, `Header polish ${info.project.name} ${Date.now()}`);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  const shot = async (name: string) => {
    if (process.env.E2E_SHOTS) await page.screenshot({ path: `${process.env.E2E_SHOTS}/${info.project.name}-${name}.png` });
  };

  const a = await createTrack(page, "Midi");
  const b = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${a.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).length).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;

  // --- name room -------------------------------------------------------------------------
  const header = page.getByRole("group", { name: `${b.name} track` });
  const name = header.locator(".eth-arr-header__name");
  await page.mouse.move(0, 0);
  await expect(name).toBeVisible();
  const idle = (await name.boundingBox())!;
  await header.hover();
  await expect.poll(async () => (await name.boundingBox())!.width).toBeLessThan(idle.width - 40);
  const arm = header.getByRole("button", { name: `arm ${b.name}` });
  await arm.click();
  await expect(arm).toHaveAttribute("aria-pressed", "true");
  await page.mouse.move(0, 0);
  // Armed and not hovered: the name stops at the arm button, but is wider than hovered.
  await expect
    .poll(async () => {
      const n = (await name.boundingBox())!;
      const r = (await arm.boundingBox())!;
      return n.x + n.width <= r.x + 0.5 && n.width < idle.width - 10 && n.width > idle.width - 70;
    })
    .toBe(true);
  await shot("armed-not-hovered");

  // --- group fold ------------------------------------------------------------------------
  await page.getByRole("group", { name: `${a.name} track` }).click({ position: { x: 6, y: 10 } });
  await header.click({ position: { x: 6, y: 10 }, modifiers: ["Shift"] });
  await page.keyboard.press("ControlOrMeta+g");
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Group")).toBe(true);
  const clipDrawn = () => arrangerApi(page, "visibleClipIds").then((ids) => ids.includes(clip.id));
  await expect.poll(clipDrawn).toBe(true);
  await shot("group-open");
  await page.getByRole("button", { name: /^fold / }).click();
  await expect(page.getByRole("group", { name: `${a.name} track` })).toHaveCount(0);
  // No further input: the lanes follow.
  await expect.poll(clipDrawn).toBe(false);
  await shot("group-folded");
  await page.getByRole("button", { name: /^unfold / }).click();
  await expect.poll(clipDrawn).toBe(true);

  // --- selected clip: its header looks hovered -------------------------------------------
  const aHeader = page.getByRole("group", { name: `${a.name} track` });
  await expect(aHeader).not.toHaveAttribute("data-clip-selected", "true");
  // The unfold animates the rows: wait until the clip's header stops moving.
  let last: { x: number; y: number } | null = null;
  await expect
    .poll(async () => {
      const n = await arrangerApi(page, "handleRect", clip.id, "header");
      const still = !!n && !!last && n.x === last.x && n.y === last.y;
      last = n && { x: n.x, y: n.y };
      return still;
    })
    .toBe(true);
  const r = await arrangerApi(page, "handleRect", clip.id, "header");
  await page.mouse.click(r!.x + r!.width / 2, r!.y + r!.height / 2);
  await expect(aHeader).toHaveAttribute("data-clip-selected", "true");
  await page.mouse.move(r!.x + r!.width / 2, r!.y + r!.height + 200);
  await shot("clip-selected-header-hovered");
  expect(errors).toEqual([]);
});
