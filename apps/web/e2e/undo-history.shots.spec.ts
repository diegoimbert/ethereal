// Screenshots of the History panel for the PR (not part of the regular suite):
// `UNDO_HISTORY_SHOTS=<dir> npx playwright test undo-history.shots`.
import { expect, test, type Page } from "@playwright/test";
import { createTrack, launch } from "./ui";

const dir = process.env.UNDO_HISTORY_SHOTS;
test.skip(!dir, "set UNDO_HISTORY_SHOTS=<dir> to capture");
test.use({ viewport: { width: 1440, height: 900 } });

async function scene(page: Page) {
  await launch(page, "Verse ideas");
  const a = await createTrack(page, "Midi");
  await createTrack(page, "Midi");
  const c = await createTrack(page, "Midi");
  await page.getByRole("button", { name: `Mute ${a.name}` }).click();
  await createTrack(page, "Audio");
  await page.getByRole("button", { name: `Mute ${c.name}` }).click();
  await page.getByRole("button", { name: "History", exact: true }).click();
  const panel = page.locator('[data-feature="undo-history"]');
  const rows = panel.getByRole("list", { name: "Undo history" }).getByRole("listitem");
  await expect(rows).toHaveCount(7);
  // A checkpoint, then go back two steps (they show as undone).
  await rows.nth(3).hover();
  await rows.nth(3).getByRole("button", { name: "Name checkpoint" }).click();
  await rows.nth(3).getByRole("textbox").fill("Three synths");
  await rows.nth(3).getByRole("textbox").press("Enter");
  await rows.nth(4).locator(".eth-history__jump").click();
  await expect(rows.nth(4).locator(".eth-history__jump")).toHaveAttribute("aria-current", "step");
  await rows.nth(2).hover();
  return panel;
}

for (const theme of ["dark", "light"] as const) {
  test(`History panel (${theme})`, async ({ page }) => {
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    const panel = await scene(page);
    await page.screenshot({ path: `${dir}/history-${theme}.png` });
    await panel.screenshot({ path: `${dir}/history-panel-${theme}.png` });
    if (theme === "dark") {
      // Editing a checkpoint name.
      const row = panel.getByRole("listitem").nth(5);
      await row.hover();
      await row.getByRole("button", { name: "Name checkpoint" }).click();
      await row.getByRole("textbox").fill("Before the bass");
      await panel.screenshot({ path: `${dir}/history-panel-naming.png` });
    }
  });
}
