// Capture MIDI on the web build (real wasm controller + engine, `capture-midi`). The browser
// host has no MIDI input yet (Web MIDI is not wired), so the buffer stays empty: the
// transport-bar Capture button is shown, disabled with a hint, and the palette's
// "Capture MIDI" is a harmless no-op (the controller replies InvalidState, nothing changes).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";

interface Handle {
  state(): { project: Project | null };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state());

test("Capture MIDI: button and palette entry on the web", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => state(page).then((s) => s.project !== null), { timeout: 30_000 }).toBe(true);

  const capture = page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Capture MIDI" });
  await expect(capture).toBeVisible();
  await expect(capture).toBeDisabled();
  await expect(capture).toHaveAttribute("title", /play something first/);

  const clipsBefore = Object.keys((await state(page)).project!.clips).length;
  await page.keyboard.press("ControlOrMeta+k");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await expect(palette).toBeVisible();
  await page.keyboard.type("capture midi");
  await expect(palette.getByRole("option", { name: /Capture MIDI/ })).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(palette).toBeHidden();
  // Nothing was played: no clip, no error, the button stays off.
  await page.waitForTimeout(300);
  expect(Object.keys((await state(page)).project!.clips)).toHaveLength(clipsBefore);
  await expect(capture).toBeDisabled();
  expect(errors).toEqual([]);
});
