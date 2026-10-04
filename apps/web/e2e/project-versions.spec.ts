// Project versions and crash recovery (project-versions, CONTRACTS.md §13.11) on the web
// build: versions are `.ether` files in the project folder in the browser's storage (OPFS).
//
// 1. Save a named version from the versions dialog (Projects → Versions…), change the
//    project, compare, restore it, and undo the restore by restoring "Before restore".
// 2. Crash recovery: a reload kills the engine's Worker without a clean close, so the
//    session marker survives. The test leaves unsaved work behind the way a crash between
//    the last autosave version and the next save does (a version newer than `project.ether`,
//    with different content), reloads, and recovers it from the startup dialog.
//
// `PROJECT_VERSIONS_SHOTS=<dir>` also captures the PR screenshots (dark and light).
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, launch, newProject, openOnLaunch, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null; dirty: boolean };
}

const shots = process.env.PROJECT_VERSIONS_SHOTS;
test.use({ viewport: { width: 1440, height: 900 } });

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state());
const trackKinds = async (page: Page) =>
  Object.values((await state(page)).project?.tracks ?? {})
    .map((t) => t.kind)
    .filter((k) => k !== "Master")
    .sort();

async function shot(page: Page, name: string): Promise<void> {
  if (!shots) return;
  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    await page.screenshot({ path: `${shots}/${name}-${theme}.png` });
  }
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
}

async function saved(page: Page): Promise<void> {
  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByRole("status", { name: "Unsaved changes" })).toHaveCount(0);
}

test("versions: save, compare, restore; a killed session offers recovery", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, "Versions demo");
  await createTrack(page, "Midi");
  await saved(page);

  // Projects → Versions…
  await page.getByRole("button", { name: "Projects" }).click();
  await page.getByRole("dialog", { name: "Projects" }).getByRole("button", { name: "Versions…" }).click();
  const dialog = page.getByRole("dialog", { name: "Versions of “Versions demo”" });
  await expect(dialog.getByText(/No versions yet/)).toBeVisible();
  await dialog.getByLabel("Version name").fill("Verse idea");
  await dialog.getByRole("button", { name: "Save version" }).click();
  await expect(dialog.getByText("Verse idea", { exact: true })).toBeVisible();

  // Change the project, then compare it with the version.
  await dialog.press("Escape");
  await expect(dialog).toHaveCount(0);
  await createTrack(page, "Audio");
  expect(await trackKinds(page)).toEqual(["Audio", "Midi"]);
  await saved(page);
  await page.getByRole("button", { name: "Projects" }).click();
  await page.getByRole("dialog", { name: "Projects" }).getByRole("button", { name: "Versions…" }).click();
  await dialog.getByRole("button", { name: "Compare Verse idea with now" }).click();
  const diff = dialog.getByLabel("Changes since this version");
  await expect(diff).toContainText("Tracks");
  await expect(diff).toContainText("+1");
  await shot(page, "01-versions-compare");

  // Restore: the audio track goes; the state before is kept as a version.
  await dialog.getByRole("button", { name: "Restore Verse idea" }).click();
  await expect.poll(() => trackKinds(page)).toEqual(["Midi"]);
  await expect(dialog.getByRole("status")).toContainText("Restored “Verse idea”");
  await expect(dialog.getByText("Before restore").first()).toBeVisible();
  await shot(page, "02-versions-restored");
  await dialog.getByRole("button", { name: "Restore Before restore" }).click();
  await expect.poll(() => trackKinds(page)).toEqual(["Audio", "Midi"]);
  await dialog.press("Escape");
  await expect(dialog).toHaveCount(0);
  await saved(page);
  const pid = (await state(page)).project!.id;

  // Unsaved work left by a crash: a version newer than the saved file, with other content
  // (the "Verse idea" state, MIDI track only).
  await page.evaluate(async (pid) => {
    const root = await (await navigator.storage.getDirectory()).getDirectoryHandle("ethereal");
    const versions = await (await (await root.getDirectoryHandle("projects")).getDirectoryHandle(pid)).getDirectoryHandle("versions");
    let source: File | null = null;
    for await (const [name, handle] of versions as unknown as AsyncIterable<[string, FileSystemFileHandle]>) {
      if (name.endsWith("-manual.ether")) source = await handle.getFile();
    }
    const file = await versions.getFileHandle(`${Date.now() + 1000}-autosave.ether`, { create: true });
    const w = await file.createWritable();
    await w.write(await source!.text());
    await w.close();
  }, pid);

  // Reload = the Worker is killed (no clean close): the marker survives.
  await page.reload();
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  const recovery = page.getByRole("dialog", { name: "Recover unsaved work?" });
  await expect(recovery).toBeVisible({ timeout: 30_000 });
  await expect(recovery).toContainText("Versions demo");
  await shot(page, "03-recovery-dialog");
  await recovery.getByRole("button", { name: "Recover Versions demo" }).click();
  await expect(recovery).toHaveCount(0);
  await expect.poll(async () => (await state(page)).project?.id).toBe(pid);
  await expect.poll(() => trackKinds(page)).toEqual(["Midi"]);
  // Unsaved (dirty) until saved; then nothing is left to recover.
  await saved(page);
  await page.reload();
  await openOnLaunch(page, "Versions demo");
  await expect.poll(async () => (await state(page)).project?.id, { timeout: 30_000 }).toBe(pid);
  await expect(page.getByRole("dialog", { name: "Recover unsaved work?" })).toHaveCount(0);
  expect(await trackKinds(page)).toEqual(["Midi"]);
});
