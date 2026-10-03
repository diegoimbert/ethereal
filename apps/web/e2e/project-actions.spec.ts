// Project actions on the web build (base-114): Save as, Duplicate, a `.ether` bundle export
// (browser download) imported back (browser upload) with its media, and the warning before
// opening another project while in a collaboration session (local `ether-collab-relay`).
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Project, ProjectSummary } from "@/generated";
import { newProject, openRelayJoin } from "./ui";

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
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => state(page).then((s) => s.project !== null), { timeout: 30_000 }).toBe(true);
}

/** The project screen's "Open project" actions. */
async function projectAction(page: Page, label: string) {
  await page.getByRole("button", { name: "Projects" }).click();
  const screen = page.getByRole("dialog", { name: "Projects" });
  await screen.getByRole("group", { name: "Project actions" }).getByRole("button", { name: label }).click();
  return screen;
}

test("Save as and Duplicate", async ({ page }) => {
  await boot(page);
  await newProject(page, "Song");
  const id = (await state(page)).project.id;

  // Duplicate: a stored copy; the open project stays.
  const screen = await projectAction(page, "Duplicate");
  await expect.poll(async () => (await state(page)).names).toContain("Song copy");
  await expect(screen.getByRole("button", { name: "Open Song copy" })).toBeVisible();
  expect((await state(page)).project.id).toBe(id);
  await expect(page.getByRole("status").filter({ hasText: "Duplicated as “Song copy”" })).toBeVisible();
  await screen.getByRole("button", { name: "Continue" }).click();
  await expect(screen).toBeHidden();

  // Save as: switches to the new copy.
  await projectAction(page, "Save as…");
  await page.getByLabel("Save as name").fill("Song v2");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Song v2");
  const after = await state(page);
  expect(after.project.id).not.toBe(id);
  expect(after.names).toEqual(expect.arrayContaining(["Song", "Song copy", "Song v2"]));
});

test("export a bundle, then import it back with its media", async ({ page }) => {
  await boot(page);
  await newProject(page, "Bundle");
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("ControlOrMeta+i");
  await (await chooser).setFiles([{ name: "Pad.wav", mimeType: "audio/wav", buffer: sineWav(0.5) }]);
  await expect.poll(async () => Object.keys((await state(page)).project.media).length, { timeout: 20_000 }).toBe(1);
  const original = (await state(page)).project;

  // Export: a browser download of a ZIP-based .ether file.
  const download = page.waitForEvent("download");
  await projectAction(page, "Export…");
  const file = await download;
  expect(file.suggestedFilename()).toBe("Bundle.ether");
  const bytes = readFileSync(await file.path());
  expect(bytes.subarray(0, 4).toString("latin1")).toBe("PK\u0003\u0004");
  await expect(page.getByRole("status").filter({ hasText: "Exported “Bundle”" })).toBeVisible();

  // Import: the browser's file picker; the copy gets a free name and opens.
  const picker = page.waitForEvent("filechooser");
  await page.getByRole("dialog", { name: "Projects" }).getByRole("button", { name: "Import…" }).click();
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

test.describe("in a collaboration session", () => {
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
  const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
  const SESSION = `e2e-${Math.random().toString(36).slice(2, 8)}`;
  let relay: ChildProcess | null = null;
  let relayUrl = "";

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    const out = execFileSync("cargo", ["build", "-p", "ether-collab", "--bin", "ether-collab-relay", "--message-format=json"], {
      cwd: repoRoot,
      env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
    });
    let bin = "";
    for (const line of out.split("\n")) {
      if (!line.startsWith("{")) continue;
      const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
      if (msg.reason === "compiler-artifact" && msg.target?.name === "ether-collab-relay" && msg.executable) bin = msg.executable;
    }
    if (!bin) throw new Error("cargo did not report the ether-collab-relay binary");
    const child = spawn(bin, ["--port", "0", "--token", TOKEN], { env: { ...process.env, RUST_LOG: "warn" }, stdio: ["ignore", "pipe", "inherit"] });
    relay = child;
    relayUrl = await new Promise<string>((resolveUrl, reject) => {
      let buf = "";
      child.stdout!.on("data", (d: Buffer) => {
        buf += d.toString();
        const m = /listening on (ws:\/\/\S+)/.exec(buf);
        if (m) resolveUrl(m[1]!);
      });
      child.on("exit", (code) => reject(new Error(`ether-collab-relay exited (${code})`)));
    });
  });

  test.afterAll(() => {
    relay?.kill();
  });

  test("opening another project asks to leave the session first", async ({ page }) => {
    test.setTimeout(120_000);
    await boot(page);
    await newProject(page, "Other");
    await newProject(page, "Shared");

    // Join (the session name is checked inline first). The relay join lives in
    // Settings > Advanced since the Share rework (#202).
    await openRelayJoin(page);
    await page.getByLabel("Relay address").fill(relayUrl);
    await page.getByLabel("Session").fill("not valid!");
    await expect(page.getByRole("alert").filter({ hasText: "Use only letters, digits" })).toBeVisible();
    await page.getByLabel("Session").fill(SESSION);
    await expect(page.getByRole("alert")).toHaveCount(0);
    await page.getByLabel("Your name").fill("Ada");
    await page.getByLabel("Token").fill(TOKEN);
    await page.getByRole("button", { name: "Join" }).click();
    await expect(page.getByTestId("collab-button")).toHaveText(`● ${SESSION}`, { timeout: 20_000 });

    // Cancel keeps the session and the project.
    await page.getByRole("button", { name: "Projects" }).click();
    const screen = page.getByRole("dialog", { name: "Projects" });
    await screen.getByRole("button", { name: /^Open Other/ }).click();
    const ask = page.getByRole("dialog", { name: `Leave the “${SESSION}” session?` });
    await expect(ask).toBeVisible();
    await ask.getByRole("button", { name: "Cancel" }).click();
    await expect(ask).toBeHidden();
    await expect(page.getByTestId("project-name")).toHaveText("Shared");
    await expect(page.getByTestId("collab-button")).toHaveText(`● ${SESSION}`);

    // "Leave & open" leaves, then opens.
    await screen.getByRole("button", { name: /^Open Other/ }).click();
    await ask.getByRole("button", { name: "Leave & open" }).click();
    await expect(page.getByTestId("project-name")).toHaveText("Other");
    // Out of a session the collab button is hidden.
    await expect(page.getByTestId("collab-button")).toHaveCount(0);
  });
});
