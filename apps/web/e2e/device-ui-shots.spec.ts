// device-ui: screenshots of every built-in device panel (the shared renderer), for PR review
// and the owner's UX pass. Skipped unless `DEVICE_UI_SHOTS=<dir>` is set:
//   DEVICE_UI_SHOTS=/tmp/shots DEVICE_UI_THEME=dark npx playwright test device-ui-shots
// Each device is inserted alone on a fresh track of the right kind, captured, then removed.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, openDeviceTab, playButton } from "./ui";

const dir = process.env.DEVICE_UI_SHOTS;
const theme = process.env.DEVICE_UI_THEME ?? "dark";
/** Comma-separated device types to capture (default: every built-in). */
const only = process.env.DEVICE_UI_ONLY?.split(",").filter(Boolean);

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

test.skip(!dir, "set DEVICE_UI_SHOTS=<dir> to capture device screenshots");

test("screenshot every built-in device panel", async ({ page }) => {
  test.setTimeout(600_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  const midi = await createTrack(page, "Midi");
  await openDeviceTab(page, midi.name);
  // The picker lists every built-in type (value = the type).
  const picker = page.getByRole("combobox", { name: "Add device" });
  await picker.click();
  const listbox = page.locator(`[id="${await picker.getAttribute("aria-controls")}"]`);
  const values = await listbox.locator('[role="option"]').evaluateAll((els) => els.map((e) => e.getAttribute("data-value") ?? ""));
  await page.keyboard.press("Escape");

  const types = values.filter((v) => v && (!only || only.includes(v)));
  for (const type of types) {
    await openDeviceTab(page, midi.name);
    const before = new Set(Object.keys((await project(page))!.devices));
    await addDevice(page, type);
    let id = "";
    await expect
      .poll(async () => {
        const p = (await project(page))!;
        id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === midi.id) ?? "";
        return id !== "";
      })
      .toBe(true);
    const card = page.locator(`section[data-device="${id}"]`);
    await expect(card).toBeVisible();
    // Wait for the descriptor (the "Loading…" status goes away).
    await expect(card.getByText("Loading…")).toHaveCount(0);
    await card.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(250);
    await card.screenshot({ path: `${dir}/${type}-${theme}.png` });
    const name = (await card.getAttribute("aria-label")) ?? type;
    await card.getByRole("button", { name: `Remove ${name}` }).click({ force: true });
    await expect(card).toHaveCount(0);
  }
});
