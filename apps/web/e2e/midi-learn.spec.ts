// MIDI learn on the web build (real wasm controller + engine): the browser host has no MIDI
// input (Web MIDI is not wired), so the MIDI tab explains that learning needs the desktop app
// and MIDI mode stays off; the rest of the app keeps working normally.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";

interface Handle {
  state(): { project: Project | null };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state());

test("MIDI tab on the web: learning needs the desktop app", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => state(page).then((s) => s.project !== null), { timeout: 30_000 }).toBe(true);

  await page.getByRole("tab", { name: "MIDI" }).click();
  const panel = page.locator('[data-feature="midi-learn"]');
  await expect(panel.getByRole("note")).toContainText("MIDI learn needs the desktop app");
  await expect(panel.getByRole("switch", { name: "MIDI mode" })).toBeDisabled();
  await expect(panel).toContainText("No MIDI mappings.");
  expect((await state(page)).project!.midi_mappings).toEqual({});
  // MIDI mode never engaged: no control is highlighted or intercepted.
  await expect(page.locator("body.eth-midi-mode")).toHaveCount(0);
  await expect(page.locator("[data-midi-mappable]")).toHaveCount(0);

  expect(errors).toEqual([]);
});
