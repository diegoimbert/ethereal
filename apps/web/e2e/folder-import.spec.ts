// Importing folders into the library on the web build (base-136), against the real engine
// (WasmTransport → controller Worker → OPFS):
// - "Import folder…" with a mocked `showDirectoryPicker` (Chromium's File System Access):
//   the folder's audio is copied into OPFS under a new library root (sub-folders kept, the
//   text file skipped and counted), with progress (held mid-copy by a gate for the
//   screenshot); the root becomes a place, browsable and indexed; one of its files dragged
//   onto an audio lane creates a clip; after a reload the place is still there.
// - The `<input webkitdirectory>` fallback (Firefox / Safari), fed a real folder.
// Screenshots (both themes): the places with "Import folder…", the progress, the imported
// folder. Also written to `$FOLDER_IMPORT_SHOTS/` when that names a directory.
//
// No sleeps: every step waits on UI or engine state.
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test, type Locator, type Page, type TestInfo } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, newProject, openLibrary, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const doc = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

const panel = (page: Page) => page.locator('[data-feature="browser"][data-browser="v2"]');
const places = (page: Page) => panel(page).getByRole("tablist", { name: "Locations" });
const files = (page: Page) => panel(page).getByRole("list", { name: "Files" });

/** A mono 16-bit WAV of a sine (`seconds` long). */
function wav(seconds: number, hz: number): Buffer {
  const rate = 44_100;
  const frames = Math.round(seconds * rate);
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
  for (let i = 0; i < frames; i++) b.writeInt16LE(Math.round(Math.sin((2 * Math.PI * hz * i) / rate) * 12_000), 44 + i * 2);
  return b;
}

async function shoot(page: Page, info: TestInfo, name: string) {
  const shot = await panel(page).screenshot({ timeout: 60_000 });
  await info.attach(name, { body: shot, contentType: "image/png" });
  await writeFile(info.outputPath(`${name}.png`), shot);
  const dir = process.env.FOLDER_IMPORT_SHOTS;
  if (dir) await writeFile(join(dir, `${name}.png`), shot);
}

