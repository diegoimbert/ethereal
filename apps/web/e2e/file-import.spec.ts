// Importing audio from the user's computer (file-import, CONTRACTS.md §12.13) on the web
// build: the files' bytes are uploaded to the in-browser engine (staged in OPFS) and copied
// into the project. "Import audio…" (⌘I) → the browser's file picker → clips on new tracks at
// the playhead; OS drops on a track lane, below the tracks and on the sample browser; bad
// files are reported in the import status list.
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, launch, newProject, openLibrary } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

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

interface DropFile {
  name: string;
  bytes: Buffer;
}

/** Drop OS files on `target` at (`dx`, `dy`) from its top-left (default: its center), as a file manager does. */
async function dropFiles(target: Locator, files: DropFile[], at?: { dx: number; dy: number }): Promise<void> {
  const box = (await target.boundingBox())!;
  const x = box.x + (at?.dx ?? box.width / 2);
  const y = box.y + (at?.dy ?? box.height / 2);
  const payload = files.map((f) => ({ name: f.name, data: f.bytes.toString("base64") }));
  await target.evaluate(
    (el, { payload, x, y }) => {
      const dt = new DataTransfer();
      for (const f of payload) {
        const bin = atob(f.data);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        dt.items.add(new File([bytes], f.name));
      }
      const at = document.elementFromPoint(x, y) ?? el;
      for (const type of ["dragenter", "dragover", "drop"]) {
        at.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: dt, clientX: x, clientY: y }));
      }
    },
    { payload, x, y },
  );
}

const count = (o: object) => Object.keys(o).length;

test.beforeEach(async ({ page }) => {
  await launch(page);
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
});

test("Import audio… (⌘I) puts the picked files on new tracks at the playhead", async ({ page }) => {
  await newProject(page, "Import picker");
  const before = await project(page);
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("ControlOrMeta+i");
  await (
    await chooser
  ).setFiles([
    { name: "Pad.wav", mimeType: "audio/wav", buffer: sineWav(1) },
    { name: "Bass.wav", mimeType: "audio/wav", buffer: sineWav(0.5) },
  ]);
  await expect.poll(async () => count((await project(page)).clips), { timeout: 20_000 }).toBe(count(before.clips) + 2);
  const p = await project(page);
  expect(Object.values(p.media).map((m) => m.name).sort()).toEqual(["Bass.wav", "Pad.wav"]);
  expect(Object.values(p.media).every((m) => m.frames > 0 && m.file.startsWith("media/"))).toBe(true);
  expect(count(p.tracks)).toBe(count(before.tracks) + 2);
  await expect(page.getByTestId("import-status")).toHaveCount(0);
  // One undo step for the whole import.
  await page.getByTestId("arrangement-content").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => count((await project(page)).clips)).toBe(count(before.clips));
  await expect.poll(async () => count((await project(page)).tracks)).toBe(count(before.tracks));
});

test("OS drops on a lane and below the tracks; bad files are reported", async ({ page }) => {
  await newProject(page, "Import drops");
  const track = await createTrack(page, "Audio");
  const lane = page.locator(`[data-lane="${track.id}"]`);
  await dropFiles(lane, [{ name: "Kick.wav", bytes: sineWav(0.25) }], { dx: 120, dy: 10 });
  await expect.poll(async () => Object.values((await project(page)).clips).filter((c) => c.track === track.id).length, { timeout: 20_000 }).toBe(1);

  const tracks = count((await project(page)).tracks);
  await dropFiles(page.locator(".eth-arr__drop-area"), [
    { name: "Loop.wav", bytes: sineWav(0.5) },
    { name: "notes.txt", bytes: Buffer.from("not audio") },
    { name: "broken.wav", bytes: Buffer.from("RIFF....WAVE") },
  ]);
  await expect.poll(async () => count((await project(page)).tracks), { timeout: 20_000 }).toBe(tracks + 1);
  const rows = page.getByTestId("import-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText("notes.txt: .txt files are not supported");
  await expect(rows.nth(1)).toContainText("broken.wav: not a readable audio file");
  await rows.nth(0).getByRole("button", { name: "Dismiss" }).click();
  await rows.nth(0).getByRole("button", { name: "Dismiss" }).click();
  await expect(page.getByTestId("import-status")).toHaveCount(0);
});

test("files dropped on the browser panels are added to the project's media", async ({ page }) => {
  await newProject(page, "Import browser");
  // The library panel: added to the project (no clip).
  await openLibrary(page);
  await dropFiles(page.locator('[data-feature="browser"]'), [{ name: "Vox.wav", bytes: sineWav(0.3) }]);
  await expect.poll(async () => Object.values((await project(page)).media).map((m) => m.name), { timeout: 20_000 }).toEqual(["Vox.wav"]);
  expect(count((await project(page)).clips)).toBe(0);
  // The project media panel lists it, and takes drops too.
  await page.getByRole("navigation", { name: "Panels" }).getByRole("button", { name: "Project media", exact: true }).click();
  const media = page.locator('[data-feature="browser"]');
  await expect(media.getByRole("button", { name: /Vox\.wav$/ })).toBeVisible();
  await dropFiles(media, [{ name: "Hat.wav", bytes: sineWav(0.1) }]);
  await expect(media.getByRole("button", { name: /Hat\.wav$/ })).toBeVisible({ timeout: 20_000 });
});
