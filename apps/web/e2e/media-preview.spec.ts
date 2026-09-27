// Browser sample preview on the web build, through the UI, against the real engine
// (WasmTransport → controller Worker → AudioWorklet preview voice).
//
// library → Demo Samples → preview Kick.wav: the row shows "Stop preview", the sample plays
// to its natural end in the worklet and the row resets on PreviewEnded { Finished } →
// preview a loop, replace it with the kick (the loop's row resets at once) → the kick ends →
// preview the loop again and stop it.
//
// No sleeps: every step waits on UI state.
import { expect, test, type Page } from "@playwright/test";

const files = (page: Page) => page.getByRole("list", { name: "Files" });
const previewButton = (page: Page, name: string) => files(page).getByRole("button", { name: `Preview ${name}`, exact: true });
const stopButton = (page: Page, name: string) => files(page).getByRole("button", { name: `Stop preview of ${name}`, exact: true });

test("media preview: play to the end, replace and stop from the browser", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await page.getByRole("tablist", { name: "Locations" }).getByRole("tab", { name: "Browser library" }).click();
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
  await expect(files(page).getByRole("button", { name: /^Stop preview/ })).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(errors).toEqual([]);
});