/** Drag a library row onto the lane of `track`; resolves once the clip exists. */
async function dropOnLane(page: Page, row: Locator, track: string) {
  await row.dragTo(page.locator(`[data-lane="${track}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect
    .poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === track).length, { timeout: 20_000 })
    .toBe(1);
}

async function boot(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => doc(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

for (const theme of ["dark", "light"] as const) {
  test(`folder import: showDirectoryPicker → OPFS library root → drag onto a lane (${theme})`, async ({ page }, info) => {
    test.setTimeout(180_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    const folder = `Field Kit ${theme}`;
    const kick = wav(0.4, 60).toString("base64");
    const hat = wav(0.2, 4000).toString("base64");
    const loop = wav(2, 220).toString("base64");
    await page.addInitScript(
      ({ t, name, kick, hat, loop }) => {
        localStorage.setItem("eth-theme", t);
        const bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
        // The copy of "Kick.wav" (2nd in path order) waits for `__releaseImport()` (the progress screenshot).
        let release: () => void = () => undefined;
        const gate = new Promise<void>((ok) => (release = ok));
        (window as unknown as { __releaseImport(): void }).__releaseImport = () => release();
        const held = (f: File) => ({
          name: f.name,
          size: f.size,
          slice: (a: number, b: number) => ({ arrayBuffer: async () => (await gate, f.slice(a, b).arrayBuffer()) }),
        });
        const file = (n: string, b: Uint8Array<ArrayBuffer>, gated = false) => {
          const f = new File([b], n);
          return { kind: "file", name: n, getFile: async () => (gated ? held(f) : f) };
        };
        const dir = (n: string, entries: unknown[]) => ({
          kind: "directory",
          name: n,
          async *values() {
            yield* entries;
          },
        });
        const root = dir(name, [
          file("Kick.wav", bytes(kick), true),
          file("Hat.wav", bytes(hat)),
          file("notes.txt", new TextEncoder().encode("not audio") as Uint8Array<ArrayBuffer>),
          file(".DS_Store", new Uint8Array(4)),
          dir("Loops", [file("Groove 120.wav", bytes(loop))]),
        ]);
        (window as unknown as { showDirectoryPicker(): Promise<unknown> }).showDirectoryPicker = async () => root;
      },
      { t: theme, name: folder, kick, hat, loop },
    );
    await boot(page);
    await newProject(page, `Folder import ${theme} ${Date.now()}`);
    const audio = await createTrack(page, "Audio");
    await openLibrary(page);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    // --- The places end with "Import folder…" (no native folder dialog on the web) ------------
    const importButton = places(page).getByRole("button", { name: /Import folder…/ });
    await expect(importButton).toBeVisible();
    await shoot(page, info, `folder-import-places-${theme}`);

    // --- Import: progress (held mid-copy), then done ---------------------------------------------
    await importButton.click();
    const status = panel(page).getByRole("list", { name: "Folder imports" });
    await expect(status.getByText(`Importing “${folder}”: 1 of 3 files`, { exact: false })).toBeVisible({ timeout: 30_000 });
    await expect(status.getByRole("progressbar")).toBeVisible();
    await expect(status.getByRole("button", { name: `Cancel importing ${folder}` })).toBeVisible();
    await shoot(page, info, `folder-import-progress-${theme}`);
    await page.evaluate(() => (window as unknown as { __releaseImport(): void }).__releaseImport());
    await expect(status.getByText(`Imported 3 files into “${folder}” (copied into this browser’s storage) · 1 file skipped`, { exact: false })).toBeVisible({
      timeout: 30_000,
    });

    // --- The new place: browsable, sub-folders kept --------------------------------------------
    const place = places(page).getByRole("tab", { name: folder });
    await expect(place).toHaveAttribute("aria-selected", "true");
    await expect(files(page).getByRole("button", { name: "Loops" })).toBeVisible({ timeout: 20_000 });
    await expect(files(page).getByRole("button", { name: "Kick.wav", exact: true })).toBeVisible();
    await expect(files(page).getByRole("button", { name: "Hat.wav", exact: true })).toBeVisible();
    await expect(files(page).getByRole("button", { name: "notes.txt" })).toHaveCount(0);
    await shoot(page, info, `folder-import-done-${theme}`);

    // --- Indexed: search finds the loop inside the sub-folder ------------------------------------
    await panel(page).getByRole("searchbox", { name: "Search files" }).fill("groove");
    await expect(panel(page).getByRole("list", { name: "Search results" }).getByRole("button", { name: "Groove 120.wav", exact: true })).toBeVisible({
      timeout: 30_000,
    });
    await panel(page).getByRole("searchbox", { name: "Search files" }).fill("");

    // --- Drag one file onto the audio lane: a clip ---------------------------------------------
    await dropOnLane(page, files(page).getByRole("button", { name: "Kick.wav", exact: true }), audio.id);
    expect(Object.values((await doc(page)).media).some((m) => m.name === "Kick.wav")).toBe(true);

    // --- Reload: the place is still there -------------------------------------------------------
    await page.reload();
    await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
    await panel(page)
      .waitFor({ timeout: 5_000 })
      .catch(() => openLibrary(page, null));
    await places(page).getByRole("tab", { name: folder }).click();
    await expect(files(page).getByRole("button", { name: "Kick.wav", exact: true })).toBeVisible({ timeout: 20_000 });

    // Clean up (OPFS persists across tests): removing deletes the copy.
    await places(page).getByRole("tab", { name: folder }).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Remove folder" }).click();
    await expect(places(page).getByRole("tab", { name: folder })).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}

test("folder import: <input webkitdirectory> fallback (no showDirectoryPicker)", async ({ page }) => {
  test.setTimeout(120_000);
  const dir = await mkdtemp(join(tmpdir(), "eth-folder-"));
  const root = join(dir, "Fallback Kit");
  await mkdir(join(root, "Toms"), { recursive: true });
  await writeFile(join(root, "Clap.wav"), wav(0.3, 1000));
  await writeFile(join(root, "Toms", "Tom Low.wav"), wav(0.3, 90));
  await writeFile(join(root, "cover.jpg"), Buffer.from([0xff, 0xd8, 0xff]));
  await page.addInitScript(() => {
    delete (window as unknown as { showDirectoryPicker?: unknown }).showDirectoryPicker;
  });
  await boot(page);
  await openLibrary(page);
  const chooser = page.waitForEvent("filechooser");
  await places(page).getByRole("button", { name: /Import folder…/ }).click();
  await (await chooser).setFiles(root);
  const status = panel(page).getByRole("list", { name: "Folder imports" });
  await expect(status.getByText("Imported 2 files into “Fallback Kit”", { exact: false })).toBeVisible({ timeout: 30_000 });
  await expect(status.getByText("1 file skipped", { exact: false })).toBeVisible();
  await expect(places(page).getByRole("tab", { name: "Fallback Kit" })).toHaveAttribute("aria-selected", "true");
  await files(page).getByRole("button", { name: "Toms" }).click();
  await expect(files(page).getByRole("button", { name: "Tom Low.wav", exact: true })).toBeVisible({ timeout: 20_000 });
  await places(page).getByRole("tab", { name: "Fallback Kit" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove folder" }).click();
  await expect(places(page).getByRole("tab", { name: "Fallback Kit" })).toHaveCount(0);
});
