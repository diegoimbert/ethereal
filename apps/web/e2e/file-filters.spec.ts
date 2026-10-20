// Library file filters on the web build, against the real engine (WasmTransport →
// controller Worker → OPFS): a library folder holding a mix of files (two samples, an Ableton
// set and its `.asd`, Windows' `Thumbs.db`, a text file) lists only the samples in the
// library's folder tree; Settings > General > Library files marks a rule that isn't a regex,
// and a "Hide" rule takes a sample out of the tree at once. (Since the UX session, the
// tree lists only files of the chosen kind, so a "Show" rule for another type, e.g. `.als`,
// has no visible effect there.) Screenshots in both themes (the filtered folder, the settings
// section with an invalid rule, the folder after the rule); also written to
// `$FILE_FILTER_SHOTS/` when that names a directory.
//
// No sleeps: every step waits on UI state.
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Locator, type Page, type TestInfo } from "@playwright/test";
import { expandFolder, launch, libraryTree, openLibrary, playButton } from "./ui";

const panel = (page: Page) => page.locator('[data-feature="browser"][data-browser="v2"]');
/** A file or folder row of the library's folder tree. */
const row = (page: Page, name: string) => libraryTree(page).getByRole("treeitem", { name, exact: true });

/** A mono 16-bit WAV of silence (`frames` long). */
function wav(frames: number): number[] {
  const b = new DataView(new ArrayBuffer(44 + frames * 2));
  const text = (at: number, s: string) => [...s].forEach((c, i) => b.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF");
  b.setUint32(4, 36 + frames * 2, true);
  text(8, "WAVEfmt ");
  b.setUint32(16, 16, true);
  b.setUint16(20, 1, true);
  b.setUint16(22, 1, true);
  b.setUint32(24, 44_100, true);
  b.setUint32(28, 88_200, true);
  b.setUint16(32, 2, true);
  b.setUint16(34, 16, true);
  text(36, "data");
  b.setUint32(40, frames * 2, true);
  return [...new Uint8Array(b.buffer)];
}

/** Write `files` (name → bytes) into the web engine's library folder `Mixed/` (OPFS). */
async function seedFolder(page: Page, files: Record<string, number[]>) {
  await page.evaluate(async (files) => {
    const root = await (await navigator.storage.getDirectory()).getDirectoryHandle("ethereal", { create: true });
    const lib = await root.getDirectoryHandle("library", { create: true });
    const dir = await lib.getDirectoryHandle("Mixed", { create: true });
    for (const [name, bytes] of Object.entries(files)) {
      const w = await (await dir.getFileHandle(name, { create: true })).createWritable();
      await w.write(new Uint8Array(bytes));
      await w.close();
    }
  }, files);
}

async function shoot(target: Locator, info: TestInfo, name: string) {
  const shot = await target.screenshot({ timeout: 60_000 });
  await info.attach(name, { body: shot, contentType: "image/png" });
  await writeFile(info.outputPath(`${name}.png`), shot);
  const dir = process.env.FILE_FILTER_SHOTS;
  if (dir) await writeFile(join(dir, `${name}.png`), shot);
}

for (const theme of ["dark", "light"] as const) {
  test(`library file filters: supported files only, a Hide rule hides files (${theme})`, async ({ page }, info) => {
    test.setTimeout(120_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    await launch(page);
    await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
    const text = (s: string) => [...s].map((c) => c.charCodeAt(0));
    await seedFolder(page, {
      "Kick.wav": wav(4_410),
      "Draft snare.wav": wav(4_410),
      "Kick.wav.asd": text("asd"),
      "Live Set.als": text("als"),
      "Thumbs.db": text("db"),
      "notes.txt": text("notes"),
    });

    // --- The folder tree lists only what Ethereal can use -----------------------------------
    await openLibrary(page);
    await expect(panel(page)).toBeVisible({ timeout: 20_000 });
    await expandFolder(page, "Mixed");
    await expect(row(page, "Kick.wav")).toBeVisible({ timeout: 20_000 });
    await expect(row(page, "Draft snare.wav")).toBeVisible();
    for (const hidden of ["Kick.wav.asd", "Live Set.als", "Thumbs.db", "notes.txt"]) {
      await expect(libraryTree(page).getByText(hidden, { exact: true })).toHaveCount(0);
    }
    await shoot(panel(page), info, `file-filters-folder-${theme}`);

    // --- Settings > General > Library files -------------------------------------------------
    await page.getByRole("button", { name: "settings", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "settings" });
    await dialog.getByRole("tab", { name: "general" }).click();
    const section = dialog.getByTestId("settings-library-filters");
    await expect(section).toBeVisible();
    const showRules = section.getByRole("group", { name: "show rules" });
    await expect(showRules.getByRole("textbox")).toHaveCount(3);
    await expect(section.getByRole("button", { name: "reset to defaults" })).toBeDisabled();
    // The live count covers the files the index has seen.
    await expect(section.getByTestId("file-filter-count")).toHaveText(/^shows \d+ of \d+ files in your library/, { timeout: 30_000 });

    // A rule that isn't a regex is marked and not applied.
    const hideRules = section.getByRole("group", { name: "hide rules" });
    const defaultHides = await hideRules.getByRole("textbox").count();
    await section.getByRole("button", { name: "add hide rule" }).click();
    const rule = hideRules.getByRole("textbox", { name: `hide rule ${defaultHides + 1}` });
    await expect(rule).toBeFocused();
    await rule.fill(String.raw`^(draft`);
    await expect(rule).toHaveAttribute("aria-invalid", "true");
    await expect(section.getByTestId("file-filter-count")).toHaveText("fix the marked rules to apply them.");
    await shoot(section, info, `file-filters-settings-${theme}`);

    // Fixed and saved (Enter): the draft sample leaves the tree at once, the other stays.
    await rule.fill(String.raw`^draft `);
    await expect(rule).not.toHaveAttribute("aria-invalid");
    await rule.press("Enter");
    await expect(section.getByRole("button", { name: "reset to defaults" })).toBeEnabled();
    await dialog.getByRole("button", { name: "done" }).click();
    await expect(dialog).toHaveCount(0);
    await expect(row(page, "Draft snare.wav")).toHaveCount(0, { timeout: 20_000 });
    await expect(row(page, "Kick.wav")).toBeVisible();
    await shoot(panel(page), info, `file-filters-after-rule-${theme}`);

    // Back to the defaults: shown again.
    await page.getByRole("button", { name: "settings", exact: true }).click();
    await dialog.getByRole("tab", { name: "general" }).click();
    await section.getByRole("button", { name: "reset to defaults" }).click();
    await expect(hideRules.getByRole("textbox")).toHaveCount(defaultHides);
    await dialog.getByRole("button", { name: "done" }).click();
    await expect(row(page, "Draft snare.wav")).toBeVisible({ timeout: 20_000 });
    expect(errors).toEqual([]);
  });
}
