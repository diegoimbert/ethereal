// Undo brings the arrangement's section back (owner request, selectionHistory.ts), also when
// the gesture dropped it before its edit landed: pressing a clip to drag it turns the section
// into the insert marker, yet undoing the move restores the section.
import { expect, test } from "@playwright/test";
import { clipPoint } from "./clips";
import { createTrack, launch } from "./ui";

test("undoing a clip move brings back the section the clip was in", async ({ page }) => {
  await page.setViewportSize({ width: 1300, height: 600 });
  await launch(page);
  await createTrack(page, "Midi");
  const lanes = page.getByTestId("arrangement-content");
  const box = (await lanes.boundingBox())!;
  await page.mouse.dblclick(box.x + 400, box.y + 20);
  await page.keyboard.press("Escape");
  const clip = await page.evaluate(
    () => Object.keys((window as unknown as { __ether: { state(): { project: { clips: object } } } }).__ether.state().project.clips)[0]!,
  );

  // A Shift-drag makes a section over the clip.
  await page.keyboard.down("Shift");
  await page.mouse.move(box.x + 380, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 480, box.y + 25, { steps: 5 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  const section = page.getByTestId("time-selection");
  await expect(section).toBeVisible();
  const before = (await section.boundingBox())!;

  // Drag the clip: the press drops the section, the move is one undo step.
  const at = await clipPoint(page, clip, "header");
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await page.mouse.move(at.x + 130, at.y, { steps: 8 });
  await page.mouse.up();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { __ether: { state(): { history: { undo_label: string | null } } } }).__ether.state().history.undo_label),
    )
    .toBe("move clip");
  await expect(section).toHaveCount(0);

  await page.keyboard.press("ControlOrMeta+z");
  await expect(section).toBeVisible();
  expect(await section.boundingBox()).toEqual(before);
});
