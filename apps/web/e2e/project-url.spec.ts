// The open project in the address bar: `/p/<slug>/<id>`. A rename updates the slug, a
// reload reopens the project, a wrong slug is corrected, Back opens the previous project, and
// an id this browser doesn't have shows the picker with a notice. The tab title follows.
import { expect, test, type Page } from "@playwright/test";
import { launch, newProject, projectScreen, renameProject } from "./ui";

const projectId = (page: Page) =>
  page.evaluate(() => (window as unknown as { __ether?: { state(): { project: { id: string } | null } } }).__ether?.state().project?.id ?? null);
const path = (page: Page) => new URL(page.url()).pathname;

test("the project's URL: rename, reload, wrong slug, Back", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, "First Song");
  const first = (await projectId(page))!;
  await expect.poll(() => path(page)).toBe(`/p/first-song/${first}`);
  // The tab title is the project's name, `• ` in front while unsaved.
  await expect(page).toHaveTitle(/^(• )?ethereal - First Song$/);

  // Rename: the slug follows, in place.
  await renameProject(page, "Café Déjà Vu!");
  await expect.poll(() => path(page)).toBe(`/p/cafe-deja-vu/${first}`);
  // Saved (the autosave), so the reload finds the new name.
  await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0, { timeout: 10_000 });
  await expect(page).toHaveTitle("ethereal - Café Déjà Vu!");

  // Reload: the same project opens by itself, no picker.
  await page.reload();
  await expect.poll(() => projectId(page), { timeout: 30_000 }).toBe(first);
  await expect(page.getByTestId("project-name")).toHaveText("Café Déjà Vu!");
  await expect(projectScreen(page)).toHaveCount(0);
  expect(path(page)).toBe(`/p/cafe-deja-vu/${first}`);

  // A wrong slug (query kept): the project opens and the slug is corrected.
  await page.goto(`/p/some-old-name/${first}?force-desktop`);
  await expect.poll(() => projectId(page), { timeout: 30_000 }).toBe(first);
  await expect.poll(() => path(page)).toBe(`/p/cafe-deja-vu/${first}`);
  expect(new URL(page.url()).search).toBe("?force-desktop");

  // Another project: a new history entry; Back returns to the first one.
  await newProject(page, "Second");
  const second = (await projectId(page))!;
  expect(second).not.toBe(first);
  await expect.poll(() => path(page)).toBe(`/p/second/${second}`);
  await expect(page).toHaveTitle(/^(• )?ethereal - Second$/);
  await page.goBack();
  await expect.poll(() => projectId(page), { timeout: 30_000 }).toBe(first);
  await expect(page.getByTestId("project-name")).toHaveText("Café Déjà Vu!");
  expect(path(page)).toBe(`/p/cafe-deja-vu/${first}`);
  await expect(page).toHaveTitle("ethereal - Café Déjà Vu!");
  await page.goForward();
  await expect.poll(() => projectId(page), { timeout: 30_000 }).toBe(second);

  // Not stored in this browser: the picker says so and the URL becomes /.
  await page.goto("/p/whatever/00000000-0000-7000-8000-000000000000");
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("project-screen-notice")).toHaveText("this project isn't saved in this browser");
  await expect.poll(() => path(page)).toBe("/");
  await expect(page).toHaveTitle("ethereal");

  expect(errors).toEqual([]);
});
