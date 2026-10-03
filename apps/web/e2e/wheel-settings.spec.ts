// base-135: configurable wheel zoom/scroll. Windows-like mouse-wheel notches (deltaMode =
// LINE, deltaY = ±3, as Firefox reports them; Chromium's 100 px + wheelDelta 120 notches are
// normalized the same way) zoom the arrangement by the fixed per-notch factor, whatever the
// platform; Settings > Input flips the direction and scales it, and the settings persist.
//
// `UX_SHOTS=<dir>` also writes the PR screenshots of the Input tab (`UX_SHOTS_THEME=light`).
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, playButton } from "./ui";

const shots = process.env.UX_SHOTS;
const theme = process.env.UX_SHOTS_THEME === "light" ? "light" : "dark";
/** `NOTCH_ZOOM` in ui/src/timeline/wheelInput.ts. */
const NOTCH_ZOOM = 1.12;

test.use({ viewport: { width: 1440, height: 900 } });

interface Handle {
  state(): { project: Project | null };
}

async function open(page: Page): Promise<void> {
  // Instant zoom (no spring animation), so each notch lands at once.
  await page.emulateMedia({ reducedMotion: "reduce" });
  if (shots) await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project !== null), {
      timeout: 30_000,
    })
    .toBe(true);
}

/** The arrangement's live zoom (px per beat), from a track lane's `--ppb`. */
const zoom = (page: Page): Promise<number> =>
  page.evaluate(() => parseFloat(document.querySelector<HTMLElement>(".eth-arr-lane__layer")!.style.getPropertyValue("--ppb")));

/** One Windows-like wheel notch over the arrangement lanes, with Ctrl physically held. */
async function notch(page: Page, lines: number): Promise<void> {
  await page.keyboard.down("Control");
  await page.evaluate((deltaY) => {
    const el = document.querySelector<HTMLElement>(".eth-arr__scroll")!;
    const r = el.getBoundingClientRect();
    el.dispatchEvent(
      new WheelEvent("wheel", {
        deltaY,
        deltaMode: WheelEvent.DOM_DELTA_LINE,
        ctrlKey: true,
        clientX: r.left + r.width * 0.6,
        clientY: r.top + 20,
        bubbles: true,
        cancelable: true,
      }),
    );
  }, lines);
  await page.keyboard.up("Control");
}

/** Zoom ratio produced by one notch. */
async function notchRatio(page: Page, lines: number): Promise<number> {
  const before = await zoom(page);
  await notch(page, lines);
  await expect.poll(() => zoom(page)).not.toBe(before);
  return (await zoom(page)) / before;
}

async function openInputSettings(page: Page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("tab", { name: "Input" }).click();
  await expect(dialog.getByTestId("input-settings")).toBeVisible();
  return dialog;
}

test("mouse-wheel notches zoom by the configured per-notch factor; inversion flips it", async ({ page }) => {
  test.setTimeout(90_000);
  await open(page);
  await createTrack(page, "Midi");
  await expect(page.locator(".eth-arr-lane__layer").first()).toBeVisible();

  // Default (Linux): wheel up (deltaY < 0) zooms in by one notch; down zooms back out exactly.
  expect(await notchRatio(page, -3)).toBeCloseTo(NOTCH_ZOOM, 3);
  expect(await notchRatio(page, 3)).toBeCloseTo(1 / NOTCH_ZOOM, 3);
  // Chromium's own notch (Windows/Linux: deltaMode PIXEL, deltaY 100, wheelDelta 120): same step.
  const box = (await page.locator(".eth-arr__scroll").boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.6, box.y + 20);
  const before = await zoom(page);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -100);
  await page.keyboard.up("Control");
  await expect.poll(() => zoom(page)).not.toBe(before);
  expect((await zoom(page)) / before).toBeCloseTo(NOTCH_ZOOM, 3);

  // Invert zoom direction.
  let dialog = await openInputSettings(page);
  const invert = dialog.getByRole("switch", { name: "Invert zoom direction" });
  await expect(invert).toHaveAttribute("aria-checked", "false");
  await dialog.getByText("Invert zoom direction").click();
  await expect(invert).toHaveAttribute("aria-checked", "true");
  await dialog.getByRole("button", { name: "Done" }).click();
  expect(await notchRatio(page, -3)).toBeCloseTo(1 / NOTCH_ZOOM, 3);

  // Zoom sensitivity 2: two notches' worth per notch.
  dialog = await openInputSettings(page);
  const sens = dialog.getByRole("spinbutton", { name: "Zoom sensitivity" });
  await sens.fill("2");
  await sens.press("Enter");
  await expect(sens).toHaveValue(/^2\.00/);
  await dialog.getByRole("button", { name: "Done" }).click();
  expect(await notchRatio(page, 3)).toBeCloseTo(NOTCH_ZOOM ** 2, 3);

  // Kept on this device across reloads.
  await page.reload();
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  dialog = await openInputSettings(page);
  await expect(dialog.getByRole("switch", { name: "Invert zoom direction" })).toHaveAttribute("aria-checked", "true");
  await expect(dialog.getByRole("spinbutton", { name: "Zoom sensitivity" })).toHaveValue(/^2\.00/);
  await dialog.getByRole("button", { name: "Reset to defaults" }).click();
  await expect(dialog.getByRole("switch", { name: "Invert zoom direction" })).toHaveAttribute("aria-checked", "false");

  if (shots) {
    await page.mouse.move(0, 0);
    await dialog.screenshot({ path: `${shots}/input-settings-${theme}.png`, animations: "disabled" });
  }
});
