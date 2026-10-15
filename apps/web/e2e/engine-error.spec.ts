// #597: when the web engine can't start, the editor isn't left blank: a screen says why, with
// "retry" (which works once the cause is gone), "report an issue" and "reload".
import { expect, test } from "@playwright/test";
import { projectScreen } from "./ui";

test("the wasm doesn't load: the reason, then retry opens the app", async ({ page }) => {
  let block = true;
  await page.route(/\.wasm$/, (route) => (block ? route.fulfill({ status: 404, body: "" }) : route.continue()));
  await page.goto("/");
  const screen = page.getByTestId("engine-error");
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await expect(screen.getByRole("heading")).toHaveText("the audio engine didn't start");
  await expect(screen.getByRole("alert")).toContainText("404");
  await expect(screen.getByRole("button", { name: "report an issue" })).toBeVisible();
  await expect(screen.getByRole("button", { name: "reload" })).toBeVisible();

  block = false;
  await screen.getByRole("button", { name: "retry" }).click();
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("engine-error")).toHaveCount(0);
  // The engine runs: launch opened its draft.
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __ether: { state(): { project: unknown } } }).__ether.state().project !== null))
    .toBe(true);
});

test("report an issue from the error screen opens the form", async ({ page }) => {
  await page.route(/\.wasm$/, (route) => route.fulfill({ status: 404, body: "" }));
  await page.goto("/");
  const screen = page.getByTestId("engine-error");
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "report an issue" }).click();
  await expect(page.getByRole("dialog", { name: "report an issue" })).toBeVisible();
});

test("the engine's code doesn't load (a page left open across a deploy): a new version is out", async ({ page }) => {
  await page.route(/\/engine\/endpoint\.ts/, (route) => route.abort());
  await page.goto("/");
  const screen = page.getByTestId("engine-error");
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await expect(screen.getByRole("heading")).toHaveText("a new version of ethereal is out");
  await expect(screen.getByRole("button", { name: "reload" })).toBeVisible();
});
