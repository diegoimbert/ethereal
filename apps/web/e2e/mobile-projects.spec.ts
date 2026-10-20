// Projects and settings on touch screens: the project screen is a full-screen single
// column (projects first, row actions shown without hover); new / open / rename by touch; the
// project URL (/p/…) survives a reload, which keeps the mobile UI; Back returns to the
// previous project; settings are touch-sized.
import { expect, test, type Page } from "@playwright/test";
import { IPHONE, launchMobile, mobileShell, settled, shot } from "./mobile";
import { projectScreen } from "./ui";

test.use(IPHONE);

const projectName = (page: Page) => page.getByTestId("project-name");

async function minTarget(page: Page, scope: string): Promise<Array<{ name: string; h: number }>> {
  return page.evaluate(
    (sel) =>
      [...document.querySelectorAll<HTMLElement>(`${sel} button`)]
        .filter((b) => b.offsetParent !== null)
        .map((b) => ({ name: b.getAttribute("aria-label") ?? b.textContent ?? "", h: b.getBoundingClientRect().height }))
        .filter((b) => b.h < 44),
    scope,
  );
}

test("the project screen: full-screen, new project, rename by tap, the URL survives a reload in the mobile UI", async ({ page }) => {
  await page.goto("/?mobile=1");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await settled(page);
  const box = (await screen.boundingBox())!;
  expect(box.width).toBe(page.viewportSize()!.width);
  await shot(page, "project-screen");
  await screen.getByRole("button", { name: /new project/i }).tap();
  await expect(screen).toHaveCount(0);
  await expect(page).toHaveURL(/\/p\//);
  // Rename by tapping the name (a 16 px field: no iOS zoom on focus).
  await projectName(page).tap();
  const field = page.getByRole("textbox", { name: /project name/i });
  expect(await field.evaluate((el) => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(16);
  await field.fill("Night drive");
  await field.press("Enter");
  await expect(projectName(page)).toHaveText("Night drive");
  await expect(page).toHaveURL(/\/p\/night-drive\//);
  // Saved (autosave), then a reload opens the same project, still in the mobile UI (the flag
  // is kept for the tab).
  await expect.poll(() => page.evaluate(() => (window as unknown as { __ether: { state(): { dirty: boolean } } }).__ether.state().dirty)).toBe(false);
  await page.reload();
  await expect(mobileShell(page)).toBeVisible({ timeout: 30_000 });
  await expect(projectName(page)).toHaveText("Night drive", { timeout: 30_000 });
});

test("open another project from the menu; Back returns to the first", async ({ page }) => {
  await launchMobile(page);
  await projectName(page).tap();
  await page.getByRole("textbox", { name: /project name/i }).fill("first");
  await page.getByRole("textbox", { name: /project name/i }).press("Enter");
  await expect(projectName(page)).toHaveText("first");
  const firstUrl = page.url();
  await page.getByRole("button", { name: /^projects$/i }).tap();
  await page.getByRole("menuitem", { name: /^new project/i }).tap();
  await expect(projectName(page)).not.toHaveText("first");
  await projectName(page).tap();
  await page.getByRole("textbox", { name: /project name/i }).fill("second");
  await page.getByRole("textbox", { name: /project name/i }).press("Enter");
  await expect(projectName(page)).toHaveText("second");
  // Open "first" from the project screen (its row's open button).
  await page.getByRole("button", { name: /^projects$/i }).tap();
  await page.getByRole("menuitem", { name: /^open project/i }).tap();
  const screen = projectScreen(page);
  await expect(screen).toBeVisible();
  await settled(page);
  expect(await minTarget(page, ".eth-project-screen__item")).toEqual([]);
  await screen.getByRole("button", { name: "open first", exact: false }).first().tap();
  await expect(projectName(page)).toHaveText("first");
  expect(page.url()).toBe(firstUrl);
  // Back: the previous project.
  await page.goBack();
  await expect(projectName(page)).toHaveText("second");
});

test("settings at touch size; settings full-screen", async ({ page }) => {
  await launchMobile(page);
  await page.getByRole("button", { name: /^settings$/i }).tap();
  const dialog = page.getByRole("dialog", { name: /settings/i });
  await expect(dialog).toBeVisible();
  await settled(page);
  expect((await dialog.boundingBox())!.width).toBe(page.viewportSize()!.width);
  await shot(page, "settings");
});
