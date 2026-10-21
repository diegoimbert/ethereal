// Crash-safe launch. Nothing is opened on launch (a plugin that crashed the app
// can't take the next launch down with it): the project screen comes first, and a project
// (with its devices and plugins) loads only once picked. "Reopen last project on launch"
// (Settings > General, off by default) reopens it only after a clean close. "Open without
// plugins" (safe mode) holds plugin devices as placeholders until "Load plugins".
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createOnLaunch, createTrack, openOnLaunch, openProjectScreen, playButton, projectScreen, settingsPage } from "./ui";

interface Handle {
  state(): { project: Project | null; safeMode: string[] };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether?.state() ?? { project: null, safeMode: [] });
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
  await page.getByRole("button", { name: "settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "settings" });
  await settingsPage(settings, "general");
  const toggle = settings.getByRole("switch", { name: "reopen last project on launch" });
  await expect(toggle).toHaveAttribute("aria-checked", on ? "false" : "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", on ? "true" : "false");
  await settings.getByRole("button", { name: "done" }).click();
  await expect(settings).toHaveCount(0);
}

/** Whether the open project is a draft (the one launch opens behind the project screen). */
async function draftOpen(page: Page): Promise<boolean> {
  return page.evaluate(() => {
    const id = (window as unknown as { __ether?: { state(): { project: { id: string } | null } } }).__ether?.state().project?.id;
    const drafts = JSON.parse(localStorage.getItem("eth.project.drafts") ?? "[]") as string[];
    return !!id && drafts.includes(id);
  });
}

test("launch opens a draft under the project screen; Recents opens a project", async ({ page }) => {
  test.setTimeout(120_000);
  const errors = collectErrors(page);

  await page.goto("/");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  // Behind it, a draft project named after today's date is open, so the arranger is
  // there from the start. Untouched, a draft is left out of the project lists.
  await expect.poll(() => draftOpen(page), { timeout: 10_000 }).toBe(true);
  await expect(page.getByTestId("project-name")).not.toHaveText("no project");
  await expect(screen.getByRole("button", { name: "new project" })).toBeVisible();
  const draft = (await page.getByTestId("project-name").textContent())!;

  // Dismissing the screen keeps that draft.
  await page.keyboard.press("Escape");
  await expect(screen).toHaveCount(0);
  await expect(page.getByTestId("project-name")).toHaveText(draft);
  await expect(playButton(page)).toBeEnabled();
  await openProjectScreen(page);
  await expect(screen.getByRole("button", { name: `open ${draft}`, exact: true })).toHaveCount(0);

  await createOnLaunch(page, "Launch song");
  const empty = await trackCount(page);
  await createTrack(page, "Midi");
  await expect.poll(() => trackCount(page)).toBe(empty + 1);
  await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0, { timeout: 10_000 });
  const id = await projectId(page);

  // Next launch (the app's address, not the project's): nothing reopens (the setting is off by
  // default, a new draft opens); Recents opens it.
  await page.goto("/");
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => draftOpen(page), { timeout: 10_000 }).toBe(true);
  // The draft left untouched was deleted.
  await expect(projectScreen(page).getByRole("button", { name: `open ${draft}`, exact: true })).toHaveCount(0);
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
  // A new project is named after the date, then renamed: the rename is a document
  // edit, saved by the autosave a moment later. Wait for it before closing.
  await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0, { timeout: 10_000 });
  await setReopenLast(page, true);

  // A clean close (the tab unloads): the last project reopens, no project screen.
  await page.goto("/");
  await expect.poll(() => projectId(page), { timeout: 30_000 }).toBe(id);
  await expect(page.getByTestId("project-name")).toHaveText("Reopen me");
  await expect(projectScreen(page)).toHaveCount(0);

  // A crash: nothing is opened, the crash dialog offers the project (with safe open).
  await crashAndReload(page);
  const crashed = page.getByRole("dialog", { name: "ethereal didn’t close properly" });
  await expect(crashed).toBeVisible({ timeout: 30_000 });
  await expect(crashed).toContainText("Reopen me");
  await expect(crashed.getByRole("button", { name: "open Reopen me without plugins" })).toBeVisible();
  await expect.poll(() => draftOpen(page), { timeout: 10_000 }).toBe(true);
  // Deciding later: the project screen is next.
  await crashed.getByRole("button", { name: "decide later" }).click();
  await expect(projectScreen(page)).toBeVisible();
  await expect.poll(() => draftOpen(page), { timeout: 10_000 }).toBe(true);
  await openOnLaunch(page, "Reopen me");

  // That session closes cleanly again: reopened.
  await page.goto("/");
  await expect.poll(() => projectId(page), { timeout: 30_000 }).toBe(id);

  // The setting off: nothing reopens.
  await setReopenLast(page, false);
  await page.goto("/");
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => draftOpen(page), { timeout: 10_000 }).toBe(true);

  expect(errors).toEqual([]);
});

test("open without plugins: safe mode from Recents and after a crash, Load plugins leaves it", async ({ page }) => {
  test.setTimeout(120_000);
  const errors = collectErrors(page);

  await page.goto("/");
  await createOnLaunch(page, "Safe song");
  await createTrack(page, "Midi");
  await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0, { timeout: 10_000 });
  await page.goto("/");

  // Recents > More actions > Open without plugins.
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "more actions for Safe song" }).click();
  await page.getByRole("menuitem", { name: "open without plugins" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Safe song");
  const banner = page.getByTestId("safe-mode-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("plugins disabled (safe mode)");
  // Opening it changed nothing: not dirty.
  await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0);
  await banner.getByRole("button", { name: "load plugins" }).click();
  await expect(banner).toHaveCount(0);

  // After a crash, the crash dialog opens it safely too.
  await crashAndReload(page);
  const crashed = page.getByRole("dialog", { name: "ethereal didn’t close properly" });
  await expect(crashed).toBeVisible({ timeout: 30_000 });
  await crashed.getByRole("button", { name: "open Safe song without plugins" }).click();
  await expect(crashed).toHaveCount(0);
  await expect(page.getByTestId("project-name")).toHaveText("Safe song");
  await expect(page.getByTestId("safe-mode-banner")).toBeVisible();
  await expect(projectScreen(page)).toHaveCount(0);

  expect(errors).toEqual([]);
});
