// The user docs (`/docs`, ui/src/features/docs): their own small bundle that never boots
// the engine; deep links survive a reload; search; the project menu and the palette open them
// in a new tab; phones get them too (a drawer for the sections).
import { expect, test, type Page } from "@playwright/test";
import { IPHONE } from "./mobile";
import { launch, paletteCommand, projectMenu, projectScreen } from "./ui";

const HEAVY = /\.wasm|ether_wasm|\/src\/app\.tsx|engine\/endpoint/;

/** Records requests for the app and the engine. */
function watchHeavy(page: Page): string[] {
  const heavy: string[] = [];
  page.on("request", (r) => {
    if (HEAVY.test(r.url())) heavy.push(r.url());
  });
  return heavy;
}

test("/docs: the start page, without the editor or the engine", async ({ page }) => {
  const heavy = watchHeavy(page);
  await page.goto("/docs");
  const home = page.getByTestId("docs-home");
  await expect(home).toBeVisible();
  await expect(home.getByText("start here")).toBeVisible();
  // Beginners first: the first section is the welcome.
  await expect(page.getByTestId("docs-nav").getByRole("button").first()).toHaveText(/welcome/);
  await expect(page).toHaveTitle("ethereal - documentation");
  await home.getByRole("link", { name: /start here/ }).click();
  await expect(page).toHaveURL(/\/docs\/welcome\/what-is-ethereal$/);
  await expect(page.getByTestId("docs-page")).toHaveAttribute("data-path", "welcome/what-is-ethereal");
  await expect(page).toHaveTitle(/^ethereal - .+ · documentation$/);
  await page.waitForLoadState("networkidle");
  expect(heavy).toEqual([]);
  expect(await page.evaluate(() => "__etherEngine" in window || "__ether" in window)).toBe(false);
  await expect(projectScreen(page)).toHaveCount(0);
});

test("deep links and anchors survive a reload; back and forward work", async ({ page }) => {
  await page.goto("/docs/reference/keyboard-shortcuts#the-ableton-like-preset");
  const pageView = page.getByTestId("docs-page");
  await expect(pageView).toHaveAttribute("data-path", "reference/keyboard-shortcuts");
  await expect(page.getByRole("heading", { name: /the ableton-like preset/ })).toBeInViewport();
  // The generated table: Space plays.
  await expect(page.getByTestId("docs-shortcuts").locator('[data-action="transport.play"]')).toContainText("Space");
  await page.reload();
  await expect(pageView).toHaveAttribute("data-path", "reference/keyboard-shortcuts");
  await expect(page.getByRole("heading", { name: /the ableton-like preset/ })).toBeInViewport();

  // The nav, then back.
  await page.getByTestId("docs-nav").getByRole("link", { name: "where your projects live" }).click();
  await expect(page).toHaveURL(/\/docs\/welcome\/where-your-projects-live$/);
  await page.goBack();
  await expect(pageView).toHaveAttribute("data-path", "reference/keyboard-shortcuts");
  await page.goForward();
  await expect(pageView).toHaveAttribute("data-path", "welcome/where-your-projects-live");
});

test("search finds headings and text, with the keyboard", async ({ page }) => {
  await page.goto("/docs");
  await expect(page.getByTestId("docs-home")).toBeVisible();
  await page.keyboard.press("/");
  const input = page.getByRole("combobox", { name: "search the docs" });
  await expect(input).toBeFocused();
  await input.fill("one tab at a time");
  const results = page.getByRole("listbox", { name: "search results" });
  // The heading itself ranks first (a phrase in a heading).
  await expect(results.getByRole("option").first()).toContainText("where your projects live › one tab at a time");
  await input.press("Enter");
  await expect(page).toHaveURL(/\/docs\/welcome\/where-your-projects-live#one-tab-at-a-time$/);
  await expect(page.getByRole("heading", { name: /one tab at a time/ })).toBeInViewport();
  await input.fill("zzzqqq");
  await expect(results).toContainText("nothing found");
});

test("the project menu and the palette open the docs in a new tab", async ({ page }) => {
  await launch(page);
  const [fromMenu] = await Promise.all([page.waitForEvent("popup"), projectMenu(page, "documentation")]);
  await expect(fromMenu).toHaveURL(/\/docs$/);
  await expect(fromMenu.getByTestId("docs-home")).toBeVisible();
  // The editor tab keeps its project and its engine.
  await expect(projectScreen(page)).toHaveCount(0);
  expect(await page.evaluate(() => "__etherEngine" in window)).toBe(true);
  // The docs tab doesn't take the project store from the editor (no "open in another tab").
  await expect(fromMenu.getByTestId("other-tab")).toHaveCount(0);
  await expect(page.getByTestId("other-tab")).toHaveCount(0);

  const [fromPalette] = await Promise.all([page.waitForEvent("popup"), paletteCommand(page, "documentation: keyboard shortcuts")]);
  await expect(fromPalette).toHaveURL(/\/docs\/reference\/keyboard-shortcuts$/);
  await expect(fromPalette.getByTestId("docs-shortcuts")).toBeVisible();
});

test.describe("on a phone", () => {
  test.use({ ...IPHONE, storageState: { cookies: [], origins: [] } });

  test("the docs work (no mobile gate), the sections are a drawer, nothing scrolls sideways", async ({ page }) => {
    const heavy = watchHeavy(page);
    await page.goto("/docs/welcome/first-beat");
    await expect(page.getByTestId("mobile-blocked")).toHaveCount(0);
    const nav = page.getByTestId("docs-nav");
    await expect(nav).not.toBeInViewport();
    await page.getByRole("button", { name: "sections" }).click();
    await expect(nav).toBeInViewport();
    await nav.getByRole("link", { name: "keyboard shortcuts" }).click();
    await expect(page.getByTestId("docs-page")).toHaveAttribute("data-path", "reference/keyboard-shortcuts");
    await expect(nav).not.toBeInViewport();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow).toBeLessThanOrEqual(0);
    expect(heavy).toEqual([]);
  });
});
