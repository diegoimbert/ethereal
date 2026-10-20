// A plugin with 10,000 params never renders all of them. Runs the web build on the
// mock transport (`?mock`), whose fake "Mock Mega" plugin has 10,000 params.
// A plugin's card is its header; the header's "Parameters" toggle shows the
// windowed, searchable list inline, capped in height; a pin (a UI setting per plugin, not
// in the project) puts its param first and survives a reload.
// `PARAM_CAP_SHOTS=<dir>` also writes the PR screenshots (dark; `PARAM_CAP_THEME=light`).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openDeviceTab, playButton, showDeviceParams } from "./ui";

const MEGA = "dev.ethereal.mock.mega";
const MAX_ROWS = 100;
/** `--eth-size-device-params-max-height`. */
const PARAMS_MAX_PX = 240;
const shots = process.env.PARAM_CAP_SHOTS;
const theme = process.env.PARAM_CAP_THEME ?? "dark";

interface Handle {
  state(): { project: Project | null };
}
interface MockHandle {
  send(command: unknown): Promise<unknown>;
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

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
  await card.scrollIntoViewIfNeeded();
  return { id, card };
}

test("a 10,000-param plugin: header-only card, capped inline list, search, pin kept across reload", async ({ page }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);

  const opened = await openMega(page);
  const card = opened.card;
  let id = opened.id;
  // The card is its header: no control until the parameters are shown.
  expect(await card.locator("[data-param]").count()).toBe(0);
  const params = await showDeviceParams(page, id);
  const list = params.getByRole("list", { name: "Mock Mega parameters" });
  await expect(list.getByRole("listitem").first()).toBeVisible();
  // Capped: the area never grows past the token, the list scrolls inside it.
  expect((await params.boundingBox())!.height).toBeLessThanOrEqual(PARAMS_MAX_PX + 1);
  expect(await list.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  await page.mouse.move(0, 0);
  if (shots) await card.screenshot({ path: `${shots}/param-cap-inline-${theme}.png` });

  // Only the rows in view are in the DOM, even after scrolling far.
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

  // Search, then pin: the pinned param comes first.
  const search = params.getByRole("searchbox", { name: "search Mock Mega parameters" });
  await search.fill("cutoff 42");
  await expect(list.getByRole("listitem").first()).toContainText("Cutoff 42");
  if (shots) await card.screenshot({ path: `${shots}/param-cap-search-${theme}.png` });
  await search.fill("cutoff 4241");
  await expect(list.getByRole("listitem")).toHaveCount(1);
  await list.getByRole("button", { name: "pin Cutoff 4241" }).click();
  await expect(list.getByRole("button", { name: "unpin Cutoff 4241" })).toHaveAttribute("aria-pressed", "true");
  await search.fill("");
  await expect(list.locator("[data-param-row]").first()).toHaveAttribute("data-param-row", "4241");

  // Reload: the mock project starts over, but the pin is a UI setting of the plugin, so a
  // new instance of it shows Cutoff 4241 first.
  await page.reload();
  ({ id } = await openMega(page));
  const again = await showDeviceParams(page, id);
  await expect(again.locator("[data-param-row]").first()).toHaveAttribute("data-param-row", "4241");
  expect(await page.evaluate(() => localStorage.getItem("eth.devices.paramPins"))).toContain("4241");
  expect(errors).toEqual([]);
});
