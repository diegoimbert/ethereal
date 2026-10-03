// Screenshots of the keymap editor and cheat sheet for the PR (not part of the regular
// suite): `KEYMAP_SHOTS=<dir> npx playwright test keymap.shots`.
import { expect, test, type Page } from "@playwright/test";
import { createTrack, createOnLaunch } from "./ui";

const dir = process.env.KEYMAP_SHOTS;
test.skip(!dir, "set KEYMAP_SHOTS=<dir> to capture");
test.use({ viewport: { width: 1440, height: 900 } });

const editor = (page: Page) => page.getByRole("dialog", { name: "Keyboard shortcuts" });
const row = (page: Page, id: string) => editor(page).locator(`[data-action="${id}"]`);

async function open(page: Page, theme: "dark" | "light") {
  await page.goto("/");
  await page.evaluate((t) => {
    document.documentElement.dataset.theme = t;
  }, theme);
  await createOnLaunch(page, "Keymap demo");
  await createTrack(page, "Midi");
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByRole("combobox", { name: "Search commands" }).fill("keyboard shortcuts");
  await page.keyboard.press("Enter");
  await expect(editor(page)).toBeVisible();
  await page.waitForTimeout(300);
}

test("keymap editor (dark): list, recording with a conflict, conflict rows", async ({ page }) => {
  await open(page, "dark");
  await page.screenshot({ path: `${dir}/keymap-editor-dark.png` });
  await row(page, "transport.loop").getByRole("button", { name: /Add a shortcut/ }).click();
  await page.keyboard.press("ControlOrMeta+d");
  await row(page, "transport.loop").scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${dir}/keymap-recording-conflict-dark.png` });
  await page.keyboard.press("Enter");
  await editor(page).getByRole("switch").click();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${dir}/keymap-conflicts-dark.png` });
  await editor(page).getByRole("switch").click();
  await editor(page).getByRole("combobox", { name: "Keymap preset" }).click();
  await page.getByRole("option", { name: "Ableton-like" }).click();
  await editor(page).getByRole("textbox", { name: "Search shortcuts" }).fill("transport");
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${dir}/keymap-ableton-dark.png` });
  // Cheat sheet, as printed.
  await page.evaluate(() => {
    window.print = () => undefined;
  });
  await editor(page).getByRole("button", { name: /Print cheat sheet/ }).click();
  await page.emulateMedia({ media: "print" });
  await expect(page.getByTestId("keymap-cheat-sheet")).toBeVisible();
  await page.screenshot({ path: `${dir}/keymap-cheat-sheet-print.png`, fullPage: true });
});

test("keymap editor (light)", async ({ page }) => {
  await open(page, "light");
  await page.screenshot({ path: `${dir}/keymap-editor-light.png` });
});
