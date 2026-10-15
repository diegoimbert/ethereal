// Missing samples and Relink (media-references, CONTRACTS.md §12.9) on the web build. The
// web engine can't reference OS files in place (uploads are copied into the project), but a
// project file can still go missing: the test deletes an imported sample from the browser's
// storage (OPFS), reloads, and checks that the engine reports it missing when the project
// opens (a marked clip, the notice), then relinks it through the Relink dialog's "Locate…"
// (the browser's file picker; the bytes are uploaded).
//
// `MEDIA_REFS_SHOTS=<dir>` also captures the PR screenshots (dark and light).
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { clipDecor, clipMenu } from "./clips";
import { launch, newProject, openOnLaunch, playButton, resolveLater } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const shots = process.env.MEDIA_REFS_SHOTS;
test.use({ viewport: { width: 1440, height: 900 } });

const project = (page: Page): Promise<Project> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

/** A mono 16-bit WAV: `seconds` of a sine at 48 kHz. */
function sineWav(seconds: number, hz: number): Buffer {
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
  for (let i = 0; i < frames; i++) b.writeInt16LE(Math.round(12000 * Math.sin((2 * Math.PI * hz * i) / rate)), 44 + i * 2);
  return b;
}

async function shot(page: Page, name: string): Promise<void> {
  if (!shots) return;
  for (const theme of ["dark", "light"] as const) {
    // `<html data-theme>` is what the theme toggle sets (the dialog's backdrop covers it).
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    await page.screenshot({ path: `${shots}/${name}-${theme}.png` });
  }
}

test("a sample deleted from the project is reported missing and relinked", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, "Missing samples");
  const kick = sineWav(4, 110);
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("ControlOrMeta+i");
  await (
    await chooser
  ).setFiles([
    { name: "Kick.wav", mimeType: "audio/wav", buffer: kick },
    { name: "Pad.wav", mimeType: "audio/wav", buffer: sineWav(4, 330) },
  ]);
  await expect.poll(async () => Object.keys((await project(page)).clips).length, { timeout: 20_000 }).toBe(2);
  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0);
  const p = await project(page);
  const media = Object.values(p.media).find((m) => m.name === "Kick.wav")!;
  const clip = Object.values(p.clips).find((c) => c.content.type === "Audio" && c.content.media === media.id)!;
  expect(media.location).toEqual({ type: "Project" });

  // The sample file disappears from the project folder (browser storage).
  await page.evaluate(
    async ({ pid, file }) => {
      const root = await (await navigator.storage.getDirectory()).getDirectoryHandle("ethereal");
      let dir = await (await root.getDirectoryHandle("projects")).getDirectoryHandle(pid);
      const parts = file.split("/");
      for (const d of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(d);
      await dir.removeEntry(parts.at(-1)!);
    },
    { pid: p.id, file: media.file },
  );
  await page.reload();
  await openOnLaunch(page, "Missing samples");
  await expect.poll(async () => (await project(page))?.id, { timeout: 30_000 }).toBe(p.id);
  // Opening it lists what is missing: later, from the clip and the notice.
  await resolveLater(page);

  // Detected on open: the clip is marked, a notice offers to relink.
  const badges = async (id: string) => (await clipDecor(page, id))?.badges ?? null;
  await expect.poll(() => badges(clip.id), { timeout: 20_000 }).toContain("missing");
  const notice = page.getByTestId("missing-media-notice");
  await expect(notice).toContainText("1 sample is missing");
  const pad = Object.values(p.clips).find((c) => c.id !== clip.id)!;
  await expect.poll(() => badges(pad.id)).toEqual([]);
  await shot(page, "01-missing-clip");

  // The clip's menu offers Relink too.
  await clipMenu(page, clip.id);
  await expect(page.getByRole("menuitem", { name: "relink sample…" })).toBeVisible();
  await page.keyboard.press("Escape");

  await notice.getByRole("button", { name: "relink…" }).click();
  const dialog = page.getByRole("dialog", { name: "missing samples" });
  const row = dialog.getByTestId("relink-row");
  await expect(row).toContainText("Kick.wav");
  await expect(row).toContainText("missing");
  await shot(page, "02-relink-dialog");

  // Locate… → the browser's file picker → uploaded and relinked (one undo step).
  const picker = page.waitForEvent("filechooser");
  await row.getByRole("button", { name: "locate…" }).click();
  await (await picker).setFiles([{ name: "Kick.wav", mimeType: "audio/wav", buffer: kick }]);
  await expect(row).toContainText("found", { timeout: 20_000 });
  await expect(dialog.getByTestId("relink-summary")).toHaveText("every sample is linked.");
  await shot(page, "03-relinked");
  await dialog.getByRole("button", { name: "done" }).click();
  await expect.poll(() => badges(clip.id)).not.toContain("missing");
  await expect(notice).toHaveCount(0);
});
