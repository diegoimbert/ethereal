// base-131: crash-safe launch. Nothing is opened on launch (a plugin that crashed the app
// can't take the next launch down with it): the project screen comes first, and a project
// (with its devices and plugins) loads only once picked. "Reopen last project on launch"
// (Settings > General, off by default) reopens it only after a clean close. "Open without
// plugins" (safe mode) holds plugin devices as placeholders until "Load plugins".
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createOnLaunch, createTrack, openOnLaunch, playButton, projectScreen } from "./ui";

interface Handle {
  state(): { project: Project | null; safeMode: string[] };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state());
const projectId = (page: Page) => state(page).then((s) => s.project?.id ?? null);

/** Console errors and page errors, for the "no errors with no project" checks. */
function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

/**
 * A crash, then the app is loaded again: the controller Worker is killed (its session
 * markers stay) and the page never records a clean close (a crashed tab never runs its
 * `pagehide`: this listener, added after the app's, undoes what the app's records).
 */
async function crashAndReload(page: Page): Promise<void> {
  await page.evaluate(() => {
    window.addEventListener("pagehide", () => localStorage.setItem("eth.session", "open"));
    (window as unknown as { __etherEngine: { handles(): { controller: Worker } | null } }).__etherEngine.handles()?.controller.terminate();
  });
  await page.reload();
}

const trackCount = (page: Page) => state(page).then((s) => (s.project ? Object.keys(s.project.tracks).length : -1));

async function setReopenLast(page: Page, on: boolean): Promise<void> {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings" });
  await settings.getByRole("tab", { name: "General" }).click();
  const toggle = settings.getByRole("switch", { name: "Reopen last project on launch" });
  await expect(toggle).toHaveAttribute("aria-checked", on ? "false" : "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", on ? "true" : "false");
  await settings.getByRole("button", { name: "Done" }).click();
  await expect(settings).toHaveCount(0);
}

test("launch opens nothing: the project screen comes first, Recents opens a project", async ({ page }) => {
  test.setTimeout(120_000);
  const errors = collectErrors(page);

  await page.goto("/");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  // Nothing open: no project in the engine nor the UI; the screen has no "Continue".
  expect(await projectId(page)).toBeNull();
  await expect(page.getByTestId("project-name")).toHaveText("No project");
  await expect(screen.getByRole("button", { name: "Continue" })).toHaveCount(0);
  await expect(screen.getByRole("button", { name: "New project" })).toBeVisible();

  // With nothing open, the rest of the app stays calm: dismiss the screen, try the
  // shortcuts and the disabled transport.
  await page.keyboard.press("Escape");
  await expect(screen).toHaveCount(0);
  await expect(playButton(page)).toBeDisabled();
  await expect(page.getByTestId("share-button")).toBeDisabled();
  await page.keyboard.press("Space");
  await page.keyboard.press("Control+z");
  await expect(page.getByText("No project open")).toBeVisible();
  await page.getByRole("button", { name: "Open a project" }).click();
  await expect(screen).toBeVisible();

  await createOnLaunch(page, "Launch song");
  const empty = await trackCount(page);
  await createTrack(page, "Midi");
  await expect.poll(() => trackCount(page)).toBe(empty + 1);
  await expect(page.getByRole("status", { name: "Unsaved changes" })).toHaveCount(0, { timeout: 10_000 });
  const id = await projectId(page);

  // Next launch: nothing reopens (the setting is off by default); Recents opens it.
  await page.reload();
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
  expect(await projectId(page)).toBeNull();
  await openOnLaunch(page, "Launch song");
  expect(await projectId(page)).toBe(id);
  await expect.poll(() => trackCount(page)).toBe(empty + 1);
  await expect(playButton(page)).toBeEnabled();

  expect(errors).toEqual([]);
});

test("reopen last project on launch: only after a clean close", async ({ page }) => {
  test.setTimeout(120_000);
  const errors = collectErrors(page);

  await page.goto("/");
  await createOnLaunch(page, "Reopen me");
  const id = await projectId(page);
  await setReopenLast(page, true);

  // A clean close (the tab unloads): the last project reopens, no project screen.
  await page.reload();
  await expect.poll(() => projectId(page), { timeout: 30_000 }).toBe(id);
  await expect(page.getByTestId("project-name")).toHaveText("Reopen me");
  await expect(projectScreen(page)).toHaveCount(0);

  // A crash: nothing is opened, the crash dialog offers the project (with safe open).
  await crashAndReload(page);
  const crashed = page.getByRole("dialog", { name: "Ethereal didn’t close properly" });
  await expect(crashed).toBeVisible({ timeout: 30_000 });
  await expect(crashed).toContainText("Reopen me");
  await expect(crashed.getByRole("button", { name: "Open Reopen me without plugins" })).toBeVisible();
  expect(await projectId(page)).toBeNull();
  // Deciding later: the project screen is next.
  await crashed.getByRole("button", { name: "Decide later" }).click();
  await expect(projectScreen(page)).toBeVisible();
  expect(await projectId(page)).toBeNull();
  await openOnLaunch(page, "Reopen me");

  // That session closes cleanly again: reopened.
  await page.reload();
  await expect.poll(() => projectId(page), { timeout: 30_000 }).toBe(id);

  // The setting off: nothing reopens.
  await setReopenLast(page, false);
  await page.reload();
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
  expect(await projectId(page)).toBeNull();

  expect(errors).toEqual([]);
});

test("open without plugins: safe mode from Recents and after a crash, Load plugins leaves it", async ({ page }) => {
  test.setTimeout(120_000);
  const errors = collectErrors(page);

  await page.goto("/");
  await createOnLaunch(page, "Safe song");
  await createTrack(page, "Midi");
  await expect(page.getByRole("status", { name: "Unsaved changes" })).toHaveCount(0, { timeout: 10_000 });
  await page.reload();

  // Recents > More actions > Open without plugins.
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "More actions for Safe song" }).click();
  await page.getByRole("menuitem", { name: "Open without plugins" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Safe song");
  const banner = page.getByTestId("safe-mode-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("Plugins disabled (safe mode)");
  // Opening it changed nothing: not dirty.
  await expect(page.getByRole("status", { name: "Unsaved changes" })).toHaveCount(0);
  await banner.getByRole("button", { name: "Load plugins" }).click();
  await expect(banner).toHaveCount(0);

  // After a crash, the crash dialog opens it safely too.
  await crashAndReload(page);
  const crashed = page.getByRole("dialog", { name: "Ethereal didn’t close properly" });
  await expect(crashed).toBeVisible({ timeout: 30_000 });
  await crashed.getByRole("button", { name: "Open Safe song without plugins" }).click();
  await expect(crashed).toHaveCount(0);
  await expect(page.getByTestId("project-name")).toHaveText("Safe song");
  await expect(page.getByTestId("safe-mode-banner")).toBeVisible();
  await expect(projectScreen(page)).toHaveCount(0);

  expect(errors).toEqual([]);
});
