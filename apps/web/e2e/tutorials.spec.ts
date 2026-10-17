// Guided tours from the welcome screen (docs/TUTORIALS.md): the start of the record tour in the
// browser (settings, the audio tab, the browser's note). The jam and import tours are not in
// the open-source build (no collaboration, no importer); the full product tests them.
import { expect as baseExpect, test, type Page } from "@playwright/test";
import { projectScreen } from "./ui";

// A loaded machine (the merge gate) can take a few seconds per step.
const expect = baseExpect.configure({ timeout: 15_000 });

const bubble = (page: Page) => page.getByTestId("tour-bubble");
const next = (page: Page) => bubble(page).getByRole("button", { name: "next", exact: true });
const step = (page: Page) => page.getByTestId("tour");

test("record audio in the browser: settings, the audio tab, and an exit on Escape", async ({ page }) => {
  await page.goto("/");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "record audio", exact: true }).click();
  await expect(bubble(page)).toContainText("ethereal can't record yet", { timeout: 30_000 });
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "interface");
  await next(page).click();
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "open-settings");
  await page.getByRole("button", { name: "settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "settings", exact: true });
  await expect(settings).toBeVisible();
  // Opened on its last tab (audio by default): that step is already done.
  await expect(step(page)).toHaveAttribute("data-step", "browser-devices");
  await expect(bubble(page)).toContainText("your system chooses");
  // The settings' own Done is not this step's: blocked.
  await settings.getByRole("button", { name: "done" }).click({ force: true });
  await expect(settings).toBeVisible();
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "close-settings");
  await settings.getByRole("button", { name: "done" }).click();
  await expect(settings).toBeHidden();
  await expect(step(page)).toHaveAttribute("data-step", "add-track");

  // Escape asks; leaving goes back to the welcome screen, not marked done.
  await page.keyboard.press("Escape");
  await bubble(page).getByRole("button", { name: "exit", exact: true }).click();
  await expect(step(page)).toHaveCount(0);
  await expect(screen).toBeVisible();
  await expect(screen.getByRole("button", { name: "record audio", exact: true })).toBeVisible();
});
