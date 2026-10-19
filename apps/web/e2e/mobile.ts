// Helpers of the mobile UI specs (`mobile-*.spec.ts`, #522): device profiles (on Chromium, the
// only browser installed: the iPhone and iPad profiles keep their viewport, touch and user
// agent, not WebKit) and booting the mobile shell.
import { devices, expect, type Page } from "@playwright/test";
import { projectScreen } from "./ui";

const chromium = ({ defaultBrowserType: _browser, ...profile }: (typeof devices)[string]) => profile;

export const IPHONE = chromium(devices["iPhone 13"]);
export const PIXEL = chromium(devices["Pixel 7"]);
export const IPAD = chromium(devices["iPad (gen 7)"]);

export const mobileShell = (page: Page) => page.getByTestId("mobile-shell");

/** The current layout of the mobile shell (`phone-portrait`, `phone-landscape`, `tablet`). */
export const layoutOf = (page: Page) => mobileShell(page).getAttribute("data-layout");

/** Open the app with the mobile UI (`?mobile=1`) and create a new project from the picker. */
export async function launchMobile(page: Page, path = "/"): Promise<void> {
  await page.goto(`${path}${path.includes("?") ? "&" : "?"}mobile=1`);
  await expect(mobileShell(page)).toBeVisible({ timeout: 30_000 });
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "new project" }).tap();
  await expect(screen).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __ether: { state(): { project: unknown } } }).__ether.state().project !== null))
    .toBe(true);
}

/** Wait for the running enter / exit animations (a sheet sliding up) to end. */
export const settled = (page: Page) =>
  page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running" || a.effect?.getTiming().iterations === Infinity));

/** `MOBILE_SHOTS=<dir>`: save a screenshot there (for PR bodies and design review). */
export async function shot(page: Page, name: string): Promise<void> {
  if (!process.env.MOBILE_SHOTS) return;
  // After the enter / exit animations (a dialog fading out), not halfway through them.
  await settled(page);
  await page.screenshot({ path: `${process.env.MOBILE_SHOTS}/${name}.png` });
}

export interface Pt {
  x: number;
  y: number;
}

/**
 * Real touch input through the Chrome DevTools Protocol (`Input.dispatchTouchEvent`): the page
 * gets `pointerType: "touch"` pointer events, as on a phone. Playwright itself only taps.
 */
export async function touchscreen(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  // `timestamp` (seconds since the epoch): the event's own time, whatever the round trip takes.
  const send = (type: "touchStart" | "touchMove" | "touchEnd" | "touchCancel", points: Array<Pt & { id: number }>, timestamp?: number) =>
    cdp.send("Input.dispatchTouchEvent", {
      type,
      touchPoints: points.map((p) => ({ x: p.x, y: p.y, id: p.id, radiusX: 4, radiusY: 4, force: 1 })),
      ...(timestamp === undefined ? {} : { timestamp }),
    });
  const lerp = (a: Pt, b: Pt, k: number): Pt => ({ x: a.x + (b.x - a.x) * k, y: a.y + (b.y - a.y) * k });
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  return {
    /** Raw touch points (id: the finger), for gestures the helpers below don't cover. */
    start: (points: Array<Pt & { id: number }>) => send("touchStart", points),
    move: (points: Array<Pt & { id: number }>) => send("touchMove", points),
    end: (points: Array<Pt & { id: number }> = []) => send("touchEnd", points),
    sleep,
    async tap(p: Pt) {
      await send("touchStart", [{ ...p, id: 1 }]);
      await send("touchEnd", []);
    },
    /**
     * Two taps `gap` ms apart by their event times (the app's double-tap window is 300 ms), so a
     * slow round trip to the browser on a busy machine can't split them into two single taps.
     */
    async doubleTap(p: Pt, gap = 120) {
      const t = Date.now() / 1000;
      await send("touchStart", [{ ...p, id: 1 }], t);
      await send("touchEnd", [], t + 0.01);
      await send("touchStart", [{ ...p, id: 1 }], t + gap / 1000);
      await send("touchEnd", [], t + gap / 1000 + 0.01);
    },
    async longPress(p: Pt, ms = 650) {
      await send("touchStart", [{ ...p, id: 1 }]);
      await sleep(ms);
      await send("touchEnd", []);
    },
    /**
     * One finger from `a` to `b` in `steps` moves over `ms`; `rest` ms still before lifting
     * (a rest kills the fling), `hold` ms still before moving (a long-press first).
     */
    async drag(a: Pt, b: Pt, opts: { steps?: number; ms?: number; rest?: number; hold?: number } = {}) {
      const { steps = 12, ms = 240, rest = 150, hold = 0 } = opts;
      await send("touchStart", [{ ...a, id: 1 }]);
      if (hold) await sleep(hold);
      for (let i = 1; i <= steps; i++) {
        await send("touchMove", [{ ...lerp(a, b, i / steps), id: 1 }]);
        await sleep(ms / steps);
      }
      if (rest) await sleep(rest);
      await send("touchEnd", []);
    },
    /** Two fingers from `a0`/`b0` to `a1`/`b1`. */
    async pinch(a0: Pt, b0: Pt, a1: Pt, b1: Pt, steps = 10) {
      await send("touchStart", [{ ...a0, id: 1 }]);
      await send("touchStart", [
        { ...a0, id: 1 },
        { ...b0, id: 2 },
      ]);
      for (let i = 1; i <= steps; i++) {
        await send("touchMove", [
          { ...lerp(a0, a1, i / steps), id: 1 },
          { ...lerp(b0, b1, i / steps), id: 2 },
        ]);
        await sleep(16);
      }
      await send("touchEnd", []);
    },
  };
}

/** The arrangement's timeline view (zoom, scroll, follow). */
export const arrangementViewState = (page: Page) =>
  page.evaluate(() => {
    const v = (
      window as unknown as { __ether: { arrangementView(): { getState(): { pxPerBeat: number; scrollBeats: number; followPlayhead: boolean } } } }
    ).__ether
      .arrangementView()
      .getState();
    return { pxPerBeat: v.pxPerBeat, scrollBeats: v.scrollBeats, followPlayhead: v.followPlayhead };
  });

/** The centre of an element (client px). */
export async function centerOf(locator: import("@playwright/test").Locator): Promise<Pt> {
  const b = (await locator.boundingBox())!;
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
}

/** The engine's transport state, from the UI mirror. */
export const transportState = (page: Page) =>
  page.evaluate(() => {
    const t = (window as unknown as { __ether: { state(): { transport: { playing: boolean; recording: boolean } | null } } }).__ether.state().transport;
    return { playing: t?.playing ?? false, recording: t?.recording ?? false };
  });
