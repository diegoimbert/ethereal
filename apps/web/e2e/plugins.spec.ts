// Plugins on the web build: CLAP plugins are desktop-only, and the plugins panel (rail) says
// so (the app keeps working around it).
import { expect, test } from "@playwright/test";

test("the plugins tab explains plugins are desktop-only", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });

  await page.getByRole("button", { name: "Plugins", exact: true }).click();
  await expect(page.getByText("Plugins are available in the desktop app.")).toBeVisible();

  // Back to the sample browser: the shell still works.
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(page.getByText("Plugins are available in the desktop app.")).toHaveCount(0);
  expect(errors).toEqual([]);
});
