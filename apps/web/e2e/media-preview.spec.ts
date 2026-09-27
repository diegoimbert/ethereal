// Browser sample preview on the web build, through the UI, against the real engine
// (WasmTransport → controller Worker → AudioWorklet preview voice).
//
// library → Demo Samples → preview Kick.wav: the row is pressed, the sample plays
// to its natural end in the worklet and the row resets on PreviewEnded { Finished } →
// preview a loop, replace it with the kick (the loop's row resets at once) → the kick ends →
// preview the loop again and stop it.
//
// No sleeps: every step waits on UI state.
import { expect, test, type Page } from "@playwright/test";
import { openLibrary, playButton } from "./ui";

const files = (page: Page) => page.getByRole("list", { name: "Files" });
// Clicking a row toggles its preview; the previewing row is pressed.
const row = (page: Page, name: string) => files(page).getByRole("button", { name, exact: true });
const previewButton = (page: Page, name: string) => row(page, name).and(page.locator('[aria-pressed="false"]'));
const stopButton = (page: Page, name: string) => row(page, name).and(page.locator('[aria-pressed="true"]'));

test("media preview: play to the end, replace and stop from the browser", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await openLibrary(page);
  await files(page).getByRole("button", { name: "Demo Samples" }).click();
  await expect(previewButton(page, "Kick.wav")).toBeVisible({ timeout: 20_000 });

  // --- A short sample plays to its natural end ------------------------------------------
  await previewButton(page, "Kick.wav").click();
  await expect(stopButton(page, "Kick.wav")).toBeVisible();
  // 0.45 s of audio: the engine reports its end by id and the row resets by itself.
  await expect(previewButton(page, "Kick.wav")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("alert")).toHaveCount(0);

  // --- Replace a loop with the kick --------------------------------------------------------
  await previewButton(page, "Chords Loop 120.wav").click();
  await expect(stopButton(page, "Chords Loop 120.wav")).toBeVisible();
  await previewButton(page, "Kick.wav").click();
  await expect(stopButton(page, "Kick.wav")).toBeVisible();
  await expect(previewButton(page, "Chords Loop 120.wav")).toBeVisible();
  await expect(previewButton(page, "Kick.wav")).toBeVisible({ timeout: 20_000 });

  // --- Stop ---------------------------------------------------------------------------------
  await previewButton(page, "Chords Loop 120.wav").click();
  await stopButton(page, "Chords Loop 120.wav").click();
  await expect(previewButton(page, "Chords Loop 120.wav")).toBeVisible();
  await expect(files(page).locator('[aria-pressed="true"]')).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(errors).toEqual([]);
});
