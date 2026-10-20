// The launch picker paints with the app, before the engine is up (the wasm download,
// its compile and the engine's start take seconds on a slow network): its actions wait for
// the engine, then work. Where launch opens something else (a project URL, the crash dialog)
// it never flashes.
import { expect, test, type Page } from "@playwright/test";
import { createOnLaunch, projectScreen } from "./ui";

/** Records whether the launch picker (the welcome screen) ever showed, from the first byte on. */
async function watchPicker(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const w = window as unknown as { __pickerSeen?: number };
    new MutationObserver(() => {
      if (w.__pickerSeen === undefined && document.querySelector(".eth-project-screen--welcome")) w.__pickerSeen = performance.now();
    }).observe(document, { subtree: true, childList: true });
  });
}

const pickerSeen = (page: Page) => page.evaluate(() => (window as unknown as { __pickerSeen?: number }).__pickerSeen ?? null);

/** The autosave wrote the open project (its rename): a reload finds it. */
const saved = (page: Page) => expect(page.getByRole("status", { name: "Unsaved changes" })).toHaveCount(0, { timeout: 10_000 });

/** The dialog's opening animation is over (the placeholders' pulse goes on). */
const settled = (page: Page) =>
  page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((a) => a.effect?.getTiming().iterations !== Infinity)
        .map((a) => a.finished),
    ),
  );

/** Holds back the engine's wasm until `release()`. */
async function holdWasm(page: Page): Promise<() => void> {
  let release!: () => void;
  const released = new Promise<void>((r) => (release = r));
  await page.route(/\.wasm$/, async (route) => {
    await released;
    await route.continue();
  });
  return release;
}

test("the picker shows before the engine is up; its actions enable once it is", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // A stored project first, for the list's placeholder rows.
  await page.goto("/");
  await createOnLaunch(page, "Early bird");
  await saved(page);
  await page.goto("/");
  const screen = projectScreen(page);
  await expect(screen.getByRole("button", { name: "open Early bird" })).toBeVisible({ timeout: 30_000 });

  const release = await holdWasm(page);
  await page.goto("/");
  // Within a few seconds of the page on this (unthrottled, but loaded) test machine, with no
  // engine at all: the engine's own start is what used to hold it back.
  await expect(screen).toBeVisible({ timeout: 5_000 });
  const shownAt = await page.evaluate(() => performance.now());
  expect(
    await page.evaluate(() => "__etherEngine" in window && (window as unknown as { __ether?: { state(): { project: unknown } } }).__ether?.state().project),
  ).toBeFalsy();
  await expect(screen.getByTestId("project-screen-loading")).toHaveText("loading engine…");
  await expect(screen.getByRole("button", { name: "new project" })).toBeDisabled();
  await expect(screen.getByTestId("project-list-skeleton").locator("li")).toHaveCount(1);
  await settled(page);
  const before = await screen.boundingBox();

  release();
  await expect(screen.getByRole("button", { name: "new project" })).toBeEnabled({ timeout: 30_000 });
  await expect(screen.getByTestId("project-screen-loading")).toHaveCount(0);
  await expect(screen.getByRole("button", { name: "open Early bird" })).toBeVisible();
  await settled(page);
  // The list filled in its placeholder's room: the picker didn't move or resize.
  expect(await screen.boundingBox()).toEqual(before);
  test.info().annotations.push({ type: "picker shown (ms after navigation start)", description: String(Math.round(shownAt)) });

  // And the actions work.
  await screen.getByRole("button", { name: "open Early bird" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Early bird");
  expect(errors).toEqual([]);
});

test("no picker flash on a project URL or after a crash", async ({ page }) => {
  test.setTimeout(120_000);
  await watchPicker(page);
  await page.goto("/");
  await createOnLaunch(page, "Linked");
  await expect.poll(() => new URL(page.url()).pathname).toMatch(/^\/p\/linked\//);
  await saved(page);
  const url = page.url();

  // /p/…: opens that project, no picker.
  await page.goto(url);
  await expect(page.getByTestId("project-name")).toHaveText("Linked", { timeout: 30_000 });
  await page.waitForTimeout(500);
  expect(await pickerSeen(page)).toBeNull();

  // After a crash: the crash dialog first, no picker behind it until it is decided.
  await page.evaluate(() => {
    window.addEventListener("pagehide", () => localStorage.setItem("eth.session", "open"));
    (window as unknown as { __etherEngine: { handles(): { controller: Worker } | null } }).__etherEngine.handles()?.controller.terminate();
  });
  await page.goto("/");
  const crashed = page.getByRole("dialog", { name: "ethereal didn’t close properly" });
  await expect(crashed).toBeVisible({ timeout: 30_000 });
  expect(await pickerSeen(page)).toBeNull();
  await crashed.getByRole("button", { name: "decide later" }).click();
  await expect(projectScreen(page)).toBeVisible();
});
