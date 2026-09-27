// export: offline export on the web build (real wasm engine + controller). The web store
// keeps no files, so the result comes back as a download: the dialog pulls the bytes with
// `Export::ReadChunk` and the browser saves them.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, pickOption, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(
    () => (window as unknown as { __ether: Handle }).__ether.state().project,
  );

test("export the loop region as a WAV download", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({
    timeout: 30_000,
  });
  await expect
    .poll(() => project(page).then((p) => p !== null), { timeout: 30_000 })
    .toBe(true);

  await page.getByRole("button", { name: "Projects" }).click();
  const name = `Export ${Date.now()}`;
  await page.getByLabel("New project name").fill(name);
  await page
    .getByRole("dialog", { name: "Projects" })
    .getByRole("button", { name: "New" })
    .click();
  await expect(page.getByTestId("project-name")).toHaveText(name);
  await createTrack(page, "Midi");

  await page.getByRole("button", { name: "Export audio" }).click();
  const dialog = page.getByRole("dialog", { name: "Export audio" });
  await expect(dialog).toBeVisible();
  await pickOption(dialog, "Range", { value: "loop" });
  await pickOption(dialog, "Format", { value: "Wav" });
  await pickOption(dialog, "Bit depth", { value: "Int16" });
  await dialog.getByLabel("File name").fill("Loop bounce");

  const download = page.waitForEvent("download", { timeout: 60_000 });
  await dialog.getByRole("button", { name: "Export", exact: true }).click();
  const file = await download;
  expect(file.suggestedFilename()).toBe("Loop bounce.wav");
  const path = await file.path();
  const fs = await import("node:fs");
  const bytes = fs.readFileSync(path);
  expect(bytes.subarray(0, 4).toString("latin1")).toBe("RIFF");
  expect(bytes.subarray(8, 12).toString("latin1")).toBe("WAVE");
  // 16 beats at 120 bpm = 8 s of 16-bit stereo at the engine rate (>= 44.1 kHz).
  expect(bytes.length).toBeGreaterThan(8 * 44_100 * 4);
  await expect(dialog.getByTestId("export-done")).toContainText(
    "Loop bounce.wav",
  );
  await dialog.getByRole("button", { name: "Close" }).click();
  expect(errors).toEqual([]);
});
