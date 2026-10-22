// Plugins on the web build: native plugins (CLAP, VST3, AU) are desktop-only; the web
// version lists its WebCLAP plugins, and the device picker's "Plugins" source says where to
// install them while there are none (the app keeps working around it). webclap.spec.ts
// installs and plays some.
import { expect, test } from "@playwright/test";
import { createTrack, launch } from "./ui";

test("the device picker says where to install webclap plugins", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });

  // A new track is selected: the inspector shows its device sections.
  await createTrack(page, "Audio");
  const inspector = page.getByTestId("inspector");
  await inspector.locator(".eth-devices__add-card").last().click();
  await page.getByRole("group", { name: "source" }).getByRole("button", { name: "plugins", exact: true }).click();
  await expect(page.getByText("no webclap plugins yet: install some in the settings.")).toBeVisible();
  await expect(page.getByRole("button", { name: "webclap plugins…" })).toBeVisible();

  // Closing the picker: the shell still works.
  await page.keyboard.press("Escape");
  await expect(page.getByText("no webclap plugins yet: install some in the settings.")).toHaveCount(0);
  expect(errors).toEqual([]);
});
