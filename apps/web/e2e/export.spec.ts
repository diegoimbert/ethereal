// export: offline export on the web build (real wasm engine + controller). The web store
// keeps no files, so the result comes back as a download: the dialog pulls the bytes with
// `Export::ReadChunk` and the browser saves them.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, launch, newProject, pickOption, playButton, projectMenu } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

test("export the loop region as a WAV download", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(playButton(page)).toBeVisible({
    timeout: 30_000,
  });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  const name = `Export ${Date.now()}`;
  await newProject(page, name);
  await expect(page.getByTestId("project-name")).toHaveText(name);
  await createTrack(page, "Midi");

  await projectMenu(page, "export audio…");
  const dialog = page.getByRole("dialog", { name: "export audio" });
  await expect(dialog).toBeVisible();
  await pickOption(dialog, "range", { value: "loop" });
  await pickOption(dialog, "format", { value: "Wav" });
  await pickOption(dialog, "bit depth", { value: "Int16" });
  await dialog.getByLabel("file name").fill("Loop bounce");

  const download = page.waitForEvent("download", { timeout: 60_000 });
  await dialog.getByRole("button", { name: "export", exact: true }).click();
  const file = await download;
  expect(file.suggestedFilename()).toBe("Loop bounce.wav");
  const path = await file.path();
  const fs = await import("node:fs");
  const bytes = fs.readFileSync(path);
  expect(bytes.subarray(0, 4).toString("latin1")).toBe("RIFF");
  expect(bytes.subarray(8, 12).toString("latin1")).toBe("WAVE");
  // 16 beats at 120 bpm = 8 s of 16-bit stereo at the engine rate (>= 44.1 kHz).
  expect(bytes.length).toBeGreaterThan(8 * 44_100 * 4);
  await expect(dialog.getByTestId("export-done")).toContainText("Loop bounce.wav");
  await dialog.getByRole("button", { name: "close" }).click();
  // The render paused the live engine (CONTRACTS.md §13.19); done, it is back: no render
  // status, nothing frozen on the timeline, play available.
  await expect(page.getByTestId("render-preview")).toHaveCount(0);
  await expect(page.getByTestId("render-frozen")).toHaveCount(0);
  await expect(playButton(page)).toBeEnabled();
  expect(errors).toEqual([]);
});
