// import-dawproject: "Import from another DAW…" on the web build (real wasm engine): a
// DAWproject exported by Bitwig Studio and a REAPER project, each picked (project screen,
// command palette) and dropped on the window, become new projects that open, with the
// import report.
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { importFixtures } from "./importFixtures";
import { launch, projectScreen, resolveLater } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const fixtures = importFixtures((p) => readFileSync(resolve(repoRoot, p)));

const project = (page: Page) => page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project!);

/** The launch project screen (no project open). */
async function boot(page: Page) {
  await page.goto("/");
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
}

/** Tempo points' BPMs, in time order. */
const bpms = (p: Project) =>
  Object.values(p.tempo_points)
    .sort((a, b) => a.time - b.time)
    .map((t) => t.bpm);

/** Drop a file on the app shell, as an OS file drop does. */
async function dropFile(page: Page, name: string, type: string, bytes: Buffer) {
  await page.evaluate(
    ({ name, type, b64 }) => {
      const bin = atob(b64);
      const data = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i);
      const dt = new DataTransfer();
      dt.items.add(new File([data], name, { type }));
      const target = document.querySelector(".eth-shell") ?? document.body;
      target.dispatchEvent(new DragEvent("dragover", { dataTransfer: dt, bubbles: true, cancelable: true }));
      target.dispatchEvent(new DragEvent("drop", { dataTransfer: dt, bubbles: true, cancelable: true }));
    },
    { name, type, b64: bytes.toString("base64") },
  );
}

test("import a DAWproject from Bitwig Studio through the project screen", async ({ page }) => {
  await boot(page);
  const chooser = page.waitForEvent("filechooser");
  await projectScreen(page).getByRole("button", { name: "import from another DAW…" }).click();
  await (await chooser).setFiles([{ name: "Canon.dawproject", mimeType: "application/zip", buffer: fixtures.canon }]);

  const report = page.getByRole("dialog", { name: "imported “Canon”" });
  await expect(report).toBeVisible({ timeout: 30_000 });
  await expect(report).toContainText("from Bitwig Studio 5.1.6 (dawproject)");
  await expect(report).toContainText("3 tracks");
  // Bitwig's own Sampler has no Ethereal equivalent: kept as a missing device to replace.
  await expect(report.getByRole("region", { name: "needs resolving" })).toContainText("Sampler");
  await report.getByRole("button", { name: "done" }).click();
  await resolveLater(page);
  await expect(report).toBeHidden();
  await expect(projectScreen(page)).toBeHidden();

  await expect(page.getByTestId("project-name")).toHaveText("Canon");
  const p = await project(page);
  const names = Object.values(p.tracks).map((t) => t.name);
  expect(names).toEqual(expect.arrayContaining(["Acoustic Piano (Nektar)", "canon_guitar_150bpm", "FX 1"]));
  expect(Object.values(p.notes).length).toBeGreaterThan(10);
  const media = Object.values(p.media);
  expect(media).toHaveLength(1);
  expect(media[0]!.frames).toBeGreaterThan(0);
  expect(Object.values(p.warp_markers)).toHaveLength(2);
  expect(bpms(p)).toEqual([150]);
});

test("drop a REAPER project on the window", async ({ page }) => {
  await launch(page, "Before");
  await dropFile(page, "Session.rpp", "text/plain", fixtures.rpp);

  // A REAPER project has no title: the new project is named after the file.
  const report = page.getByRole("dialog", { name: "imported “Session”" });
  await expect(report).toBeVisible({ timeout: 30_000 });
  await expect(report).toContainText("from REAPER");
  // The audio file lives next to the project on the user's disk: kept as a missing reference.
  await expect(report).toContainText("audio file not found");
  await report.getByRole("button", { name: "done" }).click();
  await resolveLater(page);
  await expect(page.getByTestId("project-name")).toHaveText("Session");
  const p = await project(page);
  expect(Object.values(p.tracks).map((t) => t.name)).toEqual(expect.arrayContaining(["Band", "Keys", "Guitar", "Verb"]));
  expect(Object.values(p.clips).length).toBeGreaterThanOrEqual(2);
  expect(bpms(p)).toEqual([96]);
});

test("drop a DAWproject on an open project", async ({ page }) => {
  await launch(page, "Before");
  await dropFile(page, "Canon.dawproject", "application/zip", fixtures.canon);
  const report = page.getByRole("dialog", { name: "imported “Canon”" });
  await expect(report).toBeVisible({ timeout: 30_000 });
  await expect(report).toContainText("3 tracks");
  await report.getByRole("button", { name: "done" }).click();
  await resolveLater(page);
  await expect(page.getByTestId("project-name")).toHaveText("Canon");
  const p = await project(page);
  expect(Object.values(p.tracks).map((t) => t.name)).toEqual(expect.arrayContaining(["Acoustic Piano (Nektar)", "FX 1"]));
  expect(bpms(p)).toEqual([150]);
  // The project file was imported as a project, not as audio on the previous project.
  expect(Object.values(p.media)).toHaveLength(1);
});

test("import a REAPER project from the command palette", async ({ page }) => {
  await launch(page, "Before");
  await page.getByRole("button", { name: "command palette" }).click();
  await page.getByRole("combobox", { name: "search commands" }).fill("Import from another DAW");
  const chooser = page.waitForEvent("filechooser");
  await page
    .getByRole("listbox", { name: "commands" })
    .getByRole("option", { name: /import from another daw/ })
    .click();
  await (await chooser).setFiles([{ name: "Session.rpp", mimeType: "text/plain", buffer: fixtures.rpp }]);
  const report = page.getByRole("dialog", { name: "imported “Session”" });
  await expect(report).toBeVisible({ timeout: 30_000 });
  await expect(report).toContainText("from REAPER");
  await report.getByRole("button", { name: "done" }).click();
  await resolveLater(page);
  await expect(page.getByTestId("project-name")).toHaveText("Session");
  const p = await project(page);
  expect(Object.values(p.tracks).map((t) => t.name)).toEqual(expect.arrayContaining(["Band", "Keys", "Guitar", "Verb"]));
  expect(Object.values(p.notes)).toHaveLength(2);
  expect(Object.values(p.sends)).toHaveLength(1);
  expect(bpms(p)).toEqual([96]);
});

test("a file that is not a project fails cleanly", async ({ page }) => {
  await boot(page);
  const chooser = page.waitForEvent("filechooser");
  await projectScreen(page).getByRole("button", { name: "import from another DAW…" }).click();
  await (await chooser).setFiles([{ name: "Broken.dawproject", mimeType: "application/zip", buffer: Buffer.from("not a zip at all") }]);
  const dialog = page.getByRole("dialog", { name: "Couldn’t import “Broken.dawproject”" });
  await expect(dialog).toBeVisible({ timeout: 20_000 });
  await expect(dialog).toContainText("not a supported project file");
  await dialog.getByRole("button", { name: "close" }).click();
  await expect(projectScreen(page)).toBeVisible();
});
