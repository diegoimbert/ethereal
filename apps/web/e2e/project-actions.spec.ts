// Project actions on the web build: Save as, Duplicate, a `.ether` bundle export
// (browser download) imported back (browser upload) with its media.
//
// No sleeps: every step waits on UI or engine state.
import { readFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import type { Project, ProjectSummary } from "@/generated";
import { newProject, openProjectScreen, paletteCommand, projectMenu, projectScreen } from "./ui";

interface Handle {
  state(): { project: Project | null; projects: ProjectSummary[] };
}

const state = (page: Page) =>
  page.evaluate(() => {
    const s = (window as unknown as { __ether: Handle }).__ether.state();
    return { project: s.project!, names: s.projects.map((p) => p.name) };
  });

/** A mono 16-bit WAV: `seconds` of a 220 Hz sine at 48 kHz. */
function sineWav(seconds: number): Buffer {
  const rate = 48000;
  const frames = Math.round(rate * seconds);
  const b = Buffer.alloc(44 + frames * 2);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + frames * 2, 4);
  b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(frames * 2, 40);
  for (let i = 0; i < frames; i++) b.writeInt16LE(Math.round(12000 * Math.sin((2 * Math.PI * 220 * i) / rate)), 44 + i * 2);
  return b;
}

async function boot(page: Page) {
  await page.goto("/");
  // The app launches with no project, on the project screen.
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
}

test("Save as and Duplicate", async ({ page }) => {
  await boot(page);
  await newProject(page, "Song");
  const id = (await state(page)).project.id;
  // The in-place rename is saved (autosave) before Save as leaves the original.
  await expect(page.getByLabel("unsaved changes")).toHaveCount(0);

  // Duplicate (palette): a stored copy; the open project stays.
  await paletteCommand(page, "duplicate project");
  await expect.poll(async () => (await state(page)).names).toContain("Song copy");
  expect((await state(page)).project.id).toBe(id);
  await expect(page.getByRole("status").filter({ hasText: "duplicated as “Song copy”" })).toBeVisible();
  // The copy is listed in the picker (the open project is not).
  const screen = await openProjectScreen(page);
  await expect(screen.getByRole("button", { name: "open Song copy" })).toBeVisible();
  await expect(screen.getByRole("button", { name: "open Song", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(screen).toBeHidden();

  // Save as (palette): switches to the new copy.
  await paletteCommand(page, "save as…");
  const saveAs = page.getByRole("dialog", { name: "save as" });
  await saveAs.getByLabel("save as name").fill("Song v2");
  await saveAs.getByRole("button", { name: "save", exact: true }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Song v2");
  expect((await state(page)).project.id).not.toBe(id);
  await expect.poll(async () => (await state(page)).names).toEqual(expect.arrayContaining(["Song", "Song copy", "Song v2"]));
});

test("export a bundle, then import it back with its media", async ({ page }) => {
  await boot(page);
  await newProject(page, "Bundle");
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("ControlOrMeta+i");
  await (await chooser).setFiles([{ name: "Pad.wav", mimeType: "audio/wav", buffer: sineWav(0.5) }]);
  await expect.poll(async () => Object.keys((await state(page)).project.media).length, { timeout: 20_000 }).toBe(1);
  // The media lands first; its track and clip follow. Settled: two reads in a row agree.
  const shape = async () => {
    const p = (await state(page)).project;
    return JSON.stringify([Object.keys(p.tracks).sort(), Object.keys(p.clips).sort()]);
  };
  let last = "";
  await expect
    .poll(
      async () => {
        const now = await shape();
        const settled = now === last && (JSON.parse(now) as string[][])[1]!.length > 0;
        last = now;
        return settled;
      },
      { timeout: 20_000, intervals: [250] },
    )
    .toBe(true);
  const original = (await state(page)).project;

  // Export: a browser download of a ZIP-based .ether file.
  const download = page.waitForEvent("download");
  await projectMenu(page, "export project…");
  await page.getByRole("dialog", { name: "export project" }).getByRole("button", { name: "export" }).click();
  const file = await download;
  expect(file.suggestedFilename()).toBe("Bundle.ether");
  const bytes = readFileSync(await file.path());
  expect(bytes.subarray(0, 4).toString("latin1")).toBe("PK\u0003\u0004");
  await expect(page.getByRole("status").filter({ hasText: "exported “Bundle”" })).toBeVisible();

  // Import: the browser's file picker; the copy gets a free name and opens.
  const picker = page.waitForEvent("filechooser");
  await projectMenu(page, "import project…");
  await (await picker).setFiles([{ name: "Bundle.ether", mimeType: "application/zip", buffer: bytes }]);
  await expect(page.getByTestId("project-name")).toHaveText("Bundle 2", { timeout: 20_000 });
  const imported = (await state(page)).project;
  expect(imported.id).not.toBe(original.id);
  expect(Object.keys(imported.tracks).sort()).toEqual(Object.keys(original.tracks).sort());
  const media = Object.values(imported.media);
  expect(media.map((m) => [m.name, m.file, m.frames])).toEqual(Object.values(original.media).map((m) => [m.name, m.file, m.frames]));
  // The media file came with the bundle: same bytes in the imported project's OPFS folder.
  const size = (id: string, file: string) =>
    page.evaluate(
      async ({ id, file }) => {
        let dir = await (await navigator.storage.getDirectory()).getDirectoryHandle("ethereal");
        const parts = ["projects", id, ...file.split("/")];
        for (const p of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(p);
        return (await (await dir.getFileHandle(parts.at(-1)!)).getFile()).size;
      },
      { id, file },
    );
  const pad = media[0]!;
  expect(await size(imported.id, pad.file)).toBe(await size(original.id, pad.file));
});
