// Plugins on the web build: CLAP plugins are desktop-only, and the device picker's
// "Plugins" source says so (the app keeps working around it).
import { expect, test } from "@playwright/test";
import { createTrack, launch } from "./ui";

test("the device picker explains plugins are desktop-only", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });

  // A new track is selected: the inspector shows its device sections.
  await createTrack(page, "Audio");
  const inspector = page.getByTestId("inspector");
  await inspector.locator(".eth-devices__add-card").last().click();
  await page.getByRole("group", { name: "source" }).getByRole("button", { name: "plugins", exact: true }).click();
  await expect(page.getByText("plugins are available in the desktop app.")).toBeVisible();

  // Closing the picker: the shell still works.
  await page.keyboard.press("Escape");
  await expect(page.getByText("plugins are available in the desktop app.")).toHaveCount(0);
  expect(errors).toEqual([]);
});
