// The mobile UI's shell (#522), behind `?mobile=1` while in development: a phone or tablet gets
// the arrangement with a top bar and a bottom transport (touch-sized), the layout follows the
// viewport (rotation keeps the project and the arrangement mounted), and hiding the page stops
// playback. The gate screen offers it as a beta; desktops are unaffected.
import { expect, test, type Page } from "@playwright/test";
import { IPAD, IPHONE, launchMobile, layoutOf, mobileShell, PIXEL, shot, transportState } from "./mobile";

const dock = (page: Page) => page.getByTestId("mobile-transport");

/**
 * Every button in the mobile bars is at least 44 × 44 CSS px to the touch: its box, or the
 * invisible extension of a desktop-sized one (Share: an absolutely positioned `::after`).
 */
async function expectTouchTargets(page: Page) {
  const small = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".eth-mshell__top button, .eth-mshell__dock button")]
      .filter((b) => b.offsetParent !== null)
      .map((b) => {
        const r = b.getBoundingClientRect();
        const after = getComputedStyle(b, "::after");
        const ext = after.content !== "none" && after.position === "absolute";
        return {
          name: b.getAttribute("aria-label") ?? b.textContent,
          w: Math.max(r.width, ext ? parseFloat(after.width) || 0 : 0),
          h: Math.max(r.height, ext ? parseFloat(after.height) || 0 : 0),
        };
      })
      .filter((b) => b.w < 44 || b.h < 44),
  );
  expect(small).toEqual([]);
}

test.describe("phone (iPhone)", () => {
  test.use(IPHONE);

  test("the shell: arrangement, top bar, transport at the bottom; cross-origin isolated", async ({ page }) => {
    await launchMobile(page);
    expect(await layoutOf(page)).toBe("phone-portrait");
    expect(await page.evaluate(() => crossOriginIsolated && typeof SharedArrayBuffer === "function")).toBe(true);
    await expect(page.getByTestId("arrangement-content")).toBeVisible();
    // No desktop chrome: rail, floating panes, "Open on desktop" band.
    await expect(page.locator(".eth-rail")).toHaveCount(0);
    await expect(page.locator("[data-pane]")).toHaveCount(0);
    await expect(page.getByTestId("desktop-band")).toHaveCount(0);
    // The transport sits at the bottom of the screen, in thumb reach.
    const box = (await dock(page).boundingBox())!;
    const viewport = page.viewportSize()!;
    expect(box.y + box.height).toBeGreaterThan(viewport.height - 8);
    for (const name of ["stop", "play", "record", "loop", "metronome", "undo", "redo"])
      await expect(page.getByRole("button", { name, exact: true })).toBeVisible();
    await shot(page, "phone-portrait");
    await expectTouchTargets(page);
  });

  test("play and stop by tapping", async ({ page }) => {
    await launchMobile(page);
    await dock(page).getByRole("button", { name: "play", exact: true }).tap();
    await expect.poll(async () => (await transportState(page)).playing).toBe(true);
    await expect.poll(() => page.getByTestId("position-time").textContent()).not.toBe("0:00.000");
    await dock(page).getByRole("button", { name: "pause", exact: true }).tap();
    await expect.poll(async () => (await transportState(page)).playing).toBe(false);
    await dock(page).getByRole("button", { name: "loop" }).tap();
    await expect(dock(page).getByRole("button", { name: "loop" })).toHaveAttribute("aria-pressed", "true");
  });

  test("rotation: landscape lays out one bottom bar; the project and the arrangement stay mounted", async ({ page }) => {
    await launchMobile(page);
    const lanes = await page.getByTestId("arrangement-content").elementHandle();
    await page.setViewportSize({ width: 664, height: 390 });
    await expect(mobileShell(page)).toHaveAttribute("data-layout", "phone-landscape");
    expect(await lanes!.evaluate((el) => el.isConnected)).toBe(true);
    // The top bar's group shares the bottom row with the transport.
    const top = (await page.locator(".eth-mshell__top").boundingBox())!;
    const bar = (await dock(page).boundingBox())!;
    expect(Math.abs(top.y + top.height - (bar.y + bar.height))).toBeLessThan(2);
    await shot(page, "phone-landscape");
    await expectTouchTargets(page);
    await page.setViewportSize({ width: 390, height: 664 });
    await expect(mobileShell(page)).toHaveAttribute("data-layout", "phone-portrait");
    expect(await lanes!.evaluate((el) => el.isConnected)).toBe(true);
  });

  test("hiding the page (another app, the lock screen) stops playback", async ({ page }) => {
    await launchMobile(page);
    await dock(page).getByRole("button", { name: "play", exact: true }).tap();
    await expect.poll(async () => (await transportState(page)).playing).toBe(true);
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await expect.poll(async () => (await transportState(page)).playing).toBe(false);
  });

  test("a phone gets the mobile UI without asking; ?force-desktop gives the editor, for the tab", async ({ page }) => {
    await page.goto("/");
    await expect(mobileShell(page)).toBeVisible({ timeout: 30_000 });
    await page.goto("/?force-desktop");
    await expect(page.locator(".eth-shell").first()).toBeVisible({ timeout: 30_000 });
    await expect(mobileShell(page)).toHaveCount(0);
    // Remembered for the tab: a reload without the flag keeps the editor.
    await page.goto("/");
    await expect(page.locator(".eth-shell").first()).toBeVisible({ timeout: 30_000 });
    await expect(mobileShell(page)).toHaveCount(0);
    // ?mobile=1 brings the mobile UI back.
    await page.goto("/?mobile=1");
    await expect(mobileShell(page)).toBeVisible({ timeout: 30_000 });
  });
});

