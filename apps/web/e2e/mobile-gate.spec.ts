// Phones and touch-only tablets get the mobile UI (apps/web/src/mobile.ts) without a
// flag; `?force-desktop` gives them the desktop editor (no "Open on desktop" band: still a
// phone). A desktop keeps the editor and gets the band.
import { expect, test, devices, type Page } from "@playwright/test";

// The emulated devices run on Chromium (the only browser installed), not the device's WebKit.
const chromium = ({ defaultBrowserType: _browser, ...profile }: (typeof devices)[string]) => profile;
const IPHONE = chromium(devices["iPhone 13"]);
const IPAD = chromium(devices["iPad (gen 7)"]);

/** The wasm engine being requested. */
const ENGINE = /\.wasm|ether_wasm|engine\/endpoint/;
function engineRequests(page: Page): string[] {
  const urls: string[] = [];
  page.on("request", (r) => {
    if (ENGINE.test(r.url())) urls.push(r.url());
  });
  return urls;
}

const mobileShell = (page: Page) => page.getByTestId("mobile-shell");
const band = (page: Page) => page.getByTestId("desktop-band");

for (const [name, profile] of [
  ["a phone", IPHONE],
  ["a touch-only tablet", IPAD],
] as const) {
  test.describe(`on ${name}`, () => {
    test.use({ ...profile, storageState: { cookies: [], origins: [] } });

    test("the mobile UI boots without a flag, with no Open on desktop band", async ({ page }) => {
      await page.goto("/");
      await expect(mobileShell(page)).toBeVisible({ timeout: 30_000 });
      await expect(page.getByRole("button", { name: "new project" })).toBeVisible({ timeout: 30_000 });
      await expect(band(page)).toHaveCount(0);
      expect(await page.evaluate(() => crossOriginIsolated)).toBe(true);
    });
  });
}

test.describe("on a phone", () => {
  test.use({ ...IPHONE, storageState: { cookies: [], origins: [] } });

  test("?force-desktop boots the desktop editor (still no band: a phone)", async ({ page }) => {
    await page.goto("/?force-desktop");
    await expect(page.getByRole("dialog", { name: "open project" })).toBeVisible({ timeout: 30_000 });
    await expect(mobileShell(page)).toHaveCount(0);
    await expect(band(page)).toHaveCount(0);
  });
});

test("on a desktop the app boots as before", async ({ page }) => {
  const engine = engineRequests(page);
  await page.goto("/");
  await expect(page.getByRole("dialog", { name: "open project" })).toBeVisible({ timeout: 30_000 });
  await expect(mobileShell(page)).toHaveCount(0);
  expect(engine.length).toBeGreaterThan(0);
});

test("on a desktop, the persistent Open on desktop band sits above the top bar", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("dialog", { name: "open project" })).toBeVisible({ timeout: 30_000 });
  await page.keyboard.press("Escape");
  await expect(band(page)).toContainText("ethereal works best in the desktop app");
  const b = (await band(page).boundingBox())!;
  const top = (await page.locator(".eth-shell__top").boundingBox())!;
  expect(b.y).toBe(0);
  expect(b.height).toBeLessThanOrEqual(30);
  expect(top.y).toBeGreaterThanOrEqual(b.y + b.height);
  // The shell still fits the window: nothing scrolls.
  const fit = await page.evaluate(() => ({
    shell: document.querySelector(".eth-shell")!.getBoundingClientRect().bottom,
    height: window.innerHeight,
    scroll: document.documentElement.scrollHeight,
  }));
  expect(fit.shell).toBeLessThanOrEqual(fit.height);
  expect(fit.scroll).toBeLessThanOrEqual(fit.height);
  if (process.env.MOBILE_SHOTS) await page.screenshot({ path: `${process.env.MOBILE_SHOTS}/desktop-band.png` });
});
