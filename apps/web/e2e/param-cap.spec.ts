// base-132: a plugin with 10,000 params never renders all of them. Runs the web build on the
// mock transport (`?mock`), whose fake "Mock Mega" plugin has 10,000 params: its card is
// capped, "Show all…" opens a windowed, searchable list, and a pin survives a reload (pins
// live in UI settings per plugin, not in the project).
// `PARAM_CAP_SHOTS=<dir>` also writes the PR screenshots (dark; `PARAM_CAP_THEME=light`).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openDeviceTab, playButton } from "./ui";

const MEGA = "dev.ethereal.mock.mega";
const CAP = 16;
const MAX_ROWS = 100;
const shots = process.env.PARAM_CAP_SHOTS;
const theme = process.env.PARAM_CAP_THEME ?? "dark";

interface Handle {
  state(): { project: Project | null };
}
interface MockHandle {
  send(command: unknown): Promise<unknown>;
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

/** Opens the app on the mock engine, shows the first track's chain and inserts Mock Mega. */
async function openMega(page: Page): Promise<{ id: string; card: ReturnType<Page["locator"]> }> {
  await page.goto("/?mock");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
  const p = (await project(page))!;
  const track = Object.values(p.tracks).find((t) => t.kind === "Audio") ?? Object.values(p.tracks).find((t) => t.kind !== "Master")!;
  await openDeviceTab(page, track.name);
  const id = `e2e-mega-${Date.now()}`;
  await page.evaluate(
    ([id, track, plugin]) =>
      (window as unknown as { __etherMock: MockHandle }).__etherMock.send({
        domain: "Device",
        command: { type: "Insert", id, track, device: { type: "Plugin", plugin_id: plugin, sandboxed: null }, before: null },
      }),
    [id, track.id, MEGA] as const,
  );
  const card = page.locator(`section[data-device="${id}"]`);
  await expect(card).toBeVisible();
  await expect(card.locator("[data-param]").first()).toBeVisible();
  await card.scrollIntoViewIfNeeded();
  return { id, card };
}

test("a 10,000-param plugin: capped card, windowed list, search, pin kept across reload", async ({ page }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);

  let { card } = await openMega(page);
  const shown = await card.locator("[data-param]").count();
  expect(shown).toBeGreaterThan(0);
  expect(shown).toBeLessThanOrEqual(CAP);
  // Expanding "More controls" stays within the cap.
  await card.getByRole("button", { name: /More Mock Mega controls/ }).click();
  expect(await card.locator("[data-param]").count()).toBeLessThanOrEqual(CAP);
  await page.mouse.move(0, 0);
  if (shots) await card.screenshot({ path: `${shots}/param-cap-card-${theme}.png` });

  // The full list: only the rows in view are in the DOM, even after scrolling far.
  await card.getByRole("button", { name: /^Show all \d+ Mock Mega parameters$/ }).click();
  const list = page.getByRole("list", { name: "Mock Mega parameters" });
  await expect(list.getByRole("listitem").first()).toBeVisible();
  expect(await list.getByRole("listitem").count()).toBeLessThanOrEqual(MAX_ROWS);
  for (const frac of [0.25, 0.6, 0.95]) {
    await list.evaluate((el, f) => {
      el.scrollTop = el.scrollHeight * f;
    }, frac);
    await page.waitForTimeout(50);
    expect(await list.getByRole("listitem").count()).toBeLessThanOrEqual(MAX_ROWS);
  }
  await expect(list.getByText("Gain 0", { exact: true })).toHaveCount(0);
  await list.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await expect(list.getByText("Detune 9999", { exact: true })).toBeVisible();

  // Search, then pin.
  const search = page.getByRole("searchbox", { name: "Search Mock Mega parameters" });
  await search.fill("cutoff 42");
  await expect(list.getByRole("listitem").first()).toContainText("Cutoff 42");
  if (shots) await page.getByRole("dialog").screenshot({ path: `${shots}/param-cap-list-search-${theme}.png` });
  await search.fill("cutoff 4241");
  await expect(list.getByRole("listitem")).toHaveCount(1);
  await list.getByRole("button", { name: "Pin Cutoff 4241" }).click();
  await expect(list.getByRole("button", { name: "Unpin Cutoff 4241" })).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");
  await expect(card.locator("[data-param]").first()).toHaveAttribute("data-param", "4241");
  await page.mouse.move(0, 0);
  if (shots) await card.screenshot({ path: `${shots}/param-cap-pinned-${theme}.png` });

  // Reload: the mock project starts over, but the pin is a UI setting of the plugin, so a
  // new instance of it shows Cutoff 4241 first.
  await page.reload();
  ({ card } = await openMega(page));
  await expect(card.locator("[data-param]").first()).toHaveAttribute("data-param", "4241");
  expect(await card.locator("[data-param]").count()).toBeLessThanOrEqual(CAP);
  expect(await page.evaluate(() => localStorage.getItem("eth.devices.paramPins"))).toContain("4241");
  expect(errors).toEqual([]);
});
