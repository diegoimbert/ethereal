// The web app asks the browser to keep its storage (the projects in OPFS) permanently,
// says once when it won't, and Settings > General shows the state and the usage.
import { expect, test, type Page } from "@playwright/test";
import { launch } from "./ui";

/** The browser's answer to `persist()` (Playwright's Chromium refuses; a real one may grant). */
async function browserGrants(page: Page, grants: boolean): Promise<void> {
  await page.addInitScript((grants) => {
    let persisted = false;
    const storage = navigator.storage;
    Object.defineProperty(storage, "persisted", { configurable: true, value: () => Promise.resolve(persisted) });
    Object.defineProperty(storage, "persist", {
      configurable: true,
      value: () => {
        (window as unknown as { __persistAsked?: number }).__persistAsked = ((window as unknown as { __persistAsked?: number }).__persistAsked ?? 0) + 1;
        persisted = grants;
        return Promise.resolve(grants);
      },
    });
    // This spec tests the notice: forget the config's "already shown", once per tab.
    if (!sessionStorage.getItem("persist-spec")) {
      sessionStorage.setItem("persist-spec", "1");
      localStorage.removeItem("eth.storage.persist-notice");
    }
  }, grants);
}

const generalSettings = async (page: Page) => {
  await page.getByRole("button", { name: "settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "settings" });
  await settings.getByRole("tab", { name: "general" }).click();
  return settings.getByTestId("settings-storage");
};

test("granted: storage is persistent, settings say so with the usage, no notice", async ({ page }) => {
  await browserGrants(page, true);
  await launch(page);
  await expect.poll(() => page.evaluate(() => (window as unknown as { __persistAsked?: number }).__persistAsked)).toBe(1);
  const storage = await generalSettings(page);
  await expect(storage.getByRole("status")).toHaveText("storage: persistent: the browser keeps your projects");
  await expect(storage).toContainText(/using .+ of .+ available to ethereal in this browser/);
  await expect(storage.getByRole("button", { name: "request again" })).toHaveCount(0);
  await expect(page.getByText("projects may be cleared")).toHaveCount(0);
});

test("refused: asked again on the first click, then a notice once that opens the settings", async ({ page }) => {
  await browserGrants(page, false);
  await launch(page);
  // Launch asked, the first click (creating the project) asked again: refused both times.
  await expect.poll(() => page.evaluate(() => (window as unknown as { __persistAsked?: number }).__persistAsked)).toBe(2);
  const notice = page.getByText("projects may be cleared");
  await expect(notice).toBeVisible({ timeout: 10_000 });
  await notice.click();
  const storage = page.getByRole("dialog", { name: "settings" }).getByTestId("settings-storage");
  await expect(storage.getByRole("status")).toHaveText("storage: may be cleared by the browser when space runs low");
  await expect(storage).toContainText("the desktop app or exporting your projects keeps them safe");
  await storage.getByRole("button", { name: "request again" }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __persistAsked?: number }).__persistAsked)).toBe(3);

  // Once only: not after a reload.
  await page.reload();
  await expect(page.getByTestId("project-name")).not.toHaveText("no project", { timeout: 30_000 });
  await page.mouse.click(5, 5);
  await expect.poll(() => page.evaluate(() => (window as unknown as { __persistAsked?: number }).__persistAsked)).toBe(2);
  await expect(page.getByText("projects may be cleared")).toHaveCount(0);
});