test.describe("phone (Pixel)", () => {
  test.use(PIXEL);

  test("portrait shell, touch targets", async ({ page }) => {
    await launchMobile(page);
    expect(await layoutOf(page)).toBe("phone-portrait");
    await shot(page, "pixel-portrait");
    await expectTouchTargets(page);
  });
});

test.describe("tablet (iPad)", () => {
  test.use(IPAD);

  test("the tablet layout: one transport row at the bottom, in both orientations", async ({ page }) => {
    await launchMobile(page);
    expect(await layoutOf(page)).toBe("tablet");
    await shot(page, "tablet-portrait");
    await expectTouchTargets(page);
    await page.setViewportSize({ width: 1080, height: 810 });
    await expect(mobileShell(page)).toHaveAttribute("data-layout", "tablet");
    const bar = (await dock(page).boundingBox())!;
    expect(bar.height).toBeLessThan(80);
    await shot(page, "tablet-landscape");
  });

  test("track header buttons are touch-sized and fit the row (#640)", async ({ page }) => {
    await launchMobile(page);
    await page.getByRole("button", { name: /^add track$/i }).tap();
    await page.getByRole("menuitem", { name: "create audio track" }).tap();
    const toggles = page.locator(".eth-arr-header__toggle");
    await expect(toggles.first()).toBeVisible();
    const boxes = await toggles.evaluateAll((els) =>
      els
        .filter((b) => (b as HTMLElement).offsetParent !== null)
        .map((b) => {
          const r = b.getBoundingClientRect();
          const row = b.closest(".eth-arr-header")!.getBoundingClientRect();
          return { w: r.width, h: r.height, inside: r.top >= row.top - 0.5 && r.bottom <= row.bottom + 0.5 };
        }),
    );
    expect(boxes.length).toBeGreaterThanOrEqual(3);
    for (const b of boxes) expect(b).toEqual({ w: 44, h: 44, inside: true });
  });
});

test("a desktop keeps the desktop editor", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("dialog", { name: "open project" })).toBeVisible({ timeout: 30_000 });
  await expect(mobileShell(page)).toHaveCount(0);
});
