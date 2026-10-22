// Leaving an untitled project (the launch draft, changed but never named) asks to keep or
// discard it (#233); discard deletes it from the store.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createOnLaunch, createTrack, openProjectScreen, projectScreen } from "./ui";

const projectId = (page: Page) =>
  page.evaluate(() => (window as unknown as { __ether?: { state(): { project: Project | null } } }).__ether?.state().project?.id ?? null);

/** From a named project: a new draft (the project menu), changed, saved. Returns its name. */
async function untitledDraft(page: Page): Promise<string> {
  const before = await projectId(page);
  await page.getByRole("button", { name: "projects" }).click();
  await page.getByRole("menu", { name: "project" }).getByRole("menuitem", { name: "new project" }).click();
  await expect.poll(() => projectId(page)).not.toBe(before);
  await createTrack(page, "Midi");
  await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0, { timeout: 10_000 });
  return (await page.getByTestId("project-name").textContent())!;
}

test("opening another project from an untitled one asks: discard deletes it, keep names it", async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto("/");
  await createOnLaunch(page, "Other song");

  // Discard: the untitled project goes, the other one opens.
  const first = await untitledDraft(page);
  let screen = await openProjectScreen(page);
  await screen.getByRole("button", { name: "open Other song", exact: true }).click();
  let dialog = page.getByRole("dialog", { name: "discard this untitled project?" });
  await expect(dialog).toContainText(first);
  await dialog.getByRole("button", { name: "discard" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Other song");
  screen = await openProjectScreen(page);
  await expect(screen.getByRole("button", { name: `open ${first}`, exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(screen).toHaveCount(0);

  // Keep with a name: listed under that name, and no question the next time.
  await untitledDraft(page);
  screen = await openProjectScreen(page);
  await screen.getByRole("button", { name: "open Other song", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "discard this untitled project?" });
  await dialog.getByRole("textbox", { name: "project name" }).fill("Kept song");
  await dialog.getByRole("button", { name: "keep" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Other song");
  screen = await openProjectScreen(page);
  await screen.getByRole("button", { name: "open Kept song", exact: true }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Kept song");
  screen = await openProjectScreen(page);
  await screen.getByRole("button", { name: "open Other song", exact: true }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Other song");
  await expect(page.getByRole("dialog", { name: "discard this untitled project?" })).toHaveCount(0);
  await expect(projectScreen(page)).toHaveCount(0);
});
