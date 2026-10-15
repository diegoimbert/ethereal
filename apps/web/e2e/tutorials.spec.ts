// Guided tours from the welcome screen (docs/TUTORIALS.md): the jam tour end to end (its
// project, the share window opened by the user, clicks outside the highlight blocked, the
// check mark and "do it again?"), and the start of the record tour in the browser (settings,
// the audio tab, the browser's note).
import { expect as baseExpect, test, type Page } from "@playwright/test";
import { openProjectScreen, projectScreen } from "./ui";

// A loaded machine (the merge gate) can take a few seconds per step.
const expect = baseExpect.configure({ timeout: 15_000 });

const bubble = (page: Page) => page.getByTestId("tour-bubble");
const next = (page: Page) => bubble(page).getByRole("button", { name: "next", exact: true });
const step = (page: Page) => page.getByTestId("tour");

test("jam with your friends: a guided tour from the welcome screen and back", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "jam with your friends", exact: true }).click();

  // The welcome screen hides; the tour runs in a project of its own.
  await expect(screen).toBeHidden();
  await expect(bubble(page)).toContainText("make music together", { timeout: 30_000 });
  await expect(page.getByTestId("project-name")).toHaveText("jam tutorial");
  await next(page).click();

  // Waits for the share button; nothing else takes clicks meanwhile.
  await expect(step(page)).toHaveAttribute("data-step", "share-button");
  await expect(next(page)).toHaveCount(0);
  await page.getByRole("button", { name: "projects" }).click({ force: true });
  await expect(page.getByRole("menu", { name: "project" })).toHaveCount(0);
  await page.getByTestId("share-button").click();
  await expect(step(page)).toHaveAttribute("data-step", "modes");
  await expect(page.getByTestId("share-popover")).toBeVisible();
  // The bubble's buttons don't close the share window.
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "start");
  await expect(page.getByTestId("share-popover")).toBeVisible();
  // Not a waiting step: starting a session from here is blocked.
  await page.getByTestId("share-start").click({ force: true });
  await expect(page.getByTestId("share-start")).toBeVisible();
  // Back and forth works.
  await bubble(page).getByRole("button", { name: "back" }).click();
  await expect(step(page)).toHaveAttribute("data-step", "modes");
  await next(page).click();
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "together");
  await expect(page.getByTestId("share-popover")).toHaveCount(0);
  for (const id of ["listen", "offline", "join", "done"]) {
    await next(page).click();
    await expect(step(page)).toHaveAttribute("data-step", id);
  }
  await expect(bubble(page)).toContainText("stays in your projects");
  await bubble(page).getByRole("button", { name: "finish" }).click();

  // Back on the welcome screen, done.
  await expect(step(page)).toHaveCount(0);
  await expect(screen).toBeVisible();
  const done = screen.getByRole("button", { name: "jam with your friends (completed)" });
  await expect(done).toBeVisible();
  await done.click();
  await expect(screen.getByTestId("tutorial-confirm-jam")).toContainText("you already completed this tutorial. do it again?");
  await screen.getByRole("button", { name: "cancel" }).click();
  await expect(done).toBeVisible();

  // Kept after a reload (the address bar reopens the tutorial's project).
  await page.reload();
  await expect(page.getByTestId("project-name")).toHaveText("jam tutorial", { timeout: 30_000 });
  const again = await openProjectScreen(page);
  await expect(again.getByRole("button", { name: "jam with your friends (completed)" })).toBeVisible();
  expect(errors).toEqual([]);
});

test("record audio in the browser: settings, the audio tab, and an exit on Escape", async ({ page }) => {
  await page.goto("/");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "record audio", exact: true }).click();
  await expect(bubble(page)).toContainText("ethereal can't record yet", { timeout: 30_000 });
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "interface");
  await next(page).click();
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "open-settings");
  await page.getByRole("button", { name: "settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "settings", exact: true });
  await expect(settings).toBeVisible();
  // Opened on its last tab (audio by default): that step is already done.
  await expect(step(page)).toHaveAttribute("data-step", "browser-devices");
  await expect(bubble(page)).toContainText("your system chooses");
  // The settings' own Done is not this step's: blocked.
  await settings.getByRole("button", { name: "done" }).click({ force: true });
  await expect(settings).toBeVisible();
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "close-settings");
  await settings.getByRole("button", { name: "done" }).click();
  await expect(settings).toBeHidden();
  await expect(step(page)).toHaveAttribute("data-step", "add-track");

  // Escape asks; leaving goes back to the welcome screen, not marked done.
  await page.keyboard.press("Escape");
  await bubble(page).getByRole("button", { name: "exit", exact: true }).click();
  await expect(step(page)).toHaveCount(0);
  await expect(screen).toBeVisible();
  await expect(screen.getByRole("button", { name: "record audio", exact: true })).toBeVisible();
});

test("the highlight of a part inside a menu is drawn above the menu", async ({ page }) => {
  await page.goto("/");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "bring a project from another DAW", exact: true }).click();
  await expect(bubble(page)).toContainText("bring your songs with you", { timeout: 30_000 });
  await next(page).click();
  await next(page).click();
  await expect(step(page)).toHaveAttribute("data-step", "menu");
  await page.getByRole("button", { name: "projects" }).click();
  const menu = page.getByRole("menu", { name: "project" });
  await expect(menu).toBeVisible();
  await expect(step(page)).toHaveAttribute("data-step", "entry");
  const ring = page.getByTestId("tour-ring");
  await expect(ring).toHaveCount(1);
  // The ring settles around the entry (it animates there from the menu button).
  const entry = menu.locator('[data-tour="import-daw"]');
  await expect
    .poll(async () => {
      const [r, e] = [await ring.boundingBox(), await entry.boundingBox()];
      return !!r && !!e && r.x <= e.x && r.y <= e.y && r.x + r.width >= e.x + e.width && r.y + r.height >= e.y + e.height;
    })
    .toBe(true);
  // A point of the ring's border over the menu: the ring is on top there. The ring takes no
  // presses (they go through to the entry), so hit-testing is turned on just for this check.
  const hit = await page.evaluate(() => {
    const ringEl = document.querySelector<HTMLElement>('[data-testid="tour-ring"]')!;
    const menuEl = document.querySelector('[data-tour="project-menu-panel"]')!;
    const r = ringEl.getBoundingClientRect();
    const m = menuEl.getBoundingClientRect();
    const inMenu = ([x, y]: [number, number]) => x > m.left && x < m.right && y > m.top && y < m.bottom;
    const edges: [number, number][] = [
      [r.left + 0.5, r.top + r.height / 2],
      [r.right - 0.5, r.top + r.height / 2],
      [r.left + r.width / 2, r.top + 0.5],
      [r.left + r.width / 2, r.bottom - 0.5],
    ];
    const point = edges.find(inMenu);
    if (!point) return "no ring edge over the menu";
    const layer = ringEl.parentElement!;
    const before = [layer.style.pointerEvents, ringEl.style.pointerEvents] as const;
    layer.style.pointerEvents = ringEl.style.pointerEvents = "auto";
    const top = document.elementFromPoint(point[0], point[1]);
    [layer.style.pointerEvents, ringEl.style.pointerEvents] = before;
    return top === ringEl ? "ring" : top?.closest('[role="menu"]') ? "menu" : String(top?.className);
  });
  expect(hit).toBe("ring");
  // The menu is above the dim (its entry reads), the bubble above everything.
  await expect(entry).toBeVisible();
  await expect(bubble(page)).toBeVisible();
});
